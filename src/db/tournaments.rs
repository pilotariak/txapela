// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `tournaments`, and the reads and writes of a whole tournament with its `courts` and its
//! `tournament_specialties`.
//!
//! The INTEGER PRIMARY KEY of a tournament never leaves D1: every statement finds a tournament by
//! its slug, the public key.

use std::collections::HashMap;

use serde::Deserialize;
use worker::{D1Database, D1PreparedStatement, Result};

use super::{
    Guard, Selection, batch, corrupt, courts, date, int, read_date, real, statement, text,
    tournament_specialties, written,
};
use crate::domain::slug::Slug;
use crate::domain::status::Status;
use crate::domain::team::TeamStatus;
use crate::domain::tournament::{Dates, Points, SpecialtyTarget, Tournament, Venue};

const COLUMNS: &str = "slug, name, organizing_club, venue_name, venue_address, venue_latitude, \
    venue_longitude, aire, points_win, points_loss, points_forfeit, ranking, registration_opens, \
    registration_closes, first_match, finals, fee_cents, max_teams, status";

#[derive(Deserialize)]
struct Row {
    slug: String,
    name: String,
    organizing_club: String,
    venue_name: String,
    venue_address: String,
    venue_latitude: f64,
    venue_longitude: f64,
    aire: String,
    points_win: i64,
    points_loss: i64,
    points_forfeit: i64,
    ranking: String,
    registration_opens: String,
    registration_closes: String,
    first_match: String,
    finals: String,
    fee_cents: i64,
    max_teams: i64,
    status: String,
}

impl Row {
    /// The tournament of this row, with its courts and its specialties.
    fn into_tournament(
        self,
        courts: Vec<String>,
        mut specialties: Vec<SpecialtyTarget>,
    ) -> Result<Tournament> {
        specialties.sort_by_key(|specialty| specialty.specialty);
        Ok(Tournament {
            slug: Slug::parse(&self.slug).ok_or_else(|| corrupt("tournaments.slug"))?,
            name: self.name,
            organizing_club: self.organizing_club,
            venue: Venue {
                name: self.venue_name,
                address: self.venue_address,
                latitude: self.venue_latitude,
                longitude: self.venue_longitude,
            },
            aire: self
                .aire
                .parse()
                .map_err(|()| corrupt("tournaments.aire"))?,
            courts,
            specialties,
            points: Points {
                win: self.points_win,
                loss: self.points_loss,
                forfeit: self.points_forfeit,
            },
            ranking: self
                .ranking
                .parse()
                .map_err(|()| corrupt("tournaments.ranking"))?,
            dates: Dates {
                registration_opens: read_date(
                    "tournaments.registration_opens",
                    &self.registration_opens,
                )?,
                registration_closes: read_date(
                    "tournaments.registration_closes",
                    &self.registration_closes,
                )?,
                first_match: read_date("tournaments.first_match", &self.first_match)?,
                finals: read_date("tournaments.finals", &self.finals)?,
            },
            fee_cents: self.fee_cents,
            max_teams: self.max_teams,
            status: self
                .status
                .parse()
                .map_err(|()| corrupt("tournaments.status"))?,
        })
    }
}

/// The selected tournaments, by first match date then slug, each with its courts and its
/// specialties. One batch: three reads of one snapshot, in one round trip.
async fn load(db: &D1Database, selection: &Selection) -> Result<Vec<Tournament>> {
    let results = batch(
        db,
        vec![
            selection.statement(
                db,
                &format!(
                    "SELECT {COLUMNS} FROM tournaments t WHERE {{filter}} \
                     ORDER BY t.first_match, t.slug"
                ),
            )?,
            courts::select(db, selection)?,
            tournament_specialties::select(db, selection)?,
        ],
    )
    .await?;
    let [tournaments, court_rows, specialty_rows] = results.as_slice() else {
        return Err(worker::Error::RustError(
            "a batch of three reads returned another number of results".to_owned(),
        ));
    };
    let mut courts_of: HashMap<String, Vec<String>> = HashMap::new();
    for row in court_rows.results::<courts::Row>()? {
        courts_of.entry(row.slug).or_default().push(row.name);
    }
    let mut specialties_of: HashMap<String, Vec<SpecialtyTarget>> = HashMap::new();
    for row in specialty_rows.results::<tournament_specialties::Row>()? {
        let slug = row.slug.clone();
        specialties_of
            .entry(slug)
            .or_default()
            .push(SpecialtyTarget::try_from(row)?);
    }
    tournaments
        .results::<Row>()?
        .into_iter()
        .map(|row| {
            let courts = courts_of.remove(&row.slug).unwrap_or_default();
            let specialties = specialties_of.remove(&row.slug).unwrap_or_default();
            row.into_tournament(courts, specialties)
        })
        .collect()
}

/// The tournament of a slug, whatever its status.
pub(crate) async fn find(db: &D1Database, slug: &Slug) -> Result<Option<Tournament>> {
    Ok(load(db, &Selection::slug(slug.as_str()))
        .await?
        .into_iter()
        .next())
}

