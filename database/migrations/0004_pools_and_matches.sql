-- SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
-- SPDX-License-Identifier: Apache-2.0

-- Migration number: 0004 	 2026-11-27T14:44:04.302Z

-- Pools, the teams of each pool and the matches of a series (pilotariak/txapela#8).
--
-- A draw splits the registered teams of a series into pools, named A, B, C… by their number
-- (1 is A, Frontis's pool 1), and generates the round robin of each pool. A pool, its number
-- within the series, and a match, its number within the pool, are the public keys: the INTEGER
-- PRIMARY KEYs never leave D1. Composite foreign keys keep each team in a pool of its own series,
-- and each match between two teams of its pool. Deleting a series, or a tournament, deletes its
-- pools, their teams and their matches.
--
-- phase, state and forfeit are TEXT without CHECK, parsed by the Rust enums of src/domain/: the
-- final phase (#10) is a code change, not a table rebuild. Standings are computed from the
-- matches on every read, never stored.

-- Every draw and move adds one, under the same guard as the rest of its batch, so that a write
-- that depends on the teams of the pools, a move or a tiebreak, applies only to the pools it read.
ALTER TABLE series ADD COLUMN pools_revision INTEGER NOT NULL DEFAULT 0;

-- The parent key of pool_teams (team_id, series_id): a team joins a pool of its own series.
CREATE UNIQUE INDEX teams_id_series ON teams(id, series_id);

CREATE TABLE pools (
  id        INTEGER PRIMARY KEY,
  series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
  number    INTEGER NOT NULL,                     -- 1, 2, 3…: pool A, B, C…
  UNIQUE (series_id, number),
  UNIQUE (id, series_id)                          -- the parent key of pool_teams and matches
);

-- The teams of each pool. A team is in one pool, of its own series.
CREATE TABLE pool_teams (
  pool_id   INTEGER NOT NULL,
  series_id INTEGER NOT NULL,
  team_id   INTEGER NOT NULL,
  seed      INTEGER NOT NULL,                     -- 1 for the first seed of the draw
  tiebreak  INTEGER,                              -- the organizer's order: for every team of the pool, or none
  PRIMARY KEY (pool_id, team_id),                 -- the parent key of matches (pool_id, team_a_id)
  UNIQUE (team_id),
  FOREIGN KEY (pool_id, series_id) REFERENCES pools(id, series_id) ON DELETE CASCADE,
  FOREIGN KEY (team_id, series_id) REFERENCES teams(id, series_id) ON DELETE CASCADE
);

-- The matches of each pool. A result records both scores in the order of the teams: a forfeit
-- gives the target score of the series to the other team and 0 to the team that forfeited.
CREATE TABLE matches (
  id        INTEGER PRIMARY KEY,
  series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
  phase     TEXT NOT NULL,                        -- 'pool'; the final phase comes with #10
  pool_id   INTEGER,                              -- the pool of a pool match
  number    INTEGER NOT NULL,                     -- 1, 2…: the public key within the pool
  round     INTEGER NOT NULL,                     -- 1, 2…: the round of the circle method
  team_a_id INTEGER NOT NULL,                     -- the better seed
  team_b_id INTEGER NOT NULL,
  state     TEXT NOT NULL,                        -- 'scheduled', 'played', 'forfeit' or 'postponed'
  score_a   INTEGER,                              -- NULL until played or forfeit
  score_b   INTEGER,
  forfeit   TEXT,                                 -- 'a' or 'b': the team that forfeited
  UNIQUE (pool_id, number),
  FOREIGN KEY (pool_id, series_id) REFERENCES pools(id, series_id) ON DELETE CASCADE,
  FOREIGN KEY (pool_id, team_a_id) REFERENCES pool_teams(pool_id, team_id) ON DELETE CASCADE,
  FOREIGN KEY (pool_id, team_b_id) REFERENCES pool_teams(pool_id, team_id) ON DELETE CASCADE,
  FOREIGN KEY (team_a_id, series_id) REFERENCES teams(id, series_id) ON DELETE CASCADE,
  FOREIGN KEY (team_b_id, series_id) REFERENCES teams(id, series_id) ON DELETE CASCADE
);
CREATE INDEX matches_series ON matches(series_id, state);
