// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The pools API (src/api/pools.rs): the draw, the pools of a series and the move of a team,
// for an organizer and without a token, against a migrated D1 in Miniflare. Players and clubs
// are fictional.

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import {
  ORGANIZER_TOKEN,
  assertProblem,
  createSeries,
  createTeams,
  createTournament,
  drawnTournament,
  moveTo,
  recordResult,
  seriesPath,
  startApi,
} from "./worker.mjs";

let mf;
let db;
let call;

before(async () => {
  ({ mf, db, call } = await startApi());
});

after(async () => {
  await mf.dispose();
});

const asOrganizer = { token: ORGANIZER_TOKEN };

let lastSlug = 0;
function nextSlug() {
  lastSlug += 1;
  return `pools-${lastSlug}`;
}

// A tournament in pools with 8 teams drawn in two pools of 4: A holds 101, 104, 105 and 108, B
// holds 102, 103, 106 and 107 (the snake: every team plays for the same fixture club).
async function drawn(options = {}) {
  const slug = nextSlug();
  const pools = await drawnTournament(call, slug, options);
  return { slug, pools };
}

// A tournament with the series 1re-serie-a and `count` registered teams, moved to `status`.
async function registered(count, status = "pools") {
  const slug = nextSlug();
  await createTournament(call, { slug });
  await createSeries(call, slug, { max_teams: 16 });
  await createTeams(call, slug, { count });
  await moveTo(call, slug, status);
  return slug;
}

// Moves a tournament in registration to pools.
async function toPools(slug) {
  const res = await call("PATCH", `/tournaments/${slug}`, { ...asOrganizer, body: { status: "pools" } });
  assert.equal(res.status, 200, `${slug} moved to pools`);
  await res.body?.cancel();
}

function draw(slug, body, options = asOrganizer) {
  return call("POST", seriesPath(slug, "/draw"), { ...options, body });
}

function move(slug, pool, number) {
  return call("POST", seriesPath(slug, `/pools/${pool}/teams`), { ...asOrganizer, body: { number } });
}

// The team numbers of each pool, by pool name.
function members(list) {
  return Object.fromEntries(list.pools.map((pool) => [pool.name, pool.teams.map((team) => team.number)]));
}

async function assertInvalid(res, detail) {
  const body = await assertProblem(res, 422, "invalid-request");
  assert.match(body.detail ?? "", detail, `the detail names the rule: ${body.detail}`);
  return body;
}

// For each pool of the series 1re-serie-a: its teams, its matches, the distinct pairs they
// play, and the matches with a team outside the pool.
async function poolRows(slug) {
  const { results } = await db
    .prepare(
      `SELECT p.number AS pool,
         (SELECT count(*) FROM pool_teams pt WHERE pt.pool_id = p.id) AS teams,
         (SELECT count(*) FROM matches x WHERE x.pool_id = p.id) AS matches,
         (SELECT count(DISTINCT min(x.team_a_id, x.team_b_id) || '-' || max(x.team_a_id, x.team_b_id))
            FROM matches x WHERE x.pool_id = p.id AND x.team_a_id <> x.team_b_id) AS pairs,
         (SELECT count(*) FROM matches x WHERE x.pool_id = p.id
            AND (x.team_a_id NOT IN (SELECT team_id FROM pool_teams WHERE pool_id = p.id)
              OR x.team_b_id NOT IN (SELECT team_id FROM pool_teams WHERE pool_id = p.id))) AS outside
       FROM pools p WHERE p.series_id = (SELECT s.id FROM series s JOIN tournaments t ON t.id = s.tournament_id
         WHERE t.slug = ? AND s.slug = '1re-serie-a') ORDER BY p.number`,
    )
    .bind(slug)
    .all();
  return results;
}

// Every pool plays each pair of its teams exactly once, and no team outside it.
async function assertRoundRobins(slug, what) {
  const rows = await poolRows(slug);
  assert.ok(rows.length > 0, `${what}: the series has pools`);
  for (const row of rows) {
    const expected = (row.teams * (row.teams - 1)) / 2;
    assert.equal(row.outside, 0, `${what}: pool ${row.pool} plays no team outside it`);
    assert.equal(row.matches, expected, `${what}: pool ${row.pool} of ${row.teams} teams has ${expected} matches`);
    assert.equal(row.pairs, expected, `${what}: pool ${row.pool} plays every pair once`);
  }
  return rows;
}

