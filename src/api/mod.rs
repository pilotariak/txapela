// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The REST API, one module per resource. Every route lives under `/api/v1/`.
//!
//! Each resource module holds its handlers, its request and response types and their
//! `#[utoipa::path]` annotations, which `openapi.rs` (compiled for tests only) gathers into
//! `openapi.json`. A handler returns an [`ApiResult`]: its `Err` is a problem (`problem.rs`), sent
//! as `application/problem+json`. Handlers read D1 through `db` and apply the rules of `domain`;
//! a rule that needs the time gets it from the Worker's clock, passed in.

mod auth;
pub(crate) mod cors;
mod health;
mod matches;
#[cfg(test)]
mod openapi;
mod pools;
pub(crate) mod problem;
mod series;
mod standings;
mod teams;
mod tournaments;

use core::future::Future;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use worker::{D1Database, Env, Response, RouteContext, Router};

use crate::domain::pool::PoolName;
use crate::domain::slug::Slug;
use crate::domain::team::parse_number;
use cors::Cors;
use problem::{ApiResult, PROBLEM_JSON, Problem, ProblemType};

/// Prefixes a path with `/api/v1`, the only place the API version appears. A breaking change goes
/// to `/api/v2/` while installed app versions keep calling `/api/v1/`; the CORS of `cors.rs`
/// covers every version.
macro_rules! v1 {
    ($path:literal) => {
        concat!("/api/v1", $path)
    };
}

/// The largest request body: a tournament is a few kilobytes at most.
const MAX_BODY_BYTES: usize = 16 * 1024;

/// How many times a write of the pools reads the state D1 holds, checks it with the rules of
/// `domain`, and runs its guarded write. D1 refuses the write when another request changed the
/// pools in between, such as a move of another team: the next attempt reads the new state, and
/// answers the problem of a rule it now breaks. After the last refusal, [`refusal`] answers 500.
const ATTEMPTS: usize = 3;

/// Sends the problem of a failed handler.
async fn run(handler: impl Future<Output = ApiResult<Response>>) -> worker::Result<Response> {
    match handler.await {
        Ok(response) => Ok(response),
        Err(problem) => problem.into_response(),
    }
}

