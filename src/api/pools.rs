// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Pools: `POST /tournaments/:slug/series/:series/draw`,
//! `GET /tournaments/:slug/series/:series/pools` and
//! `POST /tournaments/:slug/series/:series/pools/:pool/teams`.
//!
//! The organizer draws the pools of a series while the tournament is `pools`, when its
//! registered teams are fixed, and draws them again or moves a team until the first result of
//! the series. Reads are public, but the pools of a draft tournament answer 404 without the
//! organizer token, like the tournament. A pool shows the public view of its teams to every
//! caller: numbers, seeds, player names and clubs, never a licence or a contact. Checks come in
//! this order: 401, then 404, then the body (422), then the rules of `domain`: the status (409
//! `not-in-pools`), the rules (422), then the results of the series (409 `series-has-results`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use worker::{Request, Response, RouteContext};

use super::auth;
use super::problem::{ApiResult, Problem, ProblemType};
use super::series::find as find_series;
use super::tournaments::{stored, visible};
use super::{
    ATTEMPTS, database, from_value, json, pool_param, read_value, refusal, series_param, slug_param,
};
use crate::db;
use crate::db::pools::Drawn;
use crate::domain::draw::{self, DrawRequest, DrawSize};
use crate::domain::matches::Match;
use crate::domain::player::Player;
use crate::domain::pool::Pool as DomainPool;

/// A player as everyone sees it: no licence and no contact, whoever calls.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct PublicPlayer {
    #[cfg_attr(test, schema(example = "Ane"))]
    first_name: String,
    #[cfg_attr(test, schema(example = "Exemple"))]
    last_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, schema(nullable = false, example = "Club Exemple de Pelote"))]
    club: Option<String>,
}

/// The public view of the players of a team.
fn public_players(number: i64, players: &HashMap<i64, Vec<Player>>) -> Vec<PublicPlayer> {
    players.get(&number).map_or_else(Vec::new, |players| {
        players
            .iter()
            .map(|player| PublicPlayer {
                first_name: player.first_name.clone(),
                last_name: player.last_name.clone(),
                club: player.club.clone(),
            })
            .collect()
    })
}

/// A team as the pools, matches and standings show it, to every caller: its number and the
/// names and clubs of its players.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct TeamSummary {
    #[cfg_attr(test, schema(minimum = 100, maximum = 9999, example = 101))]
    number: i64,
    /// In the organizer's order.
    #[cfg_attr(test, schema(min_items = 2, max_items = 2))]
    players: Vec<PublicPlayer>,
}

/// The public view of a team.
pub(super) fn summary(number: i64, players: &HashMap<i64, Vec<Player>>) -> TeamSummary {
    TeamSummary {
        number,
        players: public_players(number, players),
    }
}

/// A team of a pool.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct PoolTeam {
    /// Its place in the seed order of the draw, from 1. It keeps it when it moves.
    #[cfg_attr(test, schema(minimum = 1, example = 1))]
    seed: i64,
    #[cfg_attr(test, schema(minimum = 100, maximum = 9999, example = 101))]
    number: i64,
    /// In the organizer's order.
    #[cfg_attr(test, schema(min_items = 2, max_items = 2))]
    players: Vec<PublicPlayer>,
}

/// A pool of a series.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct Pool {
    /// The public key within the series: A, B, C… Pool A is Frontis's `pool` 1.
    #[cfg_attr(test, schema(pattern = "^[A-Z]$", example = "A"))]
    name: String,
    /// In seed order.
    teams: Vec<PoolTeam>,
}

/// `GET /tournaments/:slug/series/:series/pools`.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct PoolList {
    /// By name. Empty until the draw.
    pools: Vec<Pool>,
}

fn pool_view(pool: &DomainPool, players: &HashMap<i64, Vec<Player>>) -> Pool {
    Pool {
        name: pool.name.to_string(),
        teams: pool
            .teams
            .iter()
            .map(|team| PoolTeam {
                seed: team.seed,
                number: team.number,
                players: public_players(team.number, players),
            })
            .collect(),
    }
}

fn list_view(drawn: &Drawn) -> PoolList {
    PoolList {
        pools: drawn
            .pools
            .pools
            .iter()
            .map(|pool| pool_view(pool, &drawn.players))
            .collect(),
    }
}

/// List the pools of a series with their teams, by name. The pools of a draft only with the
/// organizer token.
#[cfg_attr(test, utoipa::path(
    get,
    path = "/api/v1/tournaments/{slug}/series/{series}/pools",
    operation_id = "listPools",
    tag = "pools",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("series" = String, Path, description = "Series slug"),
    ),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The pools of the series, empty until the draw", body = PoolList),
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
    json(200, &list_view(&db::pools::load(&db, &slug, &key).await?))
}

