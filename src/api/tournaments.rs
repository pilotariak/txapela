// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Tournaments: `GET|POST /tournaments` and `GET|PATCH|DELETE /tournaments/:slug`.
//!
//! Writes need the organizer token (`auth.rs`). Reads are public, but a draft is seen only with
//! the organizer token: without it, `GET /tournaments/:slug` answers 404 as for an unknown slug,
//! and the list leaves drafts out. Checks come in this order: 401, then 404, then the body (422),
//! then the rules of `domain` (409, then 422).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use worker::{Request, Response, RouteContext};

use super::auth::{self, Viewer};
use super::problem::{ApiResult, Problem, ProblemType};
use super::{database, from_value, json, no_content, read_value, slug_param};
use crate::db;
use crate::domain::civil::parse_date;
use crate::domain::slug::{SLUG_RULE, Slug};
use crate::domain::status::Status;
use crate::domain::tournament::{self as domain, Aire, Ranking, Specialty};

/// Where the tournament is played.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Venue {
    #[cfg_attr(test, schema(example = "Trinquet de l'Exemple"))]
    name: String,
    #[cfg_attr(test, schema(example = "1 place de l'Exemple, Ville-Exemple"))]
    address: String,
    #[cfg_attr(test, schema(minimum = -90, maximum = 90, example = 43.0))]
    latitude: f64,
    #[cfg_attr(test, schema(minimum = -180, maximum = 180, example = -1.0))]
    longitude: f64,
}

/// A named court of the venue, such as `Trinquet A`, used by the calendar.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Court {
    /// Trimmed, not empty, unique within the tournament.
    #[cfg_attr(test, schema(example = "Trinquet A"))]
    name: String,
}

/// A specialty of the tournament and the score a team must reach to win a match of it.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct SpecialtyTarget {
    specialty: Specialty,
    #[cfg_attr(test, schema(minimum = 1, example = 40))]
    target_score: i64,
}

/// Points of a match in the pool ranking: win > loss >= forfeit >= 0.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct Points {
    #[cfg_attr(test, schema(minimum = 0, example = 3))]
    win: i64,
    #[cfg_attr(test, schema(minimum = 0, example = 1))]
    loss: i64,
    #[cfg_attr(test, schema(minimum = 0, example = 0))]
    forfeit: i64,
}

/// Points in a request. At creation, an absent value takes its default: 3 for a win, 1 for a
/// loss, 0 for a forfeit. In a `PATCH`, it keeps the stored value.
#[derive(Debug, Default, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct PointsChanges {
    #[cfg_attr(test, schema(nullable = false, minimum = 0))]
    win: Option<i64>,
    #[cfg_attr(test, schema(nullable = false, minimum = 0))]
    loss: Option<i64>,
    #[cfg_attr(test, schema(nullable = false, minimum = 0))]
    forfeit: Option<i64>,
}

impl From<PointsChanges> for domain::PointsChanges {
    fn from(points: PointsChanges) -> Self {
        Self {
            win: points.win,
            loss: points.loss,
            forfeit: points.forfeit,
        }
    }
}

/// The dates of the tournament, `YYYY-MM-DD` in Europe/Paris: registration opens <=
/// registration closes <= first match <= finals.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Dates {
    #[cfg_attr(test, schema(format = Date, example = "2027-03-01"))]
    registration_opens: String,
    #[cfg_attr(test, schema(format = Date, example = "2027-03-31"))]
    registration_closes: String,
    #[cfg_attr(test, schema(format = Date, example = "2027-04-10"))]
    first_match: String,
    #[cfg_attr(test, schema(format = Date, example = "2027-06-19"))]
    finals: String,
}

/// A date of a request: 422 naming the field when it is not a `YYYY-MM-DD` date.
fn date_field(field: &str, value: &str) -> ApiResult<jiff::civil::Date> {
    parse_date(value).ok_or_else(|| Problem::invalid(format!("dates.{field} is a YYYY-MM-DD date")))
}