describe("POST /tournaments/:slug/series/:series/draw", () => {
  test("draws the registered teams in snake order, and generates the matches of each pool", async () => {
    const slug = await registered(8);
    const res = await draw(slug, { pool_size: 4 });
    assert.equal(res.status, 201);
    assert.equal(res.headers.get("location"), `/api/v1/tournaments/${slug}/series/1re-serie-a/pools`);
    const body = await res.json();
    assert.deepEqual(members(body), { A: [101, 104, 105, 108], B: [102, 103, 106, 107] }, "A, B, then B, A");
    assert.deepEqual(
      body.pools[0].teams.map((team) => team.seed),
      [1, 4, 5, 8],
      "the seeds of pool A",
    );
    assert.deepEqual(
      body.pools[0].teams[0],
      {
        seed: 1,
        number: 101,
        players: [
          { first_name: "Ane", last_name: "Exemple", club: "Club Exemple de Pelote" },
          { first_name: "Kepa", last_name: "Exemple", club: "Club Exemple de Pelote" },
        ],
      },
      "the public view of a team, for the organizer too",
    );
    await assertRoundRobins(slug, "after the draw");
    const { matches } = await (await call("GET", seriesPath(slug, "/matches"))).json();
    assert.equal(matches.length, 12, "6 matches in each pool of 4");
    assert.ok(matches.every((match) => match.state === "scheduled"), "all scheduled");
  });

  test("leaves withdrawn teams out", async () => {
    const slug = nextSlug();
    await createTournament(call, { slug });
    await createSeries(call, slug, { max_teams: 16 });
    await createTeams(call, slug, { count: 9 });
    const res = await call("PATCH", `/tournaments/${slug}/teams/109`, { ...asOrganizer, body: { status: "withdrawn" } });
    assert.equal(res.status, 200, "109 withdraws");
    await moveTo(call, slug, "pools");
    const pools = await (await draw(slug, { pools: 2 })).json();
    assert.deepEqual(members(pools), { A: [101, 104, 105, 108], B: [102, 103, 106, 107] }, "8 registered teams");
  });

  test("seeds the organizer's teams first, then the others by number", async () => {
    const slug = await registered(8);
    const pools = await (await draw(slug, { pools: 2, seeds: [105, 103] })).json();
    assert.deepEqual(members(pools), { A: [105, 102, 104, 108], B: [103, 101, 106, 107] });
    assert.deepEqual(pools.pools[1].teams.map((team) => team.seed), [2, 3, 6, 7], "seeds kept in each pool");
  });

  test("keeps two teams of a club apart when the draw can", async () => {
    const clubs = { 101: "Club Bleu", 104: "CLUB BLEU" };
    const { pools } = await drawn({ club: (number) => clubs[number] });
    assert.deepEqual(
      members(pools),
      { A: [101, 103, 105, 108], B: [102, 104, 106, 107] },
      "104 swaps with 103, its partner in the pot: clubs compare without case",
    );
  });

  test("draws again until the first result of the series, then answers 409 series-has-results", async () => {
    const { slug } = await drawn();
    const again = await draw(slug, { pool_size: 4, seeds: [108, 107] });
    assert.equal(again.status, 201, "no result yet: the draw is replaced");
    assert.deepEqual(members(await again.json()), { A: [108, 102, 103, 106], B: [107, 101, 104, 105] });
    await assertRoundRobins(slug, "after the second draw");
    const result = await recordResult(call, slug, "A", 1, { teams: [108, 106], score: [40, 12] });
    assert.equal(result.status, 200, "a first result");
    await result.body?.cancel();
    const problem = await assertProblem(await draw(slug, { pool_size: 4 }), 409, "series-has-results");
    assert.equal(problem.detail, "matches: the series has 1 result: its pools no longer change");
    const pools = await (await call("GET", seriesPath(slug, "/pools"))).json();
    assert.deepEqual(members(pools), { A: [108, 102, 103, 106], B: [107, 101, 104, 105] }, "the pools stay");
  });

  test("answers 409 not-in-pools before pools and after them", async () => {
    const slug = await registered(8, "registration");
    const problem = await assertProblem(await draw(slug, { pool_size: 4 }), 409, "not-in-pools");
    assert.equal(problem.detail, "status: pools are drawn and played while the tournament is pools, not registration");
    await toPools(slug);
    const drawnPools = await draw(slug, { pool_size: 4 });
    assert.equal(drawnPools.status, 201);
    await drawnPools.body?.cancel();
    const finals = await call("PATCH", `/tournaments/${slug}`, { ...asOrganizer, body: { status: "finals" } });
    await finals.body?.cancel();
    await assertProblem(await draw(slug, { pool_size: 4 }), 409, "not-in-pools");
  });

  const invalid = [
    ["both pool_size and pools", { pool_size: 4, pools: 2 }, /^pool_size, pools: give exactly one of them$/],
    ["neither pool_size nor pools", { seeds: [101] }, /^pool_size, pools: give exactly one of them$/],
    ["a pool size of 7", { pool_size: 7 }, /^pool_size: is 4, 5 or 6$/],
    ["pools that would hold 8 teams", { pools: 1 }, /^pools: 8 teams in 1 pool make pools of 8 teams: a pool holds 4 to 6 teams$/],
    ["no pool", { pools: 0 }, /^pools: is an integer from 1 to 26$/],
    ["a seed that is not a registered team", { pool_size: 4, seeds: [201] }, /^seeds: 201 is not a registered team of the series$/],
    ["a seed listed twice", { pool_size: 4, seeds: [101, 101] }, /^seeds: 101 appears twice$/],
    ["an unknown field", { pool_size: 4, size: 4 }, /unknown field `size`/],
    ["a null", { pool_size: null }, /^pool_size cannot be null$/],
    ["a body that is not JSON", "{", /^the body is not JSON$/],
  ];
  for (const [what, body, detail] of invalid) {
    test(`${what} answers 422 invalid-request`, async () => {
      const slug = await registered(8);
      await assertInvalid(await draw(slug, body), detail);
      assert.deepEqual((await (await call("GET", seriesPath(slug, "/pools"))).json()).pools, [], "nothing drawn");
    });
  }

  test("a series needs 4 registered teams, and pools of 4 to 6", async () => {
    await assertInvalid(
      await draw(await registered(3), { pool_size: 4 }),
      /^teams: the series has 3 teams registered: a draw needs 4 at least$/,
    );
    await assertInvalid(
      await draw(await registered(7), { pool_size: 6 }),
      /^pool_size: 7 teams in 2 pools make pools of 3 to 4 teams: a pool holds 4 to 6 teams$/,
    );
  });

  test("checks 401, then 404, then the body, then the status, then the rules, then the results", async () => {
    const slug = await registered(8, "registration");
    await assertProblem(await draw(slug, "{", {}), 401, "unauthorized");
    await assertProblem(await call("POST", "/tournaments/no-such-tournament/series/1re-serie-a/draw", { ...asOrganizer, body: "{" }), 404, "not-found");
    await assertProblem(await call("POST", `/tournaments/${slug}/series/no-series/draw`, { ...asOrganizer, body: "{" }), 404, "not-found");
    await assertProblem(await call("POST", `/tournaments/${slug}/series/Série/draw`, { ...asOrganizer, body: "{" }), 404, "not-found");
    await assertInvalid(await draw(slug, { pools: 2, size: 4 }), /unknown field `size`/);
    await assertProblem(await draw(slug, { pool_size: 9 }), 409, "not-in-pools");
    await toPools(slug);
    const first = await draw(slug, { pool_size: 4 });
    await first.body?.cancel();
    const result = await recordResult(call, slug, "A", 1, { teams: [101, 108], score: [40, 0] });
    await result.body?.cancel();
    await assertInvalid(await draw(slug, { pool_size: 9 }), /^pool_size: is 4, 5 or 6$/);
    await assertProblem(await draw(slug, { pool_size: 4 }), 409, "series-has-results");
  });
});