/// `POST /tournaments/:slug/series/:series/draw`.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Draw {
    /// The largest pool: 4, 5 or 6 teams, in as few pools as it allows. Give `pool_size` or
    /// `pools`, not both.
    #[cfg_attr(test, schema(nullable = false, minimum = 4, maximum = 6, example = 4))]
    pool_size: Option<i64>,
    /// The number of pools. Every pool holds 4 to 6 teams, and pools differ by one team at most.
    #[cfg_attr(test, schema(nullable = false, minimum = 1, maximum = 26))]
    pools: Option<i64>,
    /// The team numbers of the first seeds, in order: the other registered teams follow by
    /// number.
    #[cfg_attr(test, schema(nullable = false, example = json!([105, 101])))]
    seeds: Option<Vec<i64>>,
}

/// The draw of a body: 422 unless it gives exactly one of `pool_size` and `pools`.
fn draw_request(value: Value) -> ApiResult<DrawRequest> {
    let request: Draw = from_value(value)?;
    let size = match (request.pool_size, request.pools) {
        (Some(size), None) => DrawSize::PoolSize(size),
        (None, Some(pools)) => DrawSize::Pools(pools),
        _ => {
            return Err(Problem::invalid(
                "pool_size, pools: give exactly one of them",
            ));
        }
    };
    Ok(DrawRequest {
        size,
        seeds: request.seeds.unwrap_or_default(),
    })
}

/// Draw the pools of a series from its registered teams, in snake order of their seeds and away
/// from the teams of their club when the draw can, and generate their matches (organizer). The
/// tournament is `pools`. A series is drawn again while it has no result: the new draw replaces
/// the pools, their matches and their tiebreak.
#[cfg_attr(test, utoipa::path(
    post,
    path = "/api/v1/tournaments/{slug}/series/{series}/draw",
    operation_id = "drawPools",
    tag = "pools",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("series" = String, Path, description = "Series slug"),
    ),
    request_body = Draw,
    security(("bearer" = [])),
    responses(
        (status = 201, description = "The pools of the draw", body = PoolList,
            headers(("Location" = String, description = "/api/v1/tournaments/{slug}/series/{series}/pools"))),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
        (status = 409, response = super::problem::responses::PoolsConflict),
        (status = 422, response = super::problem::responses::Unprocessable),
    )
))]
pub(super) async fn draw(mut req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    auth::organizer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    let mut tournament = stored(&db, &slug).await?;
    let key = series_param(&ctx)?;
    find_series(&db, &slug, &key).await?;
    let request = draw_request(read_value(&mut req).await?)?;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tournament = stored(&db, &slug).await?;
        }
        let entrants = db::teams::entrants(&db, &slug, &key).await?;
        let results = db::pools::load(&db, &slug, &key).await?.pools.results();
        let pools = draw::draw(tournament.status, &request, entrants, results)?;
        let games: Vec<Match> = pools.iter().flat_map(DomainPool::matches).collect();
        if db::pools::draw(&db, &slug, &key, &pools, &games).await? {
            let mut response = json(201, &list_view(&db::pools::load(&db, &slug, &key).await?))?;
            response.headers_mut().set(
                "Location",
                &format!("/api/v1/tournaments/{slug}/series/{key}/pools"),
            )?;
            return Ok(response);
        }
    }
    Err(refusal(Ok(())))
}

/// `POST /tournaments/:slug/series/:series/pools/:pool/teams`.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct MoveTeam {
    /// The number of a team in another pool of the series.
    #[cfg_attr(test, schema(minimum = 100, maximum = 9999, example = 103))]
    number: i64,
}

/// Move a team of the series into this pool (organizer), while the tournament is `pools` and
/// until the first result of the series. The matches of both pools are generated again, and
/// their tiebreak goes. A move leaves 3 to 7 teams in each pool. A team already in the pool
/// stays: nothing changes.
#[cfg_attr(test, utoipa::path(
    post,
    path = "/api/v1/tournaments/{slug}/series/{series}/pools/{pool}/teams",
    operation_id = "moveTeam",
    tag = "pools",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("series" = String, Path, description = "Series slug"),
        ("pool" = String, Path, description = "Pool name: A, B, C…"),
    ),
    request_body = MoveTeam,
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The pools of the series after the move", body = PoolList),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
        (status = 409, response = super::problem::responses::PoolsConflict),
        (status = 422, response = super::problem::responses::Unprocessable),
    )
))]
pub(super) async fn move_team(mut req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    auth::organizer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    let mut tournament = stored(&db, &slug).await?;
    let key = series_param(&ctx)?;
    find_series(&db, &slug, &key).await?;
    let pool = pool_param(&ctx)?;
    let mut drawn = db::pools::load(&db, &slug, &key).await?;
    if drawn.pools.pool(pool).is_none() {
        return Err(Problem::new(ProblemType::NotFound));
    }
    let request: MoveTeam = from_value(read_value(&mut req).await?)?;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tournament = stored(&db, &slug).await?;
            drawn = db::pools::load(&db, &slug, &key).await?;
        }
        let Some(moved) = drawn
            .pools
            .move_team(tournament.status, request.number, pool)?
        else {
            return json(200, &list_view(&drawn));
        };
        if db::pools::move_team(&db, &slug, &key, drawn.pools.revision, &moved).await? {
            return json(200, &list_view(&db::pools::load(&db, &slug, &key).await?));
        }
    }
    Err(refusal(Ok(())))
}
