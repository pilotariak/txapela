// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `teams`, and the reads and writes of a whole team with its `players`.
//!
//! The INTEGER PRIMARY KEY of a team never leaves D1: every statement finds a team by the slug
//! of its tournament and its number, its public key within the tournament.
//!
//! A write that adds a registered team, a registration or a withdrawn team registered again,
//! carries its own condition ([`ROOM`]): D1 checks it in the statement that writes, so that two
//! requests cannot both take the last place. `domain` checks the same rules first and words the
//! problem; the condition closes the gap between that check and the write.

use std::collections::HashMap;

use serde::Deserialize;
use worker::wasm_bindgen::JsValue;
use worker::{D1Database, Result};

use super::{
    Selection, batch, boolean, corrupt, date, int, players, read_date, registration_open,
    statement, text, write, written,
};
use crate::domain::draw::Entrant;
use crate::domain::player::Player;
use crate::domain::series::BLOCK_SIZE;
use crate::domain::slug::Slug;
use crate::domain::status::Status;
use crate::domain::team::{Registered, Team, TeamStatus};

/// The condition of a write that adds a registered team to the series `s` of the tournament `t`:
/// registration is open, and the series and the tournament have a place left. Its parameters
/// are [`room_values`].
const ROOM: &str = "t.status IN (?, ?) \
    AND (SELECT count(*) FROM teams r WHERE r.series_id = s.id AND r.status = ?) < s.max_teams \
    AND (SELECT count(*) FROM teams r WHERE r.tournament_id = t.id AND r.status = ?) \
    < t.max_teams";

/// The parameters of [`ROOM`].
fn room_values() -> [JsValue; 4] {
    let [draft, registration] = Status::REGISTRATION_OPEN;
    [
        text(draft.as_str()),
        text(registration.as_str()),
        text(TeamStatus::Registered.as_str()),
        text(TeamStatus::Registered.as_str()),
    ]
}

#[derive(Deserialize)]
struct Row {
    series: String,
    number: i64,
    registered_on: String,
    fee_paid: i64,
    status: String,
}

impl Row {
    /// The team of this row, with its players.
    fn into_team(self, players: Vec<Player>) -> Result<Team> {
        Ok(Team {
            number: self.number,
            series: Slug::parse(&self.series).ok_or_else(|| corrupt("series.slug"))?,
            players: players.try_into().map_err(|_| corrupt("players.team_id"))?,
            registered_on: read_date("teams.registered_on", &self.registered_on)?,
            fee_paid: match self.fee_paid {
                0 => false,
                1 => true,
                _ => return Err(corrupt("teams.fee_paid")),
            },
            status: self.status.parse().map_err(|()| corrupt("teams.status"))?,
        })
    }
}

/// The selected teams, by number, each with its players. One batch: two reads of one snapshot,
/// in one round trip.
async fn load(db: &D1Database, selection: &Selection) -> Result<Vec<Team>> {
    let results = batch(
        db,
        vec![
            selection.statement(
                db,
                "SELECT s.slug AS series, m.number AS number, m.registered_on AS registered_on, \
                 m.fee_paid AS fee_paid, m.status AS status \
                 FROM teams m JOIN series s ON s.id = m.series_id \
                 JOIN tournaments t ON t.id = m.tournament_id \
                 WHERE {filter} ORDER BY m.number",
            )?,
            players::select(db, selection)?,
        ],
    )
    .await?;
    let [teams, player_rows] = results.as_slice() else {
        return Err(worker::Error::RustError(
            "a batch of two reads returned another number of results".to_owned(),
        ));
    };
    let mut players_of: HashMap<i64, Vec<Player>> = HashMap::new();
    for row in player_rows.results::<players::Row>()? {
        players_of.entry(row.number).or_default().push(row.into());
    }
    teams
        .results::<Row>()?
        .into_iter()
        .map(|row| {
            let players = players_of.remove(&row.number).unwrap_or_default();
            row.into_team(players)
        })
        .collect()
}

/// The teams of a tournament, by number.
pub(crate) async fn list(db: &D1Database, tournament: &Slug) -> Result<Vec<Team>> {
    load(db, &Selection::slug(tournament.as_str())).await
}

/// The team of a number in a tournament.
pub(crate) async fn find(db: &D1Database, tournament: &Slug, number: i64) -> Result<Option<Team>> {
    Ok(load(db, &Selection::team(tournament.as_str(), number))
        .await?
        .into_iter()
        .next())
}

/// Inserts a registered team with its players, in one transaction, if its number is in the
/// block of its series and [`ROOM`] holds. `false` when not: the block moved, registration
/// closed, or the series or the tournament filled up since `domain` checked, and nothing is
/// written. A number that another team of the tournament has
/// fails on the UNIQUE constraint of `teams.tournament_id, teams.number`, and nothing is written.
pub(crate) async fn create(db: &D1Database, tournament: &Slug, team: &Team) -> Result<bool> {
    let mut values = vec![
        int(team.number),
        date(team.registered_on),
        boolean(team.fee_paid),
        text(team.status.as_str()),
        text(tournament.as_str()),
        text(team.series.as_str()),
        int(team.number),
        int(BLOCK_SIZE - 1),
    ];
    values.extend(room_values());
    let mut statements = vec![statement(
        db,
        &format!(
            "INSERT INTO teams (tournament_id, series_id, number, registered_on, fee_paid, \
             status) SELECT t.id, s.id, ?, ?, ?, ? \
             FROM tournaments t JOIN series s ON s.tournament_id = t.id \
             WHERE t.slug = ? AND s.slug = ? \
             AND ? BETWEEN s.number_block AND s.number_block + ? AND {ROOM}"
        ),
        &values,
    )?];
    statements.extend(players::insert(db, tournament, team.number, &team.players)?);
    written(&batch(db, statements).await?)
}

