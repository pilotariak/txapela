// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The REST API, one module per resource. Every route lives under `/api/v1/`.
//!
//! Each resource module holds its handlers and its request and response types. An error is a
//! problem (`problem.rs`), sent as `application/problem+json`. Handlers read D1 through `db` and
//! apply the rules of `domain`; they read the Worker's clock and pass the time to `domain`.

pub(crate) mod cors;
mod health;
pub(crate) mod problem;

use worker::{Response, Router};

use cors::Cors;
use problem::{PROBLEM_JSON, Problem, ProblemType};

/// Prefixes a path with `/api/v1`, the only place the API version appears. A breaking change goes
/// to `/api/v2/` while installed app versions keep calling `/api/v1/`; the CORS of `cors.rs`
/// covers every version.
macro_rules! v1 {
    ($path:literal) => {
        concat!("/api/v1", $path)
    };
}

/// Registers the API routes on the router.
pub(crate) fn routes(router: Router<'_, ()>) -> Router<'_, ()> {
    router.get(v1!("/health"), |_, _| health::get())
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
