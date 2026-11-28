// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `pools`, and the reads and writes of the pools of a series with their `pool_teams` and their
//! `matches`.
//!
//! The INTEGER PRIMARY KEY of a pool never leaves D1: every statement finds a pool by the slugs
//! of its tournament and series and its number, 1 for pool A.
//!
//! A write of the pools carries its condition ([`guard`]) in every statement of its batch: the
//! tournament is `pools`, and for a draw or a move the series has no result and, for a move, the
//! pools are at the revision the route read. The batch of a draw or a move ends with the
//! statement that adds one to `series.pools_revision`, under the same guard: whether it wrote
//! ([`written_last`]) says whether the batch applied. A move generates again the matches of the
//! pools it touches from the teams the route read: the revision makes D1 refuse it when another
//! draw or move changed those teams in between.

use std::collections::HashMap;

use serde::Deserialize;
use worker::wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, Result};

use super::{
    Guard, batch, corrupt, int, matches, players, pool_teams, statement, text, written_last,
};
use crate::domain::matches::{Match, MatchState};
use crate::domain::player::Player;
use crate::domain::pool::{Move, Pool, PoolName, SeriesPools};
use crate::domain::slug::Slug;
use crate::domain::status::Status;

/// What a write of the pools requires of its series, besides a tournament in `pools`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Requires {
    /// The series has no result: a draw and a move.
    pub(crate) no_results: bool,
    /// The pools are at this revision: a move and a tiebreak, which depend on the teams of each
    /// pool.
    pub(crate) revision: Option<i64>,
}

/// The condition of a write of the pools of `series`, on the tournament `g`: it is `pools`, and
/// its series holds what `requires` asks. A draw or a move writes the matches it reads: it
/// deletes them only while the series has no result, and inserts none with a result, so the
/// condition holds for every statement of its batch or for none.
pub(crate) fn guard(series: &Slug, requires: Requires) -> Guard {
    let mut condition = "g.status = ? AND EXISTS (SELECT 1 FROM series gs \
        WHERE gs.tournament_id = g.id AND gs.slug = ?"
        .to_owned();
    let mut values = vec![text(Status::Pools.as_str()), text(series.as_str())];
    if requires.no_results {
        condition.push_str(
            " AND NOT EXISTS (SELECT 1 FROM matches gx WHERE gx.series_id = gs.id \
             AND gx.state IN (?, ?))",
        );
        values.extend(MatchState::RESULTS.map(text));
    }
    if let Some(revision) = requires.revision {
        condition.push_str(" AND gs.pools_revision = ?");
        values.push(int(revision));
    }
    condition.push(')');
    Guard::new(condition, values)
}

/// `SELECT s.id FROM series s WHERE …`: the id of the series of a slug in a tournament, while
/// `guard` holds, and its parameters. Statements write `… IN (<this>)`.
pub(crate) fn series_id(guard: &Guard, tournament: &Slug, series: &Slug) -> (String, Vec<JsValue>) {
    let (from, guard_values) = guard.tournament(tournament.as_str());
    let mut values = vec![text(series.as_str())];
    values.extend(guard_values);
    (
        format!(
            "SELECT s.id FROM series s WHERE s.slug = ? AND s.tournament_id IN (SELECT g.id {from})"
        ),
        values,
    )
}

/// `<values> ++ <more>`.
pub(crate) fn chain(values: &[JsValue], more: impl IntoIterator<Item = JsValue>) -> Vec<JsValue> {
    values.iter().cloned().chain(more).collect()
}

/// The last statement of a draw or a move: one more revision of the pools of the series.
fn next_revision(db: &D1Database, ids: &str, values: &[JsValue]) -> Result<D1PreparedStatement> {
    statement(
        db,
        &format!("UPDATE series SET pools_revision = pools_revision + 1 WHERE id IN ({ids})"),
        values,
    )
}

/// The pools of a series, its teams and its matches, as D1 holds them now.
#[derive(Clone, Debug, Default)]
pub(crate) struct Drawn {
    pub(crate) pools: SeriesPools,
    /// The players of the teams in a pool, by team number, in the organizer's order. Their
    /// public fields only: these reads never select a licence or a contact.
    pub(crate) players: HashMap<i64, Vec<Player>>,
}

#[derive(Deserialize)]
struct RevisionRow {
    revision: i64,
}

#[derive(Deserialize)]
struct NumberRow {
    number: i64,
}

fn pool_name(number: i64) -> Result<PoolName> {
    PoolName::from_number(number).ok_or_else(|| corrupt("pools.number"))
}

