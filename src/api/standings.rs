// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Standings: `GET /tournaments/:slug/series/:series/standings`, and the organizer's tiebreak,
//! `PUT /tournaments/:slug/series/:series/pools/:pool/tiebreak`.
//!
//! Standings are computed from the matches on every read, never stored, so that a corrected
//! score updates them. Reads are public, but the standings of a draft tournament answer 404
//! without the organizer token, like the tournament. A line shows the public view of its team
//! to every caller. Checks of the tiebreak come in this order: 401, then 404, then the body
//! (422), then the status (409 `not-in-pools`), then the list (422).

use serde::{Deserialize, Serialize};
use worker::{Request, Response, RouteContext};

use super::auth;
use super::pools::{TeamSummary, summary};
use super::problem::{ApiResult, Problem, ProblemType};
use super::series::find as find_series;
use super::tournaments::{stored, visible};
use super::{
    ATTEMPTS, database, from_value, json, pool_param, read_value, refusal, series_param, slug_param,
};
use crate::db;
use crate::db::pools::Drawn;
use crate::domain::standings::standings;
use crate::domain::tournament::Points;

/// The line of a team in the standings of its pool.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct Standing {
    /// From 1, by points then goal average. Teams still level share it, unless the organizer's
    /// tiebreak separates them.
    #[cfg_attr(test, schema(minimum = 1, example = 1))]
    rank: i64,
    team: TeamSummary,
    /// Matches with a result, played or forfeit.
    #[cfg_attr(test, schema(minimum = 0))]
    played: i64,
    #[cfg_attr(test, schema(minimum = 0))]
    wins: i64,
    /// Forfeits included.
    #[cfg_attr(test, schema(minimum = 0))]
    losses: i64,
    /// The matches the team forfeited: each gives it the forfeit points of the tournament.
    #[cfg_attr(test, schema(minimum = 0))]
    forfeits: i64,
    #[cfg_attr(test, schema(minimum = 0))]
    points: i64,
    #[cfg_attr(test, schema(minimum = 0))]
    goals_for: i64,
    #[cfg_attr(test, schema(minimum = 0))]
    goals_against: i64,
    /// Goals for minus goals against.
    goal_average: i64,
}

/// The standings of a pool.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct PoolStandings {
    #[cfg_attr(test, schema(pattern = "^[A-Z]$", example = "A"))]
    pool: String,
    /// By rank, then team number among teams that share a rank.
    standings: Vec<Standing>,
}

/// `GET /tournaments/:slug/series/:series/standings`.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct StandingsList {
    /// By pool. Empty until the draw.
    pools: Vec<PoolStandings>,
}

fn view(drawn: &Drawn, points: Points) -> StandingsList {
    StandingsList {
        pools: drawn
            .pools
            .pools
            .iter()
            .map(|pool| PoolStandings {
                pool: pool.name.to_string(),
                standings: standings(pool, &drawn.pools.matches, points)
                    .into_iter()
                    .map(|line| Standing {
                        rank: line.rank,
                        team: summary(line.team, &drawn.players),
                        played: line.played,
                        wins: line.wins,
                        losses: line.losses,
                        forfeits: line.forfeits,
                        points: line.points,
                        goals_for: line.goals_for,
                        goals_against: line.goals_against,
                        goal_average: line.goal_average,
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// The standings of each pool of a series, computed from its matches and the points of the
/// tournament. The standings of a draft only with the organizer token.
#[cfg_attr(test, utoipa::path(
    get,
    path = "/api/v1/tournaments/{slug}/series/{series}/standings",
    operation_id = "getStandings",
    tag = "standings",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("series" = String, Path, description = "Series slug"),
    ),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The standings of the pools of the series", body = StandingsList),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
    )
))]
pub(super) async fn get(req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    let viewer = auth::viewer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    let tournament = visible(&db, &slug, viewer).await?;
    let key = series_param(&ctx)?;
    find_series(&db, &slug, &key).await?;
    let drawn = db::pools::load(&db, &slug, &key).await?;
    json(200, &view(&drawn, tournament.points))
}

/// `PUT /tournaments/:slug/series/:series/pools/:pool/tiebreak`.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Tiebreak {
    /// Every team number of the pool once, in the organizer's order, or none to remove the
    /// order. It separates only the teams level on points and goal average.
    #[cfg_attr(test, schema(example = json!([104, 101, 103, 102])))]
    teams: Vec<i64>,
}

/// Order the teams of a pool by hand (organizer), while the tournament is `pools`: the order
/// separates the teams still level on points and goal average, and only them. A draw or a move
/// of the pool removes it.
#[cfg_attr(test, utoipa::path(
    put,
    path = "/api/v1/tournaments/{slug}/series/{series}/pools/{pool}/tiebreak",
    operation_id = "setTiebreak",
    tag = "standings",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("series" = String, Path, description = "Series slug"),
        ("pool" = String, Path, description = "Pool name: A, B, C…"),
    ),
    request_body = Tiebreak,
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The standings of the pools of the series with the order", body = StandingsList),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
        (status = 409, response = super::problem::responses::PoolsConflict),
        (status = 422, response = super::problem::responses::Unprocessable),
    )
))]
pub(super) async fn tiebreak(mut req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    auth::organizer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    let mut tournament = stored(&db, &slug).await?;
    let key = series_param(&ctx)?;
    find_series(&db, &slug, &key).await?;
    let name = pool_param(&ctx)?;
    let mut drawn = db::pools::load(&db, &slug, &key).await?;
    if drawn.pools.pool(name).is_none() {
        return Err(Problem::new(ProblemType::NotFound));
    }
    let request: Tiebreak = from_value(read_value(&mut req).await?)?;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tournament = stored(&db, &slug).await?;
            drawn = db::pools::load(&db, &slug, &key).await?;
        }
        let pool = drawn
            .pools
            .pool(name)
            .ok_or_else(|| Problem::invalid(format!("pool: the series has no pool {name}")))?;
        let order = pool.tiebreak(tournament.status, &request.teams)?;
        if db::pool_teams::tiebreak(&db, &slug, &key, drawn.pools.revision, name, &order).await? {
            let after = db::pools::load(&db, &slug, &key).await?;
            return json(200, &view(&after, tournament.points));
        }
    }
    Err(refusal(Ok(())))
}
