// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The teams API (src/api/teams.rs): each route, for an organizer and without a token, against a
// migrated D1 in Miniflare. Players, clubs, licences and contacts are fictional.

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import {
  ORGANIZER_TOKEN,
  assertProblem,
  createSeries,
  createTeam,
  createTournament,
  moveTo,
  parisToday,
  playerBody,
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

// A fresh tournament for each test, with the series 1re-serie-a (numbers 100 to 199).
let lastSlug = 0;
async function tournament(changes = {}, series = {}) {
  lastSlug += 1;
  const created = await createTournament(call, { slug: `teams-${lastSlug}`, ...changes });
  await createSeries(call, created.slug, series);
  return created;
}

async function count(table, slug) {
  const where =
    table === "players"
      ? "team_id IN (SELECT id FROM teams WHERE tournament_id = (SELECT id FROM tournaments WHERE slug = ?))"
      : "tournament_id = (SELECT id FROM tournaments WHERE slug = ?)";
  return (await db.prepare(`SELECT count(*) AS n FROM ${table} WHERE ${where}`).bind(slug).first()).n;
}

async function assertInvalid(res, detail) {
  const body = await assertProblem(res, 422, "invalid-request");
  assert.match(body.detail ?? "", detail, `the detail names the rule: ${body.detail}`);
  return body;
}

// Sets the status of a team as the organizer.
async function setStatus(slug, number, status) {
  const res = await call("PATCH", `/tournaments/${slug}/teams/${number}`, { ...asOrganizer, body: { status } });
  assert.equal(res.status, 200, `team ${number} is ${status}`);
  await res.body?.cancel();
}

describe("POST /tournaments/:slug/teams", () => {
  test("registers a team on the day of the request, unpaid by default", async () => {
    const { slug } = await tournament();
    const before = parisToday();
    const res = await call("POST", `/tournaments/${slug}/teams`, {
      ...asOrganizer,
      body: teamBody({
        players: [
          {
            first_name: " Ane ",
            last_name: " Exemple ",
            club: " Club Exemple de Pelote ",
            licence: " LIC-EXEMPLE-ANE ",
            contact: " ane.exemple@example.org ",
          },
          { first_name: "Kepa", last_name: "Fictif" },
        ],
      }),
    });
    const after = parisToday();
    assert.equal(res.status, 201);
    assert.equal(res.headers.get("location"), `/api/v1/tournaments/${slug}/teams/101`);
    const body = await res.json();
    assert.ok([before, after].includes(body.registered_on), `registered today in Europe/Paris: ${body.registered_on}`);
    assert.deepEqual(body, {
      number: 101,
      series: "1re-serie-a",
      status: "registered",
      players: [
        {
          first_name: "Ane",
          last_name: "Exemple",
          club: "Club Exemple de Pelote",
          licence: "LIC-EXEMPLE-ANE",
          contact: "ane.exemple@example.org",
        },
        { first_name: "Kepa", last_name: "Fictif" },
      ],
      registered_on: body.registered_on,
      fee_paid: false,
    }, "values are trimmed, and optional values may be absent");
    assert.equal(await count("teams", slug), 1, "one team");
    assert.equal(await count("players", slug), 2, "two players");
  });

  test("takes the fee as paid", async () => {
    const { slug } = await tournament();
    const team = await createTeam(call, slug, { fee_paid: true });
    assert.equal(team.fee_paid, true);
  });

  test("the number falls inside the block of the series", async () => {
    const { slug } = await tournament();
    for (const number of [100, 199]) {
      const team = await createTeam(call, slug, { number });
      assert.equal(team.number, number, `${number} is in the block 100`);
    }
    for (const number of [99, 200, 0]) {
      await assertInvalid(
        await call("POST", `/tournaments/${slug}/teams`, { ...asOrganizer, body: teamBody({ number }) }),
        /^number: is from 100 to 199, the block of the series 1re-serie-a$/,
      );
    }
    assert.equal(await count("teams", slug), 2, "only the two valid teams");
  });

  test("team numbers are unique within the tournament, across its series", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug, { slug: "2e-serie", number_block: 200 });
    await createTeam(call, slug, { number: 101 });
    const problem = await assertProblem(
      await call("POST", `/tournaments/${slug}/teams`, {
        ...asOrganizer,
        body: teamBody({ number: 101, players: [playerBody("Mattin"), playerBody("Peio")] }),
      }),
      409,
      "number-taken",
    );
    assert.equal(problem.detail, "number: another team of the tournament has the number 101");
    assert.equal(await count("teams", slug), 1, "nothing written");
    assert.equal(await count("players", slug), 2, "the players of the first team only");
    const other = await tournament();
    await createTeam(call, other.slug, { number: 101 });
  });

  test("a series refuses a team beyond its maximum, and withdrawn teams free a place", async () => {
    const { slug } = await tournament({}, { max_teams: 2 });
    await createTeam(call, slug, { number: 101 });
    await createTeam(call, slug, { number: 102 });
    const problem = await assertProblem(
      await call("POST", `/tournaments/${slug}/teams`, { ...asOrganizer, body: teamBody({ number: 103 }) }),
      409,
      "series-full",
    );
    assert.equal(problem.detail, "series: 1re-serie-a is full, with its 2 teams registered");
    await setStatus(slug, 102, "withdrawn");
    await createTeam(call, slug, { number: 103 });
    assert.equal(await count("teams", slug), 3, "two registered, one withdrawn");
  });

  test("the tournament refuses a team beyond its maximum teams", async () => {
    const { slug } = await tournament({ max_teams: 2 });
    await createSeries(call, slug, { slug: "2e-serie", number_block: 200 });
    await createTeam(call, slug, { number: 101 });
    await createTeam(call, slug, { number: 201, series: "2e-serie" });
    const problem = await assertProblem(
      await call("POST", `/tournaments/${slug}/teams`, { ...asOrganizer, body: teamBody({ number: 102 }) }),
      409,
      "tournament-full",
    );
    assert.equal(problem.detail, "max_teams: the tournament is full, with its 2 teams registered");
  });

  const invalid = [
    ["an unknown series", { series: "3e-serie" }, /^series: the tournament has no such series$/],
    ["a series that cannot be a slug", { series: "Série" }, /^series: the tournament has no such series$/],
    ["one player", { players: [playerBody("Ane")] }, /^players: holds exactly two players$/],
    ["three players", { players: [playerBody("Ane"), playerBody("Kepa"), playerBody("Mattin")] }, /^players: holds exactly two players$/],
    ["an empty first name", { players: [playerBody("Ane", { first_name: " " }), playerBody("Kepa")] }, /^players\[0\]\.first_name: cannot be empty$/],
    ["an empty last name", { players: [playerBody("Ane"), playerBody("Kepa", { last_name: "" })] }, /^players\[1\]\.last_name: cannot be empty$/],
    ["an empty contact", { players: [playerBody("Ane"), playerBody("Kepa", { contact: " " })] }, /^players\[1\]\.contact: cannot be empty$/],
    ["a status", { status: "withdrawn" }, /unknown field `status`/],
    ["a registration date", { registered_on: "2027-03-01" }, /unknown field `registered_on`/],
    ["an unknown field of a player", { players: [playerBody("Ane", { email: "x" }), playerBody("Kepa")] }, /unknown field `email`/],
    ["a null contact", { players: [playerBody("Ane", { contact: null }), playerBody("Kepa")] }, /^players\[0\]\.contact cannot be null$/],
    ["a number that is not an integer", { number: 101.5 }, /^invalid type, expected i64$/],
  ];
  for (const [what, changes, detail] of invalid) {
    test(`${what} answers 422 invalid-request`, async () => {
      const { slug } = await tournament();
      await assertInvalid(await call("POST", `/tournaments/${slug}/teams`, { ...asOrganizer, body: teamBody(changes) }), detail);
      assert.equal(await count("teams", slug), 0, "nothing written");
    });
  }

  test("teams register in draft and registration, not past them", async () => {
    const { slug } = await tournament();
    await moveTo(call, slug, "registration");
    await createTeam(call, slug);
    const pools = await call("PATCH", `/tournaments/${slug}`, { ...asOrganizer, body: { status: "pools" } });
    await pools.body?.cancel();
    const problem = await assertProblem(
      await call("POST", `/tournaments/${slug}/teams`, { ...asOrganizer, body: teamBody({ number: 102, series: "no-series" }) }),
      409,
      "registration-closed",
    );
    assert.match(problem.detail, /not pools$/, "registration-closed comes before the rules");
    assert.equal(await count("teams", slug), 1, "nothing written");
  });

  test("answers 401 without the token, then 404 for an unknown tournament", async () => {
    const { slug } = await tournament();
    await assertProblem(await call("POST", `/tournaments/${slug}/teams`, { body: teamBody() }), 401, "unauthorized");
    await assertProblem(await call("POST", "/tournaments/no-such-tournament/teams", { ...asOrganizer, body: teamBody() }), 404, "not-found");
    assert.equal(await count("teams", slug), 0, "nothing written");
  });
});

