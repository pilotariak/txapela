// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The DB binding of wrangler.toml and the migrations of database/migrations/, in Miniflare: the
// tables they create, the foreign keys D1 enforces, and the cascade from a tournament to its
// courts and specialties.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { applyMigrations, startWorker } from "./worker.mjs";

// Every table of the schema, by the column that ties its rows to a tournament. A new table must
// be added here, so that the deletion test covers it.
const BY_TOURNAMENT = {
  tournaments: "id",
  courts: "tournament_id",
  tournament_specialties: "tournament_id",
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

async function rowsOf(id) {
  const rows = {};
  for (const [table, column] of Object.entries(BY_TOURNAMENT)) {
    rows[table] = (await db.prepare(`SELECT count(*) AS n FROM ${table} WHERE ${column} = ?`).bind(id).first()).n;
  }
  return rows;
}

test("the migrations apply to the DB binding, which then answers queries", async () => {
  const row = await db.prepare("SELECT 1 AS one").first();
  assert.deepEqual(row, { one: 1 }, "the database answers after the migrations");
});

test("the migrations create every table of the schema", async () => {
  assert.deepEqual(await tables(), Object.keys(BY_TOURNAMENT).sort(), "tournaments, courts, tournament_specialties");
});

test("D1 enforces foreign keys and the unique keys", async () => {
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

test("deleting a tournament deletes its courts and specialties, and only them", async () => {
  const deleted = await seedTournament("cascade-deleted");
  const kept = await seedTournament("cascade-kept");
  const before = await rowsOf(deleted);
  assert.deepEqual(before, { tournaments: 1, courts: 2, tournament_specialties: 2 }, "the seed has rows in every table");
  const keptBefore = await rowsOf(kept);

  await db.prepare("DELETE FROM tournaments WHERE id = ?").bind(deleted).run();

  assert.deepEqual(
    await rowsOf(deleted),
    { tournaments: 0, courts: 0, tournament_specialties: 0 },
    "ON DELETE CASCADE leaves no row of the tournament",
  );
  assert.deepEqual(await rowsOf(kept), keptBefore, "the other tournament keeps its rows");
});
