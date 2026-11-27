// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The series API (src/api/series.rs): each route, for an organizer and without a token, against
// a migrated D1 in Miniflare.

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import {
  ORGANIZER_TOKEN,
  assertProblem,
  createSeries,
  createTeam,
  createTournament,
  moveTo,
  seriesBody,
  startApi,
  teamBody,
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

const BOTH_SPECIALTIES = [
  { specialty: "paleta_gomme_creuse", target_score: 40 },
  { specialty: "paleta_gomme_pleine", target_score: 30 },
];

// A fresh tournament for each test, playing both specialties unless `changes` says otherwise.
let lastSlug = 0;
async function tournament(changes = {}) {
  lastSlug += 1;
  return createTournament(call, { slug: `series-${lastSlug}`, specialties: BOTH_SPECIALTIES, ...changes });
}

async function seriesCount(slug) {
  const row = await db
    .prepare("SELECT count(*) AS n FROM series WHERE tournament_id = (SELECT id FROM tournaments WHERE slug = ?)")
    .bind(slug)
    .first();
  return row.n;
}

async function assertInvalid(res, detail) {
  const body = await assertProblem(res, 422, "invalid-request");
  assert.match(body.detail ?? "", detail, `the detail names the rule: ${body.detail}`);
}

describe("POST /tournaments/:slug/series", () => {
  test("creates a series that plays to the target score of its specialty", async () => {
    const { slug } = await tournament();
    const res = await call("POST", `/tournaments/${slug}/series`, {
      ...asOrganizer,
      body: seriesBody({ name: "  1re Série A ", specialty: "paleta_gomme_pleine" }),
    });
    assert.equal(res.status, 201);
    assert.equal(res.headers.get("location"), `/api/v1/tournaments/${slug}/series/1re-serie-a`);
    assert.equal(res.headers.get("cache-control"), "private, no-store");
    const expected = {
      slug: "1re-serie-a",
      name: "1re Série A",
      specialty: "paleta_gomme_pleine",
      target_score: 30,
      max_teams: 8,
      position: 1,
      number_block: 100,
    };
    assert.deepEqual(await res.json(), expected, "the name is trimmed, the target comes from the tournament");
    const stored = await call("GET", `/tournaments/${slug}/series/1re-serie-a`, asOrganizer);
    assert.deepEqual(await stored.json(), expected, "read back");
    assert.equal(await seriesCount(slug), 1, "one row");
  });

  test("refuses a specialty the tournament does not declare", async () => {
    const { slug } = await tournament({ specialties: [{ specialty: "paleta_gomme_creuse", target_score: 40 }] });
    await assertInvalid(
      await call("POST", `/tournaments/${slug}/series`, {
        ...asOrganizer,
        body: seriesBody({ specialty: "paleta_gomme_pleine" }),
      }),
      /^specialty: paleta_gomme_pleine is not a specialty of the tournament$/,
    );
    assert.equal(await seriesCount(slug), 0, "nothing written");
  });

  test("a slug or a number block of another series answers 409 and writes nothing", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    let problem = await assertProblem(
      await call("POST", `/tournaments/${slug}/series`, { ...asOrganizer, body: seriesBody({ number_block: 200 }) }),
      409,
      "slug-taken",
    );
    assert.equal(problem.detail, "slug: another series of the tournament is 1re-serie-a");
    problem = await assertProblem(
      await call("POST", `/tournaments/${slug}/series`, { ...asOrganizer, body: seriesBody({ slug: "2e-serie" }) }),
      409,
      "block-taken",
    );
    assert.equal(problem.detail, "number_block: another series of the tournament has the block 100");
    assert.equal(await seriesCount(slug), 1, "still one series");
    const other = await tournament();
    await createSeries(call, other.slug);
    assert.equal(await seriesCount(other.slug), 1, "another tournament takes the same slug and block");
  });

  const invalid = [
    ["a slug with capitals", { slug: "Serie-A" }, /^slug: 3 to 64 characters/],
    ["an empty name", { name: " " }, /^name: cannot be empty$/],
    ["an unknown specialty", { specialty: "main_nue" }, /unknown variant `main_nue`/],
    ["a single team", { max_teams: 1 }, /^max_teams: is an integer from 2 to 100, the numbers of a block$/],
    ["more teams than a block holds", { max_teams: 101 }, /^max_teams: is an integer from 2 to 100/],
    ["a position of zero", { position: 0 }, /^position: is an integer from 1/],
    ["a block that is not a multiple of 100", { number_block: 150 }, /^number_block: is a multiple of 100 from 100 to 9900$/],
    ["a block of zero", { number_block: 0 }, /^number_block: is a multiple of 100/],
    ["a block past 9900", { number_block: 10000 }, /^number_block: is a multiple of 100/],
    ["a number block that is not an integer", { number_block: 100.5 }, /^invalid type, expected i64$/],
    ["an unknown field", { level: "A" }, /unknown field `level`/],
    ["a null", { name: null }, /^name cannot be null$/],
  ];
  for (const [what, changes, detail] of invalid) {
    test(`${what} answers 422 invalid-request`, async () => {
      const { slug } = await tournament();
      await assertInvalid(
        await call("POST", `/tournaments/${slug}/series`, { ...asOrganizer, body: seriesBody(changes) }),
        detail,
      );
      assert.equal(await seriesCount(slug), 0, "nothing written");
    });
  }

  test("series are created in draft and registration, not past them", async () => {
    const { slug } = await tournament();
    await moveTo(call, slug, "registration");
    await createSeries(call, slug);
    const pools = await call("PATCH", `/tournaments/${slug}`, { ...asOrganizer, body: { status: "pools" } });
    assert.equal(pools.status, 200, "the pools are drawn");
    await pools.body?.cancel();
    const problem = await assertProblem(
      await call("POST", `/tournaments/${slug}/series`, {
        ...asOrganizer,
        body: seriesBody({ slug: "2e-serie", number_block: 200, name: "" }),
      }),
      409,
      "registration-closed",
    );
    assert.equal(
      problem.detail,
      "status: series and teams change only while the tournament is draft or registration, not pools",
      "registration-closed comes before the rules",
    );
    assert.equal(await seriesCount(slug), 1, "nothing written");
  });

  test("answers 401 without the token, then 404 for an unknown tournament, then 422 for the body", async () => {
    const { slug } = await tournament();
    await assertProblem(await call("POST", `/tournaments/${slug}/series`, { body: seriesBody() }), 401, "unauthorized");
    await assertProblem(await call("POST", `/tournaments/${slug}/series`, { body: "{" }), 401, "unauthorized");
    await assertProblem(
      await call("POST", "/tournaments/no-such-tournament/series", { ...asOrganizer, body: "{" }),
      404,
      "not-found",
    );
    await assertInvalid(await call("POST", `/tournaments/${slug}/series`, { ...asOrganizer, body: "{" }), /^the body is not JSON$/);
    assert.equal(await seriesCount(slug), 0, "nothing written");
  });
});

