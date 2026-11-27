// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Matches: `GET /tournaments/:slug/series/:series/matches`,
//! `GET /tournaments/:slug/series/:series/pools/:pool/matches/:match` and
//! `PUT /tournaments/:slug/series/:series/pools/:pool/matches/:match/result`.
//!
//! The draw generates the matches of each pool. The organizer records a result while the
//! tournament is `pools`, and records it again to correct it. Reads are public, but the matches
//! of a draft tournament answer 404 without the organizer token, like the tournament. A match
//! shows the public view of its teams to every caller. Checks come in this order: 401, then 404,
//! then the body (422), then the rules of `domain`: the status (409 `not-in-pools`), then the
//! teams and the score (422).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use worker::{Request, Response, RouteContext};

use super::auth;
use super::pools::{TeamSummary, summary};
use super::problem::{ApiResult, Problem, ProblemType};
use super::series::find as find_series;
use super::tournaments::{stored, visible};
use super::{
    ATTEMPTS, database, from_value, json, match_param, pool_param, read_value, refusal,
    series_param, slug_param,
};
use crate::db;
use crate::db::pools::Drawn;
use crate::domain::matches::{Match as DomainMatch, MatchState as DomainState};
use crate::domain::player::Player;
use crate::domain::pool::PoolName;
use crate::domain::score::{self, Entered, ResultEntry};

/// `phase`: the part of the tournament a match belongs to, as Frontis's `phase`. The pools play
/// `pool`; the final phase (#10) adds values, so clients read the list as open.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub(super) struct MatchPhase(&'static str);

#[cfg(test)]
impl utoipa::PartialSchema for MatchPhase {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        extensible_enum(
            "The part of the tournament a match belongs to. The list grows: a client shows the \
             values it knows and falls back on the raw value for the others.",
            &["pool"],
        )
    }
}

#[cfg(test)]
impl utoipa::ToSchema for MatchPhase {}

/// `state`: where a match stands. The calendar (#9) sets `postponed`; clients read the list as
/// open.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub(super) struct MatchState(&'static str);

#[cfg(test)]
impl utoipa::PartialSchema for MatchState {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        extensible_enum(
            "Where a match stands: scheduled, played, forfeit or postponed. The list grows: a \
             client shows the values it knows and falls back on the raw value for the others.",
            &["scheduled", "played", "forfeit", "postponed"],
        )
    }
}

#[cfg(test)]
impl utoipa::ToSchema for MatchState {}

/// A string schema listing `values` as an `x-extensible-enum`, which oasdiff and clients read as
/// open: a value added later is not a breaking change, unlike a closed `enum`.
#[cfg(test)]
fn extensible_enum(
    description: &str,
    values: &[&str],
) -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
    utoipa::openapi::ObjectBuilder::new()
        .schema_type(utoipa::openapi::schema::Type::String)
        .description(Some(description))
        .extensions(Some(
            utoipa::openapi::extensions::ExtensionsBuilder::new()
                .add("x-extensible-enum", values.to_vec())
                .build(),
        ))
        .into()
}

/// A match of a pool.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct Match {
    /// `pool` for the matches of the pools.
    phase: MatchPhase,
    /// Its pool: A, B, C…
    #[cfg_attr(test, schema(pattern = "^[A-Z]$", example = "A"))]
    pool: String,
    /// The public key within the pool, from 1, in the order of the rounds.
    #[cfg_attr(test, schema(minimum = 1, example = 1))]
    number: i64,
    /// The round of the round robin, from 1. In a pool with an odd number of teams, one team
    /// rests each round.
    #[cfg_attr(test, schema(minimum = 1, example = 1))]
    round: i64,
    /// The two teams, the better seed first.
    #[cfg_attr(test, schema(min_items = 2, max_items = 2))]
    teams: Vec<TeamSummary>,
    state: MatchState,
    /// Once played or forfeit: the scores of `teams`, in their order. A forfeit gives the target
    /// score of the series to the other team and 0 to the team that forfeited.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, schema(nullable = false, min_items = 2, max_items = 2, example = json!([40, 31])))]
    score: Option<Vec<i64>>,
    /// The number of the team that forfeited.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, schema(nullable = false))]
    forfeited_by: Option<i64>,
}

fn view(game: &DomainMatch, players: &HashMap<i64, Vec<Player>>) -> Match {
    Match {
        phase: MatchPhase(game.phase.as_str()),
        pool: game.pool.to_string(),
        number: game.number,
        round: game.round,
        teams: game
            .teams
            .iter()
            .map(|&number| summary(number, players))
            .collect(),
        state: MatchState(game.state.as_str()),
        score: game.state.score().map(Vec::from),
        forfeited_by: game.state.forfeit().map(|side| game.teams[side.index()]),
    }
}

/// `GET /tournaments/:slug/series/:series/matches`.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct MatchList {
    /// By pool, then number. Empty until the draw.
    matches: Vec<Match>,
}

/// List the matches of the pools of a series, by pool then number. The matches of a draft only
/// with the organizer token.
#[cfg_attr(test, utoipa::path(
    get,
    path = "/api/v1/tournaments/{slug}/series/{series}/matches",
    operation_id = "listMatches",
    tag = "matches",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("series" = String, Path, description = "Series slug"),
    ),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The matches of the series, empty until the draw", body = MatchList),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
    )
))]
pub(super) async fn list(req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    let viewer = auth::viewer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    visible(&db, &slug, viewer).await?;
    let key = series_param(&ctx)?;
    find_series(&db, &slug, &key).await?;
    let drawn = db::pools::load(&db, &slug, &key).await?;
    json(
        200,
        &MatchList {
            matches: drawn
                .pools
                .matches
                .iter()
                .map(|game| view(game, &drawn.players))
                .collect(),
        },
    )
}

