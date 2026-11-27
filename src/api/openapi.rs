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

use super::problem::responses::{
    Conflict, ExtensibleProblemDocument, NotFound, SeriesAndTeamsConflict, Unauthorized,
    Unprocessable,
};
use super::problem::{ExtensibleProblemType, ProblemDocument, ProblemType};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "txapela",
        // The version of the contract, which `/api/v1` carries: not the crate version, which
        // release-please bumps on every release.
        version = "1",
        description = "Pelota tournaments run by a club. Paths are under /api/v1. Organizers \
            write with a bearer token; reads are public, and a draft tournament, with its series \
            and teams, is seen only with the organizer token. The licence numbers and contacts \
            of players are for the organizer only. Errors are RFC 9457 problem details with a \
            stable type. Dates are civil dates YYYY-MM-DD in Europe/Paris. Every API response \
            sends Cache-Control: private, no-store.",
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
        super::series::list,
        super::series::create,
        super::series::get,
        super::series::update,
        super::series::delete,
        super::teams::list,
        super::teams::create,
        super::teams::get,
        super::teams::update,
        super::teams::delete,
    ),
    components(
        schemas(
            ProblemDocument,
            ProblemType,
            ExtensibleProblemDocument,
            ExtensibleProblemType
        ),
        responses(
            Unauthorized,
            NotFound,
            Conflict,
            Unprocessable,
            SeriesAndTeamsConflict
        ),
    ),
    modifiers(&SecuritySchemes),
    tags(
        (name = "health", description = "Liveness"),
        (name = "tournaments", description = "Create, read, change and delete tournaments"),
        (name = "series", description = "The series of a tournament: one specialty at one level, \
            with a block of team numbers"),
        (name = "teams", description = "The teams that the organizer registers, two players each"),
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
    use crate::api::problem::{FROZEN, ProblemType};

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
            operations, 16,
            "the health route and the 5 routes of each of tournaments, series and teams"
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
            ("/api/v1/tournaments/{slug}/series", "post"),
            ("/api/v1/tournaments/{slug}/series/{series}", "patch"),
            ("/api/v1/tournaments/{slug}/series/{series}", "delete"),
            ("/api/v1/tournaments/{slug}/teams", "post"),
            ("/api/v1/tournaments/{slug}/teams/{number}", "patch"),
            ("/api/v1/tournaments/{slug}/teams/{number}", "delete"),
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
            ("/api/v1/tournaments/{slug}/series", "get"),
            ("/api/v1/tournaments/{slug}/series/{series}", "get"),
            ("/api/v1/tournaments/{slug}/teams", "get"),
            ("/api/v1/tournaments/{slug}/teams/{number}", "get"),
        ] {
            assert_eq!(
                security(path, method),
                optional,
                "{method} {path} is public, the token shows drafts"
            );
        }
    }

    #[test]
    fn the_tournaments_routes_keep_a_closed_list_and_the_newer_ones_an_open_one() {
        let document: serde_json::Value =
            serde_json::from_str(&document()).expect("should be JSON");
        let schemas = &document["components"]["schemas"];
        let frozen: Vec<&str> = FROZEN.iter().map(|kind| kind.uri()).collect();
        assert_eq!(
            schemas["ProblemType"]["enum"],
            serde_json::json!(frozen),
            "the closed enum of the tournaments routes never grows"
        );
        let all: Vec<&str> = ProblemType::ALL.iter().map(|kind| kind.uri()).collect();
        assert_eq!(
            schemas["ExtensibleProblemType"]["x-extensible-enum"],
            serde_json::json!(all),
            "every type, as an extensible enum"
        );
        assert!(
            schemas["ExtensibleProblemType"].get("enum").is_none(),
            "no closed enum: a new type stays additive"
        );
        assert_eq!(
            schemas["ExtensibleProblemDocument"]["properties"]["type"]["$ref"],
            "#/components/schemas/ExtensibleProblemType",
            "the document of the newer routes refers to the open list"
        );
        for (path, item) in document["paths"].as_object().expect("should have paths") {
            for (method, operation) in item.as_object().expect("should be an object") {
                let conflict = operation["responses"]["409"]["$ref"].as_str();
                if path.contains("/series") || path.contains("/teams") {
                    assert!(
                        conflict.is_none_or(|r| r.ends_with("/SeriesAndTeamsConflict")),
                        "{method} {path}: its 409 lists the open types"
                    );
                } else {
                    assert!(
                        conflict.is_none_or(|r| r.ends_with("/Conflict")),
                        "{method} {path}: its 409 keeps the closed types"
                    );
                }
            }
        }
    }
}
