// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Teams: `GET|POST /tournaments/:slug/teams` and
//! `GET|PATCH|DELETE /tournaments/:slug/teams/:number`.
//!
//! The organizer registers each team with its two players: online registration by players waits
//! for #5. Reads are public, but the teams of a draft tournament answer 404 without the
//! organizer token, like the tournament. A public response holds the number, series and status
//! of a team, and the names and clubs of its players: never a licence number, a contact, the
//! registration date or whether the fee is paid. Checks come in this order: 401, then 404, then
//! the body (422), then the rules of `domain` (409 `registration-closed`, then 422), then the
//! maximums and the other teams (409).

use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use worker::{D1Database, Request, Response, RouteContext};

use super::auth::{self, Viewer};
use super::problem::{ApiResult, Problem, ProblemType};
use super::tournaments::{stored, visible};
use super::{
    database, from_value, json, no_content, number_param, read_value, refusal, slug_param,
};
use crate::db;
use crate::domain::civil::paris_date;
use crate::domain::player::Player as DomainPlayer;
use crate::domain::slug::Slug;
use crate::domain::team::{self as domain, NewTeam, TeamError, TeamStatus};

/// A player. `licence` and `contact` are personal data: the organizer sends and reads them, and
/// public responses leave them out.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Player {
    #[cfg_attr(test, schema(example = "Ane"))]
    first_name: String,
    #[cfg_attr(test, schema(example = "Exemple"))]
    last_name: String,
    /// Free text.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, schema(nullable = false, example = "Club Exemple de Pelote"))]
    club: Option<String>,
    /// Organizer only: absent from public responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, schema(nullable = false))]
    licence: Option<String>,
    /// A phone number or an email address. Organizer only: absent from public responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, schema(nullable = false, example = "joueur@example.org"))]
    contact: Option<String>,
}

impl From<Player> for DomainPlayer {
    fn from(player: Player) -> Self {
        Self {
            first_name: player.first_name,
            last_name: player.last_name,
            club: player.club,
            licence: player.licence,
            contact: player.contact,
        }
    }
}

/// A player as `viewer` sees it: the public view never reads the licence or the contact.
fn player_view(player: &DomainPlayer, viewer: Viewer) -> Player {
    let (licence, contact) = match viewer {
        Viewer::Organizer => (player.licence.clone(), player.contact.clone()),
        Viewer::Public => (None, None),
    };
    Player {
        first_name: player.first_name.clone(),
        last_name: player.last_name.clone(),
        club: player.club.clone(),
        licence,
        contact,
    }
}

/// A team of a tournament.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct Team {
    /// The public key within the tournament, in the number block of its series. Immutable.
    #[cfg_attr(test, schema(minimum = 100, maximum = 9999, example = 101))]
    number: i64,
    /// The slug of its series. Immutable.
    #[cfg_attr(test, schema(example = "1re-serie-a"))]
    series: String,
    status: TeamStatus,
    /// The two players, in the organizer's order.
    #[cfg_attr(test, schema(min_items = 2, max_items = 2))]
    players: Vec<Player>,
    /// Organizer only: the date in Europe/Paris when the organizer registered the team, set by
    /// txapela.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, schema(nullable = false, format = Date, example = "2027-03-05"))]
    registered_on: Option<String>,
    /// Organizer only.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, schema(nullable = false))]
    fee_paid: Option<bool>,
}

/// A team as `viewer` sees it.
fn view(team: &domain::Team, viewer: Viewer) -> Team {
    let organizer = viewer == Viewer::Organizer;
    Team {
        number: team.number,
        series: team.series.to_string(),
        status: team.status,
        players: team
            .players
            .iter()
            .map(|player| player_view(player, viewer))
            .collect(),
        registered_on: organizer.then(|| team.registered_on.to_string()),
        fee_paid: organizer.then_some(team.fee_paid),
    }
}

/// `GET /tournaments/:slug/teams`.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct TeamList {
    /// By number.
    teams: Vec<Team>,
}

/// List the teams of a tournament, by number. The teams of a draft only with the organizer
/// token, and licence numbers, contacts, registration dates and fees only for the organizer.
#[cfg_attr(test, utoipa::path(
    get,
    path = "/api/v1/tournaments/{slug}/teams",
    operation_id = "listTeams",
    tag = "teams",
    params(("slug" = String, Path, description = "Tournament slug")),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The teams of the tournament", body = TeamList),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
    )
))]
pub(super) async fn list(req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    let viewer = auth::viewer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    visible(&db, &slug, viewer).await?;
    let teams = db::teams::list(&db, &slug).await?;
    json(
        200,
        &TeamList {
            teams: teams.iter().map(|team| view(team, viewer)).collect(),
        },
    )
}

/// `POST /tournaments/:slug/teams`. A new team is registered, on the day of the request.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct CreateTeam {
    /// In the number block of the series, unique within the tournament. Immutable.
    #[cfg_attr(test, schema(minimum = 100, maximum = 9999, example = 101))]
    number: i64,
    /// The slug of a series of the tournament. Immutable.
    #[cfg_attr(test, schema(example = "1re-serie-a"))]
    series: String,
    /// Exactly two.
    #[cfg_attr(test, schema(min_items = 2, max_items = 2))]
    players: Vec<Player>,
    /// Default `false`.
    #[cfg_attr(test, schema(nullable = false))]
    fee_paid: Option<bool>,
}

