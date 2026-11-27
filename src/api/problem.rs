// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Errors as RFC 9457 problem details (`application/problem+json`).
//!
//! Each error has a stable `type` URI, which clients map to a translated message. Never rename
//! one: add a new type instead. The `detail` says what to fix in English, for developers; it never
//! holds a token, a hash or a player's data.
//!
//! `openapi.json` lists the types twice. The `ProblemType` schema is the closed `enum` of the
//! types the tournaments routes send (`FROZEN`, under `cfg(test)`): every error response of
//! those routes refers to it, and a value added to it would break their contract, so it never
//! grows. The `ExtensibleProblemType` schema lists every type as an `x-extensible-enum`, which
//! clients must read as open: the routes added since then refer to it, so that a new type stays
//! additive.

use serde::Serialize;
use worker::{Error, Response, console_error};

use crate::domain::series::SeriesError;
use crate::domain::team::TeamError;
use crate::domain::tournament::ChangeError;

/// The media type of every error response.
pub(crate) const PROBLEM_JSON: &str = "application/problem+json";

macro_rules! problem_types {
    ($($(#[$doc:meta])* $variant:ident = ($uri:tt, $status:literal, $title:literal)),* $(,)?) => {
        /// The `type` of a problem: one per error, stable across versions.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
        pub(crate) enum ProblemType {
            $($(#[$doc])* #[serde(rename = $uri)] $variant,)*
        }

        impl ProblemType {
            /// Every problem type, in the order of declaration.
            #[cfg(test)]
            pub(crate) const ALL: &[Self] = &[$(Self::$variant,)*];

            pub(crate) fn status(self) -> u16 {
                match self {
                    $(Self::$variant => $status,)*
                }
            }

            fn title(self) -> &'static str {
                match self {
                    $(Self::$variant => $title,)*
                }
            }

            /// The `type` URI, as serialized.
            #[cfg(test)]
            pub(crate) fn uri(self) -> &'static str {
                match self {
                    $(Self::$variant => $uri,)*
                }
            }
        }
    };
}

problem_types! {
    /// No organizer token, a malformed one, a wrong one, or no `ORGANIZER_TOKEN_SHA256` secret.
    Unauthorized = ("https://txapela.pilotariak.com/problems/unauthorized", 401, "An organizer token is required"),
    /// No route has this path, or no tournament that the caller may see has this slug, or the
    /// tournament has no series or team with this key.
    NotFound = ("https://txapela.pilotariak.com/problems/not-found", 404, "Not found"),
    /// The path exists with other methods.
    MethodNotAllowed = ("https://txapela.pilotariak.com/problems/method-not-allowed", 405, "Method not allowed"),
    /// Another tournament has this slug, or another series of the tournament has it.
    SlugTaken = ("https://txapela.pilotariak.com/problems/slug-taken", 409, "This slug is taken"),
    /// Only a draft tournament can do this: change its aire, specialties, points, ranking or
    /// courts, or be deleted.
    NotADraft = ("https://txapela.pilotariak.com/problems/not-a-draft", 409, "The tournament is not a draft"),
    /// A status change other than one step forward.
    InvalidTransition = ("https://txapela.pilotariak.com/problems/invalid-transition", 409, "This status change is not allowed"),
    /// The body or a value breaks a rule: the `detail` names it.
    InvalidRequest = ("https://txapela.pilotariak.com/problems/invalid-request", 422, "The request is invalid"),
    /// txapela failed.
    Internal = ("https://txapela.pilotariak.com/problems/internal", 500, "Internal error"),
    /// The tournament is past registration: its series and teams no longer change, except the
    /// name and position of a series and the players and fee of a team.
    RegistrationClosed = ("https://txapela.pilotariak.com/problems/registration-closed", 409, "Registration is closed"),
    /// The series has its maximum of registered teams, or a lower maximum than its registered
    /// teams.
    SeriesFull = ("https://txapela.pilotariak.com/problems/series-full", 409, "The series is full"),
    /// The tournament has its maximum of registered teams.
    TournamentFull = ("https://txapela.pilotariak.com/problems/tournament-full", 409, "The tournament is full"),
    /// Another team of the tournament has this number.
    NumberTaken = ("https://txapela.pilotariak.com/problems/number-taken", 409, "This team number is taken"),
    /// Another series of the tournament has this number block.
    BlockTaken = ("https://txapela.pilotariak.com/problems/block-taken", 409, "This number block is taken"),
    /// The series has teams: it cannot be deleted, and its number block cannot move.
    SeriesHasTeams = ("https://txapela.pilotariak.com/problems/series-has-teams", 409, "The series has teams"),
}

/// The problem types of the health and tournaments routes, the closed `enum` of the
/// `ProblemType` schema of `openapi.json`. Every error response of those routes refers to it,
/// and oasdiff rates a value added to a response `enum` as a breaking change (ERR): this list
/// never grows, and those routes send no other type.
#[cfg(test)]
pub(crate) const FROZEN: [ProblemType; 8] = [
    ProblemType::Unauthorized,
    ProblemType::NotFound,
    ProblemType::MethodNotAllowed,
    ProblemType::SlugTaken,
    ProblemType::NotADraft,
    ProblemType::InvalidTransition,
    ProblemType::InvalidRequest,
    ProblemType::Internal,
];

/// The `ProblemType` schema: the closed `enum` of [`FROZEN`].
#[cfg(test)]
impl utoipa::PartialSchema for ProblemType {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .description(Some(
                "The `type` of a problem: one per error, stable across versions.",
            ))
            .enum_values(Some(FROZEN.map(ProblemType::uri)))
            .into()
    }
}

#[cfg(test)]
impl utoipa::ToSchema for ProblemType {}

/// The `ExtensibleProblemType` schema: every problem type, as an `x-extensible-enum`. Never
/// built: `Serialize` lets the document that holds it carry serde's `rename`.
#[cfg(test)]
#[derive(Serialize)]
pub(crate) struct ExtensibleProblemType;

#[cfg(test)]
impl utoipa::PartialSchema for ExtensibleProblemType {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        let types: Vec<&str> = ProblemType::ALL.iter().map(|kind| kind.uri()).collect();
        utoipa::openapi::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .description(Some(
                "The `type` of a problem: one per error, stable across versions. The list grows: \
                 a client maps the types it knows and falls back on the status for the others.",
            ))
            .extensions(Some(
                utoipa::openapi::extensions::ExtensionsBuilder::new()
                    .add("x-extensible-enum", types)
                    .build(),
            ))
            .into()
    }
}

