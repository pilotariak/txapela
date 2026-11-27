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
pub(crate) mod players;
pub(crate) mod series;
pub(crate) mod teams;
pub(crate) mod tournament_specialties;
pub(crate) mod tournaments;

use jiff::civil::Date;
use worker::wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, Error, Result};

use crate::domain::civil::parse_date;
use crate::domain::rule::MAX_INTEGER;
use crate::domain::status::Status;

/// A text parameter.
pub(crate) fn text(value: &str) -> JsValue {
    JsValue::from_str(value)
}

/// A nullable text parameter: `NULL` when absent.
pub(crate) fn optional_text(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, text)
}

/// A boolean parameter, stored as the INTEGER 0 or 1.
pub(crate) fn boolean(value: bool) -> JsValue {
    int(i64::from(value))
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

/// Whether the first statement of a batch wrote a row: `false` when its condition did not hold.
/// A write that takes a place, or that lowers a limit other rows depend on, carries its
/// condition in its SQL, so that D1 checks it in the statement that writes: a check of `domain`
/// followed by a separate write lets a concurrent request slip between them.
pub(crate) fn written(results: &[D1Result]) -> Result<bool> {
    results
        .first()
        .and_then(|result| result.meta().ok().flatten())
        .and_then(|meta| meta.changes)
        .map(|changes| changes > 0)
        .ok_or_else(|| {
            Error::RustError("D1 returned no change count for a guarded write".to_owned())
        })
}

/// Runs one guarded statement: whether it wrote a row.
pub(crate) async fn write(statement: D1PreparedStatement) -> Result<bool> {
    let result = statement.run().await?;
    written(core::slice::from_ref(&result))
}

/// `<column> IN (?, ?)` with the statuses of an open registration ([`Status::REGISTRATION_OPEN`]),
/// and its parameters.
pub(crate) fn registration_open(column: &str) -> (String, Vec<JsValue>) {
    let statuses = Status::REGISTRATION_OPEN;
    (
        format!("{column} IN ({})", vec!["?"; statuses.len()].join(", ")),
        statuses
            .iter()
            .map(|status| text(status.as_str()))
            .collect(),
    )
}

/// A condition on the tournament `g` that every statement of a batch carries: as it reads only
/// rows that the batch does not write, it holds for all of them or for none, and the batch
/// applies whole or not at all.
pub(crate) struct Guard {
    condition: String,
    values: Vec<JsValue>,
}

impl Guard {
    /// No condition.
    pub(crate) fn none() -> Self {
        Self {
            condition: "1 = 1".to_owned(),
            values: Vec::new(),
        }
    }

    pub(crate) fn new(condition: String, values: Vec<JsValue>) -> Self {
        Self { condition, values }
    }

    /// `FROM tournaments g WHERE g.slug = ? AND <condition>` and its parameters, for a statement
    /// that reads the tournament of a slug only while the condition holds.
    pub(crate) fn tournament(&self, slug: &str) -> (String, Vec<JsValue>) {
        let mut values = vec![text(slug)];
        values.extend(self.values.iter().cloned());
        (
            format!(
                "FROM tournaments g WHERE g.slug = ? AND ({})",
                self.condition
            ),
            values,
        )
    }
}

/// Whether a D1 error is the UNIQUE constraint of `table.column`, such as `tournaments.slug`.
/// The message names the constraint, never a bound value.
pub(crate) fn is_unique_violation(error: &Error, column: &str) -> bool {
    error
        .to_string()
        .contains(&format!("UNIQUE constraint failed: {column}"))
}

/// Which tournaments a read loads, as a `WHERE` clause on `tournaments t` and its parameters.
/// The reads of teams and players also join `teams m`, which [`Selection::team`] filters on.
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

    /// The team of a number in the tournament of a slug.
    pub(crate) fn team(slug: &str, number: i64) -> Self {
        Self {
            filter: "t.slug = ? AND m.number = ?",
            values: vec![text(slug), int(number)],
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
