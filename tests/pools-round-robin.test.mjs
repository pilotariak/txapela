// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Extra pools tests through the Worker: the round robin of an odd pool, the snake order over three
// pools, and the same-club rule across pots. Players and clubs are fictional.

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import {
  ORGANIZER_TOKEN,
  assertProblem,
  createSeries,
  createTournament,
  drawnTournament,
  recordResult,
  seriesPath,
  startApi,
} from "./worker.mjs";

let mf;
let call;

before(async () => {
  ({ mf, call } = await startApi());
});

after(async () => {
  await mf.dispose();
});

const asOrganizer = { token: ORGANIZER_TOKEN };

let lastSlug = 0;
function nextSlug() {
  lastSlug += 1;
  return `pools-round-robin-${lastSlug}`;
}

// The team numbers of each pool, by pool name.
function poolMembers(body) {
  return Object.fromEntries(body.pools.map((pool) => [pool.name, pool.teams.map((team) => team.number)]));
}

async function members(slug) {
  return poolMembers(await (await call("GET", seriesPath(slug, "/pools"))).json());
}

describe("round robin of a pool through the routes", () => {
  test("five teams: every pair meets once, five rounds, and each team rests once", async () => {
    const slug = nextSlug();
    const drawn = await drawnTournament(call, slug, { teams: 10, draw: { pool_size: 5 } });
    assert.deepEqual(
      drawn.pools.map((pool) => pool.name),
      ["A", "B"],
      "10 teams in two pools of five",
    );

    const { A: teamNumbers } = await members(slug);
    assert.deepEqual(teamNumbers, [101, 104, 105, 108, 109], "the snake puts these teams in A");
    const { matches } = await (await call("GET", seriesPath(slug, "/matches"))).json();
    const inA = matches.filter((match) => match.pool === "A");
    assert.equal(inA.length, 10, "five teams play ten matches");

    const pairs = new Set();
    const playing = new Map();
    for (const match of inA) {
      const [first, second] = match.teams.map((team) => team.number);
      assert.ok(teamNumbers.includes(first) && teamNumbers.includes(second), "both teams of pool A");
      pairs.add([first, second].sort((a, b) => a - b).join("-"));
      playing.set(match.round, [...(playing.get(match.round) ?? []), first, second]);
    }
    assert.equal(pairs.size, 10, "every pair of pool A meets exactly once");
    assert.deepEqual(
      [...playing.keys()].sort((a, b) => a - b),
      [1, 2, 3, 4, 5],
      "five rounds, numbered without a gap",
    );

    const byes = Object.fromEntries(teamNumbers.map((number) => [number, 0]));
    for (const [round, present] of playing) {
      assert.equal(present.length, 4, `round ${round}: four of the five teams play`);
      assert.equal(new Set(present).size, 4, `round ${round}: nobody plays twice`);
      for (const number of teamNumbers) {
        if (!present.includes(number)) byes[number] += 1;
      }
    }
    assert.deepEqual(
      byes,
      Object.fromEntries(teamNumbers.map((number) => [number, 1])),
      "with an odd number of teams, each team has exactly one bye",
    );
  });

  test("12 teams fill three pools in snake order, A B C then C B A", async () => {
    const slug = nextSlug();
    const drawn = await drawnTournament(call, slug, { teams: 12, draw: { pool_size: 4 } });
    assert.deepEqual(
      drawn.pools.map((pool) => pool.name),
      ["A", "B", "C"],
      "pools named A, B, C",
    );
    assert.deepEqual(
      drawn.pools.map((pool) => pool.teams.map((team) => team.seed)),
      [
        [1, 6, 7, 12],
        [2, 5, 8, 11],
        [3, 4, 9, 10],
      ],
      "each pot fills the pools in alternate directions",
    );
    assert.deepEqual(await members(slug), {
      A: [101, 106, 107, 112],
      B: [102, 105, 108, 111],
      C: [103, 104, 109, 110],
    });

    const { matches } = await (await call("GET", seriesPath(slug, "/matches"))).json();
    for (const pool of ["A", "B", "C"]) {
      const games = matches.filter((match) => match.pool === pool);
      const pairs = new Set(
        games.map((match) => match.teams.map((team) => team.number).sort((a, b) => a - b).join("-")),
      );
      assert.equal(games.length, 6, `pool ${pool}: six matches`);
      assert.equal(pairs.size, 6, `pool ${pool}: every pair once`);
    }
  });

  test("a team takes another pool of its pot when its snake pool already holds its club", async () => {
    const slug = nextSlug();
    const clubs = { 101: "Club Bleu", 105: "CLUB BLEU" };
    const drawn = await drawnTournament(call, slug, {
      teams: 8,
      draw: { pools: 2 },
      club: (number) => clubs[number],
    });
    // 101 (pot 1) and 105 (pot 3) both go to A in the snake. 105 swaps with 106 in its pot,
    // because A already holds a team of its club; clubs compare without case.
    assert.deepEqual(
      await members(slug),
      { A: [101, 104, 106, 108], B: [102, 103, 105, 107] },
      "the club rule swaps teams in different pots, keeping one team per pot in each pool",
    );
    assert.deepEqual(
      drawn.pools.map((pool) => pool.teams.map((team) => team.seed)),
      [
        [1, 4, 6, 8],
        [2, 3, 5, 7],
      ],
      "seeds stay in order inside each pool",
    );
  });
});

describe("when pools stop changing", () => {
  test("a forfeit is a result: a new draw is refused and the pools stay", async () => {
    const slug = nextSlug();
    const drawn = await drawnTournament(call, slug);
    const result = await recordResult(call, slug, "A", 1, { teams: [101, 108], forfeited_by: 101 });
    assert.equal(result.status, 200, "a forfeit recorded");
    await result.body?.cancel();

    const problem = await assertProblem(
      await call("POST", seriesPath(slug, "/draw"), { ...asOrganizer, body: { pool_size: 4 } }),
      409,
      "series-has-results",
    );
    assert.equal(problem.detail, "matches: the series has 1 result: its pools no longer change");
    assert.deepEqual(
      poolMembers(await (await call("GET", seriesPath(slug, "/pools"))).json()),
      poolMembers(drawn),
      "the draw left the pools alone",
    );
  });
});

describe("the new read routes of a draft", () => {
  test("a wrong token answers 401; without a token a draft answers 404 like an unknown one", async () => {
    const slug = nextSlug();
    await createTournament(call, { slug });
    await createSeries(call, slug);
    for (const rest of ["/pools", "/matches", "/standings", "/pools/A/matches/1"]) {
      await assertProblem(await call("GET", seriesPath(slug, rest), { token: "wrong-token" }), 401, "unauthorized");
      await assertProblem(await call("GET", seriesPath(slug, rest)), 404, "not-found");
    }
    for (const rest of ["/pools", "/matches", "/standings"]) {
      const res = await call("GET", seriesPath(slug, rest), asOrganizer);
      assert.equal(res.status, 200, `${rest}: the organizer reads the draft`);
      await res.body?.cancel();
    }
  });
});
