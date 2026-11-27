// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Errors as RFC 9457 problem details (`application/problem+json`).
//!
//! Each error has a stable `type` URI, which clients map to a translated message. Never rename
//! one: add a new type instead.

use serde::Serialize;
use worker::{Error, Response, console_error};

/// The media type of every error response.
pub(crate) const PROBLEM_JSON: &str = "application/problem+json";

macro_rules! problem_types {
    ($($(#[$doc:meta])* $variant:ident = ($uri:literal, $status:literal, $title:literal)),* $(,)?) => {
        /// The `type` of a problem: one per error, stable across versions.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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
    /// No route has this path.
    NotFound = ("https://txapela.pilotariak.com/problems/not-found", 404, "Not found"),
    /// The path exists with other methods.
    MethodNotAllowed = ("https://txapela.pilotariak.com/problems/method-not-allowed", 405, "Method not allowed"),
    /// txapela failed.
    Internal = ("https://txapela.pilotariak.com/problems/internal", 500, "Internal error"),
}

/// The body of an error response.
#[derive(Serialize)]
struct ProblemDocument {
    #[serde(rename = "type")]
    kind: ProblemType,
    title: &'static str,
    status: u16,
}

/// An error of a route.
#[derive(Debug)]
pub(crate) struct Problem {
    kind: ProblemType,
}

impl Problem {
    pub(crate) fn new(kind: ProblemType) -> Self {
        Self { kind }
    }

    /// The response, with `Content-Type: application/problem+json`.
    pub(crate) fn into_response(self) -> worker::Result<Response> {
        let status = self.kind.status();
        let mut response = Response::from_json(&ProblemDocument {
            kind: self.kind,
            title: self.kind.title(),
            status,
        })?
        .with_status(status);
        response.headers_mut().set("Content-Type", PROBLEM_JSON)?;
        Ok(response)
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

#[cfg(test)]
mod tests {
    use super::ProblemType;

    #[test]
    fn problem_types_carry_their_status_and_title() {
        for (kind, status, title) in [
            (ProblemType::NotFound, 404, "Not found"),
            (ProblemType::MethodNotAllowed, 405, "Method not allowed"),
            (ProblemType::Internal, 500, "Internal error"),
        ] {
            assert_eq!(kind.status(), status, "{kind:?} answers {status}");
            assert_eq!(kind.title(), title, "{kind:?} is titled {title:?}");
        }
    }
}
