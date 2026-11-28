// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The standings API (src/api/standings.rs): the standings of each pool, computed from its
// matches on every read, and the organizer's tiebreak, against a migrated D1 in Miniflare.
// Players and clubs are fictional.

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

// A tournament in pools with 8 teams in two pools of 4. Pool A holds 101, 104, 105 and 108:
// A1 is 101-108, A2 104-105, A3 101-105, A4 104-108, A5 101-104, A6 105-108.
async function drawn() {
  lastSlug += 1;
  const slug = `standings-${lastSlug}`;
  await drawnTournament(call, slug);
  return slug;
}

async function record(slug, number, body) {
  const res = await recordResult(call, slug, "A", number, body);
  assert.equal(res.status, 200, `A${number} recorded`);
  await res.body?.cancel();
}

async function standingsOf(slug, pool = "A", options = {}) {
  const res = await call("GET", seriesPath(slug, "/standings"), options);
  assert.equal(res.status, 200);
  const body = await res.json();
  return body.pools.find((entry) => entry.pool === pool).standings;
}

// Each line as [team, rank, played, wins, losses, forfeits, points, for, against, average].
function lines(standings) {
  return standings.map((line) => [
    line.team.number,
    line.rank,
    line.played,
    line.wins,
    line.losses,
    line.forfeits,
    line.points,
    line.goals_for,
    line.goals_against,
    line.goal_average,
  ]);
}

function ranks(standings) {
  return standings.map((line) => [line.team.number, line.rank]);
}

function tiebreak(slug, pool, teams, options = asOrganizer) {
  return call("PUT", seriesPath(slug, `/pools/${pool}/tiebreak`), { ...options, body: { teams } });
}

async function assertInvalid(res, detail) {
  const body = await assertProblem(res, 422, "invalid-request");
  assert.match(body.detail ?? "", detail, `the detail names the rule: ${body.detail}`);
  return body;
}

describe("GET /tournaments/:slug/series/:series/standings", () => {
  test("before any result, every team of a pool shares the first rank", async () => {
    const slug = await drawn();
    const res = await call("GET", seriesPath(slug, "/standings"));
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.deepEqual(body.pools.map((entry) => entry.pool), ["A", "B"], "by pool");
    assert.deepEqual(
      body.pools[0].standings[0],
      {
        rank: 1,
        team: {
          number: 101,
          players: [
            { first_name: "Ane", last_name: "Exemple", club: "Club Exemple de Pelote" },
            { first_name: "Kepa", last_name: "Exemple", club: "Club Exemple de Pelote" },
          ],
        },
        played: 0,
        wins: 0,
        losses: 0,
        forfeits: 0,
        points: 0,
        goals_for: 0,
        goals_against: 0,
        goal_average: 0,
      },
      "the public view of the team, and nothing played",
    );
    assert.deepEqual(ranks(body.pools[0].standings), [[101, 1], [104, 1], [105, 1], [108, 1]], "level: by number");
  });

  test("ranks by points then goal average, with a forfeit as the target score to 0", async () => {
    const slug = await drawn();
    await record(slug, 1, { teams: [101, 108], score: [40, 30] });
    await record(slug, 2, { teams: [104, 105], score: [40, 35] });
    await record(slug, 3, { teams: [101, 105], forfeited_by: 105 });
    await record(slug, 4, { teams: [104, 108], score: [20, 40] });
    assert.deepEqual(
      lines(await standingsOf(slug)),
      [
        [101, 1, 2, 2, 0, 0, 6, 80, 30, 50],
        [108, 2, 2, 1, 1, 0, 4, 70, 60, 10],
        [104, 3, 2, 1, 1, 0, 4, 60, 75, -15],
        [105, 4, 2, 0, 2, 1, 1, 35, 80, -45],
      ],
      "105 lost 35-40 and forfeited 0-40: one loss point, no forfeit point",
    );
    const organizer = await standingsOf(slug, "A", asOrganizer);
    assert.doesNotMatch(JSON.stringify(organizer), /licence|contact|LIC-|example\.org/, "no licence or contact");
  });

  test("a corrected score updates the standings at once", async () => {
    const slug = await drawn();
    await record(slug, 1, { teams: [101, 108], score: [40, 30] });
    await record(slug, 3, { teams: [101, 105], forfeited_by: 105 });
    await record(slug, 4, { teams: [104, 108], score: [20, 40] });
    assert.deepEqual(ranks(await standingsOf(slug)), [[101, 1], [108, 2], [104, 3], [105, 4]], "101 first");
    await record(slug, 1, { teams: [108, 101], score: [40, 38] });
    assert.deepEqual(
      lines(await standingsOf(slug)),
      [
        [108, 1, 2, 2, 0, 0, 6, 80, 58, 22],
        [101, 2, 2, 1, 1, 0, 4, 78, 40, 38],
        [104, 3, 1, 0, 1, 0, 1, 20, 40, -20],
        [105, 4, 1, 0, 1, 1, 0, 0, 40, -40],
      ],
      "computed again from the matches: 108 won A1",
    );
  });

  test("is empty until the draw, and hidden with its draft", async () => {
    lastSlug += 1;
    const slug = `standings-${lastSlug}`;
    await createTournament(call, { slug });
    await createSeries(call, slug);
    assert.deepEqual(await (await call("GET", seriesPath(slug, "/standings"), asOrganizer)).json(), { pools: [] });
    await assertProblem(await call("GET", seriesPath(slug, "/standings")), 404, "not-found");
  });
});

