// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `series`: the series of a tournament, each found by the slug of its tournament and its own.
//!
//! The INTEGER PRIMARY KEY of a series never leaves D1. A series stores its specialty as TEXT,
//! without a foreign key to `tournament_specialties`: `domain` keeps it declared.

use serde::Deserialize;
use worker::{D1Database, Result};

use super::{batch, corrupt, int, registration_open, statement, text, write};
use crate::domain::series::{Series, SeriesTeams};
use crate::domain::slug::Slug;
use crate::domain::team::TeamStatus;
use crate::domain::tournament::{Entries, Specialty};

/// The columns of a series, read as `s`.
const SELECT: &str = "SELECT s.slug AS slug, s.name AS name, s.specialty AS specialty, \
    s.max_teams AS max_teams, s.position AS position, s.number_block AS number_block \
    FROM series s JOIN tournaments t ON t.id = s.tournament_id";

#[derive(Deserialize)]
struct Row {
    slug: String,
    name: String,
    specialty: String,
    max_teams: i64,
    position: i64,
    number_block: i64,
}

fn specialty(value: &str) -> Result<Specialty> {
    value.parse().map_err(|()| corrupt("series.specialty"))
}

impl TryFrom<Row> for Series {
    type Error = worker::Error;

    fn try_from(row: Row) -> Result<Self> {
        Ok(Self {
            slug: Slug::parse(&row.slug).ok_or_else(|| corrupt("series.slug"))?,
            name: row.name,
            specialty: specialty(&row.specialty)?,
            max_teams: row.max_teams,
            position: row.position,
            number_block: row.number_block,
        })
    }
}

/// The series of a tournament, by position then slug.
pub(crate) async fn list(db: &D1Database, tournament: &Slug) -> Result<Vec<Series>> {
    statement(
        db,
        &format!("{SELECT} WHERE t.slug = ? ORDER BY s.position, s.slug"),
        &[text(tournament.as_str())],
    )?
    .all()
    .await?
    .results::<Row>()?
    .into_iter()
    .map(Series::try_from)
    .collect()
}

/// The series of a slug in a tournament.
pub(crate) async fn find(
    db: &D1Database,
    tournament: &Slug,
    series: &Slug,
) -> Result<Option<Series>> {
    statement(
        db,
        &format!("{SELECT} WHERE t.slug = ? AND s.slug = ?"),
        &[text(tournament.as_str()), text(series.as_str())],
    )?
    .first::<Row>(None)
    .await?
    .map(Series::try_from)
    .transpose()
}

/// Inserts a series while the registration of its tournament is open and the tournament
/// declares its specialty: `false` when either does not hold, and nothing is written. A slug or
/// a number block that another series of the tournament has fails on the UNIQUE constraint of
/// `series.tournament_id, series.slug` or of `series.tournament_id, series.number_block`.
pub(crate) async fn create(db: &D1Database, tournament: &Slug, series: &Series) -> Result<bool> {
    let (open, statuses) = registration_open("t.status");
    let mut values = vec![
        text(series.slug.as_str()),
        text(&series.name),
        text(series.specialty.as_str()),
        int(series.max_teams),
        int(series.position),
        int(series.number_block),
        text(tournament.as_str()),
    ];
    values.extend(statuses);
    values.push(text(series.specialty.as_str()));
    write(statement(
        db,
        &format!(
            "INSERT INTO series (tournament_id, slug, name, specialty, max_teams, position, \
             number_block) SELECT t.id, ?, ?, ?, ?, ?, ? FROM tournaments t \
             WHERE t.slug = ? AND {open} AND EXISTS (SELECT 1 FROM tournament_specialties ts \
             WHERE ts.tournament_id = t.id AND ts.specialty = ?)"
        ),
        &values,
    )?)
    .await
}

