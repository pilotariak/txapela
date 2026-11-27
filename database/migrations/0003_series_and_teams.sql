-- SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
-- SPDX-License-Identifier: Apache-2.0

-- Migration number: 0003 	 2026-11-27T10:46:18.006Z

-- Series, their teams and the players of each team (pilotariak/txapela#7).
--
-- A series plays one specialty of its tournament at one level. Its slug is its public key within
-- the tournament. Its number block holds the numbers of its teams, number_block to
-- number_block + 99: a multiple of 100, one block per series, so that two blocks never overlap.
-- A team is a pair of players, and its number is its public key within the tournament. The
-- INTEGER PRIMARY KEYs never leave D1.
--
-- specialty and status are TEXT without CHECK, parsed by the Rust enums of src/domain/. A series
-- has no foreign key to tournament_specialties: a draft tournament rewrites those rows when a
-- target score changes, and src/domain/ keeps every series on a specialty its tournament declares.
-- Deleting a tournament deletes its series, teams and players. src/domain/ refuses to delete a
-- series that has teams, and the foreign key of teams refuses it too.
--
-- licence and contact are personal data, organizer only: no public route returns them.

CREATE TABLE series (
  id            INTEGER PRIMARY KEY,
  tournament_id INTEGER NOT NULL REFERENCES tournaments(id) ON DELETE CASCADE,
  slug          TEXT NOT NULL,
  name          TEXT NOT NULL,
  specialty     TEXT NOT NULL,                    -- 'paleta_gomme_creuse' or 'paleta_gomme_pleine'
  max_teams     INTEGER NOT NULL,
  position      INTEGER NOT NULL,                 -- display order
  number_block  INTEGER NOT NULL,                 -- 100, 200…: team numbers number_block to number_block + 99
  UNIQUE (tournament_id, slug),
  UNIQUE (tournament_id, number_block),
  UNIQUE (id, tournament_id)                      -- the parent key of teams (series_id, tournament_id)
);

-- A team belongs to a series of its own tournament: the foreign key holds both columns.
CREATE TABLE teams (
  id            INTEGER PRIMARY KEY,
  tournament_id INTEGER NOT NULL REFERENCES tournaments(id) ON DELETE CASCADE,
  series_id     INTEGER NOT NULL,
  number        INTEGER NOT NULL,
  registered_on TEXT NOT NULL,                    -- YYYY-MM-DD in Europe/Paris, set by txapela
  fee_paid      INTEGER NOT NULL,                 -- 0 or 1, set by the organizer
  status        TEXT NOT NULL,                    -- 'registered' or 'withdrawn'
  UNIQUE (tournament_id, number),
  FOREIGN KEY (series_id, tournament_id) REFERENCES series(id, tournament_id)
);
CREATE INDEX teams_series ON teams(series_id, tournament_id);

-- The two players of a team, in the organizer's order.
CREATE TABLE players (
  id         INTEGER PRIMARY KEY,
  team_id    INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  position   INTEGER NOT NULL,                    -- 1 or 2
  first_name TEXT NOT NULL,
  last_name  TEXT NOT NULL,
  club       TEXT,
  licence    TEXT,                                -- organizer only
  contact    TEXT,                                -- phone or email, organizer only
  UNIQUE (team_id, position)
);