describe("GET /tournaments/:slug/teams", () => {
  test("lists by number; the organizer sees licences, contacts, dates and fees, the public does not", async () => {
    const { slug } = await tournament();
    await createTeam(call, slug, { number: 102, players: [playerBody("Mattin"), playerBody("Peio", { club: undefined })] });
    await createTeam(call, slug, { number: 101, fee_paid: true });
    await moveTo(call, slug, "registration");

    const organizer = await (await call("GET", `/tournaments/${slug}/teams`, asOrganizer)).json();
    assert.deepEqual(organizer.teams.map((team) => team.number), [101, 102], "by number");
    assert.equal(organizer.teams[0].players[0].contact, "ane.exemple@example.org", "the organizer reads the contact");
    assert.equal(organizer.teams[0].players[0].licence, "LIC-EXEMPLE-ANE", "and the licence");
    assert.equal(organizer.teams[0].fee_paid, true, "and the fee");
    assert.equal(organizer.teams[0].registered_on, parisToday(), "and the registration date");

    const res = await call("GET", `/tournaments/${slug}/teams`);
    assert.equal(res.status, 200);
    assert.deepEqual(await res.json(), {
      teams: [
        {
          number: 101,
          series: "1re-serie-a",
          status: "registered",
          players: [
            { first_name: "Ane", last_name: "Exemple", club: "Club Exemple de Pelote" },
            { first_name: "Kepa", last_name: "Exemple", club: "Club Exemple de Pelote" },
          ],
        },
        {
          number: 102,
          series: "1re-serie-a",
          status: "registered",
          players: [
            { first_name: "Mattin", last_name: "Exemple", club: "Club Exemple de Pelote" },
            { first_name: "Peio", last_name: "Exemple" },
          ],
        },
      ],
    }, "numbers, series, status, names and clubs only");
  });

  test("the teams of a draft are invisible without the token: 404 as for an unknown tournament", async () => {
    const { slug } = await tournament();
    await createTeam(call, slug);
    for (const path of [`/tournaments/${slug}/teams`, `/tournaments/${slug}/teams/101`]) {
      const hidden = await assertProblem(await call("GET", path), 404, "not-found");
      const unknown = await assertProblem(await call("GET", path.replace(slug, "no-such-tournament")), 404, "not-found");
      assert.deepEqual(hidden, unknown, `${path}: a draft and an unknown tournament answer the same`);
    }
  });

  test("a team by its number; an unknown number or a path that cannot be one answers 404", async () => {
    const { slug } = await tournament();
    const created = await createTeam(call, slug);
    await moveTo(call, slug, "registration");
    const res = await call("GET", `/tournaments/${slug}/teams/101`, asOrganizer);
    assert.deepEqual(await res.json(), created, "the organizer view");
    const publicView = await (await call("GET", `/tournaments/${slug}/teams/101`)).json();
    assert.equal(publicView.fee_paid, undefined, "no fee in the public view");
    assert.equal(publicView.players[0].contact, undefined, "no contact in the public view");
    for (const number of ["102", "0101", "+101", "abc", "1e2"]) {
      await assertProblem(await call("GET", `/tournaments/${slug}/teams/${number}`), 404, "not-found");
    }
  });
});

