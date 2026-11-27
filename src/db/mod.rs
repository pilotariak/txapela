// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! D1 queries, one module per table, returning `domain` types.
//!
//! The Worker reaches the `pilotariak-txapela` database through the `DB` binding of
//! wrangler.toml. `api` calls this module, and this module uses `domain`, never `api`. The tables
//! come with the resources that need them, each one in a migration of `database/migrations/`.
//!
//! Values are bound as plain JavaScript values: text and civil dates as strings, integers and
//! reals as numbers. Writes that go together run in one [`batch`], a transaction.

pub(crate) mod courts;
pub(crate) mod tournament_specialties;
pub(crate) mod tournaments;

use jiff::civil::Date;
use worker::wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, Error, Result};

use crate::domain::civil::parse_date;
use crate::domain::tournament::MAX_INTEGER;

/// A text parameter.
pub(crate) fn text(value: &str) -> JsValue {
    JsValue::from_str(value)
}

/// An integer parameter. D1 takes JavaScript numbers, not `BigInt`: `domain` keeps every stored
/// integer at most [`MAX_INTEGER`], so the number is exact.
#[expect(
    clippy::cast_precision_loss,
    reason = "domain keeps stored integers within 2^53 - 1, which f64 holds exactly"
)]
pub(crate) fn int(value: i64) -> JsValue {
    debug_assert!(
        value.abs() <= MAX_INTEGER,
        "an integer D1 cannot store exactly"
    );
    JsValue::from_f64(value as f64)
}

/// A REAL parameter.
pub(crate) fn real(value: f64) -> JsValue {
    JsValue::from_f64(value)
}

/// A civil date parameter, `YYYY-MM-DD`.
pub(crate) fn date(value: Date) -> JsValue {
    text(&value.to_string())
}

/// A row that the code should not have written. The message names the column, never its value.
pub(crate) fn corrupt(column: &str) -> Error {
    Error::RustError(format!("unexpected value in column {column}"))
}

/// A stored `YYYY-MM-DD` date.
pub(crate) fn read_date(column: &str, value: &str) -> Result<Date> {
    parse_date(value).ok_or_else(|| corrupt(column))
}

/// A statement with its parameters bound.
pub(crate) fn statement(
    db: &D1Database,
    sql: &str,
    values: &[JsValue],
) -> Result<D1PreparedStatement> {
    db.prepare(sql).bind(values)
}

/// Runs statements in one transaction: if one fails, none applies.
pub(crate) async fn batch(
    db: &D1Database,
    statements: Vec<D1PreparedStatement>,
) -> Result<Vec<D1Result>> {
    db.batch(statements).await
}

/// Whether a D1 error is the UNIQUE constraint of `table.column`, such as `tournaments.slug`.
/// The message names the constraint, never a bound value.
pub(crate) fn is_unique_violation(error: &Error, column: &str) -> bool {
    error
        .to_string()
        .contains(&format!("UNIQUE constraint failed: {column}"))
}

/// Which tournaments a read loads, as a `WHERE` clause on `tournaments t` and its parameters.
pub(crate) struct Selection {
    filter: &'static str,
    values: Vec<JsValue>,
}

impl Selection {
    /// The tournament of a slug, whatever its status.
    pub(crate) fn slug(slug: &str) -> Self {
        Self {
            filter: "t.slug = ?",
            values: vec![text(slug)],
        }
    }

    /// Every tournament, drafts included.
    pub(crate) fn all() -> Self {
        Self {
            filter: "1 = 1",
            values: Vec::new(),
        }
    }

    /// Every tournament whose status is not `status`: the public list leaves drafts out.
    pub(crate) fn all_but(status: &str) -> Self {
        Self {
            filter: "t.status <> ?",
            values: vec![text(status)],
        }
    }

    /// A statement of `sql`, whose `{filter}` is replaced by this selection.
    fn statement(&self, db: &D1Database, sql: &str) -> Result<D1PreparedStatement> {
        statement(db, &sql.replace("{filter}", self.filter), &self.values)
    }
}
