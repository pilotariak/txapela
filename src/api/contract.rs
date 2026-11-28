// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `GET /api/v1/openapi.json`: the contract of the API, the committed `openapi.json`.
//!
//! The resource is the contract, hence `contract.rs`: `openapi.rs` is taken by the code that
//! generates the document, which exists under `cfg(test)` only, while this route is in the
//! Worker. It serves the committed file unchanged, embedded at compile time, never a document
//! generated at runtime: utoipa stays a dev-dependency, and the wasm carries the JSON text but no
//! schema code. The unit test `openapi_json_is_up_to_date` of `openapi.rs` fails when the
//! committed file differs from the code, so the document served is the one the handlers were
//! tested against. Viewers, such as a read-only API reference, read it from here: txapela serves
//! the JSON only.

use worker::{Response, ResponseBuilder, Result};

/// `openapi.json` as committed. Cargo rebuilds the crate when the file changes, so the next build
/// after `make openapi` serves the new document.
const DOCUMENT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/openapi.json"));

/// Answers this document, the contract that clients generate their types from. Needs no token.
#[cfg_attr(test, utoipa::path(
    get,
    path = "/api/v1/openapi.json",
    operation_id = "getContract",
    tag = "contract",
    security(()),
    responses((
        status = 200,
        description = "The OpenAPI 3.1 document of the API, this one",
        content_type = "application/json",
        body = Object
    ))
))]
pub(super) fn get() -> Result<Response> {
    // Public like `/health`: the document describes the API and holds no data.
    // `Response::from_json` would encode the text as one JSON string, and parsing it to serialize
    // it again would change its layout: the bytes go out as committed, with the Content-Type that
    // `from_json` sets. `api::finish` and `fetch` add the headers of every API response,
    // `Cache-Control: private, no-store` included, so the document's own sentence about it holds
    // for this route too.
    Ok(ResponseBuilder::new()
        .with_header("Content-Type", "application/json")?
        .fixed(DOCUMENT.as_bytes().to_vec()))
}
