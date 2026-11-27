// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Cross-cutting checks of the series and teams routes (issue #7): the order of the checks the
// routes share, values that span tournaments, series and teams, and the personal data of players
// in responses other than a public GET. All fixtures are fictional.

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import {
  ORGANIZER_TOKEN,
  allowedOrigins,
  assertProblem,
  createSeries,
  createTeam,
  createTournament,
  moveTo,
  playerBody,
  seriesBody,
  startApi,
  teamBody,
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

const BOTH = [
  { specialty: "paleta_gomme_creuse", target_score: 40 },
  { specialty: "paleta_gomme_pleine", target_score: 30 },
];

let lastSlug = 0;
async function tournament(changes = {}) {
  lastSlug += 1;
  return createTournament(call, { slug: `checks-${lastSlug}`, specialties: BOTH, ...changes });
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

describe("the order of checks on the series and teams routes", () => {
  test("401 comes before a bad key, a missing tournament and a malformed body", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    for (const [method, path, body] of [
      ["POST", "/tournaments/Serie/series", "{"],
      ["POST", "/tournaments/no-such-tournament/series", "{"],
      ["PATCH", `/tournaments/${slug}/series/Serie`, "{"],
      ["PATCH", `/tournaments/${slug}/series/no-series`, "{"],
      ["POST", "/tournaments/no-such-tournament/teams", "{"],
      ["PATCH", `/tournaments/${slug}/teams/0101`, "{"],
      ["DELETE", `/tournaments/${slug}/teams/0101`, undefined],
    ]) {
      const res = await call(method, path, { body });
      await assertProblem(res, 401, "unauthorized");
      assert.equal(res.headers.get("www-authenticate"), "Bearer", `${method} ${path}: WWW-Authenticate`);
    }
  });

  test("404 for a path that cannot be a key comes before the body", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    for (const [method, path] of [
      ["POST", "/tournaments/Serie/series"],
      ["PATCH", `/tournaments/${slug}/series/Serie`],
      ["PATCH", `/tournaments/${slug}/series/no-series`],
      ["POST", "/tournaments/no-such-tournament/teams"],
      ["PATCH", `/tournaments/${slug}/teams/0101`],
      ["DELETE", `/tournaments/${slug}/teams/0101`],
    ]) {
      const body = method === "DELETE" ? undefined : "{";
      await assertProblem(await call(method, path, { ...asOrganizer, body }), 404, "not-found");
    }
  });

  test("a bad body answers 422 before the status rules", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await createTeam(call, slug);
    await moveTo(call, slug, "pools");
    for (const [body, detail] of [
      ["{", /^the body is not JSON$/],
      ["null", /^the body cannot be null$/],
      ['{"level":"A"}', /unknown field `level`/],
    ]) {
      await assertInvalid(
        await call("POST", `/tournaments/${slug}/series`, { ...asOrganizer, body }),
        detail,
      );
      await assertInvalid(
        await call("PATCH", `/tournaments/${slug}/series/1re-serie-a`, { ...asOrganizer, body }),
        detail,
      );
      await assertInvalid(await call("POST", `/tournaments/${slug}/teams`, { ...asOrganizer, body }), detail);
      await assertInvalid(
        await call("PATCH", `/tournaments/${slug}/teams/101`, { ...asOrganizer, body }),
        detail,
      );
    }
    await assertInvalid(
      await call("PATCH", `/tournaments/${slug}/series/1re-serie-a`, { ...asOrganizer, body: { slug: "autre" } }),
      /^slug cannot change: it is the public key of the series$/,
    );
    await assertInvalid(
      await call("PATCH", `/tournaments/${slug}/teams/101`, { ...asOrganizer, body: { number: 102 } }),
      /^number cannot change: it is the public key of the team$/,
    );
  });

  test("registration-closed answers 409 before a broken rule on the merged state", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await createTeam(call, slug);
    await moveTo(call, slug, "pools");
    const detail = "status: series and teams change only while the tournament is draft or registration, not pools";
    for (const [method, path, body] of [
      ["POST", `/tournaments/${slug}/series`, seriesBody({ slug: "2e-serie", name: " ", number_block: 200 })],
      ["PATCH", `/tournaments/${slug}/series/1re-serie-a`, { name: " ", number_block: 200 }],
      ["POST", `/tournaments/${slug}/teams`, teamBody({ number: 500, players: [playerBody("Ane")] })],
      ["PATCH", `/tournaments/${slug}/teams/101`, { status: "withdrawn", players: [playerBody("Ane")] }],
    ]) {
      const problem = await assertProblem(await call(method, path, { ...asOrganizer, body }), 409, "registration-closed");
      assert.equal(problem.detail, detail, `${method} ${path}: the status comes first`);
    }
  });

  test("a broken value on the merged state answers 422 before the maximums", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug, { max_teams: 2 });
    await createTeam(call, slug, { number: 101 });
    await createTeam(call, slug, { number: 102 });
    await assertInvalid(
      await call("POST", `/tournaments/${slug}/teams`, { ...asOrganizer, body: teamBody({ number: 500 }) }),
      /^number: is from 100 to 199, the block of the series 1re-serie-a$/,
    );
    await assertInvalid(
      await call("PATCH", `/tournaments/${slug}/series/1re-serie-a`, { ...asOrganizer, body: { name: " ", max_teams: 1 } }),
      /^name: cannot be empty$/,
    );
    await setStatus(slug, 101, "withdrawn");
    await assertInvalid(
      await call("PATCH", `/tournaments/${slug}/teams/101`, {
        ...asOrganizer,
        body: { status: "registered", players: [playerBody("Ane")] },
      }),
      /^players: holds exactly two players$/,
    );
  });

  test("a tournament PATCH keeps 401, 404, body, status and entry rules in order", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug, { slug: "feminines", specialty: "paleta_gomme_pleine", number_block: 200 });
    await createTeam(call, slug, { number: 201, series: "feminines" });
    const path = `/tournaments/${slug}`;

    await assertProblem(await call("PATCH", "/tournaments/Serie", { body: "{" }), 401, "unauthorized");
    await assertProblem(await call("PATCH", "/tournaments/Serie", { ...asOrganizer, body: "{" }), 404, "not-found");
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: "{" }), /^the body is not JSON$/);

    const transition = await assertProblem(
      await call("PATCH", path, { ...asOrganizer, body: { specialties: [BOTH[0]], status: "pools" } }),
      409,
      "invalid-transition",
    );
    assert.equal(transition.detail, "status: a draft tournament moves to registration only, not to pools");

    await assertInvalid(
      await call("PATCH", path, { ...asOrganizer, body: { specialties: [BOTH[0]], max_teams: 1 } }),
      /^max_teams: is an integer from 2 to 9007199254740991$/,
    );
    await assertInvalid(
      await call("PATCH", path, { ...asOrganizer, body: { specialties: [BOTH[0]] } }),
      /^specialties: the series feminines plays paleta_gomme_pleine: the tournament keeps it while the series does$/,
    );

    const moved = await call("PATCH", path, { ...asOrganizer, body: { status: "registration" } });
    assert.equal(moved.status, 200, "the tournament moves to registration");
    await moved.body?.cancel();
    const problem = await assertProblem(
      await call("PATCH", path, { ...asOrganizer, body: { specialties: [BOTH[0]] } }),
      409,
      "not-a-draft",
    );
    assert.equal(
      problem.detail,
      "specialties: only a draft tournament can change its aire, specialties, points, ranking and courts",
      "not-a-draft comes before the check of the series",
    );
  });

  test("registration-closed comes before conflicts with another series or team", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await createSeries(call, slug, { slug: "2e-serie", number_block: 200 });
    await createTeam(call, slug);
    await moveTo(call, slug, "pools");
    const detail = "status: series and teams change only while the tournament is draft or registration, not pools";
    for (const [method, path, body] of [
      ["DELETE", `/tournaments/${slug}/series/1re-serie-a`, undefined],
      ["POST", `/tournaments/${slug}/series`, seriesBody({ name: "Autre série" })],
      ["PATCH", `/tournaments/${slug}/series/1re-serie-a`, { number_block: 200 }],
    ]) {
      const problem = await assertProblem(await call(method, path, { ...asOrganizer, body }), 409, "registration-closed");
      assert.equal(problem.detail, detail, `${method} ${path}: the status comes first`);
    }
  });

  test("a full series answers series-full before number-taken", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug, { max_teams: 2 });
    await createTeam(call, slug, { number: 101 });
    await createTeam(call, slug, { number: 102 });
    const problem = await assertProblem(
      await call("POST", `/tournaments/${slug}/teams`, { ...asOrganizer, body: teamBody({ number: 101 }) }),
      409,
      "series-full",
    );
    assert.equal(problem.detail, "series: 1re-serie-a is full, with its 2 teams registered");
  });

  test("a status patch to the current value past registration still answers 409", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await createTeam(call, slug);
    await moveTo(call, slug, "pools");
    const problem = await assertProblem(
      await call("PATCH", `/tournaments/${slug}/teams/101`, { ...asOrganizer, body: { status: "registered" } }),
      409,
      "registration-closed",
    );
    assert.equal(
      problem.detail,
      "status: series and teams change only while the tournament is draft or registration, not pools",
    );
  });

  test("a bad team number in a PATCH or DELETE path answers 404", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await createTeam(call, slug);
    for (const number of ["0", "0101", "+101", "-101", "1e2"]) {
      await assertProblem(
        await call("PATCH", `/tournaments/${slug}/teams/${number}`, { ...asOrganizer, body: {} }),
        404,
        "not-found",
      );
      await assertProblem(await call("DELETE", `/tournaments/${slug}/teams/${number}`, asOrganizer), 404, "not-found");
    }
  });
});

