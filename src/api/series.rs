// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Series: `GET|POST /tournaments/:slug/series` and
//! `GET|PATCH|DELETE /tournaments/:slug/series/:series`.
//!
//! Writes need the organizer token (`auth.rs`). Reads are public, but the series of a draft
//! tournament answer 404 without the organizer token, like the tournament. Checks come in this
//! order: 401, then 404, then the body (422), then the rules of `domain` (409
//! `registration-closed`, then 422), then the other series and the teams (409).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use worker::{D1Database, Request, Response, RouteContext};

use super::auth;
use super::problem::{ApiResult, Problem, ProblemType};
use super::tournaments::{stored, visible};
use super::{
    database, from_value, json, no_content, read_value, refusal, series_param, slug_param,
};
use crate::db;
use crate::domain::series::{self as domain, SeriesChanges as DomainChanges};
use crate::domain::slug::{SLUG_RULE, Slug};
use crate::domain::tournament::{Specialty, Tournament};

/// A series of a tournament.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct Series {
    /// The public key within the tournament, chosen at creation and immutable.
    #[cfg_attr(test, schema(example = "1re-serie-a"))]
    slug: String,
    #[cfg_attr(test, schema(example = "1re Série A"))]
    name: String,
    /// One of the specialties of the tournament.
    specialty: Specialty,
    /// The score a team must reach to win a match: the target score of the specialty in the
    /// tournament.
    #[cfg_attr(test, schema(minimum = 1, example = 40))]
    target_score: i64,
    /// The most registered teams the series takes: withdrawn teams do not count.
    #[cfg_attr(test, schema(minimum = 2, maximum = 100, example = 16))]
    max_teams: i64,
    /// Display order: series are listed by position, then slug.
    #[cfg_attr(test, schema(minimum = 1, example = 1))]
    position: i64,
    /// The first team number of the series: its teams are numbered from `number_block` to
    /// `number_block` + 99, and no other series of the tournament has this block.
    #[cfg_attr(
        test,
        schema(minimum = 100, maximum = 9900, multiple_of = 100, example = 100)
    )]
    number_block: i64,
}

/// The view of a series: its target score comes from the tournament.
fn view(series: &domain::Series, tournament: &Tournament) -> ApiResult<Series> {
    let target_score = series.target_score(tournament).ok_or_else(|| {
        Problem::from(worker::Error::RustError(
            "a series plays a specialty that its tournament does not declare".to_owned(),
        ))
    })?;
    Ok(Series {
        slug: series.slug.to_string(),
        name: series.name.clone(),
        specialty: series.specialty,
        target_score,
        max_teams: series.max_teams,
        position: series.position,
        number_block: series.number_block,
    })
}

/// `GET /tournaments/:slug/series`.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct SeriesList {
    /// By position, then slug.
    series: Vec<Series>,
}

/// List the series of a tournament, by position then slug. The series of a draft only with the
/// organizer token.
#[cfg_attr(test, utoipa::path(
    get,
    path = "/api/v1/tournaments/{slug}/series",
    operation_id = "listSeries",
    tag = "series",
    params(("slug" = String, Path, description = "Tournament slug")),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The series of the tournament", body = SeriesList),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
    )
))]
pub(super) async fn list(req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    let viewer = auth::viewer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    let tournament = visible(&db, &slug, viewer).await?;
    let series = db::series::list(&db, &slug)
        .await?
        .iter()
        .map(|series| view(series, &tournament))
        .collect::<ApiResult<Vec<_>>>()?;
    json(200, &SeriesList { series })
}

/// `POST /tournaments/:slug/series`.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct CreateSeries {
    /// 3 to 64 characters among a-z, 0-9 and single hyphens, starting and ending with a letter
    /// or a digit, unique within the tournament. Immutable.
    #[cfg_attr(
        test,
        schema(
            min_length = 3,
            max_length = 64,
            pattern = "^[a-z0-9]+(-[a-z0-9]+)*$",
            example = "1re-serie-a"
        )
    )]
    slug: String,
    #[cfg_attr(test, schema(example = "1re Série A"))]
    name: String,
    /// A specialty the tournament declares.
    specialty: Specialty,
    #[cfg_attr(test, schema(minimum = 2, maximum = 100))]
    max_teams: i64,
    #[cfg_attr(test, schema(minimum = 1))]
    position: i64,
    /// A multiple of 100 that no other series of the tournament has.
    #[cfg_attr(test, schema(minimum = 100, maximum = 9900, multiple_of = 100))]
    number_block: i64,
}