describe("GET /tournaments/:slug/series", () => {
  test("lists by position then slug", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug, { slug: "feminines", name: "Féminines", position: 2, number_block: 300 });
    await createSeries(call, slug, { slug: "2e-serie", name: "2e Série", position: 2, number_block: 200 });
    await createSeries(call, slug, { slug: "1re-serie", name: "1re Série", position: 1, number_block: 100 });
    const res = await call("GET", `/tournaments/${slug}/series`, asOrganizer);
    assert.equal(res.status, 200);
    assert.deepEqual(
      (await res.json()).series.map((series) => series.slug),
      ["1re-serie", "2e-serie", "feminines"],
      "position first, then slug",
    );
  });

  test("the series of a draft are invisible without the token: 404 as for an unknown tournament", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    for (const path of [`/tournaments/${slug}/series`, `/tournaments/${slug}/series/1re-serie-a`]) {
      const hidden = await assertProblem(await call("GET", path), 404, "not-found");
      const unknown = await assertProblem(await call("GET", path.replace(slug, "no-such-tournament")), 404, "not-found");
      assert.deepEqual(hidden, unknown, `${path}: a draft and an unknown tournament answer the same`);
    }
    await moveTo(call, slug, "registration");
    const list = await call("GET", `/tournaments/${slug}/series`);
    assert.equal(list.status, 200, "public once the tournament leaves draft");
    assert.equal((await list.json()).series.length, 1);
  });

  test("an unknown series, or a path that cannot be a slug, answers 404", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    for (const series of ["2e-serie", "Serie-A", "ab", "a--b"]) {
      await assertProblem(await call("GET", `/tournaments/${slug}/series/${series}`, asOrganizer), 404, "not-found");
    }
  });
});