describe("PATCH /tournaments/:slug/teams/:number", () => {
  test("replaces the players and the fee, and keeps the rest", async () => {
    const { slug } = await tournament();
    const created = await createTeam(call, slug);
    const path = `/tournaments/${slug}/teams/101`;
    const res = await call("PATCH", path, {
      ...asOrganizer,
      body: { fee_paid: true, players: [playerBody("Ane", { contact: undefined }), playerBody("Beñat")] },
    });
    assert.equal(res.status, 200);
    const expected = {
      ...created,
      fee_paid: true,
      players: [
        { first_name: "Ane", last_name: "Exemple", club: "Club Exemple de Pelote", licence: "LIC-EXEMPLE-ANE" },
        {
          first_name: "Beñat",
          last_name: "Exemple",
          club: "Club Exemple de Pelote",
          licence: "LIC-EXEMPLE-BEÑAT",
          contact: "beñat.exemple@example.org",
        },
      ],
    };
    assert.deepEqual(await res.json(), expected, "a contact left out is removed");
    assert.deepEqual(await (await call("GET", path, asOrganizer)).json(), expected, "stored");
    assert.equal(await count("players", slug), 2, "still two players");
  });

  test("the number, the series and the registration date never change", async () => {
    const { slug } = await tournament();
    const created = await createTeam(call, slug);
    const path = `/tournaments/${slug}/teams/101`;
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: { number: 102 } }), /^number cannot change/);
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: { series: "2e-serie" } }), /^series cannot change/);
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: { registered_on: "2027-01-01" } }), /^registered_on cannot change/);
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: { status: "forfeit" } }), /unknown variant `forfeit`/);
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: { players: [playerBody("Ane")] } }), /^players: holds exactly two players$/);
    assert.deepEqual(await (await call("GET", path, asOrganizer)).json(), created, "nothing changed");
  });

  test("a team withdraws, and registers again only if its series and tournament have room", async () => {
    const { slug } = await tournament({}, { max_teams: 2 });
    await createTeam(call, slug, { number: 101 });
    await createTeam(call, slug, { number: 102 });
    await setStatus(slug, 101, "withdrawn");
    await createTeam(call, slug, { number: 103 });
    const problem = await assertProblem(
      await call("PATCH", `/tournaments/${slug}/teams/101`, { ...asOrganizer, body: { status: "registered" } }),
      409,
      "series-full",
    );
    assert.equal(problem.detail, "series: 1re-serie-a is full, with its 2 teams registered");
    await setStatus(slug, 103, "withdrawn");
    await setStatus(slug, 101, "registered");
  });

  test("past registration, the status is fixed, and the players and fee stay editable", async () => {
    const { slug } = await tournament();
    await createTeam(call, slug);
    await moveTo(call, slug, "pools");
    const path = `/tournaments/${slug}/teams/101`;
    for (const status of ["withdrawn", "registered"]) {
      await assertProblem(
        await call("PATCH", path, { ...asOrganizer, body: { status } }),
        409,
        "registration-closed",
      );
    }
    const res = await call("PATCH", path, { ...asOrganizer, body: { fee_paid: true } });
    assert.equal(res.status, 200, "the fee is paid at the first match");
    assert.equal((await res.json()).fee_paid, true);
  });

  test("answers 401 without the token, then 404", async () => {
    const { slug } = await tournament();
    await createTeam(call, slug);
    await assertProblem(await call("PATCH", `/tournaments/${slug}/teams/101`, { body: { fee_paid: true } }), 401, "unauthorized");
    await assertProblem(await call("PATCH", `/tournaments/${slug}/teams/102`, { ...asOrganizer, body: {} }), 404, "not-found");
  });
});

