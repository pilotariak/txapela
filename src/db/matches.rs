// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! `matches`: the matches of the pools of a series, each with its state and the score of its
//! result.
//!
//! The INTEGER PRIMARY KEY of a match never leaves D1: statements find a match by its pool and
//! its number within the pool. Its two teams reference their places in its pool
//! (`pool_teams`), so that no match holds a team outside its pool.

use serde::Deserialize;
use serde_json::json;
use worker::wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, Result};

use super::pools::{Requires, chain, guard, series_id};
use super::{corrupt, int, statement, text, write};
use crate::domain::matches::{Match, MatchState, Side};
use crate::domain::pool::PoolName;
use crate::domain::slug::Slug;

/// A match, with the number of its pool and of its teams.
#[derive(Deserialize)]
pub(crate) struct Row {
    pool: i64,
    phase: String,
    number: i64,
    round: i64,
    team_a: i64,
    team_b: i64,
    state: String,
    score_a: Option<i64>,
    score_b: Option<i64>,
    forfeit: Option<String>,
}

impl TryFrom<Row> for Match {
    type Error = worker::Error;

    fn try_from(row: Row) -> Result<Self> {
        let score = match (row.score_a, row.score_b) {
            (Some(a), Some(b)) => Some([a, b]),
            (None, None) => None,
            _ => return Err(corrupt("matches.score_b")),
        };
        let forfeit = row
            .forfeit
            .map(|side| {
                side.parse::<Side>()
                    .map_err(|()| corrupt("matches.forfeit"))
            })
            .transpose()?;
        Ok(Self {
            phase: row.phase.parse().map_err(|()| corrupt("matches.phase"))?,
            pool: PoolName::from_number(row.pool).ok_or_else(|| corrupt("pools.number"))?,
            number: row.number,
            round: row.round,
            teams: [row.team_a, row.team_b],
            state: MatchState::from_columns(&row.state, score, forfeit)
                .ok_or_else(|| corrupt("matches.state"))?,
        })
    }
}

/// The matches of the pools of a series (`keys`: the slugs of the tournament and the series), by
/// pool then number.
pub(crate) fn select(db: &D1Database, keys: &[JsValue]) -> Result<D1PreparedStatement> {
    statement(
        db,
        "SELECT p.number AS pool, x.phase AS phase, x.number AS number, x.round AS round, \
         a.number AS team_a, b.number AS team_b, x.state AS state, x.score_a AS score_a, \
         x.score_b AS score_b, x.forfeit AS forfeit \
         FROM matches x JOIN pools p ON p.id = x.pool_id \
         JOIN teams a ON a.id = x.team_a_id JOIN teams b ON b.id = x.team_b_id \
         JOIN series s ON s.id = x.series_id JOIN tournaments t ON t.id = s.tournament_id \
         WHERE t.slug = ? AND s.slug = ? ORDER BY p.number, x.number",
        keys,
    )
}

/// Inserts scheduled matches in the pools of the series `ids` selects, rows
/// `[pool, phase, number, round, team a, team b]` read with `json_each`. Each team is found in
/// the pool of its match: a team outside it inserts no row, and the foreign keys of `matches`
/// refuse it too.
pub(crate) fn insert(
    db: &D1Database,
    ids: &str,
    values: &[JsValue],
    games: &[Match],
) -> Result<D1PreparedStatement> {
    let rows: Vec<_> = games
        .iter()
        .map(|game| {
            json!([
                game.pool.number(),
                game.phase.as_str(),
                game.number,
                game.round,
                game.teams[0],
                game.teams[1]
            ])
        })
        .collect();
    let mut bound = vec![
        text(MatchState::Scheduled.as_str()),
        super::json(&json!(rows)),
    ];
    bound.extend(values.iter().cloned());
    statement(
        db,
        &format!(
            "INSERT INTO matches (series_id, phase, pool_id, number, round, team_a_id, team_b_id, \
             state) SELECT p.series_id, json_extract(j.value, '$[1]'), p.id, \
             json_extract(j.value, '$[2]'), json_extract(j.value, '$[3]'), a.team_id, b.team_id, ? \
             FROM json_each(?) j JOIN pools p ON p.number = json_extract(j.value, '$[0]') \
             JOIN pool_teams a ON a.pool_id = p.id AND a.team_id = (SELECT m.id FROM teams m \
             WHERE m.series_id = p.series_id AND m.number = json_extract(j.value, '$[4]')) \
             JOIN pool_teams b ON b.pool_id = p.id AND b.team_id = (SELECT m.id FROM teams m \
             WHERE m.series_id = p.series_id AND m.number = json_extract(j.value, '$[5]')) \
             WHERE p.series_id IN ({ids})"
        ),
        &bound,
    )
}

/// Deletes the matches of two pools of the series `ids` selects, before a move writes them
/// again.
pub(crate) fn delete_in(
    db: &D1Database,
    ids: &str,
    values: &[JsValue],
    pools: [PoolName; 2],
) -> Result<D1PreparedStatement> {
    statement(
        db,
        &format!(
            "DELETE FROM matches WHERE pool_id IN (SELECT p.id FROM pools p \
             WHERE p.series_id IN ({ids}) AND p.number IN (?, ?))"
        ),
        &chain(values, pools.map(|pool| int(pool.number()))),
    )
}

/// Writes the state of a match after a result, while the tournament is `pools` and the match
/// still opposes the teams the route read: `false` when not, or the match is gone, and nothing
/// is written.
pub(crate) async fn record(
    db: &D1Database,
    tournament: &Slug,
    series: &Slug,
    game: &Match,
    state: MatchState,
) -> Result<bool> {
    let guard = guard(series, Requires::default());
    let (ids, values) = series_id(&guard, tournament, series);
    let score = state.score();
    let score_value = |index: usize| score.map_or(JsValue::NULL, |score| int(score[index]));
    let mut bound = vec![
        text(state.as_str()),
        score_value(0),
        score_value(1),
        state
            .forfeit()
            .map_or(JsValue::NULL, |side| text(side.as_str())),
    ];
    bound.extend(values);
    bound.extend([
        int(game.pool.number()),
        int(game.number),
        int(game.teams[0]),
        int(game.teams[1]),
    ]);
    write(statement(
        db,
        &format!(
            "UPDATE matches SET state = ?, score_a = ?, score_b = ?, forfeit = ? \
             WHERE pool_id = (SELECT p.id FROM pools p WHERE p.series_id IN ({ids}) \
             AND p.number = ?) AND number = ? \
             AND team_a_id = (SELECT m.id FROM teams m WHERE m.series_id = matches.series_id \
             AND m.number = ?) \
             AND team_b_id = (SELECT m.id FROM teams m WHERE m.series_id = matches.series_id \
             AND m.number = ?)"
        ),
        &bound,
    )?)
    .await
}