describe("PATCH /tournaments/:slug/series/:series", () => {
  test("changes the fields of the body and keeps the others", async () => {
    const { slug } = await tournament();
    const created = await createSeries(call, slug);
    const path = `/tournaments/${slug}/series/1re-serie-a`;
    const res = await call("PATCH", path, {
      ...asOrganizer,
      body: { name: " Première série ", specialty: "paleta_gomme_pleine", number_block: 500 },
    });
    assert.equal(res.status, 200);
    const expected = {
      ...created,
      name: "Première série",
      specialty: "paleta_gomme_pleine",
      target_score: 30,
      number_block: 500,
    };
    assert.deepEqual(await res.json(), expected, "the series after the change");
    assert.deepEqual(await (await call("GET", path, asOrganizer)).json(), expected, "stored");
    const empty = await call("PATCH", path, { ...asOrganizer, body: {} });
    assert.deepEqual(await empty.json(), expected, "an empty body changes nothing");
  });

  test("rejects the slug, unknown fields, nulls and broken rules", async () => {
    const { slug } = await tournament({ specialties: [{ specialty: "paleta_gomme_creuse", target_score: 40 }] });
    const created = await createSeries(call, slug);
    const path = `/tournaments/${slug}/series/1re-serie-a`;
    const patch = (body) => call("PATCH", path, { ...asOrganizer, body });
    await assertInvalid(await patch({ slug: "autre" }), /^slug cannot change: it is the public key of the series$/);
    await assertInvalid(await patch({ level: "A" }), /unknown field `level`/);
    await assertInvalid(await patch({ max_teams: null }), /^max_teams cannot be null$/);
    await assertInvalid(await patch({ specialty: "paleta_gomme_pleine" }), /^specialty: paleta_gomme_pleine is not a specialty/);
    await assertInvalid(await patch({ number_block: 250 }), /^number_block: is a multiple of 100/);
    assert.deepEqual(await (await call("GET", path, asOrganizer)).json(), created, "nothing changed");
  });

  test("a block of another series answers 409 block-taken", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await createSeries(call, slug, { slug: "2e-serie", number_block: 200 });
    const problem = await assertProblem(
      await call("PATCH", `/tournaments/${slug}/series/2e-serie`, { ...asOrganizer, body: { number_block: 100 } }),
      409,
      "block-taken",
    );
    assert.equal(problem.detail, "number_block: another series of the tournament has the block 100");
  });

  test("a block cannot move under teams, and the maximum stays at least the registered teams", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug, { max_teams: 4 });
    const path = `/tournaments/${slug}/series/1re-serie-a`;
    for (const number of [101, 102, 103, 104]) {
      await createTeam(call, slug, { number });
    }
    const withdrawn = await call("PATCH", `/tournaments/${slug}/teams/104`, {
      ...asOrganizer,
      body: { status: "withdrawn" },
    });
    assert.equal(withdrawn.status, 200, "team 104 withdraws");
    await withdrawn.body?.cancel();

    let problem = await assertProblem(
      await call("PATCH", path, { ...asOrganizer, body: { number_block: 200 } }),
      409,
      "series-has-teams",
    );
    assert.equal(problem.detail, "number_block: the series has 4 teams, whose numbers are in its block: it cannot move");
    problem = await assertProblem(await call("PATCH", path, { ...asOrganizer, body: { max_teams: 2 } }), 409, "series-full");
    assert.equal(problem.detail, "max_teams: the series has 3 teams registered, max_teams cannot be below 3");
    const lowered = await call("PATCH", path, { ...asOrganizer, body: { max_teams: 3 } });
    assert.equal(lowered.status, 200, "withdrawn teams do not count: 3 registered");
    assert.equal((await lowered.json()).max_teams, 3);
  });

  test("past registration, only the name and position change", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await moveTo(call, slug, "pools");
    const path = `/tournaments/${slug}/series/1re-serie-a`;
    for (const [field, value] of [
      ["specialty", "paleta_gomme_creuse"],
      ["max_teams", 8],
      ["number_block", 100],
    ]) {
      await assertProblem(
        await call("PATCH", path, { ...asOrganizer, body: { [field]: value } }),
        409,
        "registration-closed",
      );
    }
    const res = await call("PATCH", path, { ...asOrganizer, body: { name: "1re Série", position: 3 } });
    assert.equal(res.status, 200, "name and position stay editable");
    const body = await res.json();
    assert.deepEqual([body.name, body.position], ["1re Série", 3]);
  });

  test("the target score follows the tournament", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    const res = await call("PATCH", `/tournaments/${slug}`, {
      ...asOrganizer,
      body: {
        specialties: [
          { specialty: "paleta_gomme_creuse", target_score: 35 },
          { specialty: "paleta_gomme_pleine", target_score: 30 },
        ],
      },
    });
    assert.equal(res.status, 200, "a draft changes its target scores");
    await res.body?.cancel();
    const series = await call("GET", `/tournaments/${slug}/series/1re-serie-a`, asOrganizer);
    assert.equal((await series.json()).target_score, 35, "the series plays to the new target");
  });

  test("answers 401 without the token, then 404", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await assertProblem(await call("PATCH", `/tournaments/${slug}/series/1re-serie-a`, { body: {} }), 401, "unauthorized");
    await assertProblem(
      await call("PATCH", `/tournaments/${slug}/series/2e-serie`, { ...asOrganizer, body: {} }),
      404,
      "not-found",
    );
  });
});