/// Writes a series after a change. Its slug never changes. The statement carries the
/// conditions of what changes, checked by D1 as it writes: an open registration for the
/// specialty, the maximum or the block; a specialty the tournament declares; a maximum at least
/// the registered teams; no team under a block that moves. `false` when one does not hold, or
/// the series is gone, and nothing is written.
pub(crate) async fn save(
    db: &D1Database,
    tournament: &Slug,
    before: &Series,
    after: &Series,
) -> Result<bool> {
    let mut values = vec![
        text(&after.name),
        text(after.specialty.as_str()),
        int(after.max_teams),
        int(after.position),
        int(after.number_block),
        text(tournament.as_str()),
        text(after.slug.as_str()),
    ];
    let mut conditions = vec![
        "tournament_id = (SELECT id FROM tournaments WHERE slug = ?)".to_owned(),
        "slug = ?".to_owned(),
    ];
    let specialty = before.specialty != after.specialty;
    let block = before.number_block != after.number_block;
    if specialty || after.max_teams != before.max_teams || block {
        let (open, statuses) = registration_open("t.status");
        conditions.push(format!(
            "EXISTS (SELECT 1 FROM tournaments t WHERE t.id = series.tournament_id AND {open})"
        ));
        values.extend(statuses);
    }
    if specialty {
        conditions.push(
            "EXISTS (SELECT 1 FROM tournament_specialties ts \
             WHERE ts.tournament_id = series.tournament_id AND ts.specialty = ?)"
                .to_owned(),
        );
        values.push(text(after.specialty.as_str()));
    }
    if after.max_teams < before.max_teams {
        conditions.push(
            "(SELECT count(*) FROM teams r WHERE r.series_id = series.id AND r.status = ?) <= ?"
                .to_owned(),
        );
        values.push(text(TeamStatus::Registered.as_str()));
        values.push(int(after.max_teams));
    }
    if block {
        conditions
            .push("NOT EXISTS (SELECT 1 FROM teams r WHERE r.series_id = series.id)".to_owned());
    }
    let conditions = conditions.join(" AND ");
    write(statement(
        db,
        &format!(
            "UPDATE series SET name = ?, specialty = ?, max_teams = ?, position = ?, \
             number_block = ? WHERE {conditions}"
        ),
        &values,
    )?)
    .await
}

/// Deletes a series while the registration of its tournament is open and it has no team:
/// `false` when either does not hold, or the series is gone, and nothing is written. The
/// foreign key of `teams` refuses it too.
pub(crate) async fn delete(db: &D1Database, tournament: &Slug, series: &Slug) -> Result<bool> {
    let (open, statuses) = registration_open("t.status");
    let mut values = vec![text(tournament.as_str())];
    values.extend(statuses);
    values.push(text(series.as_str()));
    write(statement(
        db,
        &format!(
            "DELETE FROM series \
             WHERE tournament_id = (SELECT t.id FROM tournaments t WHERE t.slug = ? AND {open}) \
             AND slug = ? AND NOT EXISTS (SELECT 1 FROM teams r WHERE r.series_id = series.id)"
        ),
        &values,
    )?)
    .await
}

#[derive(Deserialize)]
struct TeamsRow {
    teams: i64,
    registered: i64,
}

/// The teams of a series: all of them, and the registered ones.
pub(crate) async fn teams(
    db: &D1Database,
    tournament: &Slug,
    series: &Slug,
) -> Result<SeriesTeams> {
    let row = statement(
        db,
        "SELECT count(m.id) AS teams, coalesce(sum(m.status = ?), 0) AS registered \
         FROM teams m JOIN series s ON s.id = m.series_id \
         JOIN tournaments t ON t.id = m.tournament_id \
         WHERE t.slug = ? AND s.slug = ?",
        &[
            text(TeamStatus::Registered.as_str()),
            text(tournament.as_str()),
            text(series.as_str()),
        ],
    )?
    .first::<TeamsRow>(None)
    .await?
    .unwrap_or(TeamsRow {
        teams: 0,
        registered: 0,
    });
    Ok(SeriesTeams {
        teams: row.teams,
        registered: row.registered,
    })
}

#[derive(Deserialize)]
struct SpecialtyRow {
    slug: String,
    specialty: String,
}

#[derive(Deserialize)]
struct CountRow {
    n: i64,
}

/// What the series and teams of a tournament hold: the specialty of each series and the
/// registered teams. One batch: two reads of one snapshot.
pub(crate) async fn entries(db: &D1Database, tournament: &Slug) -> Result<Entries> {
    let results = batch(
        db,
        vec![
            statement(
                db,
                "SELECT s.slug AS slug, s.specialty AS specialty \
                 FROM series s JOIN tournaments t ON t.id = s.tournament_id \
                 WHERE t.slug = ? ORDER BY s.slug",
                &[text(tournament.as_str())],
            )?,
            statement(
                db,
                "SELECT count(*) AS n FROM teams m JOIN tournaments t ON t.id = m.tournament_id \
                 WHERE t.slug = ? AND m.status = ?",
                &[
                    text(tournament.as_str()),
                    text(TeamStatus::Registered.as_str()),
                ],
            )?,
        ],
    )
    .await?;
    let [series, registered] = results.as_slice() else {
        return Err(worker::Error::RustError(
            "a batch of two reads returned another number of results".to_owned(),
        ));
    };
    let series = series
        .results::<SpecialtyRow>()?
        .into_iter()
        .map(|row| {
            Ok((
                Slug::parse(&row.slug).ok_or_else(|| corrupt("series.slug"))?,
                specialty(&row.specialty)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let registered_teams = registered
        .results::<CountRow>()?
        .first()
        .map_or(0, |row| row.n);
    Ok(Entries {
        series,
        registered_teams,
    })
}
