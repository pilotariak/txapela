// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `players`: the two players of each team, in the organizer's order.
//!
//! `licence` and `contact` are personal data: this module reads them for the organizer views,
//! and the API leaves them out of public responses. A D1 error names a column, never a value.

use serde::Deserialize;
use worker::{D1Database, D1PreparedStatement, Result};

use super::{Selection, int, optional_text, statement, text};
use crate::domain::player::Player;
use crate::domain::slug::Slug;
use crate::domain::team::TeamStatus;

/// A player and the number of its team.
#[derive(Deserialize)]
pub(crate) struct Row {
    pub(crate) number: i64,
    first_name: String,
    last_name: String,
    club: Option<String>,
    licence: Option<String>,
    contact: Option<String>,
}

impl From<Row> for Player {
    fn from(row: Row) -> Self {
        Self {
            first_name: row.first_name,
            last_name: row.last_name,
            club: row.club,
            licence: row.licence,
            contact: row.contact,
        }
    }
}

/// The players of the selected teams, by team number then position.
pub(crate) fn select(db: &D1Database, selection: &Selection) -> Result<D1PreparedStatement> {
    selection.statement(
        db,
        "SELECT m.number AS number, p.first_name AS first_name, p.last_name AS last_name, \
         p.club AS club, p.licence AS licence, p.contact AS contact \
         FROM players p JOIN teams m ON m.id = p.team_id \
         JOIN tournaments t ON t.id = m.tournament_id \
         WHERE {filter} ORDER BY m.number, p.position",
    )
}

/// Inserts the players of a team, in order, each in a position the team has free. The team is
/// found by its tournament's slug and its number, so that these statements run in the batch that
/// inserts it. When a guarded write of that batch did not apply, they insert nothing: a team that
/// was not inserted has no row to find, and a team that already had this number keeps its own
/// players.
pub(crate) fn insert(
    db: &D1Database,
    tournament: &Slug,
    number: i64,
    players: &[Player],
) -> Result<Vec<D1PreparedStatement>> {
    players
        .iter()
        .zip(1_i64..)
        .map(|(player, position)| {
            statement(
                db,
                "INSERT INTO players (team_id, position, first_name, last_name, club, licence, \
                 contact) SELECT m.id, ?, ?, ?, ?, ?, ? FROM teams m \
                 JOIN tournaments t ON t.id = m.tournament_id WHERE t.slug = ? AND m.number = ? \
                 AND NOT EXISTS (SELECT 1 FROM players q WHERE q.team_id = m.id AND q.position = ?)",
                &[
                    int(position),
                    text(&player.first_name),
                    text(&player.last_name),
                    optional_text(player.club.as_deref()),
                    optional_text(player.licence.as_deref()),
                    optional_text(player.contact.as_deref()),
                    text(tournament.as_str()),
                    int(number),
                    int(position),
                ],
            )
        })
        .collect()
}

/// Deletes the players of a team whose status is `status`, before [`insert`] writes the new
/// ones. In the batch of a change, `status` is the one the change writes: when a guarded change
/// did not apply, the team keeps its players.
pub(crate) fn delete(
    db: &D1Database,
    tournament: &Slug,
    number: i64,
    status: TeamStatus,
) -> Result<D1PreparedStatement> {
    statement(
        db,
        "DELETE FROM players WHERE team_id = (SELECT m.id FROM teams m \
         JOIN tournaments t ON t.id = m.tournament_id \
         WHERE t.slug = ? AND m.number = ? AND m.status = ?)",
        &[
            text(tournament.as_str()),
            int(number),
            text(status.as_str()),
        ],
    )
}
