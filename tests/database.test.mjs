// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The DB binding of wrangler.toml and the migrations of database/migrations/, in Miniflare: the
// tables they create, the foreign keys D1 enforces, and the cascade from a tournament to its
// courts, specialties, series, teams and players.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { applyMigrations, startWorker } from "./worker.mjs";

// Every table of the schema, with the query that counts the rows of one tournament (bound to its
// id). A new table must be added here, so that the deletion test covers it.
const BY_TOURNAMENT = {
  tournaments: "SELECT count(*) AS n FROM tournaments WHERE id = ?",
  courts: "SELECT count(*) AS n FROM courts WHERE tournament_id = ?",
  tournament_specialties: "SELECT count(*) AS n FROM tournament_specialties WHERE tournament_id = ?",
  series: "SELECT count(*) AS n FROM series WHERE tournament_id = ?",
  teams: "SELECT count(*) AS n FROM teams WHERE tournament_id = ?",
  players:
    "SELECT count(*) AS n FROM players WHERE team_id IN (SELECT id FROM teams WHERE tournament_id = ?)",
};

let mf;
let db;

before(async () => {
  mf = startWorker();
  db = await mf.getD1Database("DB");
  await applyMigrations(db);
});

after(async () => {
  await mf.dispose();
});

async function tables() {
  const { results } = await db
    .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
    .all();
  return results
    .map(({ name }) => name)
    .filter((name) => !name.startsWith("sqlite_") && !name.startsWith("_cf_"));
}

// A fictional tournament with two courts and two specialties. Returns its id.
async function seedTournament(slug) {
  const { meta } = await db
    .prepare(
      `INSERT INTO tournaments (slug, name, organizing_club, venue_name, venue_address, venue_latitude,
         venue_longitude, aire, points_win, points_loss, points_forfeit, ranking, registration_opens,
         registration_closes, first_match, finals, fee_cents, max_teams, status)
       VALUES (?, 'Open', 'Club Exemple', 'Trinquet Exemple', '1 rue Exemple', 43.0, -1.0, 'trinquet',
         3, 1, 0, 'points_then_goal_average', '2027-03-01', '2027-03-31', '2027-04-10', '2027-06-19',
         2000, 16, 'draft')`,
    )
    .bind(slug)
    .run();
  const id = meta.last_row_id;
  await db.batch([
    db.prepare("INSERT INTO courts (tournament_id, position, name) VALUES (?, 1, 'Trinquet A')").bind(id),
    db.prepare("INSERT INTO courts (tournament_id, position, name) VALUES (?, 2, 'Trinquet B')").bind(id),
    db
      .prepare("INSERT INTO tournament_specialties (tournament_id, specialty, target_score) VALUES (?, 'paleta_gomme_creuse', 40)")
      .bind(id),
    db
      .prepare("INSERT INTO tournament_specialties (tournament_id, specialty, target_score) VALUES (?, 'paleta_gomme_pleine', 30)")
      .bind(id),
  ]);
  return id;
}

// A series of a tournament. Returns its id.
async function seedSeries(tournamentId, slug, block) {
  const { meta } = await db
    .prepare(
      `INSERT INTO series (tournament_id, slug, name, specialty, max_teams, position, number_block)
       VALUES (?, ?, 'Série Exemple', 'paleta_gomme_creuse', 8, 1, ?)`,
    )
    .bind(tournamentId, slug, block)
    .run();
  return meta.last_row_id;
}

// A team of a series with two fictional players. Returns its id.
async function seedTeam(tournamentId, seriesId, number) {
  const { meta } = await db
    .prepare(
      `INSERT INTO teams (tournament_id, series_id, number, registered_on, fee_paid, status)
       VALUES (?, ?, ?, '2027-03-05', 0, 'registered')`,
    )
    .bind(tournamentId, seriesId, number)
    .run();
  const id = meta.last_row_id;
  await db.batch([
    db
      .prepare(
        "INSERT INTO players (team_id, position, first_name, last_name, club, licence, contact) VALUES (?, 1, 'Ane', 'Exemple', NULL, NULL, 'ane@example.org')",
      )
      .bind(id),
    db
      .prepare(
        "INSERT INTO players (team_id, position, first_name, last_name, club, licence, contact) VALUES (?, 2, 'Kepa', 'Fictif', 'Club Exemple', 'LIC-0002', NULL)",
      )
      .bind(id),
  ]);
  return id;
}

// A tournament with courts, specialties, two series and three teams. Returns its id.
async function seedEverything(slug) {
  const id = await seedTournament(slug);
  const first = await seedSeries(id, "1re-serie", 100);
  const second = await seedSeries(id, "2e-serie", 200);
  await seedTeam(id, first, 101);
  await seedTeam(id, first, 102);
  await seedTeam(id, second, 201);
  return id;
}

async function rowsOf(id) {
  const rows = {};
  for (const [table, sql] of Object.entries(BY_TOURNAMENT)) {
    rows[table] = (await db.prepare(sql).bind(id).first()).n;
  }
  return rows;
}

test("the migrations apply to the DB binding, which then answers queries", async () => {
  const row = await db.prepare("SELECT 1 AS one").first();
  assert.deepEqual(row, { one: 1 }, "the database answers after the migrations");
});

test("the migrations create every table of the schema", async () => {
  assert.deepEqual(
    await tables(),
    Object.keys(BY_TOURNAMENT).sort(),
    "tournaments, courts, tournament_specialties, series, teams, players",
  );
});