describe("DELETE /tournaments/:slug/series/:series", () => {
  test("deletes a series without teams", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await createSeries(call, slug, { slug: "2e-serie", number_block: 200 });
    const res = await call("DELETE", `/tournaments/${slug}/series/1re-serie-a`, asOrganizer);
    assert.equal(res.status, 204);
    assert.equal(await res.text(), "", "no body");
    assert.equal(await seriesCount(slug), 1, "the other series stays");
    await assertProblem(await call("DELETE", `/tournaments/${slug}/series/1re-serie-a`, asOrganizer), 404, "not-found");
  });

  test("a series with teams, even withdrawn ones, answers 409 series-has-teams", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await createTeam(call, slug);
    const withdrawn = await call("PATCH", `/tournaments/${slug}/teams/101`, { ...asOrganizer, body: { status: "withdrawn" } });
    await withdrawn.body?.cancel();
    const problem = await assertProblem(
      await call("DELETE", `/tournaments/${slug}/series/1re-serie-a`, asOrganizer),
      409,
      "series-has-teams",
    );
    assert.equal(problem.detail, "teams: the series has 1 team: delete its teams first");
    assert.equal(await seriesCount(slug), 1, "the series stays");
  });

  test("past registration answers 409 registration-closed", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await moveTo(call, slug, "pools");
    await assertProblem(await call("DELETE", `/tournaments/${slug}/series/1re-serie-a`, asOrganizer), 409, "registration-closed");
    assert.equal(await seriesCount(slug), 1, "the series stays");
  });

  test("answers 401 without the token", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await assertProblem(await call("DELETE", `/tournaments/${slug}/series/1re-serie-a`), 401, "unauthorized");
    assert.equal(await seriesCount(slug), 1, "the series stays");
  });
});

describe("other methods", () => {
  test("PUT on the series paths answers 405", async () => {
    const { slug } = await tournament();
    await assertProblem(await call("PUT", `/tournaments/${slug}/series`, asOrganizer), 405, "method-not-allowed");
    await assertProblem(await call("PUT", `/tournaments/${slug}/series/1re-serie-a`, asOrganizer), 405, "method-not-allowed");
  });
});

