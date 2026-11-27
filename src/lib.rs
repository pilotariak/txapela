// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! txapela, the pelota tournaments of a club: a Cloudflare Worker with a D1 database.
//!
//! `api` holds the REST API under `/api/v1/`, one module per resource, `db` the D1 queries, one
//! module per table, `domain` the pure logic and types, and `web` the public pages outside the
//! API. This file wires them into the Worker entry point, and adds after the router what every
//! response needs.

mod api;
mod db;
mod domain;
mod web;

use worker::{Context, Env, Request, Response, Result, Router, event};

use api::cors::Cors;
use api::problem::Problem;

/// Security headers of every response, set by txapela itself: nothing in front of the Worker
/// adds them. A header the route already set, like the landing page's
/// `Content-Security-Policy`, is kept. `Cross-Origin-Resource-Policy: same-origin` does not
/// block the CORS calls of the Kancha website: the Fetch Standard runs that check only for an
/// opaque response, which a `no-cors` request gets and a `cors` request does not.
const SECURITY_HEADERS: &[(&str, &str)] = &[
    (
        "Content-Security-Policy",
        "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
    ),
    ("Cross-Origin-Resource-Policy", "same-origin"),
    ("Referrer-Policy", "no-referrer"),
    (
        "Strict-Transport-Security",
        "max-age=31536000; includeSubDomains",
    ),
    ("X-Content-Type-Options", "nosniff"),
    ("X-Frame-Options", "DENY"),
];

// The workers-rs Router answers 405 to any other method on a registered path, and 404 to an
// unknown path: `api::finish` turns both into problems under /api/. The router takes the request
// and the environment, so the CORS side of an API request is read first, and a preflight from an
// allowed origin gets its 204 before the router's 405.
#[event(fetch)]
async fn fetch(request: Request, env: Env, _context: Context) -> Result<Response> {
    let cors = if request.path().starts_with("/api/") {
        Some(Cors::of(&request, &env)?)
    } else {
        None
    };
    let response = if cors.as_ref().is_some_and(Cors::answers_preflight) {
        Cors::preflight()?
    } else {
        let router = web::routes(api::routes(Router::new()));
        match router.run(request, env).await {
            Ok(response) => response,
            Err(error) => Problem::from(error).into_response()?,
        }
    };
    let mut response = match &cors {
        Some(cors) => api::finish(response, cors)?,
        None => response,
    };
    let headers = response.headers_mut();
    for (name, value) in SECURITY_HEADERS {
        if !headers.has(name)? {
            headers.set(name, value)?;
        }
    }
    Ok(response)
}
