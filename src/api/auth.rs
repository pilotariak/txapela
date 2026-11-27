// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Who calls: the interim organizer check, until the ADR of #5 settles how organizers
//! authenticate.
//!
//! An organizer sends `Authorization: Bearer <token>`. txapela hashes the token with SHA-256 and
//! compares the lowercase hexadecimal with the `ORGANIZER_TOKEN_SHA256` Worker secret. It fails
//! closed: a missing or malformed secret, a missing or malformed header, or a wrong token
//! answers 401 `unauthorized`. No cookie, no account, no organizers table, no per-tournament
//! ownership. Neither the token nor its hash is ever logged.
//!
//! [`check`] is the only place that compares a token: the write routes call it through
//! [`organizer`], and the public reads through [`viewer`].

use worker::{Env, Request, console_error};

use super::problem::{ApiResult, Problem, ProblemType};
use crate::domain::token::{OrganizerToken, is_sha256_hex};

/// The Worker secret holding the SHA-256 of the organizer token, in lowercase hexadecimal.
/// The maintainer sets it with `wrangler secret put`; local development reads `.dev.vars`.
pub(super) const ORGANIZER_TOKEN_SHA256: &str = "ORGANIZER_TOKEN_SHA256";

/// Who reads a tournament.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Viewer {
    /// No `Authorization` header: drafts stay hidden.
    Public,
    /// A valid organizer token: drafts included.
    Organizer,
}

/// The expected hash: `None` when the secret is missing or is not a lowercase SHA-256, which
/// is logged without its value.
fn expected_hash(env: &Env) -> Option<String> {
    let Ok(secret) = env.secret(ORGANIZER_TOKEN_SHA256) else {
        console_error!(
            "the {ORGANIZER_TOKEN_SHA256} secret is not set: organizer calls answer 401"
        );
        return None;
    };
    let secret = secret.to_string();
    let secret = secret.trim();
    if !is_sha256_hex(secret) {
        console_error!(
            "the {ORGANIZER_TOKEN_SHA256} secret is not a lowercase SHA-256 in hexadecimal: \
             organizer calls answer 401"
        );
        return None;
    }
    Some(secret.to_owned())
}

/// Checks an `Authorization` header: 401 unless it carries the organizer token.
fn check(authorization: &str, env: &Env) -> ApiResult<()> {
    let unauthorized = || Problem::new(ProblemType::Unauthorized);
    let token = OrganizerToken::from_authorization(authorization).ok_or_else(unauthorized)?;
    let expected = expected_hash(env).ok_or_else(unauthorized)?;
    if token.matches(&expected) {
        Ok(())
    } else {
        Err(unauthorized())
    }
}

/// The organizer check of the write routes: 401 unless the request carries the organizer token.
pub(super) fn organizer(req: &Request, env: &Env) -> ApiResult<()> {
    match req.headers().get("Authorization")? {
        Some(authorization) => check(&authorization, env),
        None => Err(Problem::new(ProblemType::Unauthorized)),
    }
}

/// Who reads: [`Viewer::Public`] without an `Authorization` header, [`Viewer::Organizer`] with
/// the organizer token, and 401 with any other header, so that an organizer whose token is wrong
/// is told so instead of seeing drafts vanish.
pub(super) fn viewer(req: &Request, env: &Env) -> ApiResult<Viewer> {
    match req.headers().get("Authorization")? {
        Some(authorization) => check(&authorization, env).map(|()| Viewer::Organizer),
        None => Ok(Viewer::Public),
    }
}