/// An optional date of a request.
fn optional_date(field: &str, value: Option<&str>) -> ApiResult<Option<jiff::civil::Date>> {
    value.map(|value| date_field(field, value)).transpose()
}

impl Dates {
    fn parse(&self) -> ApiResult<domain::Dates> {
        Ok(domain::Dates {
            registration_opens: date_field("registration_opens", &self.registration_opens)?,
            registration_closes: date_field("registration_closes", &self.registration_closes)?,
            first_match: date_field("first_match", &self.first_match)?,
            finals: date_field("finals", &self.finals)?,
        })
    }
}

/// A tournament.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct Tournament {
    /// The public key, chosen at creation and immutable.
    #[cfg_attr(test, schema(example = "open-de-printemps-2027"))]
    slug: String,
    #[cfg_attr(test, schema(example = "Open de printemps"))]
    name: String,
    /// Free text.
    #[cfg_attr(test, schema(example = "Club Exemple de Pelote"))]
    organizing_club: String,
    venue: Venue,
    aire: Aire,
    /// In the organizer's order.
    courts: Vec<Court>,
    /// In a fixed order: `paleta_gomme_creuse`, then `paleta_gomme_pleine`.
    specialties: Vec<SpecialtyTarget>,
    points: Points,
    ranking: Ranking,
    dates: Dates,
    /// Fee per team in euro cents, shown only: payment is out of scope.
    #[cfg_attr(test, schema(minimum = 0, example = 2000))]
    fee_cents: i64,
    #[cfg_attr(test, schema(minimum = 2, example = 16))]
    max_teams: i64,
    status: Status,
}

fn view(tournament: &domain::Tournament) -> Tournament {
    Tournament {
        slug: tournament.slug.to_string(),
        name: tournament.name.clone(),
        organizing_club: tournament.organizing_club.clone(),
        venue: Venue {
            name: tournament.venue.name.clone(),
            address: tournament.venue.address.clone(),
            latitude: tournament.venue.latitude,
            longitude: tournament.venue.longitude,
        },
        aire: tournament.aire,
        courts: tournament
            .courts
            .iter()
            .map(|name| Court { name: name.clone() })
            .collect(),
        specialties: tournament
            .specialties
            .iter()
            .map(|specialty| SpecialtyTarget {
                specialty: specialty.specialty,
                target_score: specialty.target_score,
            })
            .collect(),
        points: Points {
            win: tournament.points.win,
            loss: tournament.points.loss,
            forfeit: tournament.points.forfeit,
        },
        ranking: tournament.ranking,
        dates: Dates {
            registration_opens: tournament.dates.registration_opens.to_string(),
            registration_closes: tournament.dates.registration_closes.to_string(),
            first_match: tournament.dates.first_match.to_string(),
            finals: tournament.dates.finals.to_string(),
        },
        fee_cents: tournament.fee_cents,
        max_teams: tournament.max_teams,
        status: tournament.status,
    }
}

fn specialty_targets(specialties: Vec<SpecialtyTarget>) -> Vec<domain::SpecialtyTarget> {
    specialties
        .into_iter()
        .map(|specialty| domain::SpecialtyTarget {
            specialty: specialty.specialty,
            target_score: specialty.target_score,
        })
        .collect()
}

fn court_names(courts: Vec<Court>) -> Vec<String> {
    courts.into_iter().map(|court| court.name).collect()
}

/// `GET /tournaments`.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct TournamentList {
    /// By first match date, then slug.
    tournaments: Vec<Tournament>,
}