test("D1 enforces foreign keys and the unique keys of tournaments", async () => {
  assert.equal((await db.prepare("PRAGMA foreign_keys").first()).foreign_keys, 1, "foreign keys are on");
  await assert.rejects(
    db.prepare("INSERT INTO courts (tournament_id, position, name) VALUES (999999, 1, 'Orphan')").run(),
    /FOREIGN KEY constraint failed/,
    "a court needs its tournament",
  );
  await assert.rejects(
    db
      .prepare("INSERT INTO tournament_specialties (tournament_id, specialty, target_score) VALUES (999999, 'paleta_gomme_creuse', 40)")
      .run(),
    /FOREIGN KEY constraint failed/,
    "a specialty needs its tournament",
  );

  const id = await seedTournament("unique-keys");
  await assert.rejects(seedTournament("unique-keys"), /UNIQUE constraint failed: tournaments.slug/, "one slug per tournament");
  await assert.rejects(
    db.prepare("INSERT INTO courts (tournament_id, position, name) VALUES (?, 3, 'Trinquet A')").bind(id).run(),
    /UNIQUE constraint failed: courts.tournament_id, courts.name/,
    "court names are unique within a tournament",
  );
  await assert.rejects(
    db
      .prepare("INSERT INTO tournament_specialties (tournament_id, specialty, target_score) VALUES (?, 'paleta_gomme_creuse', 30)")
      .bind(id)
      .run(),
    /UNIQUE constraint failed/,
    "one target score per (tournament, specialty)",
  );
  // aire, specialty, status and ranking have no CHECK: a new variant needs no table rebuild.
  await db.prepare("UPDATE tournaments SET aire = 'future_aire', status = 'future_status' WHERE id = ?").bind(id).run();
});

test("D1 enforces the foreign keys and unique keys of series, teams and players", async () => {
  const id = await seedTournament("series-keys");
  const other = await seedTournament("series-keys-other");
  await assert.rejects(seedSeries(999999, "orphan", 100), /FOREIGN KEY constraint failed/, "a series needs its tournament");
  const series = await seedSeries(id, "1re-serie", 100);
  await assert.rejects(
    seedSeries(id, "1re-serie", 200),
    /UNIQUE constraint failed: series.tournament_id, series.slug/,
    "series slugs are unique within a tournament",
  );
  await assert.rejects(
    seedSeries(id, "2e-serie", 100),
    /UNIQUE constraint failed: series.tournament_id, series.number_block/,
    "one series per number block within a tournament",
  );
  await seedSeries(other, "1re-serie", 100);

  await assert.rejects(seedTeam(id, 999999, 101), /FOREIGN KEY constraint failed/, "a team needs its series");
  await assert.rejects(
    seedTeam(other, series, 101),
    /FOREIGN KEY constraint failed/,
    "a team belongs to a series of its own tournament",
  );
  const team = await seedTeam(id, series, 101);
  await assert.rejects(
    seedTeam(id, series, 101),
    /UNIQUE constraint failed: teams.tournament_id, teams.number/,
    "team numbers are unique within a tournament",
  );
  await assert.rejects(
    db
      .prepare("INSERT INTO players (team_id, position, first_name, last_name) VALUES (?, 1, 'Autre', 'Exemple')")
      .bind(team)
      .run(),
    /UNIQUE constraint failed: players.team_id, players.position/,
    "one player per position in a team",
  );
  await assert.rejects(
    db.prepare("INSERT INTO players (team_id, position, first_name, last_name) VALUES (999999, 1, 'Orphan', 'Exemple')").run(),
    /FOREIGN KEY constraint failed/,
    "a player needs its team",
  );
  await assert.rejects(
    db.prepare("DELETE FROM series WHERE id = ?").bind(series).run(),
    /FOREIGN KEY constraint failed/,
    "a series with teams cannot be deleted",
  );
  // specialty and status have no CHECK either.
  await db.prepare("UPDATE teams SET status = 'future_status' WHERE id = ?").bind(team).run();
  await db.prepare("UPDATE series SET specialty = 'future_specialty' WHERE id = ?").bind(series).run();
});

test("deleting a tournament deletes its courts, specialties, series, teams and players, and only them", async () => {
  const deleted = await seedEverything("cascade-deleted");
  const kept = await seedEverything("cascade-kept");
  const before = await rowsOf(deleted);
  assert.deepEqual(
    before,
    { tournaments: 1, courts: 2, tournament_specialties: 2, series: 2, teams: 3, players: 6 },
    "the seed has rows in every table",
  );
  const keptBefore = await rowsOf(kept);

  await db.prepare("DELETE FROM tournaments WHERE id = ?").bind(deleted).run();

  assert.deepEqual(
    await rowsOf(deleted),
    { tournaments: 0, courts: 0, tournament_specialties: 0, series: 0, teams: 0, players: 0 },
    "ON DELETE CASCADE leaves no row of the tournament",
  );
  assert.deepEqual(await rowsOf(kept), keptBefore, "the other tournament keeps its rows");
  const orphans = await db.prepare("SELECT count(*) AS n FROM players WHERE team_id NOT IN (SELECT id FROM teams)").first();
  assert.equal(orphans.n, 0, "no player is left without its team");
});

test("deleting a team deletes its players", async () => {
  const id = await seedTournament("team-cascade");
  const series = await seedSeries(id, "1re-serie", 100);
  const team = await seedTeam(id, series, 101);
  const kept = await seedTeam(id, series, 102);
  await db.prepare("DELETE FROM teams WHERE id = ?").bind(team).run();
  const count = async (teamId) => (await db.prepare("SELECT count(*) AS n FROM players WHERE team_id = ?").bind(teamId).first()).n;
  assert.equal(await count(team), 0, "its players went with it");
  assert.equal(await count(kept), 2, "the other team keeps its players");
});