impl CreateSeries {
    /// The series to create: 422 when the slug is not one.
    fn series(self) -> ApiResult<domain::Series> {
        Ok(domain::Series {
            slug: Slug::parse(&self.slug).ok_or_else(|| Problem::invalid(SLUG_RULE))?,
            name: self.name,
            specialty: self.specialty,
            max_teams: self.max_teams,
            position: self.position,
            number_block: self.number_block,
        })
    }
}

/// A slug or a number block that another series of the tournament has: 409, or the error.
fn taken(error: worker::Error, series: &domain::Series) -> Problem {
    if db::is_unique_violation(&error, "series.tournament_id, series.slug") {
        Problem::detail(
            ProblemType::SlugTaken,
            format!("slug: another series of the tournament is {}", series.slug),
        )
    } else if db::is_unique_violation(&error, "series.tournament_id, series.number_block") {
        Problem::detail(
            ProblemType::BlockTaken,
            format!(
                "number_block: another series of the tournament has the block {}",
                series.number_block
            ),
        )
    } else {
        error.into()
    }
}

/// The series of a slug in a tournament: 404 when it has none.
async fn find(db: &D1Database, slug: &Slug, key: &Slug) -> ApiResult<domain::Series> {
    db::series::find(db, slug, key)
        .await?
        .ok_or_else(|| Problem::new(ProblemType::NotFound))
}

/// The checks of a creation that D1 refused ([`db::series::create`] answered `false`), run again
/// on the state D1 holds now: another request moved the tournament past registration or dropped
/// the specialty since the first checks.
async fn recheck_create(db: &D1Database, slug: &Slug, series: domain::Series) -> ApiResult<()> {
    series.create(&stored(db, slug).await?)?;
    Ok(())
}

/// The checks of a change that D1 refused ([`db::series::save`] answered `false`), run again on
/// the state D1 holds now: another request registered a team, moved the tournament past
/// registration, dropped the specialty or deleted the series since the first checks.
async fn recheck_change(
    db: &D1Database,
    slug: &Slug,
    key: &Slug,
    changes: DomainChanges,
) -> ApiResult<()> {
    let tournament = stored(db, slug).await?;
    let series = find(db, slug, key).await?;
    let teams = db::series::teams(db, slug, key).await?;
    series.change(changes, &tournament, teams)?;
    Ok(())
}

/// The checks of a deletion that D1 refused ([`db::series::delete`] answered `false`), run again
/// on the state D1 holds now: another request registered a team, moved the tournament past
/// registration or deleted the series since the first checks.
async fn recheck_delete(db: &D1Database, slug: &Slug, key: &Slug) -> ApiResult<()> {
    let tournament = stored(db, slug).await?;
    find(db, slug, key).await?;
    let teams = db::series::teams(db, slug, key).await?;
    domain::Series::check_delete(&tournament, teams)?;
    Ok(())
}

/// Create a series in a tournament, while it is a draft or in registration (organizer).
#[cfg_attr(test, utoipa::path(
    post,
    path = "/api/v1/tournaments/{slug}/series",
    operation_id = "createSeries",
    tag = "series",
    params(("slug" = String, Path, description = "Tournament slug")),
    request_body = CreateSeries,
    security(("bearer" = [])),
    responses(
        (status = 201, description = "Series created", body = Series,
            headers(("Location" = String, description = "/api/v1/tournaments/{slug}/series/{series}"))),
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
    let request: CreateSeries = from_value(read_value(&mut req).await?)?;
    let series = request.series()?.create(&tournament)?;
    let written = db::series::create(&db, &slug, &series)
        .await
        .map_err(|error| taken(error, &series))?;
    if !written {
        return Err(refusal(recheck_create(&db, &slug, series).await));
    }
    let mut response = json(201, &view(&series, &tournament)?)?;
    response.headers_mut().set(
        "Location",
        &format!("/api/v1/tournaments/{slug}/series/{}", series.slug),
    )?;
    Ok(response)
}