describe("DELETE /tournaments/:slug/teams/:number", () => {
  test("deletes a team with its players", async () => {
    const { slug } = await tournament();
    await createTeam(call, slug, { number: 101 });
    await createTeam(call, slug, { number: 102 });
    const res = await call("DELETE", `/tournaments/${slug}/teams/101`, asOrganizer);
    assert.equal(res.status, 204);
    assert.equal(await res.text(), "", "no body");
    assert.equal(await count("teams", slug), 1, "the other team stays");
    assert.equal(await count("players", slug), 2, "with its two players only");
    await assertProblem(await call("DELETE", `/tournaments/${slug}/teams/101`, asOrganizer), 404, "not-found");
  });

  test("past registration answers 409 registration-closed", async () => {
    const { slug } = await tournament();
    await createTeam(call, slug);
    await moveTo(call, slug, "pools");
    await assertProblem(await call("DELETE", `/tournaments/${slug}/teams/101`, asOrganizer), 409, "registration-closed");
    assert.equal(await count("teams", slug), 1, "the team stays");
  });

  test("answers 401 without the token", async () => {
    const { slug } = await tournament();
    await createTeam(call, slug);
    await assertProblem(await call("DELETE", `/tournaments/${slug}/teams/101`), 401, "unauthorized");
    assert.equal(await count("teams", slug), 1, "the team stays");
  });
});