/// Registers the API routes on the router.
pub(crate) fn routes(router: Router<'_, ()>) -> Router<'_, ()> {
    router
        .get(v1!("/health"), |_, _| health::get())
        .get_async(v1!("/tournaments"), |req, ctx| {
            run(tournaments::list(req, ctx))
        })
        .post_async(v1!("/tournaments"), |req, ctx| {
            run(tournaments::create(req, ctx))
        })
        .get_async(v1!("/tournaments/:slug"), |req, ctx| {
            run(tournaments::get(req, ctx))
        })
        .patch_async(v1!("/tournaments/:slug"), |req, ctx| {
            run(tournaments::update(req, ctx))
        })
        .delete_async(v1!("/tournaments/:slug"), |req, ctx| {
            run(tournaments::delete(req, ctx))
        })
        .get_async(v1!("/tournaments/:slug/series"), |req, ctx| {
            run(series::list(req, ctx))
        })
        .post_async(v1!("/tournaments/:slug/series"), |req, ctx| {
            run(series::create(req, ctx))
        })
        .get_async(v1!("/tournaments/:slug/series/:series"), |req, ctx| {
            run(series::get(req, ctx))
        })
        .patch_async(v1!("/tournaments/:slug/series/:series"), |req, ctx| {
            run(series::update(req, ctx))
        })
        .delete_async(v1!("/tournaments/:slug/series/:series"), |req, ctx| {
            run(series::delete(req, ctx))
        })
        .get_async(v1!("/tournaments/:slug/teams"), |req, ctx| {
            run(teams::list(req, ctx))
        })
        .post_async(v1!("/tournaments/:slug/teams"), |req, ctx| {
            run(teams::create(req, ctx))
        })
        .get_async(v1!("/tournaments/:slug/teams/:number"), |req, ctx| {
            run(teams::get(req, ctx))
        })
        .patch_async(v1!("/tournaments/:slug/teams/:number"), |req, ctx| {
            run(teams::update(req, ctx))
        })
        .delete_async(v1!("/tournaments/:slug/teams/:number"), |req, ctx| {
            run(teams::delete(req, ctx))
        })
        .post_async(v1!("/tournaments/:slug/series/:series/draw"), |req, ctx| {
            run(pools::draw(req, ctx))
        })
        .get_async(
            v1!("/tournaments/:slug/series/:series/pools"),
            |req, ctx| run(pools::list(req, ctx)),
        )
        .post_async(
            v1!("/tournaments/:slug/series/:series/pools/:pool/teams"),
            |req, ctx| run(pools::move_team(req, ctx)),
        )
        .put_async(
            v1!("/tournaments/:slug/series/:series/pools/:pool/tiebreak"),
            |req, ctx| run(standings::tiebreak(req, ctx)),
        )
        .get_async(
            v1!("/tournaments/:slug/series/:series/matches"),
            |req, ctx| run(matches::list(req, ctx)),
        )
        .get_async(
            v1!("/tournaments/:slug/series/:series/pools/:pool/matches/:match"),
            |req, ctx| run(matches::get(req, ctx)),
        )
        .put_async(
            v1!("/tournaments/:slug/series/:series/pools/:pool/matches/:match/result"),
            |req, ctx| run(matches::record(req, ctx)),
        )
        .get_async(
            v1!("/tournaments/:slug/series/:series/standings"),
            |req, ctx| run(standings::get(req, ctx)),
        )
}

/// What every `/api/*` response needs, after the router: no caching, a problem instead of the
/// router's plain-text 404 and 405, and the CORS headers, added last so that these problems carry
/// them too.
pub(crate) fn finish(response: Response, cors: &Cors) -> worker::Result<Response> {
    let status = response.status_code();
    let plain = !response
        .headers()
        .get("Content-Type")?
        .is_some_and(|content_type| content_type.starts_with(PROBLEM_JSON));
    let mut response = match status {
        404 if plain => Problem::new(ProblemType::NotFound).into_response()?,
        405 if plain => Problem::new(ProblemType::MethodNotAllowed).into_response()?,
        _ => response,
    };
    response
        .headers_mut()
        .set("Cache-Control", "private, no-store")?;
    cors.apply(&mut response)?;
    Ok(response)
}

/// The D1 binding.
fn database(env: &Env) -> ApiResult<D1Database> {
    Ok(env.d1("DB")?)
}

/// The `:slug` of a path: 404 when it cannot be a slug, as for a slug that no tournament has.
fn slug_param(ctx: &RouteContext<()>) -> ApiResult<Slug> {
    ctx.param("slug")
        .and_then(|value| Slug::parse(value))
        .ok_or_else(|| Problem::new(ProblemType::NotFound))
}

/// The `:series` of a path, the slug of a series: 404 when it cannot be a slug, as for a slug
/// that no series of the tournament has.
fn series_param(ctx: &RouteContext<()>) -> ApiResult<Slug> {
    ctx.param("series")
        .and_then(|value| Slug::parse(value))
        .ok_or_else(|| Problem::new(ProblemType::NotFound))
}

/// The `:number` of a path, a team number: 404 when it cannot be one, as for a number that no
/// team of the tournament has.
fn number_param(ctx: &RouteContext<()>) -> ApiResult<i64> {
    ctx.param("number")
        .and_then(|value| parse_number(value))
        .ok_or_else(|| Problem::new(ProblemType::NotFound))
}

/// The `:pool` of a path, the name of a pool: 404 when it cannot be one, as for a pool that the
/// series does not have.
fn pool_param(ctx: &RouteContext<()>) -> ApiResult<PoolName> {
    ctx.param("pool")
        .and_then(|value| PoolName::parse(value))
        .ok_or_else(|| Problem::new(ProblemType::NotFound))
}

/// The `:match` of a path, the number of a match within its pool: 404 when it cannot be one, as
/// for a number that no match of the pool has.
fn match_param(ctx: &RouteContext<()>) -> ApiResult<i64> {
    ctx.param("match")
        .and_then(|value| parse_number(value))
        .ok_or_else(|| Problem::new(ProblemType::NotFound))
}

/// The JSON body of a request: 422 when it is too large, not JSON, or holds a `null`. No field
/// of the API takes `null`: an absent field keeps its value or takes its default.
async fn read_value(req: &mut worker::Request) -> ApiResult<Value> {
    let text = req.text().await?;
    if text.len() > MAX_BODY_BYTES {
        return Err(Problem::invalid(format!(
            "the body is larger than {MAX_BODY_BYTES} bytes"
        )));
    }
    let value =
        serde_json::from_str(&text).map_err(|_| Problem::invalid("the body is not JSON"))?;
    reject_nulls(&value, "")?;
    Ok(value)
}

/// 422 naming the first `null` of a JSON value, such as `venue.name` or `courts[1]`.
fn reject_nulls(value: &Value, path: &str) -> ApiResult<()> {
    match value {
        Value::Null if path.is_empty() => Err(Problem::invalid("the body cannot be null")),
        Value::Null => Err(Problem::invalid(format!("{path} cannot be null"))),
        Value::Object(fields) => fields.iter().try_for_each(|(name, field)| {
            let path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}.{name}")
            };
            reject_nulls(field, &path)
        }),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .try_for_each(|(index, item)| reject_nulls(item, &format!("{path}[{index}]"))),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
    }
}