describe("PUT /tournaments/:slug/series/:series/pools/:pool/tiebreak", () => {
  test("separates the teams level on points and goal average, and only them", async () => {
    const slug = await drawn();
    const res = await tiebreak(slug, "A", [108, 105, 104, 101]);
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.deepEqual(ranks(body.pools[0].standings), [[108, 1], [105, 2], [104, 3], [101, 4]], "all level: the order");
    assert.deepEqual(ranks(body.pools[1].standings), [[102, 1], [103, 1], [106, 1], [107, 1]], "pool B has none");
    await record(slug, 1, { teams: [101, 108], score: [40, 30] });
    assert.deepEqual(
      ranks(await standingsOf(slug)),
      [[101, 1], [108, 2], [105, 3], [104, 4]],
      "points first: 101 leads, the order separates 105 and 104 only",
    );
    const cleared = await tiebreak(slug, "A", []);
    assert.equal(cleared.status, 200, "an empty list removes the order");
    await cleared.body?.cancel();
    assert.deepEqual(ranks(await standingsOf(slug)), [[101, 1], [108, 2], [104, 3], [105, 3]], "level teams share rank 3");
  });

  test("lists every team of the pool once, or none", async () => {
    const slug = await drawn();
    for (const teams of [[101, 104, 105], [101, 104, 105, 105], [101, 104, 105, 102], [101, 104, 105, 108, 102]]) {
      await assertInvalid(await tiebreak(slug, "A", teams), /^teams: lists every team of pool A once, in the organizer's order, or none$/);
    }
    await assertInvalid(await tiebreak(slug, "A", "nope"), /^invalid type, expected a sequence$/);
  });

  test("a move removes the tiebreak of both pools it touches", async () => {
    const slug = await drawn();
    for (const [pool, teams] of [["A", [108, 105, 104, 101]], ["B", [107, 106, 103, 102]]]) {
      const res = await tiebreak(slug, pool, teams);
      await res.body?.cancel();
    }
    const moved = await call("POST", seriesPath(slug, "/pools/A/teams"), { ...asOrganizer, body: { number: 102 } });
    assert.equal(moved.status, 200);
    await moved.body?.cancel();
    assert.deepEqual(ranks(await standingsOf(slug, "A")).map(([, rank]) => rank), [1, 1, 1, 1, 1], "A: shared again");
    assert.deepEqual(ranks(await standingsOf(slug, "B")).map(([, rank]) => rank), [1, 1, 1], "B: shared again");
  });

  test("checks 401, then 404, then the body, then the status", async () => {
    const slug = await drawn();
    await assertProblem(await tiebreak(slug, "A", [101], {}), 401, "unauthorized");
    await assertProblem(await tiebreak(slug, "C", "{"), 404, "not-found");
    await assertProblem(await tiebreak(slug, "a", [101]), 404, "not-found");
    const res = await call("PATCH", `/tournaments/${slug}`, { ...asOrganizer, body: { status: "finals" } });
    await res.body?.cancel();
    await assertInvalid(await call("PUT", seriesPath(slug, "/pools/A/tiebreak"), { ...asOrganizer, body: { order: [] } }), /unknown field `order`/);
    const problem = await assertProblem(await tiebreak(slug, "A", [101]), 409, "not-in-pools");
    assert.match(problem.detail, /not finals$/);
  });
});

// A tiebreak and a move at once: the tiebreak carries the revision of the pools it read, so D1
// refuses it after a move, and the next attempt finds a list that no longer fits the pool.
describe("concurrent tiebreaks and moves", () => {
  test("a pool's tiebreak covers every team of the pool or none", async () => {
    for (let round = 0; round < 6; round += 1) {
      const slug = await drawn();
      const requests = [
        () => tiebreak(slug, "A", [108, 105, 104, 101]),
        () => call("POST", seriesPath(slug, "/pools/A/teams"), { ...asOrganizer, body: { number: 102 } }),
      ];
      const responses = await Promise.all((round % 2 === 0 ? requests : [...requests].reverse()).map((send) => send()));
      const statuses = [];
      for (const res of responses) {
        statuses.push(res.status);
        await res.body?.cancel();
      }
      assert.ok(statuses.every((status) => [200, 422].includes(status)), `round ${round}: ${statuses}`);
      const { results } = await db
        .prepare(
          `SELECT p.number AS pool, count(*) AS teams, count(pt.tiebreak) AS ordered
           FROM pool_teams pt JOIN pools p ON p.id = pt.pool_id
           WHERE p.series_id = (SELECT s.id FROM series s JOIN tournaments t ON t.id = s.tournament_id
             WHERE t.slug = ? AND s.slug = '1re-serie-a') GROUP BY p.number`,
        )
        .bind(slug)
        .all();
      for (const row of results) {
        assert.ok([0, row.teams].includes(row.ordered), `round ${round}: pool ${row.pool} orders ${row.ordered} of ${row.teams}`);
      }
    }
  });
});
