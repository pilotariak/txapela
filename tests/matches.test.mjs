// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The matches API (src/api/matches.rs): the matches of the pools and the results the organizer
// records, against a migrated D1 in Miniflare. Players and clubs are fictional.

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

// A tournament in pools with 8 teams in two pools of 4: A holds 101, 104, 105 and 108; its
// match 1 is 101 against 108, in round 1. `target` is the target score of the series.
async function drawn(target = 40) {
  lastSlug += 1;
  const slug = `matches-${lastSlug}`;
  await drawnTournament(call, slug, {
    tournament: { specialties: [{ specialty: "paleta_gomme_creuse", target_score: target }] },
  });
  return slug;
}

async function finals(slug) {
  const res = await call("PATCH", `/tournaments/${slug}`, { ...asOrganizer, body: { status: "finals" } });
  assert.equal(res.status, 200, `${slug} moved to finals`);
  await res.body?.cancel();
}

async function assertInvalid(res, detail) {
  const body = await assertProblem(res, 422, "invalid-request");
  assert.match(body.detail ?? "", detail, `the detail names the rule: ${body.detail}`);
  return body;
}

async function matchOf(slug, pool, number) {
  return (await call("GET", seriesPath(slug, `/pools/${pool}/matches/${number}`))).json();
}

// The stored state of match 1 of pool A.
async function storedA1(slug) {
  return db
    .prepare(
      `SELECT x.state AS state, x.score_a AS score_a, x.score_b AS score_b, x.forfeit AS forfeit
       FROM matches x JOIN pools p ON p.id = x.pool_id
       WHERE p.series_id = (SELECT s.id FROM series s JOIN tournaments t ON t.id = s.tournament_id
         WHERE t.slug = ? AND s.slug = '1re-serie-a') AND p.number = 1 AND x.number = 1`,
    )
    .bind(slug)
    .first();
}

const TEAM_101 = {
  number: 101,
  players: [
    { first_name: "Ane", last_name: "Exemple", club: "Club Exemple de Pelote" },
    { first_name: "Kepa", last_name: "Exemple", club: "Club Exemple de Pelote" },
  ],
};
const TEAM_108 = {
  number: 108,
  players: [
    { first_name: "Xabi", last_name: "Exemple", club: "Club Exemple de Pelote" },
    { first_name: "Ziortza", last_name: "Exemple", club: "Club Exemple de Pelote" },
  ],
};

describe("GET /tournaments/:slug/series/:series/matches", () => {
  test("lists the matches by pool then number, each with the public view of its teams", async () => {
    const slug = await drawn();
    const res = await call("GET", seriesPath(slug, "/matches"));
    assert.equal(res.status, 200);
    const { matches } = await res.json();
    assert.deepEqual(
      matches.map((match) => `${match.pool}${match.number}:${match.round}`),
      ["A1:1", "A2:1", "A3:2", "A4:2", "A5:3", "A6:3", "B1:1", "B2:1", "B3:2", "B4:2", "B5:3", "B6:3"],
      "6 matches in 3 rounds in each pool of 4",
    );
    assert.deepEqual(
      matches[0],
      { phase: "pool", pool: "A", number: 1, round: 1, teams: [TEAM_101, TEAM_108], state: "scheduled" },
      "the first seed against the last one, no score yet",
    );
    assert.deepEqual(
      matches.filter((match) => match.pool === "B").map((match) => match.teams.map((team) => team.number)),
      [[102, 107], [103, 106], [102, 106], [103, 107], [102, 103], [106, 107]],
      "every pair of pool B once",
    );
    const organizer = await (await call("GET", seriesPath(slug, "/matches"), asOrganizer)).json();
    assert.deepEqual(organizer.matches, matches, "the organizer sees the same public view");
  });

  test("is empty until the draw", async () => {
    lastSlug += 1;
    const slug = `matches-${lastSlug}`;
    await createTournament(call, { slug });
    await createSeries(call, slug);
    await createTeams(call, slug, { count: 4 });
    await moveTo(call, slug, "pools");
    assert.deepEqual(await (await call("GET", seriesPath(slug, "/matches"))).json(), { matches: [] });
  });
});