/// A JSON value as `T`: 422 with what does not fit, without the value it found.
fn from_value<T: DeserializeOwned>(value: Value) -> ApiResult<T> {
    serde_json::from_value(value)
        .map_err(|error| Problem::invalid(without_value(&error.to_string())))
}

/// A serde message without the text of the body it quotes, which can be a player's contact:
/// ``invalid type: integer `5`, expected a string`` becomes `invalid type, expected a string`.
/// An unknown field or variant keeps its name only when it looks like a name of the API, an
/// identifier ([`is_identifier`]), to tell the developer which one is wrong: an email address, a
/// phone number or any other text is dropped, as in `unknown field, expected one of ...`. The
/// names after `expected` come from the code.
fn without_value(message: &str) -> String {
    for kind in ["invalid type", "invalid value"] {
        if let Some(found) = message
            .strip_prefix(kind)
            .and_then(|rest| rest.strip_prefix(": "))
        {
            return match found.rsplit_once(", expected ") {
                Some((_, expected)) => format!("{kind}, expected {expected}"),
                None => kind.to_owned(),
            };
        }
    }
    for kind in ["unknown field", "unknown variant"] {
        if let Some(quoted) = message
            .strip_prefix(kind)
            .and_then(|rest| rest.strip_prefix(" `"))
        {
            // The quoted name ends at the last "`, " that serde writes after it: the name itself
            // may hold backticks, the names of the code after it do not.
            let end = ["`, expected ", "`, there are no "]
                .iter()
                .filter_map(|tail| quoted.rfind(tail))
                .max();
            return match end {
                Some(end) if is_identifier(&quoted[..end]) => message.to_owned(),
                Some(end) => format!("{kind}{}", &quoted[end + 1..]),
                None => kind.to_owned(),
            };
        }
    }
    message.to_owned()
}

/// Whether a name looks like a field or a variant of the API: an ASCII letter, then letters,
/// digits or underscores, 64 characters at most. An email address or a phone number does not.
fn is_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= 64
        && bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The problem of a guarded write that D1 refused (see `db::written`): `recheck` holds the