/// The pools of a series with their teams, the players of those teams and their matches. One
/// batch: five reads of one snapshot, in one round trip. An undrawn series has no pool.
pub(crate) async fn load(db: &D1Database, tournament: &Slug, series: &Slug) -> Result<Drawn> {
    let keys = [text(tournament.as_str()), text(series.as_str())];
    let results = batch(
        db,
        vec![
            statement(
                db,
                "SELECT s.pools_revision AS revision FROM series s \
                 JOIN tournaments t ON t.id = s.tournament_id WHERE t.slug = ? AND s.slug = ?",
                &keys,
            )?,
            statement(
                db,
                "SELECT p.number AS number FROM pools p JOIN series s ON s.id = p.series_id \
                 JOIN tournaments t ON t.id = s.tournament_id WHERE t.slug = ? AND s.slug = ? \
                 ORDER BY p.number",
                &keys,
            )?,
            pool_teams::select(db, &keys)?,
            players::select_in_pools(db, &keys)?,
            matches::select(db, &keys)?,
        ],
    )
    .await?;
    let [revision, pool_rows, team_rows, player_rows, match_rows] = results.as_slice() else {
        return Err(worker::Error::RustError(
            "a batch of five reads returned another number of results".to_owned(),
        ));
    };
    let mut pools = pool_rows
        .results::<NumberRow>()?
        .into_iter()
        .map(|row| {
            Ok(Pool {
                name: pool_name(row.number)?,
                teams: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    for row in team_rows.results::<pool_teams::Row>()? {
        let name = pool_name(row.pool)?;
        let pool = pools
            .iter_mut()
            .find(|pool| pool.name == name)
            .ok_or_else(|| corrupt("pool_teams.pool_id"))?;
        pool.teams.push(row.into());
    }
    let mut players: HashMap<i64, Vec<Player>> = HashMap::new();
    for row in player_rows.results::<players::PublicRow>()? {
        players.entry(row.number).or_default().push(row.into());
    }
    let matches = match_rows
        .results::<matches::Row>()?
        .into_iter()
        .map(Match::try_from)
        .collect::<Result<Vec<_>>>()?;
    Ok(Drawn {
        pools: SeriesPools {
            revision: revision
                .results::<RevisionRow>()?
                .first()
                .map_or(0, |row| row.revision),
            pools,
            matches,
        },
        players,
    })
}

/// Replaces the pools of a series with a draw, their teams and their matches, in one
/// transaction, while the tournament is `pools` and the series has no result: `false` when not,
/// and nothing is written.
pub(crate) async fn draw(
    db: &D1Database,
    tournament: &Slug,
    series: &Slug,
    pools: &[Pool],
    games: &[Match],
) -> Result<bool> {
    let guard = guard(
        series,
        Requires {
            no_results: true,
            revision: None,
        },
    );
    let (ids, values) = series_id(&guard, tournament, series);
    let numbers: Vec<i64> = pools.iter().map(|pool| pool.name.number()).collect();
    let statements = vec![
        // The teams and matches of the pools go with them (ON DELETE CASCADE).
        statement(
            db,
            &format!("DELETE FROM pools WHERE series_id IN ({ids})"),
            &values,
        )?,
        statement(
            db,
            &format!(
                "INSERT INTO pools (series_id, number) SELECT s.id, j.value \
                 FROM series s, json_each(?) j WHERE s.id IN ({ids})"
            ),
            &chain(
                &[super::json(&serde_json::json!(numbers))],
                values.iter().cloned(),
            ),
        )?,
        pool_teams::insert(db, &ids, &values, pools)?,
        matches::insert(db, &ids, &values, games)?,
        next_revision(db, &ids, &values)?,
    ];
    written_last(&batch(db, statements).await?)
}

/// Moves a team to another pool and writes again the matches of both pools, in one transaction,
/// while the tournament is `pools`, the series has no result and its pools are at `revision`:
/// `false` when not, and nothing is written. The tiebreak of both pools goes.
pub(crate) async fn move_team(
    db: &D1Database,
    tournament: &Slug,
    series: &Slug,
    revision: i64,
    moved: &Move,
) -> Result<bool> {
    let guard = guard(
        series,
        Requires {
            no_results: true,
            revision: Some(revision),
        },
    );
    let (ids, values) = series_id(&guard, tournament, series);
    let touched = [moved.from, moved.to];
    let statements = vec![
        matches::delete_in(db, &ids, &values, touched)?,
        pool_teams::move_team(db, &ids, &values, moved.team, moved.to)?,
        pool_teams::clear_tiebreak(db, &ids, &values, touched)?,
        matches::insert(db, &ids, &values, &moved.matches)?,
        next_revision(db, &ids, &values)?,
    ];
    written_last(&batch(db, statements).await?)
}