describe("GET /tournaments/:slug/series/:series/pools/:pool/matches/:match", () => {
  test("a match by its pool and number; any other key answers 404", async () => {
    const slug = await drawn();
    const res = await call("GET", seriesPath(slug, "/pools/A/matches/1"));
    assert.equal(res.status, 200);
    assert.deepEqual(await res.json(), { phase: "pool", pool: "A", number: 1, round: 1, teams: [TEAM_101, TEAM_108], state: "scheduled" });
    for (const rest of ["/pools/A/matches/7", "/pools/A/matches/0", "/pools/A/matches/01", "/pools/A/matches/x", "/pools/C/matches/1", "/pools/a/matches/1"]) {
      await assertProblem(await call("GET", seriesPath(slug, rest)), 404, "not-found");
    }
  });
});

describe("PUT /tournaments/:slug/series/:series/pools/:pool/matches/:match/result", () => {
  test("records a score in the order the body lists the teams, and records it again to correct it", async () => {
    const slug = await drawn();
    const res = await recordResult(call, slug, "A", 1, { teams: [108, 101], score: [31, 40] });
    assert.equal(res.status, 200);
    const played = await res.json();
    assert.deepEqual(played, {
      phase: "pool",
      pool: "A",
      number: 1,
      round: 1,
      teams: [TEAM_101, TEAM_108],
      state: "played",
      score: [40, 31],
    }, "101 reached 40: the score follows the teams of the match");
    assert.deepEqual(await matchOf(slug, "A", 1), played, "stored");
    const corrected = await recordResult(call, slug, "A", 1, { teams: [101, 108], score: [38, 40] });
    assert.equal(corrected.status, 200, "a correction");
    assert.deepEqual((await corrected.json()).score, [38, 40]);
    assert.deepEqual(await storedA1(slug), { state: "played", score_a: 38, score_b: 40, forfeit: null }, "stored");
  });

  test("records a forfeit as the target score of the series to 0, never 0-0", async () => {
    const slug = await drawn(30);
    const res = await recordResult(call, slug, "A", 1, { teams: [101, 108], forfeited_by: 101 });
    assert.equal(res.status, 200);
    const forfeit = await res.json();
    assert.deepEqual(
      [forfeit.state, forfeit.score, forfeit.forfeited_by],
      ["forfeit", [0, 30], 101],
      "108 gets the target score of gomme creuse in this tournament",
    );
    assert.deepEqual(await storedA1(slug), { state: "forfeit", score_a: 0, score_b: 30, forfeit: "a" }, "stored");
    const played = await recordResult(call, slug, "A", 1, { teams: [101, 108], score: [30, 29] });
    assert.equal(played.status, 200, "a forfeit corrected to a played score");
    const body = await played.json();
    assert.equal(body.forfeited_by, undefined, "no team forfeited");
    assert.deepEqual(await storedA1(slug), { state: "played", score_a: 30, score_b: 29, forfeit: null });
  });

  test("a score has exactly one team at the target score of the series", async () => {
    for (const target of [30, 40]) {
      const slug = await drawn(target);
      for (const score of [[target, target], [target + 1, 0], [target - 1, target - 2], [0, 0], [target, -1]]) {
        await assertInvalid(
          await recordResult(call, slug, "A", 1, { teams: [101, 108], score }),
          new RegExp(`^score: one team reaches ${target}, the target score of the series, and the other stays from 0 to ${target - 1}$`),
        );
      }
      for (const score of [[target, 0], [target - 1, target]]) {
        const res = await recordResult(call, slug, "A", 1, { teams: [101, 108], score });
        assert.equal(res.status, 200, `${score} to ${target}`);
        await res.body?.cancel();
      }
    }
  });

  const invalid = [
    ["the teams of another match", { teams: [101, 104], score: [40, 0] }, /^teams: match 1 of pool A is team 101 against team 108$/],
    ["a team twice", { teams: [101, 101], score: [40, 0] }, /^teams: match 1 of pool A is team 101 against team 108$/],
    ["a forfeit by another team", { teams: [101, 108], forfeited_by: 104 }, /^forfeited_by: is team 101 or team 108, the teams of the match$/],
    ["a score and a forfeit", { teams: [101, 108], score: [40, 0], forfeited_by: 108 }, /^score, forfeited_by: give exactly one of them$/],
    ["neither a score nor a forfeit", { teams: [101, 108] }, /^score, forfeited_by: give exactly one of them$/],
    ["one team", { teams: [101], score: [40, 0] }, /^teams: holds the numbers of the two teams of the match$/],
    ["three scores", { teams: [101, 108], score: [40, 0, 1] }, /^score: holds the scores of the two teams, in the order of teams$/],
    ["no teams", { score: [40, 0] }, /missing field `teams`/],
    ["an unknown field", { teams: [101, 108], score: [40, 0], state: "played" }, /unknown field `state`/],
    ["a null", { teams: [101, 108], score: null }, /^score cannot be null$/],
    ["a score that is not an integer", { teams: [101, 108], score: [40, 1.5] }, /^invalid type, expected i64$/],
  ];
  for (const [what, body, detail] of invalid) {
    test(`${what} answers 422 invalid-request`, async () => {
      const slug = await drawn();
      await assertInvalid(await recordResult(call, slug, "A", 1, body), detail);
      assert.deepEqual(await storedA1(slug), { state: "scheduled", score_a: null, score_b: null, forfeit: null }, "nothing written");
    });
  }

  test("results are recorded and corrected while the tournament is pools only", async () => {
    const slug = await drawn();
    const res = await recordResult(call, slug, "A", 1, { teams: [101, 108], score: [40, 20] });
    await res.body?.cancel();
    await finals(slug);
    const problem = await assertProblem(
      await recordResult(call, slug, "A", 1, { teams: [101, 108], score: [20, 40] }),
      409,
      "not-in-pools",
    );
    assert.equal(problem.detail, "status: pools are drawn and played while the tournament is pools, not finals");
    assert.deepEqual(await storedA1(slug), { state: "played", score_a: 40, score_b: 20, forfeit: null }, "the result stays");
  });

  test("checks 401, then 404, then the body, then the status, then the score", async () => {
    const slug = await drawn();
    const path = seriesPath(slug, "/pools/A/matches/1/result");
    await assertProblem(await call("PUT", path, { body: "{" }), 401, "unauthorized");
    await assertProblem(await call("PUT", path, { token: "wrong-token", body: { teams: [101, 108], score: [40, 0] } }), 401, "unauthorized");
    await assertProblem(await call("PUT", seriesPath(slug, "/pools/A/matches/9/result"), { ...asOrganizer, body: "{" }), 404, "not-found");
    await assertProblem(await call("PUT", seriesPath(slug, "/pools/Q/matches/1/result"), { ...asOrganizer, body: "{" }), 404, "not-found");
    await finals(slug);
    await assertInvalid(await call("PUT", path, { ...asOrganizer, body: { teams: [101] } }), /^teams: holds/);
    await assertProblem(await call("PUT", path, { ...asOrganizer, body: { teams: [101, 104], score: [0, 0] } }), 409, "not-in-pools");
  });

  test("other methods on a result answer 405", async () => {
    const slug = await drawn();
    for (const method of ["GET", "POST", "DELETE"]) {
      await assertProblem(await call(method, seriesPath(slug, "/pools/A/matches/1/result"), asOrganizer), 405, "method-not-allowed");
    }
  });
});

// A result and the end of the pools at once: D1 checks the status in the statement that writes
// the result, so a result answered 200 is stored, and one refused is not.
describe("concurrent results and status changes", () => {
  test("a result is stored if and only if it is answered 200", async () => {
    for (let round = 0; round < 6; round += 1) {
      const slug = await drawn();
      const close = () => call("PATCH", `/tournaments/${slug}`, { ...asOrganizer, body: { status: "finals" } });
      const record = () => recordResult(call, slug, "A", 1, { teams: [101, 108], score: [40, 25] });
      const responses = await Promise.all(round % 2 === 0 ? [close(), record()] : [record(), close()]);
      const [closed, recorded] = round % 2 === 0 ? responses : [responses[1], responses[0]];
      assert.equal(closed.status, 200, `round ${round}: the tournament moves to finals`);
      await closed.body?.cancel();
      const stored = await storedA1(slug);
      if (recorded.status === 200) {
        await recorded.body?.cancel();
        assert.deepEqual(stored, { state: "played", score_a: 40, score_b: 25, forfeit: null }, `round ${round}: stored`);
      } else {
        await assertProblem(recorded, 409, "not-in-pools");
        assert.equal(stored.state, "scheduled", `round ${round}: not stored`);
      }
    }
  });
});
