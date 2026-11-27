// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Errors as RFC 9457 problem details (`application/problem+json`).
//!
//! Each error has a stable `type` URI, which clients map to a translated message. Never rename
//! one: add a new type instead. The `detail` says what to fix in English, for developers; it never
//! holds a token, a hash or a player's data.

use serde::Serialize;
use worker::{Error, Response, console_error};

use crate::domain::tournament::ChangeError;

/// The media type of every error response.
pub(crate) const PROBLEM_JSON: &str = "application/problem+json";

macro_rules! problem_types {
    ($($(#[$doc:meta])* $variant:ident = ($uri:tt, $status:literal, $title:literal)),* $(,)?) => {
        /// The `type` of a problem: one per error, stable across versions.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
        #[cfg_attr(test, derive(utoipa::ToSchema))]
        pub(crate) enum ProblemType {
            $($(#[$doc])* #[serde(rename = $uri)] $variant,)*
        }

        impl ProblemType {
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
        }
    };
}

problem_types! {
    /// No organizer token, a malformed one, a wrong one, or no `ORGANIZER_TOKEN_SHA256` secret.
    Unauthorized = ("https://txapela.pilotariak.com/problems/unauthorized", 401, "An organizer token is required"),
    /// No route has this path, or no tournament that the caller may see has this slug.
    NotFound = ("https://txapela.pilotariak.com/problems/not-found", 404, "Not found"),
    /// The path exists with other methods.
    MethodNotAllowed = ("https://txapela.pilotariak.com/problems/method-not-allowed", 405, "Method not allowed"),
    /// Another tournament has this slug.
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
}

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
/// `ToResponse` derive.
#[cfg(test)]
#[expect(
    dead_code,
    reason = "the responses are never built, utoipa reads their types"
)]
pub(crate) mod responses {
    use super::ProblemDocument;

    /// No organizer token, a malformed or wrong one, or no secret to check it against.
    #[derive(utoipa::ToResponse)]
    #[response(
        content_type = "application/problem+json",
        headers(("WWW-Authenticate" = String, description = "Bearer"))
    )]
    pub(crate) struct Unauthorized(ProblemDocument);

    /// No tournament with this slug, or a draft and no organizer token: both answer the same.
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
}

#[cfg(test)]
mod tests {
    use super::{Problem, ProblemType};
    use crate::domain::status::{InvalidTransition, Status};
    use crate::domain::tournament::{ChangeError, InvalidTournament};

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
        ] {
            assert_eq!(kind.status(), status, "{kind:?} answers {status}");
            assert_eq!(kind.title(), title, "{kind:?} is titled {title:?}");
        }
    }

    #[test]
    fn change_errors_map_to_their_problem() {
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
            (
                ChangeError::Invalid(InvalidTournament {
                    field: "name",
                    reason: "cannot be empty".to_owned(),
                }),
                ProblemType::InvalidRequest,
            ),
        ] {
            let detail = error.to_string();
            let problem = Problem::from(error);
            assert_eq!(problem.kind, kind, "{detail} answers {kind:?}");
            assert_eq!(
                problem.detail.as_deref(),
                Some(detail.as_str()),
                "the detail names the rule"
            );
        }
    }
}