#[cfg(test)]
impl utoipa::ToSchema for ExtensibleProblemType {}

/// The body of an error response.
#[derive(Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(crate) struct ProblemDocument {
    #[serde(rename = "type")]
    kind: ProblemType,
    title: &'static str,
    status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

/// An error of a route.
#[derive(Debug)]
pub(crate) struct Problem {
    kind: ProblemType,
    detail: Option<String>,
}

/// The result of a route.
pub(crate) type ApiResult<T> = Result<T, Problem>;

impl Problem {
    pub(crate) fn new(kind: ProblemType) -> Self {
        Self { kind, detail: None }
    }

    /// A problem with a `detail`.
    pub(crate) fn detail(kind: ProblemType, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: Some(detail.into()),
        }
    }

    /// 422 `invalid-request` with what is wrong.
    pub(crate) fn invalid(detail: impl Into<String>) -> Self {
        Self::detail(ProblemType::InvalidRequest, detail)
    }

    /// The response, with `Content-Type: application/problem+json`. A 401 also sends
    /// `WWW-Authenticate: Bearer`, which RFC 9110 requires.
    pub(crate) fn into_response(self) -> worker::Result<Response> {
        let status = self.kind.status();
        let mut response = Response::from_json(&ProblemDocument {
            kind: self.kind,
            title: self.kind.title(),
            status,
            detail: self.detail,
        })?
        .with_status(status);
        let headers = response.headers_mut();
        headers.set("Content-Type", PROBLEM_JSON)?;
        if self.kind == ProblemType::Unauthorized {
            headers.set("WWW-Authenticate", "Bearer")?;
        }
        Ok(response)
    }
}

impl From<ChangeError> for Problem {
    fn from(error: ChangeError) -> Self {
        let kind = match &error {
            ChangeError::NotADraft(_) | ChangeError::NotADraftToDelete(_) => ProblemType::NotADraft,
            ChangeError::InvalidTransition(_) => ProblemType::InvalidTransition,
            ChangeError::Invalid(_) => ProblemType::InvalidRequest,
        };
        Self::detail(kind, error.to_string())
    }
}

impl From<SeriesError> for Problem {
    fn from(error: SeriesError) -> Self {
        let kind = match &error {
            SeriesError::RegistrationClosed(_) => ProblemType::RegistrationClosed,
            SeriesError::Invalid(_) => ProblemType::InvalidRequest,
            SeriesError::BelowRegistered { .. } => ProblemType::SeriesFull,
            SeriesError::BlockHasTeams { .. } | SeriesError::DeleteHasTeams { .. } => {
                ProblemType::SeriesHasTeams
            }
        };
        Self::detail(kind, error.to_string())
    }
}

