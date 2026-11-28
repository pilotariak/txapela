// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The personal data of players: licence numbers and contacts are for the organizer only. They
// appear in no public response, no problem detail and no log line of the Worker. The pools,
// matches and standings show the public view of a team to every caller, the organizer included.
// The values below are fictional and distinctive, so that a leak is found by its text.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { after, before, test } from "node:test";
import {
  ORGANIZER_TOKEN,
  assertProblem,
  createSeries,
  createTournament,
  moveTo,
  playerBody,
  startApi,
  teamBody,
} from "./worker.mjs";

// Fictional values: example.org is reserved for examples (RFC 2606), and +00 is no country code.
const SECRETS = {
  ane: { licence: "LIC-SECRET-ANE-0001", contact: "secret.ane@example.org" },
  kepa: { licence: "LIC-SECRET-KEPA-0002", contact: "+00 0 00 00 00 02" },
  mattin: { licence: "LIC-SECRET-MATTIN-0003", contact: "secret.mattin@example.org" },
  peio: { licence: "LIC-SECRET-PEIO-0004", contact: "+00 0 00 00 00 04" },
};
const SECRET_VALUES = Object.values(SECRETS).flatMap(({ licence, contact }) => [licence, contact]);

// The fields that only the organizer reads.
const ORGANIZER_ONLY = /"(licence|contact|registered_on|fee_paid)"/;

const OPEN = "privacy-open";
const DRAFT = "privacy-draft";
// In pools, drawn, with a result: every route of the pools, matches and standings has data.
const POOLS = "privacy-pools";

const logs = [];
let mf;
let db;
let call;

const asOrganizer = { token: ORGANIZER_TOKEN };

function secretPlayer(name) {
  return playerBody(name, SECRETS[name.toLowerCase()]);
}

// A tournament with two series and two teams whose players carry secrets.
async function seed(slug) {
  await createTournament(call, {
    slug,
    max_teams: 3,
    specialties: [
      { specialty: "paleta_gomme_creuse", target_score: 40 },
      { specialty: "paleta_gomme_pleine", target_score: 30 },
    ],
  });
  await createSeries(call, slug, { max_teams: 2 });
  await createSeries(call, slug, { slug: "2e-serie", number_block: 200, specialty: "paleta_gomme_pleine" });
  for (const [number, series, players] of [
    [101, "1re-serie-a", ["Ane", "Kepa"]],
    [201, "2e-serie", ["Mattin", "Peio"]],
  ]) {
    const res = await call("POST", `/tournaments/${slug}/teams`, {
      ...asOrganizer,
      body: teamBody({ number, series, players: players.map(secretPlayer) }),
    });
    assert.equal(res.status, 201, `team ${number} registered`);
    await res.body?.cancel();
  }
}

// A tournament in pools: four teams whose players carry secrets, drawn in one pool, and the
// result of its first match.
async function seedPools(slug) {
  await createTournament(call, { slug });
  await createSeries(call, slug);
  for (const [number, players] of [
    [101, ["Ane", "Kepa"]],
    [102, ["Mattin", "Peio"]],
    [103, ["Ane", "Peio"]],
    [104, ["Kepa", "Mattin"]],
  ]) {
    const res = await call("POST", `/tournaments/${slug}/teams`, {
      ...asOrganizer,
      body: teamBody({ number, players: players.map(secretPlayer) }),
    });
    assert.equal(res.status, 201, `team ${number} registered`);
    await res.body?.cancel();
  }
  await moveTo(call, slug, "pools");
  const drawn = await call("POST", `/tournaments/${slug}/series/1re-serie-a/draw`, { ...asOrganizer, body: { pools: 1 } });
  assert.equal(drawn.status, 201, "pools drawn");
  await drawn.body?.cancel();
  const result = await call("PUT", `/tournaments/${slug}/series/1re-serie-a/pools/A/matches/1/result`, {
    ...asOrganizer,
    body: { teams: [101, 104], score: [40, 33] },
  });
  assert.equal(result.status, 200, "a result recorded");
  await result.body?.cancel();
}