describe("values that span tournaments, series and teams", () => {
  test("a specialty removed from the tournament is refused by a new or changed series", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    const removed = await call("PATCH", `/tournaments/${slug}`, { ...asOrganizer, body: { specialties: [BOTH[0]] } });
    assert.equal(removed.status, 200, "a specialty no series plays can be removed");
    await removed.body?.cancel();

    const detail = /^specialty: paleta_gomme_pleine is not a specialty of the tournament$/;
    await assertInvalid(
      await call("POST", `/tournaments/${slug}/series`, {
        ...asOrganizer,
        body: seriesBody({ slug: "feminines", name: "Féminines", specialty: "paleta_gomme_pleine", number_block: 200 }),
      }),
      detail,
    );
    await assertInvalid(
      await call("PATCH", `/tournaments/${slug}/series/1re-serie-a`, { ...asOrganizer, body: { specialty: "paleta_gomme_pleine" } }),
      detail,
    );
    const kept = await call("GET", `/tournaments/${slug}/series/1re-serie-a`, asOrganizer);
    assert.equal((await kept.json()).specialty, "paleta_gomme_creuse", "the existing series keeps its specialty");
  });

  test("the last number block accepts 9900 and 9999 only", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug, { number_block: 9900, max_teams: 4 });
    for (const number of [9900, 9999]) {
      const team = await createTeam(call, slug, { number });
      assert.equal(team.number, number, `${number} is in the last block`);
    }
    for (const number of [9899, 10000]) {
      await assertInvalid(
        await call("POST", `/tournaments/${slug}/teams`, { ...asOrganizer, body: teamBody({ number }) }),
        /^number: is from 9900 to 9999, the block of the series 1re-serie-a$/,
      );
    }
  });

  test("a number of another series' block is invalid, and a withdrawn team keeps its number", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await createSeries(call, slug, { slug: "2e-serie", number_block: 200 });
    await assertInvalid(
      await call("POST", `/tournaments/${slug}/teams`, { ...asOrganizer, body: teamBody({ number: 201 }) }),
      /^number: is from 100 to 199, the block of the series 1re-serie-a$/,
    );
    await createTeam(call, slug, { number: 101 });
    await setStatus(slug, 101, "withdrawn");
    const problem = await assertProblem(
      await call("POST", `/tournaments/${slug}/teams`, {
        ...asOrganizer,
        body: teamBody({ number: 101, players: [playerBody("Mattin"), playerBody("Peio")] }),
      }),
      409,
      "number-taken",
    );
    assert.equal(problem.detail, "number: another team of the tournament has the number 101");
  });

  test("withdrawing frees a tournament place, but registering the team again needs one", async () => {
    const { slug } = await tournament({ max_teams: 2 });
    await createSeries(call, slug);
    await createSeries(call, slug, { slug: "2e-serie", number_block: 200 });
    await createTeam(call, slug, { number: 101 });
    await createTeam(call, slug, { number: 201, series: "2e-serie" });
    await setStatus(slug, 101, "withdrawn");

    const next = await createTeam(call, slug, { number: 102 });
    assert.equal(next.number, 102, "a withdrawn team frees a place");
    const problem = await assertProblem(
      await call("PATCH", `/tournaments/${slug}/teams/101`, { ...asOrganizer, body: { status: "registered" } }),
      409,
      "tournament-full",
    );
    assert.equal(problem.detail, "max_teams: the tournament is full, with its 2 teams registered");

    await setStatus(slug, 102, "withdrawn");
    const back = await call("PATCH", `/tournaments/${slug}/teams/101`, { ...asOrganizer, body: { status: "registered" } });
    assert.equal(back.status, 200, "a place is free again");
    assert.equal((await back.json()).status, "registered");
  });

  test("a wrong token is 401 on an item of a draft, where no token is 404", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await createTeam(call, slug);
    for (const path of [`/tournaments/${slug}/series/1re-serie-a`, `/tournaments/${slug}/teams/101`]) {
      const hidden = await assertProblem(await call("GET", path), 404, "not-found");
      const unknown = await assertProblem(await call("GET", path.replace(slug, "no-such-tournament")), 404, "not-found");
      assert.deepEqual(hidden, unknown, `${path}: a draft and an unknown tournament answer the same`);
      await assertProblem(await call("GET", path, { token: "wrong-token" }), 401, "unauthorized");
      const organizer = await call("GET", path, asOrganizer);
      assert.equal(organizer.status, 200, `${path}: the organizer reads it`);
      await organizer.body?.cancel();
    }
  });

  test("a series with only withdrawn teams still holds its specialty", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug, { slug: "feminines", specialty: "paleta_gomme_pleine", number_block: 200 });
    await createTeam(call, slug, { number: 201, series: "feminines" });
    await setStatus(slug, 201, "withdrawn");
    await assertInvalid(
      await call("PATCH", `/tournaments/${slug}`, { ...asOrganizer, body: { specialties: [BOTH[0]] } }),
      /^specialties: the series feminines plays paleta_gomme_pleine: the tournament keeps it while the series does$/,
    );
  });

  test("a tournament PATCH counts registered teams across series, in draft and in pools", async () => {
    const { slug } = await tournament({ max_teams: 3 });
    await createSeries(call, slug);
    await createSeries(call, slug, { slug: "2e-serie", number_block: 200 });
    await createTeam(call, slug, { number: 101 });
    await createTeam(call, slug, { number: 201, series: "2e-serie" });
    await createTeam(call, slug, { number: 102 });
    const path = `/tournaments/${slug}`;
    const detail = /^max_teams: 3 teams are registered, max_teams cannot be below 3$/;
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: { max_teams: 2 } }), detail);

    await setStatus(slug, 102, "withdrawn");
    const lowered = await call("PATCH", path, { ...asOrganizer, body: { max_teams: 2 } });
    assert.equal(lowered.status, 200, "a withdrawn team is not counted");
    await lowered.body?.cancel();

    const restored = await call("PATCH", path, { ...asOrganizer, body: { max_teams: 3 } });
    assert.equal(restored.status, 200);
    await restored.body?.cancel();
    await createTeam(call, slug, { number: 103 });
    await moveTo(call, slug, "pools");
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: { max_teams: 2 } }), detail);
  });
});