impl From<TeamError> for Problem {
    fn from(error: TeamError) -> Self {
        let kind = match &error {
            TeamError::RegistrationClosed(_) => ProblemType::RegistrationClosed,
            TeamError::Invalid(_) => ProblemType::InvalidRequest,
            TeamError::SeriesFull { .. } => ProblemType::SeriesFull,
            TeamError::TournamentFull { .. } => ProblemType::TournamentFull,
        };
        Self::detail(kind, error.to_string())
    }
}

/// Logs what failed without request data: a D1 message names a table or a constraint, never a
/// bound value. Other errors are logged by kind only, since a deserialization message can quote
/// a stored value.
fn log(error: &Error) {
    match error {
        Error::D1(_) | Error::JsError(_) | Error::RustError(_) => {
            console_error!("internal error: {error}");
        }
        other => {
            let kind = format!("{other:?}");
            let kind = kind.split(['(', ' ', '{']).next().unwrap_or("unknown");
            console_error!("internal error of kind {kind}");
        }
    }
}

impl From<Error> for Problem {
    fn from(error: Error) -> Self {
        log(&error);
        Self::new(ProblemType::Internal)
    }
}

/// Reusable error responses of the `OpenAPI` document. Only their types are read, by the
/// `ToResponse` and `ToSchema` derives.
#[cfg(test)]
#[expect(
    dead_code,
    reason = "the responses are never built, utoipa reads their types"
)]
pub(crate) mod responses {
    use serde::Serialize;

    use super::{ExtensibleProblemType, ProblemDocument};

    /// No organizer token, a malformed or wrong one, or no secret to check it against.
    #[derive(utoipa::ToResponse)]
    #[response(
        content_type = "application/problem+json",
        headers(("WWW-Authenticate" = String, description = "Bearer"))
    )]
    pub(crate) struct Unauthorized(ProblemDocument);

    /// No tournament with this slug, or a draft and no organizer token: both answer the same.
    /// Under a tournament, no series or team with this key either.
    #[derive(utoipa::ToResponse)]
    #[response(content_type = "application/problem+json")]
    pub(crate) struct NotFound(ProblemDocument);

    /// Slug taken, not a draft, or a status change other than one step forward.
    #[derive(utoipa::ToResponse)]
    #[response(content_type = "application/problem+json")]
    pub(crate) struct Conflict(ProblemDocument);

    /// The body is not JSON, has an unknown or `null` field, or breaks a rule: the `detail`
    /// names it.
    #[derive(utoipa::ToResponse)]
    #[response(content_type = "application/problem+json")]
    pub(crate) struct Unprocessable(ProblemDocument);

    /// The body of an error response whose `type` list grows: the series and teams routes.
    #[derive(Serialize, utoipa::ToSchema)]
    pub(crate) struct ExtensibleProblemDocument {
        #[serde(rename = "type")]
        kind: ExtensibleProblemType,
        title: &'static str,
        status: u16,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    }

    /// A series or team that conflicts with its tournament, its series or the other series and
    /// teams: registration closed, a slug, number block or team number taken, a full series or
    /// tournament, or a series that still has teams.
    #[derive(utoipa::ToResponse)]
    #[response(content_type = "application/problem+json")]
    pub(crate) struct SeriesAndTeamsConflict(ExtensibleProblemDocument);
}

#[cfg(test)]
mod tests {
    use super::{FROZEN, Problem, ProblemType};
    use crate::domain::rule::Invalid;
    use crate::domain::series::SeriesError;
    use crate::domain::slug::Slug;
    use crate::domain::status::{InvalidTransition, RegistrationClosed, Status};
    use crate::domain::team::TeamError;
    use crate::domain::tournament::ChangeError;

    fn invalid() -> Invalid {
        Invalid {
            field: "name",
            reason: "cannot be empty".to_owned(),
        }
    }

