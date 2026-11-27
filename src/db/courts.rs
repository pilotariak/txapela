// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `courts`: the named courts of a tournament's venue, in the organizer's order.

use serde::Deserialize;
use worker::{D1Database, D1PreparedStatement, Result};

use super::{Guard, Selection, int, statement, text};
use crate::domain::slug::Slug;

/// A court and the slug of its tournament.
#[derive(Deserialize)]
pub(crate) struct Row {
    pub(crate) slug: String,
    pub(crate) name: String,
}

/// The courts of the selected tournaments, in order within each tournament.
pub(crate) fn select(db: &D1Database, selection: &Selection) -> Result<D1PreparedStatement> {
    selection.statement(
        db,
        "SELECT t.slug AS slug, c.name AS name FROM courts c \
         JOIN tournaments t ON t.id = c.tournament_id \
         WHERE {filter} ORDER BY t.slug, c.position",
    )
}

/// Inserts the courts of a tournament, in order. The tournament is found by its slug, so that
/// these statements run in the batch that inserts it, and only while `guard` holds.
pub(crate) fn insert(
    db: &D1Database,
    slug: &Slug,
    courts: &[String],
    guard: &Guard,
) -> Result<Vec<D1PreparedStatement>> {
    let (tournament, guard_values) = guard.tournament(slug.as_str());
    courts
        .iter()
        .zip(1_i64..)
        .map(|(name, position)| {
            let mut values = vec![int(position), text(name)];
            values.extend(guard_values.iter().cloned());
            statement(
                db,
                &format!(
                    "INSERT INTO courts (tournament_id, position, name) SELECT g.id, ?, ? {tournament}"
                ),
                &values,
            )
        })
        .collect()
}

/// Deletes the courts of a tournament, while `guard` holds, before [`insert`] writes the new
/// ones.
pub(crate) fn delete(db: &D1Database, slug: &Slug, guard: &Guard) -> Result<D1PreparedStatement> {
    let (tournament, values) = guard.tournament(slug.as_str());
    statement(
        db,
        &format!("DELETE FROM courts WHERE tournament_id IN (SELECT g.id {tournament})"),
        &values,
    )
}