describe("player contact details in public responses", () => {
  // Fictional and distinctive values: example.org is reserved for examples (RFC 2606).
  const SECRETS = {
    ane: { licence: "LIC-CHECKS-ANE-001", contact: "checks.ane@example.org" },
    kepa: { licence: "LIC-CHECKS-KEPA-002", contact: "+00 0 00 00 00 02" },
  };
  const VALUES = Object.values(SECRETS).flatMap(({ licence, contact }) => [licence, contact]);
  const ORGANIZER_ONLY = /"(licence|contact|registered_on|fee_paid)"/;

  function assertNoSecret(text, where) {
    for (const value of VALUES) {
      assert.ok(!text.includes(value), `${where}: no licence or contact (${value.slice(0, 8)}…)`);
    }
  }

  test("no public response, problem or preflight holds a contact or a licence", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    const registered = await call("POST", `/tournaments/${slug}/teams`, {
      ...asOrganizer,
      body: teamBody({ players: [playerBody("Ane", SECRETS.ane), playerBody("Kepa", SECRETS.kepa)] }),
    });
    assert.equal(registered.status, 201, "the team is registered");
    await registered.body?.cancel();
    const organizer = await (await call("GET", `/tournaments/${slug}/teams`, asOrganizer)).text();
    for (const value of VALUES) {
      assert.ok(organizer.includes(value), `the organizer view holds ${value.slice(0, 8)}…: the data exists`);
    }
    await moveTo(call, slug, "registration");

    for (const path of [
      `/tournaments/${slug}/teams`,
      `/tournaments/${slug}/teams/101`,
      `/tournaments/${slug}/series`,
      `/tournaments/${slug}/series/1re-serie-a`,
    ]) {
      const res = await call("GET", path);
      const text = await res.text();
      assert.equal(res.status, 200, `GET ${path}`);
      assertNoSecret(text, `GET ${path}`);
      assert.doesNotMatch(text, ORGANIZER_ONLY, `GET ${path} has no organizer-only field`);
    }

    // A withdrawn team is still public, and still has no organizer-only field.
    await setStatus(slug, 101, "withdrawn");
    const withdrawn = await call("GET", `/tournaments/${slug}/teams/101`);
    const text = await withdrawn.text();
    assert.equal(withdrawn.status, 200, "a withdrawn team stays public");
    assert.equal(JSON.parse(text).status, "withdrawn");
    assertNoSecret(text, "a withdrawn team");
    assert.doesNotMatch(text, ORGANIZER_ONLY, "a withdrawn team has no organizer-only field");

    const draft = await tournament();
    await createSeries(call, draft.slug);
    await call("POST", `/tournaments/${draft.slug}/teams`, {
      ...asOrganizer,
      body: teamBody({ players: [playerBody("Ane", SECRETS.ane), playerBody("Kepa", SECRETS.kepa)] }),
    });
    for (const [what, send] of [
      ["a draft's team list", () => call("GET", `/tournaments/${draft.slug}/teams`)],
      ["a draft's team item", () => call("GET", `/tournaments/${draft.slug}/teams/101`)],
      ["an unknown team", () => call("GET", `/tournaments/${slug}/teams/102`)],
      ["a wrong method", () => call("PUT", `/tournaments/${slug}/teams`, { body: {} })],
      ["a wrong token", () => call("GET", `/tournaments/${slug}/teams`, { token: "wrong-token" })],
    ]) {
      const res = await send();
      assertNoSecret(await res.text(), what);
    }

    const origin = allowedOrigins[0];
    assert.notEqual(origin, undefined, "wrangler.toml lists an allowed origin");
    for (const [what, method, headers] of [
      ["an allowed preflight", "OPTIONS", { Origin: origin }],
      ["a disallowed preflight", "OPTIONS", { Origin: "https://evil.example" }],
      ["a public GET with an origin", "GET", { Origin: origin }],
    ]) {
      const res = await call(method, `/tournaments/${slug}/teams`, { headers });
      assertNoSecret(await res.text(), what);
    }
  });

  test("a problem detail never quotes a contact sent as a number", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    const problem = await assertProblem(
      await call("POST", `/tournaments/${slug}/teams`, {
        ...asOrganizer,
        body: teamBody({ players: [playerBody("Ane", { contact: 1234567802 }), playerBody("Kepa")] }),
      }),
      422,
      "invalid-request",
    );
    assert.equal(problem.detail, "invalid type, expected a string", "the detail names the expected type");
  });

  test("an unknown field named after a contact is not echoed in the problem detail", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    const contact = "leak.champ@example.org";
    const res = await call("POST", `/tournaments/${slug}/teams`, {
      ...asOrganizer,
      body: { ...teamBody(), [contact]: 1 },
    });
    const text = await res.clone().text();
    const problem = await assertProblem(res, 422, "invalid-request");
    assert.ok(!text.includes(contact), `no contact value in the problem: ${problem.detail}`);
  });

  test("an unknown status equal to a contact is not echoed in the problem detail", async () => {
    const { slug } = await tournament();
    await createSeries(call, slug);
    await createTeam(call, slug);
    const contact = "leak.statut@example.org";
    const res = await call("PATCH", `/tournaments/${slug}/teams/101`, { ...asOrganizer, body: { status: contact } });
    const text = await res.clone().text();
    const problem = await assertProblem(res, 422, "invalid-request");
    assert.ok(!text.includes(contact), `no contact value in the problem: ${problem.detail}`);
  });
});