/// A failure of the Worker's clock, which never happens in practice.
fn clock_error() -> Problem {
    Problem::from(worker::Error::RustError(
        "the Worker's clock is out of range".to_owned(),
    ))
}

/// Today in Europe/Paris, from the Worker's clock: `domain` reads no clock.
fn today() -> ApiResult<Date> {
    let millis = i64::try_from(worker::Date::now().as_millis()).map_err(|_| clock_error())?;
    paris_date(millis).ok_or_else(clock_error)
}

/// The 422 of a series that the tournament does not have.
fn no_such_series() -> Problem {
    Problem::invalid("series: the tournament has no such series")
}

/// The checks of a registration that D1 refused ([`db::teams::create`] answered `false`), run
/// again on the state D1 holds now: another request took the last place, moved the block or
/// the tournament past registration, or deleted the series since the first checks.
async fn recheck_registration(
    db: &D1Database,
    slug: &Slug,
    key: &Slug,
    new: NewTeam,
    today: Date,
) -> ApiResult<()> {
    let tournament = stored(db, slug).await?;
    let series = db::series::find(db, slug, key)
        .await?
        .ok_or_else(no_such_series)?;
    let registered = db::teams::registered(db, slug, key).await?;
    domain::Team::register(new, &tournament, &series, registered, today)?;
    Ok(())
}

/// The checks of a change that D1 refused ([`db::teams::save`] answered `false`), run again on
/// the state D1 holds now: another request took the last place, moved the tournament past
/// registration or deleted the team since the first checks.
async fn recheck_change(
    db: &D1Database,
    slug: &Slug,
    number: i64,
    changes: domain::TeamChanges,
) -> ApiResult<()> {
    let tournament = stored(db, slug).await?;
    let team = db::teams::find(db, slug, number)
        .await?
        .ok_or_else(|| Problem::new(ProblemType::NotFound))?;
    let series = db::series::find(db, slug, &team.series)
        .await?
        .ok_or_else(|| Problem::new(ProblemType::NotFound))?;
    let registered = db::teams::registered(db, slug, &team.series).await?;
    team.change(changes, &tournament, &series, registered)?;
    Ok(())
}

/// The checks of a deletion that D1 refused ([`db::teams::delete`] answered `false`), run again
/// on the state D1 holds now: another request moved the tournament past registration or
/// deleted the team since the first checks.
async fn recheck_delete(db: &D1Database, slug: &Slug, number: i64) -> ApiResult<()> {
    let tournament = stored(db, slug).await?;
    db::teams::find(db, slug, number)
        .await?
        .ok_or_else(|| Problem::new(ProblemType::NotFound))?;
    domain::Team::check_delete(&tournament)?;
    Ok(())
}

/// A number that another team of the tournament has: 409, or the error.
fn taken(error: worker::Error, number: i64) -> Problem {
    if db::is_unique_violation(&error, "teams.tournament_id, teams.number") {
        Problem::detail(
            ProblemType::NumberTaken,
            format!("number: another team of the tournament has the number {number}"),
        )
    } else {
        error.into()
    }
}

/// Register a team in a series, while the tournament is a draft or in registration
/// (organizer). The registration date is the day of the request in Europe/Paris.
#[cfg_attr(test, utoipa::path(
    post,
    path = "/api/v1/tournaments/{slug}/teams",
    operation_id = "createTeam",
    tag = "teams",
    params(("slug" = String, Path, description = "Tournament slug")),
    request_body = CreateTeam,
    security(("bearer" = [])),
    responses(
        (status = 201, description = "Team registered", body = Team,
            headers(("Location" = String, description = "/api/v1/tournaments/{slug}/teams/{number}"))),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
        (status = 409, response = super::problem::responses::SeriesAndTeamsConflict),
        (status = 422, response = super::problem::responses::Unprocessable),
    )
))]
pub(super) async fn create(mut req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    auth::organizer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    let tournament = stored(&db, &slug).await?;
    let request: CreateTeam = from_value(read_value(&mut req).await?)?;
    tournament
        .status
        .check_registration_open()
        .map_err(TeamError::from)?;
    let key = Slug::parse(&request.series).ok_or_else(no_such_series)?;
    let series = db::series::find(&db, &slug, &key)
        .await?
        .ok_or_else(no_such_series)?;
    let registered = db::teams::registered(&db, &slug, &key).await?;
    let new = NewTeam {
        number: request.number,
        players: request
            .players
            .into_iter()
            .map(DomainPlayer::from)
            .collect(),
        fee_paid: request.fee_paid.unwrap_or(false),
    };
    let today = today()?;
    let team = domain::Team::register(new.clone(), &tournament, &series, registered, today)?;
    let written = db::teams::create(&db, &slug, &team)
        .await
        .map_err(|error| taken(error, team.number))?;
    if !written {
        return Err(refusal(
            recheck_registration(&db, &slug, &key, new, today).await,
        ));
    }
    let mut response = json(201, &view(&team, Viewer::Organizer))?;
    response.headers_mut().set(
        "Location",
        &format!("/api/v1/tournaments/{slug}/teams/{}", team.number),
    )?;
    Ok(response)
}

