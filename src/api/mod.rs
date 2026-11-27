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
#[cfg(test)]
mod openapi;
pub(crate) mod problem;
mod tournaments;

use core::future::Future;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use worker::{D1Database, Env, Response, RouteContext, Router};

use crate::domain::slug::Slug;
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

/// A JSON value as `T`: 422 with what does not fit.
fn from_value<T: DeserializeOwned>(value: Value) -> ApiResult<T> {
    serde_json::from_value(value).map_err(|error| Problem::invalid(error.to_string()))
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

    use super::reject_nulls;

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
}