    #[test]
    fn problem_types_carry_their_status_and_title() {
        for (kind, status, title) in [
            (
                ProblemType::Unauthorized,
                401,
                "An organizer token is required",
            ),
            (ProblemType::NotFound, 404, "Not found"),
            (ProblemType::MethodNotAllowed, 405, "Method not allowed"),
            (ProblemType::SlugTaken, 409, "This slug is taken"),
            (ProblemType::NotADraft, 409, "The tournament is not a draft"),
            (
                ProblemType::InvalidTransition,
                409,
                "This status change is not allowed",
            ),
            (ProblemType::InvalidRequest, 422, "The request is invalid"),
            (ProblemType::Internal, 500, "Internal error"),
            (
                ProblemType::RegistrationClosed,
                409,
                "Registration is closed",
            ),
            (ProblemType::SeriesFull, 409, "The series is full"),
            (ProblemType::TournamentFull, 409, "The tournament is full"),
            (ProblemType::NumberTaken, 409, "This team number is taken"),
            (ProblemType::BlockTaken, 409, "This number block is taken"),
            (ProblemType::SeriesHasTeams, 409, "The series has teams"),
        ] {
            assert_eq!(kind.status(), status, "{kind:?} answers {status}");
            assert_eq!(kind.title(), title, "{kind:?} is titled {title:?}");
        }
    }

    #[test]
    fn the_closed_list_of_the_tournaments_routes_never_grows() {
        assert_eq!(
            FROZEN.map(ProblemType::uri),
            [
                "https://txapela.pilotariak.com/problems/unauthorized",
                "https://txapela.pilotariak.com/problems/not-found",
                "https://txapela.pilotariak.com/problems/method-not-allowed",
                "https://txapela.pilotariak.com/problems/slug-taken",
                "https://txapela.pilotariak.com/problems/not-a-draft",
                "https://txapela.pilotariak.com/problems/invalid-transition",
                "https://txapela.pilotariak.com/problems/invalid-request",
                "https://txapela.pilotariak.com/problems/internal",
            ],
            "never add a type here: the ProblemType enum of openapi.json is the contract of the \
             tournaments routes, and a value added to it is a breaking change"
        );
        assert_eq!(
            &ProblemType::ALL[..FROZEN.len()],
            &FROZEN[..],
            "new types are declared after the frozen ones"
        );
    }

    #[test]
    fn change_errors_map_to_their_problem_within_the_closed_list() {
        for (error, kind) in [
            (ChangeError::NotADraft("aire"), ProblemType::NotADraft),
            (
                ChangeError::NotADraftToDelete(Status::Finished),
                ProblemType::NotADraft,
            ),
            (
                ChangeError::InvalidTransition(InvalidTransition {
                    from: Status::Draft,
                    to: Status::Finals,
                }),
                ProblemType::InvalidTransition,
            ),
            (ChangeError::Invalid(invalid()), ProblemType::InvalidRequest),
        ] {
            let detail = error.to_string();
            let problem = Problem::from(error);
            assert_eq!(problem.kind, kind, "{detail} answers {kind:?}");
            assert!(
                FROZEN.contains(&problem.kind),
                "PATCH and DELETE /tournaments/:slug send the closed types only"
            );
            assert_eq!(
                problem.detail.as_deref(),
                Some(detail.as_str()),
                "the detail names the rule"
            );
        }
    }

    #[test]
    fn series_and_team_errors_map_to_their_problem() {
        let closed = RegistrationClosed(Status::Pools);
        let series = Slug::parse("1re-serie-a").expect("should be a slug");
        let problems = [
            (
                Problem::from(SeriesError::RegistrationClosed(closed)),
                ProblemType::RegistrationClosed,
            ),
            (
                Problem::from(SeriesError::Invalid(invalid())),
                ProblemType::InvalidRequest,
            ),
            (
                Problem::from(SeriesError::BelowRegistered { registered: 3 }),
                ProblemType::SeriesFull,
            ),
            (
                Problem::from(SeriesError::BlockHasTeams { teams: 1 }),
                ProblemType::SeriesHasTeams,
            ),
            (
                Problem::from(SeriesError::DeleteHasTeams { teams: 1 }),
                ProblemType::SeriesHasTeams,
            ),
            (
                Problem::from(TeamError::RegistrationClosed(closed)),
                ProblemType::RegistrationClosed,
            ),
            (
                Problem::from(TeamError::Invalid(invalid())),
                ProblemType::InvalidRequest,
            ),
            (
                Problem::from(TeamError::SeriesFull {
                    series,
                    max_teams: 8,
                }),
                ProblemType::SeriesFull,
            ),
            (
                Problem::from(TeamError::TournamentFull { max_teams: 16 }),
                ProblemType::TournamentFull,
            ),
        ];
        for (problem, kind) in problems {
            assert_eq!(problem.kind, kind, "{:?} answers {kind:?}", problem.detail);
            assert!(problem.detail.is_some(), "{kind:?} names the rule");
        }
    }
}
