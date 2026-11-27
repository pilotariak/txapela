-- SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
-- SPDX-License-Identifier: Apache-2.0

-- Migration number: 0002 	 2026-11-27T09:34:26.718Z

-- Tournaments, their courts and their specialties (pilotariak/txapela#6).
--
-- A tournament's INTEGER PRIMARY KEY never leaves D1: the slug is its public key. Dates are
-- civil dates YYYY-MM-DD in Europe/Paris. aire, specialty, status and ranking are TEXT without
-- CHECK, parsed by the Rust enums of src/domain/: a new variant (main nue, pala…) is a code
-- change, not a table rebuild. src/domain/ checks every rule before a write; the constraints
-- below are the ones SQLite can keep without a rebuild. D1 enforces foreign keys, so deleting a
-- tournament deletes its courts and its specialties.

CREATE TABLE tournaments (
  id                  INTEGER PRIMARY KEY,
  slug                TEXT NOT NULL UNIQUE,
  name                TEXT NOT NULL,
  organizing_club     TEXT NOT NULL,
  venue_name          TEXT NOT NULL,
  venue_address       TEXT NOT NULL,
  venue_latitude      REAL NOT NULL,
  venue_longitude     REAL NOT NULL,
  aire                TEXT NOT NULL,              -- 'fronton', 'mur_a_gauche' or 'trinquet'
  points_win          INTEGER NOT NULL,
  points_loss         INTEGER NOT NULL,
  points_forfeit      INTEGER NOT NULL,
  ranking             TEXT NOT NULL,              -- 'points_then_goal_average'
  registration_opens  TEXT NOT NULL,
  registration_closes TEXT NOT NULL,
  first_match         TEXT NOT NULL,
  finals              TEXT NOT NULL,
  fee_cents           INTEGER NOT NULL,           -- per team, shown only
  max_teams           INTEGER NOT NULL,
  status              TEXT NOT NULL               -- 'draft', 'registration', 'pools', 'finals' or 'finished'
);
CREATE INDEX tournaments_first_match ON tournaments(first_match, slug);   -- GET /tournaments

-- The named courts of the venue, in the order the organizer gave them. The calendar (#9) books
-- matches on them.
CREATE TABLE courts (
  id            INTEGER PRIMARY KEY,
  tournament_id INTEGER NOT NULL REFERENCES tournaments(id) ON DELETE CASCADE,
  position      INTEGER NOT NULL,
  name          TEXT NOT NULL,
  UNIQUE (tournament_id, position),
  UNIQUE (tournament_id, name)
);

-- The specialties of a tournament, each with the score a team must reach to win a match. Series
-- (#7) inherit it.
CREATE TABLE tournament_specialties (
  tournament_id INTEGER NOT NULL REFERENCES tournaments(id) ON DELETE CASCADE,
  specialty     TEXT NOT NULL,                    -- 'paleta_gomme_creuse' or 'paleta_gomme_pleine'
  target_score  INTEGER NOT NULL,
  PRIMARY KEY (tournament_id, specialty)
);