// Concurrent changes: D1 checks each guard in the statement that writes, so whatever the order in
// which the Worker runs the requests, the rows it leaves keep the rules. Each test checks the rows,
// not only the statuses, over several rounds.
describe("concurrent changes keep series and teams valid", () => {
  // Before the guards, a block moved under a team, or a specialty dropped under a series, in 1 to
  // 2 rounds out of 20: these tests catch a regression often, not always.
  const ROUNDS = 20;

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
  function ordered(round, first, others) {
    return round % 2 === 0 ? [first, ...others] : [...others, first];
  }

  async function one(sql, ...values) {
    return db.prepare(sql).bind(...values).first();
  }

  const register = (slug, number, series = "1re-serie-a") => () =>
    call("POST", `/tournaments/${slug}/teams`, { ...asOrganizer, body: teamBody({ number, series }) });

  test("a block never moves under a team registering into it", async () => {
    for (let round = 0; round < ROUNDS; round += 1) {
      const { slug } = await tournament();
      await createSeries(call, slug);
      const move = () =>
        call("PATCH", `/tournaments/${slug}/series/1re-serie-a`, { ...asOrganizer, body: { number_block: 300 } });
      const statuses = await settle(ordered(round, move, [register(slug, 105), register(slug, 106), register(slug, 107)]));
      assert.ok(statuses.every((status) => [200, 201, 409, 422].includes(status)), `round ${round}: ${statuses}`);
      const outside = await one(
        "SELECT count(*) AS n FROM teams m JOIN series x ON x.id = m.series_id " +
          "WHERE x.tournament_id = (SELECT id FROM tournaments WHERE slug = ?) " +
          "AND (m.number < x.number_block OR m.number > x.number_block + 99)",
        slug,
      );
      assert.equal(outside.n, 0, `round ${round}: every team number is in the block of its series`);
    }
  });

  test("a specialty is never dropped while a series is created on it", async () => {
    for (let round = 0; round < ROUNDS; round += 1) {
      const { slug } = await tournament();
      const drop = () =>
        call("PATCH", `/tournaments/${slug}`, {
          ...asOrganizer,
          body: { specialties: [{ specialty: "paleta_gomme_creuse", target_score: 40 }] },
        });
      const create = (key, block) => () =>
        call("POST", `/tournaments/${slug}/series`, {
          ...asOrganizer,
          body: seriesBody({ slug: key, number_block: block, specialty: "paleta_gomme_pleine" }),
        });
      const statuses = await settle(ordered(round, drop, [create("serie-a", 100), create("serie-b", 200)]));
      assert.ok(statuses.every((status) => [200, 201, 422].includes(status)), `round ${round}: ${statuses}`);
      const undeclared = await one(
        "SELECT count(*) AS n FROM series x WHERE x.tournament_id = (SELECT id FROM tournaments WHERE slug = ?) " +
          "AND x.specialty NOT IN (SELECT specialty FROM tournament_specialties WHERE tournament_id = x.tournament_id)",
        slug,
      );
      assert.equal(undeclared.n, 0, `round ${round}: every series plays a specialty of its tournament`);
    }
  });

  test("a series maximum never drops below its registered teams", async () => {
    for (let round = 0; round < 6; round += 1) {
      const { slug } = await tournament();
      await createSeries(call, slug);
      for (const number of [101, 102, 103]) {
        await createTeam(call, slug, { number });
      }
      const lower = () =>
        call("PATCH", `/tournaments/${slug}/series/1re-serie-a`, { ...asOrganizer, body: { max_teams: 3 } });
      const statuses = await settle(ordered(round, lower, [104, 105, 106].map((number) => register(slug, number))));
      assert.ok(statuses.every((status) => [200, 201, 409].includes(status)), `round ${round}: ${statuses}`);
      const row = await one(
        "SELECT x.max_teams AS max, (SELECT count(*) FROM teams m WHERE m.series_id = x.id AND m.status = 'registered') AS n " +
          "FROM series x WHERE x.tournament_id = (SELECT id FROM tournaments WHERE slug = ?)",
        slug,
      );
      assert.ok(row.n <= row.max, `round ${round}: ${row.n} registered, maximum ${row.max}`);
    }
  });

  test("a tournament maximum never drops below its registered teams", async () => {
    for (let round = 0; round < 6; round += 1) {
      const { slug } = await tournament();
      await createSeries(call, slug);
      for (const number of [101, 102, 103]) {
        await createTeam(call, slug, { number });
      }
      const lower = () => call("PATCH", `/tournaments/${slug}`, { ...asOrganizer, body: { max_teams: 3 } });
      const statuses = await settle(ordered(round, lower, [104, 105, 106].map((number) => register(slug, number))));
      assert.ok(statuses.every((status) => [200, 201, 409, 422].includes(status)), `round ${round}: ${statuses}`);
      const row = await one(
        "SELECT t.max_teams AS max, (SELECT count(*) FROM teams m WHERE m.tournament_id = t.id AND m.status = 'registered') AS n " +
          "FROM tournaments t WHERE t.slug = ?",
        slug,
      );
      assert.ok(row.n <= row.max, `round ${round}: ${row.n} registered, maximum ${row.max}`);
    }
  });
});