/// The match of a number in a pool: 404 when the series has none.
fn find(drawn: &Drawn, pool: PoolName, number: i64) -> ApiResult<&DomainMatch> {
    drawn
        .pools
        .find_match(pool, number)
        .ok_or_else(|| Problem::new(ProblemType::NotFound))
}

/// A match: 404 when the pool has no match with this number, or when the tournament is a draft
/// and the caller has no organizer token.
#[cfg_attr(test, utoipa::path(
    get,
    path = "/api/v1/tournaments/{slug}/series/{series}/pools/{pool}/matches/{match}",
    operation_id = "getMatch",
    tag = "matches",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("series" = String, Path, description = "Series slug"),
        ("pool" = String, Path, description = "Pool name: A, B, C…"),
        ("match" = i64, Path, description = "Match number within the pool"),
    ),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The match", body = Match),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
    )
))]
pub(super) async fn get(req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    let viewer = auth::viewer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    visible(&db, &slug, viewer).await?;
    let key = series_param(&ctx)?;
    find_series(&db, &slug, &key).await?;
    let pool = pool_param(&ctx)?;
    let number = match_param(&ctx)?;
    let drawn = db::pools::load(&db, &slug, &key).await?;
    json(200, &view(find(&drawn, pool, number)?, &drawn.players))
}

/// `PUT /tournaments/:slug/series/:series/pools/:pool/matches/:match/result`: the two teams of
/// the match, and either their score or the team that forfeits.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct MatchResult {
    /// The numbers of the two teams of the match, in either order: the score follows this order.
    /// A result never lands on another pair.
    #[cfg_attr(test, schema(min_items = 2, max_items = 2, example = json!([101, 104])))]
    teams: Vec<i64>,
    /// The scores of `teams`, in their order: one team reaches exactly the target score of the
    /// series, and the other stays below it. Give `score` or `forfeited_by`, not both.
    #[cfg_attr(test, schema(nullable = false, min_items = 2, max_items = 2, example = json!([40, 31])))]
    score: Option<Vec<i64>>,
    /// The number of the team that forfeits: the other team gets the target score, and it gets
    /// 0, which counts in the goal average.
    #[cfg_attr(test, schema(nullable = false))]
    forfeited_by: Option<i64>,
}

/// The result of a body: 422 unless it names two teams and gives exactly one of `score` and
/// `forfeited_by`, a score being two numbers.
fn entry(value: Value) -> ApiResult<ResultEntry> {
    let result: MatchResult = from_value(value)?;
    let teams: [i64; 2] = result
        .teams
        .try_into()
        .map_err(|_| Problem::invalid("teams: holds the numbers of the two teams of the match"))?;
    let entered = match (result.score, result.forfeited_by) {
        (Some(score), None) => Entered::Score(score.try_into().map_err(|_| {
            Problem::invalid("score: holds the scores of the two teams, in the order of teams")
        })?),
        (None, Some(team)) => Entered::ForfeitedBy(team),
        _ => {
            return Err(Problem::invalid(
                "score, forfeited_by: give exactly one of them",
            ));
        }
    };
    Ok(ResultEntry { teams, entered })
}

/// Record the result of a match, or record it again to correct it (organizer), while the
/// tournament is `pools`. A forfeit records the target score of the series to 0 for the other
/// team. Standings follow at once: they are computed on every read.
#[cfg_attr(test, utoipa::path(
    put,
    path = "/api/v1/tournaments/{slug}/series/{series}/pools/{pool}/matches/{match}/result",
    operation_id = "recordResult",
    tag = "matches",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("series" = String, Path, description = "Series slug"),
        ("pool" = String, Path, description = "Pool name: A, B, C…"),
        ("match" = i64, Path, description = "Match number within the pool"),
    ),
    request_body = MatchResult,
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The match with its result", body = Match),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
        (status = 409, response = super::problem::responses::PoolsConflict),
        (status = 422, response = super::problem::responses::Unprocessable),
    )
))]
pub(super) async fn record(mut req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    auth::organizer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    let mut tournament = stored(&db, &slug).await?;
    let key = series_param(&ctx)?;
    let series = find_series(&db, &slug, &key).await?;
    let pool = pool_param(&ctx)?;
    let number = match_param(&ctx)?;
    let mut drawn = db::pools::load(&db, &slug, &key).await?;
    find(&drawn, pool, number)?;
    let entry = entry(read_value(&mut req).await?)?;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tournament = stored(&db, &slug).await?;
            drawn = db::pools::load(&db, &slug, &key).await?;
        }
        let game = find(&drawn, pool, number)?;
        let target = series.target_score(&tournament).ok_or_else(|| {
            Problem::from(worker::Error::RustError(
                "a series plays a specialty that its tournament does not declare".to_owned(),
            ))
        })?;
        let state: DomainState = score::record(tournament.status, game, entry, target)?;
        if db::matches::record(&db, &slug, &key, game, state).await? {
            let after = DomainMatch {
                state,
                ..game.clone()
            };
            return json(200, &view(&after, &drawn.players));
        }
    }
    Err(refusal(Ok(())))
}