describe("other methods", () => {
  test("PUT on the teams paths answers 405", async () => {
    const { slug } = await tournament();
    await assertProblem(await call("PUT", `/tournaments/${slug}/teams`, asOrganizer), 405, "method-not-allowed");
    await assertProblem(await call("PUT", `/tournaments/${slug}/teams/101`, asOrganizer), 405, "method-not-allowed");
  });
});

// Concurrent writes for the last places: D1 checks the room in the statement that writes, so the
// requests that come too late answer 409, whatever the order in which the Worker runs them.
describe("concurrent registrations", () => {
  async function registerAll(slug, bodies) {
    const responses = await Promise.all(
      bodies.map((body) => call("POST", `/tournaments/${slug}/teams`, { ...asOrganizer, body: teamBody(body) })),
    );
    const results = [];
    for (const res of responses) {
      results.push([res.status, res.status === 201 ? null : (await res.json()).type]);
      if (res.status === 201) await res.body?.cancel();
    }
    return results;
  }

  async function registeredCount(slug) {
    const row = await db
      .prepare(
        "SELECT count(*) AS n FROM teams WHERE status = 'registered' AND tournament_id = (SELECT id FROM tournaments WHERE slug = ?)",
      )
      .bind(slug)
      .first();
    return row.n;
  }

  test("never take a series past its maximum", async () => {
    for (let round = 0; round < 5; round += 1) {
      const { slug } = await tournament({}, { max_teams: 2 });
      const results = await registerAll(
        slug,
        [101, 102, 103, 104, 105, 106].map((number) => ({ number })),
      );
      assert.equal(results.filter(([status]) => status === 201).length, 2, `round ${round}: two places`);
      for (const [status, type] of results.filter(([code]) => code !== 201)) {
        assert.deepEqual([status, type], [409, "https://txapela.pilotariak.com/problems/series-full"], `round ${round}`);
      }
      assert.equal(await registeredCount(slug), 2, `round ${round}: two teams stored`);
      assert.equal(await count("players", slug), 4, `round ${round}: the players of the two teams only`);
    }
  });

  test("never take a tournament past its maximum, across series", async () => {
    for (let round = 0; round < 5; round += 1) {
      const { slug } = await tournament({ max_teams: 2 });
      await createSeries(call, slug, { slug: "2e-serie", number_block: 200 });
      const results = await registerAll(slug, [
        { number: 101 },
        { number: 201, series: "2e-serie" },
        { number: 102 },
        { number: 202, series: "2e-serie" },
      ]);
      assert.equal(results.filter(([status]) => status === 201).length, 2, `round ${round}: two places`);
      for (const [status, type] of results.filter(([code]) => code !== 201)) {
        assert.deepEqual([status, type], [409, "https://txapela.pilotariak.com/problems/tournament-full"], `round ${round}`);
      }
      assert.equal(await registeredCount(slug), 2, `round ${round}: two teams stored`);
    }
  });

  test("never register withdrawn teams again past the maximum", async () => {
    const { slug } = await tournament({}, { max_teams: 2 });
    for (const number of [101, 102, 103, 104]) {
      if (number > 102) {
        await setStatus(slug, number - 2, "withdrawn");
      }
      await createTeam(call, slug, { number });
    }
    await setStatus(slug, 103, "withdrawn");
    await setStatus(slug, 104, "withdrawn");
    const responses = await Promise.all(
      [101, 102, 103, 104].map((number) =>
        call("PATCH", `/tournaments/${slug}/teams/${number}`, {
          ...asOrganizer,
          body: { status: "registered", players: [playerBody("Mattin"), playerBody("Peio")] },
        }),
      ),
    );
    const statuses = [];
    for (const res of responses) {
      statuses.push(res.status);
      await res.body?.cancel();
    }
    assert.equal(statuses.filter((status) => status === 200).length, 2, `two places: ${statuses}`);
    assert.equal(statuses.filter((status) => status === 409).length, 2, `the others are refused: ${statuses}`);
    assert.equal(await registeredCount(slug), 2, "two teams registered");
    assert.equal(await count("players", slug), 8, "every team keeps two players, refused or not");
    const { teams } = await (await call("GET", `/tournaments/${slug}/teams`, asOrganizer)).json();
    for (const team of teams.filter((t) => t.status === "withdrawn")) {
      assert.deepEqual(
        team.players.map((player) => player.first_name),
        ["Ane", "Kepa"],
        `a refused change leaves the players of team ${team.number} as they were`,
      );
    }
  });
});
