// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `pool_teams`: the team of each place in a pool, its seed and its place in the organizer's
//! tiebreak.
//!
//! A team is in one pool of its own series: the foreign keys hold the series on both sides, and
//! `team_id` is unique. Statements find a pool by its number and a team by its number, within
//! the series that [`super::pools::series_id`] selects while the guard of the write holds.

use serde::Deserialize;
use serde_json::json;
use worker::wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, Result};

use super::pools::{Requires, chain, guard, series_id};
use super::{int, statement, text, write};
use crate::domain::pool::{Pool, PoolName, PoolTeam};
use crate::domain::slug::Slug;
use crate::domain::team::TeamStatus;

/// A team of a pool, and the number of its pool.
#[derive(Deserialize)]
pub(crate) struct Row {
    pub(crate) pool: i64,
    number: i64,
    seed: i64,
    tiebreak: Option<i64>,
}

impl From<Row> for PoolTeam {
    fn from(row: Row) -> Self {
        Self {
            number: row.number,
            seed: row.seed,
            tiebreak: row.tiebreak,
        }
    }
}

/// The teams of the pools of a series (`keys`: the slugs of the tournament and the series), by
/// pool then seed.
pub(crate) fn select(db: &D1Database, keys: &[JsValue]) -> Result<D1PreparedStatement> {
    statement(
        db,
        "SELECT p.number AS pool, m.number AS number, pt.seed AS seed, pt.tiebreak AS tiebreak \
         FROM pool_teams pt JOIN pools p ON p.id = pt.pool_id JOIN teams m ON m.id = pt.team_id \
         JOIN series s ON s.id = pt.series_id JOIN tournaments t ON t.id = s.tournament_id \
         WHERE t.slug = ? AND s.slug = ? ORDER BY p.number, pt.seed",
        keys,
    )
}

/// Inserts the teams of drawn pools, in the series `ids` selects: registered teams only, rows
/// `[pool, team, seed]` read with `json_each`.
pub(crate) fn insert(
    db: &D1Database,
    ids: &str,
    values: &[JsValue],
    pools: &[Pool],
) -> Result<D1PreparedStatement> {
    let rows: Vec<_> = pools
        .iter()
        .flat_map(|pool| {
            pool.teams
                .iter()
                .map(move |team| json!([pool.name.number(), team.number, team.seed]))
        })
        .collect();
    let mut bound = vec![super::json(&json!(rows))];
    bound.extend(values.iter().cloned());
    bound.push(text(TeamStatus::Registered.as_str()));
    statement(
        db,
        &format!(
            "INSERT INTO pool_teams (pool_id, series_id, team_id, seed) \
             SELECT p.id, p.series_id, m.id, json_extract(j.value, '$[2]') FROM json_each(?) j \
             JOIN pools p ON p.number = json_extract(j.value, '$[0]') \
             JOIN teams m ON m.series_id = p.series_id AND m.number = json_extract(j.value, '$[1]') \
             WHERE p.series_id IN ({ids}) AND m.status = ?"
        ),
        &bound,
    )
}

/// Moves a team to the pool `to` of its series: it keeps its seed and loses its tiebreak.
pub(crate) fn move_team(
    db: &D1Database,
    ids: &str,
    values: &[JsValue],
    team: i64,
    to: PoolName,
) -> Result<D1PreparedStatement> {
    let mut bound = vec![int(to.number())];
    bound.extend(values.iter().cloned());
    bound.push(int(team));
    statement(
        db,
        &format!(
            "UPDATE pool_teams SET pool_id = (SELECT p.id FROM pools p \
             WHERE p.series_id = pool_teams.series_id AND p.number = ?), tiebreak = NULL \
             WHERE series_id IN ({ids}) AND team_id = (SELECT m.id FROM teams m \
             WHERE m.series_id = pool_teams.series_id AND m.number = ?)"
        ),
        &bound,
    )
}

/// Removes the tiebreak of two pools, which a move changes.
pub(crate) fn clear_tiebreak(
    db: &D1Database,
    ids: &str,
    values: &[JsValue],
    pools: [PoolName; 2],
) -> Result<D1PreparedStatement> {
    statement(
        db,
        &format!(
            "UPDATE pool_teams SET tiebreak = NULL WHERE pool_id IN (SELECT p.id FROM pools p \
             WHERE p.series_id IN ({ids}) AND p.number IN (?, ?))"
        ),
        &chain(values, pools.map(|pool| int(pool.number()))),
    )
}

/// Writes the organizer's order of a pool, `(team, place)` for each of its teams, or none to
/// remove it, while the tournament is `pools` and the pools of the series are at `revision`, so
/// that the order lists the teams the pool holds: `false` when not, and nothing is written.
pub(crate) async fn tiebreak(
    db: &D1Database,
    tournament: &Slug,
    series: &Slug,
    revision: i64,
    pool: PoolName,
    order: &[(i64, i64)],
) -> Result<bool> {
    let guard = guard(
        series,
        Requires {
            no_results: false,
            revision: Some(revision),
        },
    );
    let (ids, values) = series_id(&guard, tournament, series);
    let places: Vec<_> = order
        .iter()
        .map(|(team, place)| json!([team, place]))
        .collect();
    let mut bound = vec![super::json(&json!(places))];
    bound.extend(values);
    bound.push(int(pool.number()));
    write(statement(
        db,
        &format!(
            "UPDATE pool_teams SET tiebreak = (SELECT json_extract(j.value, '$[1]') \
             FROM json_each(?) j WHERE json_extract(j.value, '$[0]') = \
             (SELECT m.number FROM teams m WHERE m.id = pool_teams.team_id)) \
             WHERE pool_id = (SELECT p.id FROM pools p WHERE p.series_id IN ({ids}) \
             AND p.number = ?)"
        ),
        &bound,
    )?)
    .await
}