/// List the tournaments, by first match date then slug. Drafts only with the organizer token.
#[cfg_attr(test, utoipa::path(
    get,
    path = "/api/v1/tournaments",
    operation_id = "listTournaments",
    tag = "tournaments",
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The tournaments; drafts only for an organizer", body = TournamentList),
        (status = 401, response = super::problem::responses::Unauthorized),
    )
))]
pub(super) async fn list(req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    let viewer = auth::viewer(&req, &ctx.env)?;
    let db = database(&ctx.env)?;
    let tournaments = db::tournaments::list(&db, viewer == Viewer::Organizer).await?;
    json(
        200,
        &TournamentList {
            tournaments: tournaments.iter().map(view).collect(),
        },
    )
}

/// `POST /tournaments`. A new tournament is a draft.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct CreateTournament {
    /// 3 to 64 characters among a-z, 0-9 and single hyphens, starting and ending with a letter
    /// or a digit. Immutable.
    #[cfg_attr(
        test,
        schema(
            min_length = 3,
            max_length = 64,
            pattern = "^[a-z0-9]+(-[a-z0-9]+)*$",
            example = "open-de-printemps-2027"
        )
    )]
    slug: String,
    name: String,
    organizing_club: String,
    venue: Venue,
    aire: Aire,
    /// At least one.
    #[cfg_attr(test, schema(min_items = 1))]
    courts: Vec<Court>,
    /// At least one, none twice.
    #[cfg_attr(test, schema(min_items = 1))]
    specialties: Vec<SpecialtyTarget>,
    /// Default: 3 for a win, 1 for a loss, 0 for a forfeit.
    #[cfg_attr(test, schema(nullable = false))]
    points: Option<PointsChanges>,
    /// Default `points_then_goal_average`.
    #[cfg_attr(test, schema(nullable = false))]
    ranking: Option<Ranking>,
    dates: Dates,
    /// Fee per team in euro cents, shown only.
    #[cfg_attr(test, schema(minimum = 0))]
    fee_cents: i64,
    #[cfg_attr(test, schema(minimum = 2))]
    max_teams: i64,
}

impl CreateTournament {
    /// The draft to create: 422 when a value breaks a rule.
    fn tournament(self) -> ApiResult<domain::Tournament> {
        let slug = Slug::parse(&self.slug).ok_or_else(|| Problem::invalid(SLUG_RULE))?;
        let mut tournament = domain::Tournament {
            slug,
            name: self.name,
            organizing_club: self.organizing_club,
            venue: domain::Venue {
                name: self.venue.name,
                address: self.venue.address,
                latitude: self.venue.latitude,
                longitude: self.venue.longitude,
            },
            aire: self.aire,
            courts: court_names(self.courts),
            specialties: specialty_targets(self.specialties),
            points: domain::PointsChanges::from(self.points.unwrap_or_default())
                .apply_to(domain::Points::default()),
            ranking: self.ranking.unwrap_or_default(),
            dates: self.dates.parse()?,
            fee_cents: self.fee_cents,
            max_teams: self.max_teams,
            status: Status::Draft,
        };
        tournament
            .validate()
            .map_err(|error| Problem::invalid(error.to_string()))?;
        Ok(tournament)
    }
}

/// Create a tournament, as a draft (organizer).
#[cfg_attr(test, utoipa::path(
    post,
    path = "/api/v1/tournaments",
    operation_id = "createTournament",
    tag = "tournaments",
    request_body = CreateTournament,
    security(("bearer" = [])),
    responses(
        (status = 201, description = "Tournament created as a draft", body = Tournament,
            headers(("Location" = String, description = "/api/v1/tournaments/{slug}"))),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 409, response = super::problem::responses::Conflict),
        (status = 422, response = super::problem::responses::Unprocessable),
    )
))]
pub(super) async fn create(mut req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    auth::organizer(&req, &ctx.env)?;
    let db = database(&ctx.env)?;
    let request: CreateTournament = from_value(read_value(&mut req).await?)?;
    let tournament = request.tournament()?;
    match db::tournaments::create(&db, &tournament).await {
        Ok(()) => {}
        Err(error) if db::is_unique_violation(&error, "tournaments.slug") => {
            return Err(Problem::detail(
                ProblemType::SlugTaken,
                format!("slug: another tournament is {}", tournament.slug),
            ));
        }
        Err(error) => return Err(error.into()),
    }
    let mut response = json(201, &view(&tournament))?;
    response.headers_mut().set(
        "Location",
        &format!("/api/v1/tournaments/{}", tournament.slug),
    )?;
    Ok(response)
}

