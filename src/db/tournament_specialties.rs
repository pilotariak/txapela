// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `tournament_specialties`: the specialties of a tournament, each with its target score.

use serde::Deserialize;
use worker::{D1Database, D1PreparedStatement, Result};

use super::{Guard, Selection, corrupt, int, statement, text};
use crate::domain::slug::Slug;
use crate::domain::tournament::SpecialtyTarget;

/// A specialty and the slug of its tournament.
#[derive(Deserialize)]
pub(crate) struct Row {
    pub(crate) slug: String,
    specialty: String,
    target_score: i64,
}

impl TryFrom<Row> for SpecialtyTarget {
    type Error = worker::Error;

    fn try_from(row: Row) -> Result<Self> {
        Ok(Self {
            specialty: row
                .specialty
                .parse()
                .map_err(|()| corrupt("tournament_specialties.specialty"))?,
            target_score: row.target_score,
        })
    }
}

/// The specialties of the selected tournaments. `domain` orders them.
pub(crate) fn select(db: &D1Database, selection: &Selection) -> Result<D1PreparedStatement> {
    selection.statement(
        db,
        "SELECT t.slug AS slug, s.specialty AS specialty, s.target_score AS target_score \
         FROM tournament_specialties s JOIN tournaments t ON t.id = s.tournament_id \
         WHERE {filter} ORDER BY t.slug, s.specialty",
    )
}

/// Inserts the specialties of a tournament, found by its slug, so that these statements run in
/// the batch that inserts it, and only while `guard` holds.
pub(crate) fn insert(
    db: &D1Database,
    slug: &Slug,
    specialties: &[SpecialtyTarget],
    guard: &Guard,
) -> Result<Vec<D1PreparedStatement>> {
    let (tournament, guard_values) = guard.tournament(slug.as_str());
    specialties
        .iter()
        .map(|specialty| {
            let mut values = vec![
                text(specialty.specialty.as_str()),
                int(specialty.target_score),
            ];
            values.extend(guard_values.iter().cloned());
            statement(
                db,
                &format!(
                    "INSERT INTO tournament_specialties (tournament_id, specialty, target_score) \
                     SELECT g.id, ?, ? {tournament}"
                ),
                &values,
            )
        })
        .collect()
}

/// Deletes the specialties of a tournament, while `guard` holds, before [`insert`] writes the new
/// ones.
pub(crate) fn delete(db: &D1Database, slug: &Slug, guard: &Guard) -> Result<D1PreparedStatement> {
    let (tournament, values) = guard.tournament(slug.as_str());
    statement(
        db,
        &format!(
            "DELETE FROM tournament_specialties WHERE tournament_id IN (SELECT g.id {tournament})"
        ),
        &values,
    )
}