/// Every tournament, by first match date then slug. Drafts only when `drafts` is set.
pub(crate) async fn list(db: &D1Database, drafts: bool) -> Result<Vec<Tournament>> {
    let selection = if drafts {
        Selection::all()
    } else {
        Selection::all_but(Status::Draft.as_str())
    };
    load(db, &selection).await
}

/// The values of a tournament's own row, in the order of [`COLUMNS`] after the slug.
fn values(tournament: &Tournament) -> [worker::wasm_bindgen::JsValue; 18] {
    [
        text(&tournament.name),
        text(&tournament.organizing_club),
        text(&tournament.venue.name),
        text(&tournament.venue.address),
        real(tournament.venue.latitude),
        real(tournament.venue.longitude),
        text(tournament.aire.as_str()),
        int(tournament.points.win),
        int(tournament.points.loss),
        int(tournament.points.forfeit),
        text(tournament.ranking.as_str()),
        date(tournament.dates.registration_opens),
        date(tournament.dates.registration_closes),
        date(tournament.dates.first_match),
        date(tournament.dates.finals),
        int(tournament.fee_cents),
        int(tournament.max_teams),
        text(tournament.status.as_str()),
    ]
}

fn insert(db: &D1Database, tournament: &Tournament) -> Result<D1PreparedStatement> {
    let sql = format!(
        "INSERT INTO tournaments ({COLUMNS}) VALUES ({})",
        ["?"; 19].join(", ")
    );
    let mut bound = vec![text(tournament.slug.as_str())];
    bound.extend(values(tournament));
    statement(db, &sql, &bound)
}

/// Inserts a tournament with its courts and its specialties, in one transaction. A slug that
/// exists fails on the UNIQUE constraint of `tournaments.slug`, and nothing is written.
pub(crate) async fn create(db: &D1Database, tournament: &Tournament) -> Result<()> {
    let guard = Guard::none();
    let mut statements = vec![insert(db, tournament)?];
    statements.extend(courts::insert(
        db,
        &tournament.slug,
        &tournament.courts,
        &guard,
    )?);
    statements.extend(tournament_specialties::insert(
        db,
        &tournament.slug,
        &tournament.specialties,
        &guard,
    )?);
    batch(db, statements).await?;
    Ok(())
}

/// The condition of a change that can break the series and teams of the tournament `g`
/// ([`Tournament::touches_entries`]): its registered teams fit `after.max_teams`, and no series
/// plays a specialty that `after` drops. It reads `teams` and `series` only, which a save never
/// writes.
fn entries_guard(after: &Tournament) -> Guard {
    let specialties = vec!["?"; after.specialties.len()].join(", ");
    let mut values = vec![text(TeamStatus::Registered.as_str()), int(after.max_teams)];
    values.extend(
        after
            .specialties
            .iter()
            .map(|specialty| text(specialty.specialty.as_str())),
    );
    Guard::new(
        format!(
            "(SELECT count(*) FROM teams r WHERE r.tournament_id = g.id AND r.status = ?) <= ? \
             AND NOT EXISTS (SELECT 1 FROM series x WHERE x.tournament_id = g.id \
             AND x.specialty NOT IN ({specialties}))"
        ),
        values,
    )
}

/// Writes a tournament after a change, in one transaction. Courts and specialties are replaced
/// only when they changed, which `domain` allows in draft only. A change that can break its
/// series and teams carries [`entries_guard`] in every statement: `false` when it does not
/// hold, and nothing is written.
pub(crate) async fn save(db: &D1Database, before: &Tournament, after: &Tournament) -> Result<bool> {
    let guard = if before.touches_entries(after) {
        entries_guard(after)
    } else {
        Guard::none()
    };
    let assignments = COLUMNS
        .split(", ")
        .skip(1)
        .map(|column| format!("{column} = ?"))
        .collect::<Vec<_>>()
        .join(", ");
    let (tournament, guard_values) = guard.tournament(after.slug.as_str());
    let mut bound = values(after).to_vec();
    bound.extend(guard_values);
    let mut statements = vec![statement(
        db,
        &format!("UPDATE tournaments SET {assignments} WHERE id IN (SELECT g.id {tournament})"),
        &bound,
    )?];
    if before.courts != after.courts {
        statements.push(courts::delete(db, &after.slug, &guard)?);
        statements.extend(courts::insert(db, &after.slug, &after.courts, &guard)?);
    }
    if before.specialties != after.specialties {
        statements.push(tournament_specialties::delete(db, &after.slug, &guard)?);
        statements.extend(tournament_specialties::insert(
            db,
            &after.slug,
            &after.specialties,
            &guard,
        )?);
    }
    written(&batch(db, statements).await?)
}

/// Deletes a tournament: its courts and its specialties go with it (ON DELETE CASCADE).
pub(crate) async fn delete(db: &D1Database, slug: &Slug) -> Result<()> {
    statement(
        db,
        "DELETE FROM tournaments WHERE slug = ?",
        &[text(slug.as_str())],
    )?
    .run()
    .await?;
    Ok(())
}