/// checks of `domain` run again on the state D1 holds now, which name what changed since the
/// first checks. A recheck that passes answers 500: either a third request undid the change in
/// between (a team withdrew and freed the place), which the client can retry, or a guard of `db`
/// is stricter than the rules of `domain`, a bug.
fn refusal(recheck: ApiResult<()>) -> Problem {
    match recheck {
        Err(problem) => problem,
        Ok(()) => Problem::from(worker::Error::RustError(
            "D1 refused a guarded write that the rules, checked again, allow".to_owned(),
        )),
    }
}

/// A JSON response.
fn json<T: Serialize>(status: u16, body: &T) -> ApiResult<Response> {
    Ok(Response::from_json(body)?.with_status(status))
}

/// A 204 response.
fn no_content() -> ApiResult<Response> {
    Ok(Response::empty()?.with_status(204))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{is_identifier, reject_nulls, without_value};

    #[test]
    fn a_null_anywhere_is_named_by_its_path() {
        let detail = |value| {
            reject_nulls(&value, "")
                .err()
                .map(|problem| format!("{problem:?}"))
        };
        assert_eq!(
            detail(json!({ "name": "Open", "courts": [{ "name": "A" }] })),
            None,
            "no null"
        );
        for (value, path) in [
            (json!({ "name": null }), "name cannot be null"),
            (
                json!({ "venue": { "name": null } }),
                "venue.name cannot be null",
            ),
            (
                json!({ "courts": [{ "name": "A" }, null] }),
                "courts[1] cannot be null",
            ),
            (json!(null), "the body cannot be null"),
        ] {
            let found = detail(value).unwrap_or_default();
            assert!(found.contains(path), "{found} names {path}");
        }
    }

    #[test]
    fn serde_messages_lose_the_value_they_quote() {
        for (message, expected) in [
            (
                "invalid type: integer `612345678`, expected a string",
                "invalid type, expected a string",
            ),
            (
                "invalid type: string \"secret, expected x\", expected i64",
                "invalid type, expected i64",
            ),
            (
                "invalid value: integer `-1`, expected u16",
                "invalid value, expected u16",
            ),
            ("invalid type: map", "invalid type"),
            ("missing field `name`", "missing field `name`"),
            (
                "unknown variant `pelote`, expected one of `fronton`",
                "unknown variant `pelote`, expected one of `fronton`",
            ),
            (
                "unknown field `fee_paid_on`, expected one of `number`, `players`",
                "unknown field `fee_paid_on`, expected one of `number`, `players`",
            ),
            (
                "unknown field `ane.exemple@example.org`, expected one of `number`, `players`",
                "unknown field, expected one of `number`, `players`",
            ),
            (
                "unknown variant `+00 0 00 00 00 02`, expected `registered` or `withdrawn`",
                "unknown variant, expected `registered` or `withdrawn`",
            ),
            (
                "unknown variant `0600000000`, expected `registered` or `withdrawn`",
                "unknown variant, expected `registered` or `withdrawn`",
            ),
            (
                "unknown field `a`, expected x`, expected one of `number`",
                "unknown field, expected one of `number`",
            ),
            (
                "unknown field `x@example.org`, there are no fields",
                "unknown field, there are no fields",
            ),
        ] {
            assert_eq!(without_value(message), expected, "{message}");
        }
    }

    #[test]
    fn identifiers_are_names_of_the_api_and_nothing_else() {
        for name in ["level", "fee_paid", "Colour", "main_nue", "x1"] {
            assert!(is_identifier(name), "{name:?} is an identifier");
        }
        let long = "a".repeat(65);
        for name in [
            "",
            "1st",
            "_name",
            "ane.exemple@example.org",
            "+00 0 00 00 00 02",
            "0600000000",
            "LIC-0001",
            "joueur exemple",
            long.as_str(),
        ] {
            assert!(!is_identifier(name), "{name:?} is not an identifier");
        }
    }
}