describe("GET /tournaments/:slug/series/:series/pools", () => {
  test("is empty until the draw, then lists the pools by name", async () => {
    const slug = await registered(8);
    assert.deepEqual(await (await call("GET", seriesPath(slug, "/pools"))).json(), { pools: [] }, "no draw yet");
    const created = await (await draw(slug, { pool_size: 4 })).json();
    const res = await call("GET", seriesPath(slug, "/pools"));
    assert.equal(res.status, 200);
    assert.deepEqual(await res.json(), created, "the pools of the draw, for everyone");
    const organizer = await (await call("GET", seriesPath(slug, "/pools"), asOrganizer)).json();
    assert.deepEqual(organizer, created, "the organizer sees the same public view");
    assert.doesNotMatch(JSON.stringify(organizer), /licence|contact|LIC-|example\.org/, "no licence or contact");
  });

  test("the pools of a draft are invisible without the token: 404 as for an unknown tournament", async () => {
    const slug = nextSlug();
    await createTournament(call, { slug });
    await createSeries(call, slug);
    for (const rest of ["/pools", "/matches", "/standings", "/pools/A/matches/1"]) {
      const hidden = await assertProblem(await call("GET", seriesPath(slug, rest)), 404, "not-found");
      const unknown = await assertProblem(await call("GET", seriesPath("no-such-tournament", rest)), 404, "not-found");
      assert.deepEqual(hidden, unknown, `${rest}: a draft and an unknown tournament answer the same`);
    }
    assert.deepEqual(await (await call("GET", seriesPath(slug, "/pools"), asOrganizer)).json(), { pools: [] }, "the organizer reads it");
    await assertProblem(await call("GET", seriesPath(slug, "/pools"), { token: "wrong-token" }), 401, "unauthorized");
    await assertProblem(await call("GET", `/tournaments/${slug}/series/no-series/pools`, asOrganizer), 404, "not-found");
  });
});