/// A tournament: 404 when no tournament has this slug, or when it is a draft and the caller has
/// no organizer token.
#[cfg_attr(test, utoipa::path(
    get,
    path = "/api/v1/tournaments/{slug}",
    operation_id = "getTournament",
    tag = "tournaments",
    params(("slug" = String, Path, description = "Tournament slug")),
    security((), ("bearer" = [])),
    responses(
        (status = 200, description = "The tournament", body = Tournament),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
    )
))]
pub(super) async fn get(req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    let viewer = auth::viewer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    let tournament = db::tournaments::find(&db, &slug)
        .await?
        .filter(|tournament| viewer == Viewer::Organizer || tournament.is_public())
        .ok_or_else(|| Problem::new(ProblemType::NotFound))?;
    json(200, &view(&tournament))
}

/// The venue in a `PATCH`: an absent value keeps the stored one.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct VenueChanges {
    #[cfg_attr(test, schema(nullable = false))]
    name: Option<String>,
    #[cfg_attr(test, schema(nullable = false))]
    address: Option<String>,
    #[cfg_attr(test, schema(nullable = false, minimum = -90, maximum = 90))]
    latitude: Option<f64>,
    #[cfg_attr(test, schema(nullable = false, minimum = -180, maximum = 180))]
    longitude: Option<f64>,
}

/// The dates in a `PATCH`: an absent date keeps the stored one, and the four dates must stay in
/// order.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct DatesChanges {
    #[cfg_attr(test, schema(nullable = false, format = Date))]
    registration_opens: Option<String>,
    #[cfg_attr(test, schema(nullable = false, format = Date))]
    registration_closes: Option<String>,
    #[cfg_attr(test, schema(nullable = false, format = Date))]
    first_match: Option<String>,
    #[cfg_attr(test, schema(nullable = false, format = Date))]
    finals: Option<String>,
}

/// `PATCH /tournaments/:slug`: absent fields keep their value, and no field takes `null`. The
/// slug never changes. Aire, specialties, points, ranking and courts change only while the
/// tournament is a draft.
#[derive(Debug, Deserialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct TournamentChanges {
    #[cfg_attr(test, schema(nullable = false))]
    name: Option<String>,
    #[cfg_attr(test, schema(nullable = false))]
    organizing_club: Option<String>,
    #[cfg_attr(test, schema(nullable = false))]
    venue: Option<VenueChanges>,
    /// Draft only.
    #[cfg_attr(test, schema(nullable = false))]
    aire: Option<Aire>,
    /// Draft only. Replaces the courts.
    #[cfg_attr(test, schema(nullable = false, min_items = 1))]
    courts: Option<Vec<Court>>,
    /// Draft only. Replaces the specialties and their target scores.
    #[cfg_attr(test, schema(nullable = false, min_items = 1))]
    specialties: Option<Vec<SpecialtyTarget>>,
    /// Draft only. Absent values keep the stored ones.
    #[cfg_attr(test, schema(nullable = false))]
    points: Option<PointsChanges>,
    /// Draft only.
    #[cfg_attr(test, schema(nullable = false))]
    ranking: Option<Ranking>,
    #[cfg_attr(test, schema(nullable = false))]
    dates: Option<DatesChanges>,
    #[cfg_attr(test, schema(nullable = false, minimum = 0))]
    fee_cents: Option<i64>,
    #[cfg_attr(test, schema(nullable = false, minimum = 2))]
    max_teams: Option<i64>,
    /// One step forward only: draft → registration → pools → finals → finished.
    #[cfg_attr(test, schema(nullable = false))]
    status: Option<Status>,
}

