// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `GET /`: the landing page, plain HTML with inline CSS and script, no build step.

use std::sync::LazyLock;

use worker::{Response, Result};

const LANDING_TEMPLATE: &str = include_str!("landing.html");

/// Replaced by the crate version, which release-please bumps in Cargo.toml.
const VERSION_PLACEHOLDER: &str = "__TXAPELA_VERSION__";

/// Azkena's policy plus `connect-src 'self'`, since the status script fetches /api/v1/health.
/// Everything is inline except Google Fonts, hence `'unsafe-inline'` for styles and script.
const LANDING_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; \
    style-src 'unsafe-inline' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; \
    img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'";

/// Unlike the API, the landing page holds no private data, so it may be cached briefly.
const LANDING_CACHE_CONTROL: &str = "public, max-age=300";

/// The landing page with the crate version, built once per isolate instead of on every request.
static LANDING_HTML: LazyLock<String> =
    LazyLock::new(|| LANDING_TEMPLATE.replace(VERSION_PLACEHOLDER, env!("CARGO_PKG_VERSION")));

/// Answers the landing page with its security and cache headers.
pub(super) fn get() -> Result<Response> {
    // from_html sets Content-Type: text/html; charset=utf-8.
    let mut response = Response::from_html(LANDING_HTML.as_str())?;
    let headers = response.headers_mut();
    headers.set("Content-Security-Policy", LANDING_CSP)?;
    headers.set("Cache-Control", LANDING_CACHE_CONTROL)?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::{LANDING_HTML, LANDING_TEMPLATE, VERSION_PLACEHOLDER};

    #[test]
    fn landing_html_shows_the_crate_version() {
        assert!(
            LANDING_TEMPLATE.contains(VERSION_PLACEHOLDER),
            "landing.html should carry the version placeholder"
        );
        assert!(
            !LANDING_HTML.contains(VERSION_PLACEHOLDER),
            "every version placeholder should be replaced"
        );
        assert!(
            LANDING_HTML.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))),
            "the page should show the crate version"
        );
    }
}