before(async () => {
  ({ mf, db, call } = await startApi({ logs }));
  await seed(OPEN);
  await moveTo(call, OPEN, "registration");
  await seed(DRAFT);
  await seedPools(POOLS);
});

after(async () => {
  await mf.dispose();
});

function assertNoSecret(text, where) {
  for (const secret of SECRET_VALUES) {
    assert.ok(!text.includes(secret), `${where} holds no licence or contact (${secret.slice(0, 10)}…)`);
  }
}

// Every GET path of the contract, with its parameters filled in for `slug`. A new path parameter
// needs a value here: left as `{name}`, its route would answer 404 and check nothing.
function publicPaths(slug) {
  const contract = JSON.parse(readFileSync("openapi.json", "utf8"));
  return Object.entries(contract.paths)
    .filter(([, operations]) => operations.get !== undefined)
    .map(([path]) => {
      const filled = path
        .replace("/api/v1", "")
        .replace("{slug}", slug)
        .replace("{series}", "1re-serie-a")
        .replace("{number}", "101")
        .replace("{pool}", "A")
        .replace("{match}", "1");
      assert.doesNotMatch(filled, /[{}]/, `${path}: every path parameter has a value`);
      return filled;
    });
}

test("the organizer reads the licences and contacts", async () => {
  const text = await (await call("GET", `/tournaments/${OPEN}/teams`, asOrganizer)).text();
  for (const secret of SECRET_VALUES) {
    assert.ok(text.includes(secret), `the organizer view holds ${secret.slice(0, 10)}…: the data is there to leak`);
  }
});

test("no public route returns a licence, a contact, a registration date or a fee", async () => {
  const paths = [...publicPaths(OPEN), ...publicPaths(DRAFT), `/tournaments/${OPEN}/teams/201`];
  assert.ok(paths.includes(`/tournaments/${OPEN}/teams`), "the contract lists the team routes");
  for (const path of paths) {
    const res = await call("GET", path);
    const text = await res.text();
    assert.ok([200, 404].includes(res.status), `GET ${path}: ${res.status}`);
    assertNoSecret(text, `GET ${path}`);
    assert.doesNotMatch(text, ORGANIZER_ONLY, `GET ${path} holds no organizer-only field`);
  }
  const pools = publicPaths(POOLS).filter((path) => /\/(pools|matches|standings)/.test(path));
  assert.ok(pools.includes(`/tournaments/${POOLS}/series/1re-serie-a/pools/A/matches/1`), "the contract lists the match route");
  for (const options of [{}, asOrganizer]) {
    for (const path of pools) {
      const res = await call("GET", path, options);
      const text = await res.text();
      assert.equal(res.status, 200, `GET ${path}: the pools of the series are drawn`);
      assert.match(text, /"first_name":"Ane"/, `GET ${path} shows the names of the players`);
      assertNoSecret(text, `GET ${path}${options.token ? " as the organizer" : ""}`);
      assert.doesNotMatch(text, ORGANIZER_ONLY, `GET ${path} holds no organizer-only field, whoever calls`);
    }
  }
  const teams = await (await call("GET", `/tournaments/${OPEN}/teams`)).json();
  assert.deepEqual(
    teams.teams.flatMap((team) => team.players.map((player) => player.first_name)),
    ["Ane", "Kepa", "Mattin", "Peio"],
    "the public list holds the names",
  );
});