/// Writes a team after a change, in one transaction. Its players are replaced only when they
/// changed. A withdrawn team registered again is written only if [`ROOM`] holds, and a team
/// that withdraws only while registration is open: `false` when not, and nothing is written.
pub(crate) async fn save(
    db: &D1Database,
    tournament: &Slug,
    before: &Team,
    after: &Team,
) -> Result<bool> {
    let mut values = vec![
        boolean(after.fee_paid),
        text(after.status.as_str()),
        text(tournament.as_str()),
        int(after.number),
    ];
    let takes_a_place =
        before.status == TeamStatus::Withdrawn && after.status == TeamStatus::Registered;
    let condition = if takes_a_place {
        values.extend(room_values());
        format!(
            " AND EXISTS (SELECT 1 FROM tournaments t JOIN series s ON s.tournament_id = t.id \
             WHERE t.id = teams.tournament_id AND s.id = teams.series_id AND {ROOM})"
        )
    } else if before.status != after.status {
        let (open, statuses) = registration_open("t.status");
        values.extend(statuses);
        format!(
            " AND EXISTS (SELECT 1 FROM tournaments t WHERE t.id = teams.tournament_id AND {open})"
        )
    } else {
        String::new()
    };
    let mut statements = vec![statement(
        db,
        &format!(
            "UPDATE teams SET fee_paid = ?, status = ? \
             WHERE tournament_id = (SELECT id FROM tournaments WHERE slug = ?) AND number = ?\
             {condition}"
        ),
        &values,
    )?];
    if before.players != after.players {
        statements.push(players::delete(db, tournament, after.number, after.status)?);
        statements.extend(players::insert(
            db,
            tournament,
            after.number,
            &after.players,
        )?);
    }
    written(&batch(db, statements).await?)
}

/// Deletes a team while the registration of its tournament is open: its players go with it
/// (ON DELETE CASCADE). `false` when registration closed, or the team is gone, and nothing is
/// written.
pub(crate) async fn delete(db: &D1Database, tournament: &Slug, number: i64) -> Result<bool> {
    let (open, statuses) = registration_open("t.status");
    let mut values = vec![text(tournament.as_str())];
    values.extend(statuses);
    values.push(int(number));
    write(statement(
        db,
        &format!(
            "DELETE FROM teams \
             WHERE tournament_id = (SELECT t.id FROM tournaments t WHERE t.slug = ? AND {open}) \
             AND number = ?"
        ),
        &values,
    )?)
    .await
}

#[derive(Deserialize)]
struct EntrantRow {
    number: i64,
    club: Option<String>,
}

/// The registered teams of a series and the clubs of their players, by number: what a draw
/// splits into pools. Withdrawn teams stay out of the pools.
pub(crate) async fn entrants(
    db: &D1Database,
    tournament: &Slug,
    series: &Slug,
) -> Result<Vec<Entrant>> {
    let rows = statement(
        db,
        "SELECT m.number AS number, p.club AS club FROM teams m \
         JOIN series s ON s.id = m.series_id JOIN tournaments t ON t.id = m.tournament_id \
         LEFT JOIN players p ON p.team_id = m.id \
         WHERE t.slug = ? AND s.slug = ? AND m.status = ? ORDER BY m.number, p.position",
        &[
            text(tournament.as_str()),
            text(series.as_str()),
            text(TeamStatus::Registered.as_str()),
        ],
    )?
    .all()
    .await?
    .results::<EntrantRow>()?;
    let mut clubs: Vec<(i64, Vec<String>)> = Vec::new();
    for row in rows {
        match clubs.last_mut() {
            Some((number, of_team)) if *number == row.number => of_team.extend(row.club),
            _ => clubs.push((row.number, row.club.into_iter().collect())),
        }
    }
    Ok(clubs
        .iter()
        .map(|(number, of_team)| Entrant::new(*number, of_team.iter().map(String::as_str)))
        .collect())
}

#[derive(Deserialize)]
struct RegisteredRow {
    series: i64,
    tournament: i64,
}

/// The registered teams of a series and of its tournament: what their maximums count.
pub(crate) async fn registered(
    db: &D1Database,
    tournament: &Slug,
    series: &Slug,
) -> Result<Registered> {
    let row = statement(
        db,
        "SELECT coalesce(sum(s.slug = ?), 0) AS series, count(*) AS tournament \
         FROM teams m JOIN series s ON s.id = m.series_id \
         JOIN tournaments t ON t.id = m.tournament_id \
         WHERE t.slug = ? AND m.status = ?",
        &[
            text(series.as_str()),
            text(tournament.as_str()),
            text(TeamStatus::Registered.as_str()),
        ],
    )?
    .first::<RegisteredRow>(None)
    .await?
    .unwrap_or(RegisteredRow {
        series: 0,
        tournament: 0,
    });
    Ok(Registered {
        series: row.series,
        tournament: row.tournament,
    })
}