/// A series: 404 when the tournament has no series with this slug, or when the tournament is a
/// draft and the caller has no organizer token.
#[cfg_attr(test, utoipa::path(
    get,
    path = "/api/v1/tournaments/{slug}/series/{series}",
    operation_id = "getSeries",
    tag = "series",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("series" = String, Path, description = "Series slug"),
    ),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The series", body = Series),
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
    let series = find(&db, &slug, &key).await?;
    json(200, &view(&series, &tournament)?)
}

/// `PATCH /tournaments/:slug/series/:series`: absent fields keep their value, and no field
/// takes `null`. The slug never changes. Specialty, maximum and number block change only while
/// the tournament is a draft or in registration.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct SeriesChanges {
    #[cfg_attr(test, schema(nullable = false))]
    name: Option<String>,
    /// Draft or registration only. A specialty the tournament declares.
    #[cfg_attr(test, schema(nullable = false))]
    specialty: Option<Specialty>,
    /// Draft or registration only. At least the registered teams of the series.
    #[cfg_attr(test, schema(nullable = false, minimum = 2, maximum = 100))]
    max_teams: Option<i64>,
    #[cfg_attr(test, schema(nullable = false, minimum = 1))]
    position: Option<i64>,
    /// Draft or registration only, and only while the series has no team.
    #[cfg_attr(
        test,
        schema(nullable = false, minimum = 100, maximum = 9900, multiple_of = 100)
    )]
    number_block: Option<i64>,
}

/// The body of a `PATCH`: 422 when it sets the slug, which never changes.
fn changes(value: Value) -> ApiResult<DomainChanges> {
    if value.get("slug").is_some() {
        return Err(Problem::invalid(
            "slug cannot change: it is the public key of the series",
        ));
    }
    let changes: SeriesChanges = from_value(value)?;
    Ok(DomainChanges {
        name: changes.name,
        specialty: changes.specialty,
        max_teams: changes.max_teams,
        position: changes.position,
        number_block: changes.number_block,
    })
}

/// Change a series (organizer): a partial body, checked on the merged state with the rules of
/// creation. Name and position change in every status; specialty, maximum and number block only
/// while the tournament is a draft or in registration. The block cannot move under teams, and
/// the maximum stays at least the registered teams.
#[cfg_attr(test, utoipa::path(
    patch,
    path = "/api/v1/tournaments/{slug}/series/{series}",
    operation_id = "updateSeries",
    tag = "series",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("series" = String, Path, description = "Series slug"),
    ),
    request_body = SeriesChanges,
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The series after the change", body = Series),
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
    let key = series_param(&ctx)?;
    let series = find(&db, &slug, &key).await?;
    let patch = changes(read_value(&mut req).await?)?;
    let teams = db::series::teams(&db, &slug, &key).await?;
    let after = series.change(patch.clone(), &tournament, teams)?;
    let written = db::series::save(&db, &slug, &series, &after)
        .await
        .map_err(|error| taken(error, &after))?;
    if !written {
        return Err(refusal(recheck_change(&db, &slug, &key, patch).await));
    }
    json(200, &view(&after, &tournament)?)
}

/// Delete a series without teams, while the tournament is a draft or in registration
/// (organizer).
#[cfg_attr(test, utoipa::path(
    delete,
    path = "/api/v1/tournaments/{slug}/series/{series}",
    operation_id = "deleteSeries",
    tag = "series",
    params(
        ("slug" = String, Path, description = "Tournament slug"),
        ("series" = String, Path, description = "Series slug"),
    ),
    security(("bearer" = [])),
    responses(
        (status = 204, description = "Series deleted"),
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
    let key = series_param(&ctx)?;
    find(&db, &slug, &key).await?;
    let teams = db::series::teams(&db, &slug, &key).await?;
    domain::Series::check_delete(&tournament, teams)?;
    if !db::series::delete(&db, &slug, &key).await? {
        return Err(refusal(recheck_delete(&db, &slug, &key).await));
    }
    no_content()
}
