// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `GET /api/v1/health`: liveness check.

use serde::Serialize;
use worker::{Response, Result};

/// Liveness of txapela.
#[derive(Serialize)]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(super) struct Health {
    #[cfg_attr(test, schema(example = "ok"))]
    status: &'static str,
}

/// Answers `{"status":"ok"}`. Needs no token.
#[cfg_attr(test, utoipa::path(
    get,
    path = "/api/v1/health",
    operation_id = "getHealth",
    tag = "health",
    security(()),
    responses((status = 200, description = "txapela is up", body = Health))
))]
pub(super) fn get() -> Result<Response> {
    Response::from_json(&Health { status: "ok" })
}
