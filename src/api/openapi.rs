// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The `OpenAPI` 3.1 document of the API, committed as `openapi.json`: the contract between
//! txapela and its clients, which generate their types from it.
//!
//! utoipa builds it from the `#[utoipa::path]` annotations of the handlers and the `ToSchema`
//! derives of their types. txapela does not serve the document, so all of it exists under
//! `cfg(test)` only, with utoipa as a dev-dependency: the Worker's wasm carries no schema code.
//! The annotations are not tied to the router, so [`tests::openapi_json_is_up_to_date`] fails
//! when the generated document differs from the committed file. Regenerate it with
//! `make openapi`.

use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use super::problem::responses::{Conflict, NotFound, Unauthorized, Unprocessable};
use super::problem::{ProblemDocument, ProblemType};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "txapela",
        // The version of the contract, which `/api/v1` carries: not the crate version, which
        // release-please bumps on every release.
        version = "1",
        description = "Pelota tournaments run by a club. Paths are under /api/v1. Organizers \
            write with a bearer token; reads are public, and a draft tournament is seen only \
            with the organizer token. Errors are RFC 9457 problem details with a stable type. \
            Dates are civil dates YYYY-MM-DD in Europe/Paris. Every API response sends \
            Cache-Control: private, no-store.",
        license(name = "Apache-2.0", identifier = "Apache-2.0"),
    ),
    servers((url = "https://txapela.pilotariak.com")),
    paths(
        super::health::get,
        super::tournaments::list,
        super::tournaments::create,
        super::tournaments::get,
        super::tournaments::update,
        super::tournaments::delete,
    ),
    components(
        schemas(ProblemDocument, ProblemType),
        responses(Unauthorized, NotFound, Conflict, Unprocessable),
    ),
    modifiers(&SecuritySchemes),
    tags(
        (name = "health", description = "Liveness"),
        (name = "tournaments", description = "Create, read, change and delete tournaments"),
    ),
)]
struct ApiDoc;

/// The organizer token, until the ADR of #5 (`auth.rs`).
struct SecuritySchemes;

impl Modify for SecuritySchemes {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "Organizer token, compared by its SHA-256 with a Worker secret. Required \
                         to write; on reads, it shows drafts too.",
                    ))
                    .build(),
            ),
        );
    }
}

/// The document as committed: pretty JSON with a final newline.
fn document() -> String {
    let mut json = ApiDoc::openapi()
        .to_pretty_json()
        .expect("should serialize the OpenAPI document");
    json.push('\n');
    json
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::document;

    const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/openapi.json");

    /// Set to rewrite openapi.json instead of checking it: `make openapi`.
    const WRITE: &str = "TXAPELA_WRITE_OPENAPI";

    #[test]
    fn openapi_json_is_up_to_date() {
        let generated = document();
        if std::env::var_os(WRITE).is_some() {
            fs::write(PATH, &generated).expect("should write openapi.json");
            return;
        }
        let committed = fs::read_to_string(PATH).unwrap_or_default();
        assert!(
            committed == generated,
            "openapi.json differs from the document generated from the code: run `make openapi` \
             and commit the result"
        );
    }

    #[test]
    fn the_document_is_openapi_3_1_with_every_route() {
        let document: serde_json::Value =
            serde_json::from_str(&document()).expect("should be JSON");
        assert_eq!(document["openapi"], "3.1.0", "OpenAPI 3.1");
        let operations: usize = document["paths"]
            .as_object()
            .expect("should have paths")
            .values()
            .map(|path| path.as_object().expect("should be an object").len())
            .sum();
        assert_eq!(
            operations, 6,
            "the health route and the 5 routes of the tournaments API"
        );
    }

    #[test]
    fn writes_need_the_bearer_token_and_reads_take_it_optionally() {
        let document: serde_json::Value =
            serde_json::from_str(&document()).expect("should be JSON");
        let security =
            |path: &str, method: &str| document["paths"][path][method]["security"].clone();
        let bearer = serde_json::json!([{ "bearer": [] }]);
        let optional = serde_json::json!([{}, { "bearer": [] }]);
        for (path, method) in [
            ("/api/v1/tournaments", "post"),
            ("/api/v1/tournaments/{slug}", "patch"),
            ("/api/v1/tournaments/{slug}", "delete"),
        ] {
            assert_eq!(
                security(path, method),
                bearer,
                "{method} {path} needs the token"
            );
        }
        for (path, method) in [
            ("/api/v1/tournaments", "get"),
            ("/api/v1/tournaments/{slug}", "get"),
        ] {
            assert_eq!(
                security(path, method),
                optional,
                "{method} {path} is public, the token shows drafts"
            );
        }
    }
}