/// A team: 404 when the tournament has no team with this number, or when the tournament is a
/// draft and the caller has no organizer token. Licence numbers, contacts, registration date
/// and fee only for the organizer.
#[cfg_attr(test, utoipa::path(
    get,
    path = "/api/v1/tournaments/{slug}/teams/{number}",
    operation_id = "getTeam",
    tag = "teams",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("number" = i64, Path, description = "Team number"),
    ),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The team", body = Team),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
    )
))]
pub(super) async fn get(req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    let viewer = auth::viewer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    visible(&db, &slug, viewer).await?;
    let number = number_param(&ctx)?;
    let team = db::teams::find(&db, &slug, number)
        .await?
        .ok_or_else(|| Problem::new(ProblemType::NotFound))?;
    json(200, &view(&team, viewer))
}

/// `PATCH /tournaments/:slug/teams/:number`: absent fields keep their value, and no field takes
/// `null`. The number, the series and the registration date never change.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct TeamChanges {
    /// Replaces both players.
    #[cfg_attr(test, schema(nullable = false, min_items = 2, max_items = 2))]
    players: Option<Vec<Player>>,
    #[cfg_attr(test, schema(nullable = false))]
    fee_paid: Option<bool>,
    /// Draft or registration only: a withdrawn team registered again needs a place in its
    /// series and tournament.
    #[cfg_attr(test, schema(nullable = false))]
    status: Option<TeamStatus>,
}

/// The body of a `PATCH`: 422 when it sets a value that never changes.
fn changes(value: Value) -> ApiResult<domain::TeamChanges> {
    for (field, why) in [
        ("number", "it is the public key of the team"),
        (
            "series",
            "delete the team and register it again in the other series",
        ),
        ("registered_on", "txapela sets it when the team registers"),
    ] {
        if value.get(field).is_some() {
            return Err(Problem::invalid(format!("{field} cannot change: {why}")));
        }
    }
    let changes: TeamChanges = from_value(value)?;
    Ok(domain::TeamChanges {
        players: changes
            .players
            .map(|players| players.into_iter().map(DomainPlayer::from).collect()),
        fee_paid: changes.fee_paid,
        status: changes.status,
    })
}

/// Change a team (organizer): its players and fee in every status, its status while the
/// tournament is a draft or in registration. Withdrawing once the pools are drawn is a forfeit,
/// left to the pools (#8).
#[cfg_attr(test, utoipa::path(
    patch,
    path = "/api/v1/tournaments/{slug}/teams/{number}",
    operation_id = "updateTeam",
    tag = "teams",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("number" = i64, Path, description = "Team number"),
    ),
    request_body = TeamChanges,
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The team after the change", body = Team),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
        (status = 409, response = super::problem::responses::SeriesAndTeamsConflict),
        (status = 422, response = super::problem::responses::Unprocessable),
    )
))]
pub(super) async fn update(mut req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    auth::organizer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    let tournament = stored(&db, &slug).await?;
    let number = number_param(&ctx)?;
    let team = db::teams::find(&db, &slug, number)
        .await?
        .ok_or_else(|| Problem::new(ProblemType::NotFound))?;
    let patch = changes(read_value(&mut req).await?)?;
    let series = db::series::find(&db, &slug, &team.series)
        .await?
        .ok_or_else(|| {
            Problem::from(worker::Error::RustError(
                "a team belongs to a series that its tournament does not have".to_owned(),
            ))
        })?;
    let registered = db::teams::registered(&db, &slug, &team.series).await?;
    let after = team.change(patch.clone(), &tournament, &series, registered)?;
    if !db::teams::save(&db, &slug, &team, &after).await? {
        return Err(refusal(recheck_change(&db, &slug, number, patch).await));
    }
    json(200, &view(&after, Viewer::Organizer))
}

/// Delete a team with its players, while the tournament is a draft or in registration
/// (organizer).
#[cfg_attr(test, utoipa::path(
    delete,
    path = "/api/v1/tournaments/{slug}/teams/{number}",
    operation_id = "deleteTeam",
    tag = "teams",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("number" = i64, Path, description = "Team number"),
    ),
    security(("bearer" = [])),
    responses(
        (status = 204, description = "Team deleted, with its players"),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
        (status = 409, response = super::problem::responses::SeriesAndTeamsConflict),
    )
))]
pub(super) async fn delete(req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    auth::organizer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    let tournament = stored(&db, &slug).await?;
    let number = number_param(&ctx)?;
    db::teams::find(&db, &slug, number)
        .await?
        .ok_or_else(|| Problem::new(ProblemType::NotFound))?;
    domain::Team::check_delete(&tournament)?;
    if !db::teams::delete(&db, &slug, number).await? {
        return Err(refusal(recheck_delete(&db, &slug, number).await));
    }
    no_content()
}