test("no problem detail quotes a licence or a contact", async () => {
  const path = `/tournaments/${OPEN}/teams`;
  const problems = [
    [
      "a contact sent as a number",
      () =>
        call("POST", path, {
          ...asOrganizer,
          body: teamBody({ number: 102, players: [playerBody("Ane", { contact: 1234567802 }), secretPlayer("Kepa")] }),
        }),
      422,
      "invalid-request",
      "1234567802",
    ],
    [
      "a blank name beside a contact",
      () =>
        call("POST", path, {
          ...asOrganizer,
          body: teamBody({ number: 102, players: [secretPlayer("Ane"), { ...secretPlayer("Kepa"), last_name: " " }] }),
        }),
      422,
      "invalid-request",
    ],
    [
      "a number taken",
      () => call("POST", path, { ...asOrganizer, body: teamBody({ number: 101, players: ["Ane", "Kepa"].map(secretPlayer) }) }),
      409,
      "number-taken",
    ],
    [
      "a full tournament",
      async () => {
        const res = await call("POST", path, {
          ...asOrganizer,
          body: teamBody({ number: 102, players: ["Mattin", "Peio"].map(secretPlayer) }),
        });
        assert.equal(res.status, 201, "the third and last place");
        await res.body?.cancel();
        return call("POST", path, {
          ...asOrganizer,
          body: teamBody({ number: 202, series: "2e-serie", players: ["Ane", "Peio"].map(secretPlayer) }),
        });
      },
      409,
      "tournament-full",
    ],
    [
      "a team of one player",
      () => call("PATCH", `${path}/101`, { ...asOrganizer, body: { players: [secretPlayer("Ane")] } }),
      422,
      "invalid-request",
    ],
    [
      "a contact sent without the token",
      () => call("PATCH", `${path}/101`, { body: { players: ["Ane", "Kepa"].map(secretPlayer) } }),
      401,
      "unauthorized",
    ],
    [
      "a contact sent as a team of a result",
      () =>
        call("PUT", `/tournaments/${POOLS}/series/1re-serie-a/pools/A/matches/1/result`, {
          ...asOrganizer,
          body: { teams: [SECRETS.ane.contact, 104], score: [40, 33] },
        }),
      422,
      "invalid-request",
    ],
    [
      "a field named after a contact in a move",
      () =>
        call("POST", `/tournaments/${POOLS}/series/1re-serie-a/pools/A/teams`, {
          ...asOrganizer,
          body: { number: 101, [SECRETS.mattin.contact]: 1 },
        }),
      422,
      "invalid-request",
    ],
  ];
  for (const [what, send, status, type, extra] of problems) {
    const res = await send();
    const text = await res.clone().text();
    const problem = await assertProblem(res, status, type);
    assertNoSecret(text, what);
    if (extra !== undefined) {
      assert.ok(!text.includes(extra), `${what}: the value sent is not quoted`);
      assert.equal(problem.detail, "invalid type, expected a string", `${what}: the detail names what was expected`);
    }
  }
});

// Waits until a log line matches, or fails after a few seconds: the Worker's logs reach the test
// through workerd's stdout and stderr.
async function logLine(pattern) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const line = logs.find((log) => pattern.test(log.message));
    if (line !== undefined) return line;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  assert.fail(`no log line matches ${pattern}: ${logs.length} lines captured`);
}

test("no log line holds a licence or a contact", async () => {
  // An internal error while the Worker holds the players in memory, so that it logs a line.
  await db
    .prepare("UPDATE teams SET status = 'future_status' WHERE number = 101 AND tournament_id = (SELECT id FROM tournaments WHERE slug = ?)")
    .bind(OPEN)
    .run();
  try {
    await assertProblem(await call("GET", `/tournaments/${OPEN}/teams`), 500, "internal");
    await assertProblem(await call("GET", `/tournaments/${OPEN}/teams/101`, asOrganizer), 500, "internal");
  } finally {
    await db
      .prepare("UPDATE teams SET status = 'registered' WHERE number = 101 AND tournament_id = (SELECT id FROM tournaments WHERE slug = ?)")
      .bind(OPEN)
      .run();
  }
  const line = await logLine(/internal error: unexpected value in column teams\.status/);
  assert.equal(line.level, "error", "the internal error is logged as an error");
  assert.ok(logs.length > 0, "the Worker's logs are captured");
  for (const log of logs) {
    assertNoSecret(log.message, `the log line ${JSON.stringify(log.message.slice(0, 40))}`);
  }
});