impl TournamentChanges {
    fn into_domain(self) -> ApiResult<domain::TournamentChanges> {
        let dates = match self.dates {
            Some(dates) => Some(domain::DatesChanges {
                registration_opens: optional_date(
                    "registration_opens",
                    dates.registration_opens.as_deref(),
                )?,
                registration_closes: optional_date(
                    "registration_closes",
                    dates.registration_closes.as_deref(),
                )?,
                first_match: optional_date("first_match", dates.first_match.as_deref())?,
                finals: optional_date("finals", dates.finals.as_deref())?,
            }),
            None => None,
        };
        Ok(domain::TournamentChanges {
            name: self.name,
            organizing_club: self.organizing_club,
            venue: self.venue.map(|venue| domain::VenueChanges {
                name: venue.name,
                address: venue.address,
                latitude: venue.latitude,
                longitude: venue.longitude,
            }),
            aire: self.aire,
            courts: self.courts.map(court_names),
            specialties: self.specialties.map(specialty_targets),
            points: self.points.map(domain::PointsChanges::from),
            ranking: self.ranking,
            dates,
            fee_cents: self.fee_cents,
            max_teams: self.max_teams,
            status: self.status,
        })
    }
}

/// The body of a `PATCH`: 422 when it sets the slug, which never changes.
fn changes(value: Value) -> ApiResult<domain::TournamentChanges> {
    if value.get("slug").is_some() {
        return Err(Problem::invalid(
            "slug cannot change: it is the public key of the tournament",
        ));
    }
    from_value::<TournamentChanges>(value)?.into_domain()
}

/// Change a tournament (organizer): a partial body, checked on the merged state with the rules of
/// creation. Aire, specialties, points, ranking and courts change only in draft. `status` moves
/// one step forward.
#[cfg_attr(test, utoipa::path(
    patch,
    path = "/api/v1/tournaments/{slug}",
    operation_id = "updateTournament",
    tag = "tournaments",
    params(("slug" = String, Path, description = "Tournament slug")),
    request_body = TournamentChanges,
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The tournament after the change", body = Tournament),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
        (status = 409, response = super::problem::responses::Conflict),
        (status = 422, response = super::problem::responses::Unprocessable),
    )
))]
pub(super) async fn update(mut req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    auth::organizer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    let stored = db::tournaments::find(&db, &slug)
        .await?
        .ok_or_else(|| Problem::new(ProblemType::NotFound))?;
    let patch = changes(read_value(&mut req).await?)?;
    let after = stored.change(patch)?;
    db::tournaments::save(&db, &stored, &after).await?;
    json(200, &view(&after))
}

/// Delete a draft tournament with its courts and specialties (organizer).
#[cfg_attr(test, utoipa::path(
    delete,
    path = "/api/v1/tournaments/{slug}",
    operation_id = "deleteTournament",
    tag = "tournaments",
    params(("slug" = String, Path, description = "Tournament slug")),
    security(("bearer" = [])),
    responses(
        (status = 204, description = "Tournament deleted, with its courts and specialties"),
        (status = 401, response = super::problem::responses::Unauthorized),
        (status = 404, response = super::problem::responses::NotFound),
        (status = 409, response = super::problem::responses::Conflict),
    )
))]
pub(super) async fn delete(req: Request, ctx: RouteContext<()>) -> ApiResult<Response> {
    auth::organizer(&req, &ctx.env)?;
    let slug = slug_param(&ctx)?;
    let db = database(&ctx.env)?;
    let stored = db::tournaments::find(&db, &slug)
        .await?
        .ok_or_else(|| Problem::new(ProblemType::NotFound))?;
    stored.check_delete()?;
    db::tournaments::delete(&db, &slug).await?;
    no_content()
}