describe("POST /tournaments/:slug/series/:series/pools/:pool/teams", () => {
  test("moves a team and generates again the matches of both pools", async () => {
    const { slug } = await drawn();
    const res = await move(slug, "B", 101);
    assert.equal(res.status, 200);
    assert.deepEqual(members(await res.json()), { A: [104, 105, 108], B: [101, 102, 103, 106, 107] }, "101 keeps its seed");
    const rows = await assertRoundRobins(slug, "after the move");
    assert.deepEqual(rows.map((row) => [row.pool, row.teams, row.matches]), [[1, 3, 3], [2, 5, 10]], "3 and 10 matches");
    const { matches } = await (await call("GET", seriesPath(slug, "/matches"))).json();
    const rounds = matches.filter((match) => match.pool === "A").map((match) => match.round);
    assert.deepEqual(rounds, [1, 2, 3], "3 teams: 3 rounds, one team rests each round");
  });

  test("a team already in the pool stays: nothing changes", async () => {
    const { slug, pools } = await drawn();
    const res = await move(slug, "A", 101);
    assert.equal(res.status, 200);
    assert.deepEqual(await res.json(), pools, "the pools of the draw");
  });

  test("a move leaves 3 to 7 teams in each pool, so that two teams swap in two moves", async () => {
    const { slug } = await drawn();
    for (const [pool, number] of [["B", 101], ["A", 102]]) {
      const res = await move(slug, pool, number);
      assert.equal(res.status, 200, `${number} to ${pool}`);
      await res.body?.cancel();
    }
    const pools = await (await call("GET", seriesPath(slug, "/pools"))).json();
    assert.deepEqual(members(pools), { A: [102, 104, 105, 108], B: [101, 103, 106, 107] }, "101 and 102 swapped");
    const first = await move(slug, "B", 102);
    await first.body?.cancel();
    await assertInvalid(
      await move(slug, "B", 104),
      /^pool: the move leaves 2 teams in pool A: a move leaves 3 to 7 teams in each pool$/,
    );
  });

  test("a team in no pool, or a pool the series does not have, is refused", async () => {
    const { slug } = await drawn();
    await assertInvalid(await move(slug, "B", 199), /^number: the series has no team 199 in a pool$/);
    for (const pool of ["C", "a", "AA", "1"]) {
      await assertProblem(await move(slug, pool, 101), 404, "not-found");
    }
  });

  test("after the first result, answers 409 series-has-results and the pools stay", async () => {
    const { slug, pools } = await drawn();
    const result = await recordResult(call, slug, "B", 2, { teams: [103, 106], forfeited_by: 103 });
    assert.equal(result.status, 200);
    await result.body?.cancel();
    const problem = await assertProblem(await move(slug, "A", 102), 409, "series-has-results");
    assert.equal(problem.detail, "matches: the series has 1 result: its pools no longer change");
    assert.deepEqual(members(await (await call("GET", seriesPath(slug, "/pools"))).json()), members(pools));
  });

  test("checks 401, then 404, then the body, then the status", async () => {
    const { slug } = await drawn();
    const path = seriesPath(slug, "/pools/B/teams");
    await assertProblem(await call("POST", path, { body: { number: 101 } }), 401, "unauthorized");
    await assertProblem(await call("POST", seriesPath(slug, "/pools/Z/teams"), { ...asOrganizer, body: "{" }), 404, "not-found");
    await assertInvalid(await call("POST", path, { ...asOrganizer, body: { team: 101 } }), /unknown field `team`/);
    const finals = await call("PATCH", `/tournaments/${slug}`, { ...asOrganizer, body: { status: "finals" } });
    await finals.body?.cancel();
    await assertProblem(await call("POST", path, { ...asOrganizer, body: { number: 199 } }), 409, "not-in-pools");
  });
});

describe("other methods", () => {
  test("GET on the draw and on a pool's teams, and PATCH on the pools, answer 405", async () => {
    const { slug } = await drawn();
    await assertProblem(await call("GET", seriesPath(slug, "/draw"), asOrganizer), 405, "method-not-allowed");
    await assertProblem(await call("GET", seriesPath(slug, "/pools/A/teams"), asOrganizer), 405, "method-not-allowed");
    await assertProblem(await call("PATCH", seriesPath(slug, "/pools"), asOrganizer), 405, "method-not-allowed");
  });
});

// Concurrent writes of the pools: D1 checks each guard in the statements that write, so whatever
// the order in which the Worker runs the requests, the rows it leaves keep the rules. Each test
// checks the rows, not only the statuses, over several rounds.
describe("concurrent writes keep every pool a round robin of its teams", () => {
  const ROUNDS = 6;

  async function settle(requests) {
    const responses = await Promise.all(requests.map((send) => send()));
    const statuses = [];
    for (const res of responses) {
      statuses.push(res.status);
      await res.body?.cancel();
    }
    return statuses;
  }

  // The first request last in odd rounds, so that both orders run.
  function ordered(round, first, second) {
    return round % 2 === 0 ? [first, second] : [second, first];
  }

  async function playedBetween(slug, a, b) {
    return db
      .prepare(
        `SELECT x.state AS state FROM matches x JOIN teams ta ON ta.id = x.team_a_id JOIN teams tb ON tb.id = x.team_b_id
         WHERE x.series_id = (SELECT s.id FROM series s JOIN tournaments t ON t.id = s.tournament_id
           WHERE t.slug = ? AND s.slug = '1re-serie-a')
         AND ((ta.number = ? AND tb.number = ?) OR (ta.number = ? AND tb.number = ?)) AND x.state = 'played'`,
      )
      .bind(slug, a, b, b, a)
      .first();
  }

  test("two moves into one pool both apply, and both pools they leave keep their round robin", async () => {
    for (let round = 0; round < ROUNDS; round += 1) {
      // 12 teams in pools of 4: A 101 106 107 112, B 102 105 108 111, C 103 104 109 110.
      const { slug } = await drawn({ teams: 12 });
      const statuses = await settle(ordered(round, () => move(slug, "B", 101), () => move(slug, "B", 103)));
      assert.deepEqual(statuses, [200, 200], `round ${round}: both moves apply, the second one on the new pools`);
      const rows = await assertRoundRobins(slug, `round ${round}`);
      assert.deepEqual(rows.map((row) => row.teams), [3, 6, 3], `round ${round}: B holds 6 teams`);
    }
  });

  test("a result and a draw: the result is never lost, and a draw never follows it", async () => {
    for (let round = 0; round < ROUNDS; round += 1) {
      const { slug } = await drawn();
      // The new draw makes A1 105 against 104: the result for 101 and 108 then misses its pair.
      const redraw = () => draw(slug, { pool_size: 4, seeds: [105, 106, 107, 108] });
      const result = () => recordResult(call, slug, "A", 1, { teams: [101, 108], score: [40, 31] });
      const statuses = await settle(ordered(round, redraw, result));
      const [drawStatus, resultStatus] = round % 2 === 0 ? statuses : [statuses[1], statuses[0]];
      const stored = await playedBetween(slug, 101, 108);
      if (resultStatus === 200) {
        assert.equal(drawStatus, 409, `round ${round}: the result first, then the draw is refused`);
        assert.ok(stored, `round ${round}: the result is stored`);
      } else {
        assert.deepEqual([drawStatus, resultStatus], [201, 422], `round ${round}: the draw first, then the result misses its pair`);
        assert.equal(stored, null, `round ${round}: no result stored`);
      }
      await assertRoundRobins(slug, `round ${round}`);
    }
  });

  test("a result and a move: the result is never lost, and a move never follows it", async () => {
    for (let round = 0; round < ROUNDS; round += 1) {
      const { slug } = await drawn();
      // Moving 101 to B makes A1 105 against 108: the result for 101 and 108 then misses its pair.
      const statuses = await settle(
        ordered(
          round,
          () => move(slug, "B", 101),
          () => recordResult(call, slug, "A", 1, { teams: [101, 108], score: [40, 31] }),
        ),
      );
      const [moveStatus, resultStatus] = round % 2 === 0 ? statuses : [statuses[1], statuses[0]];
      const stored = await playedBetween(slug, 101, 108);
      if (resultStatus === 200) {
        assert.equal(moveStatus, 409, `round ${round}: the result first, then the move is refused`);
        assert.ok(stored, `round ${round}: the result is stored`);
      } else {
        assert.deepEqual([moveStatus, resultStatus], [200, 422], `round ${round}: the move first`);
        assert.equal(stored, null, `round ${round}: no result stored`);
      }
      await assertRoundRobins(slug, `round ${round}`);
    }
  });
});
