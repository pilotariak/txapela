// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Extra privacy tests for pools, matches and standings: a licence or a contact sent to a write
// route is never quoted back in a problem, and no reads show it to anyone. All values are
// fictional, on example.org (reserved for examples, RFC 2606).

import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import {
  ORGANIZER_TOKEN,
  assertProblem,
  createSeries,
  createTournament,
  moveTo,
  recordResult,
  seriesPath,
  startApi,
  teamBody,
} from "./worker.mjs";

const LICENCE = "LIC-EXEMPLE-SECRET-42";
const CONTACT = "secret.exemple@example.org";
const SECRETS = [LICENCE, CONTACT];

function secretPlayer(firstName) {
  return {
    first_name: firstName,
    last_name: "Exemple",
    club: "Club Imaginaire",
    licence: LICENCE,
    contact: CONTACT,
  };
}

let mf;
let call;

before(async () => {
  ({ mf, call } = await startApi());
  await createTournament(call, { slug: "privacy-pools-extra" });
  await createSeries(call, "privacy-pools-extra");
  for (const number of [101, 102, 103, 104]) {
    const res = await call("POST", "/tournaments/privacy-pools-extra/teams", {
      token: ORGANIZER_TOKEN,
      body: teamBody({ number, players: ["Ane", "Kepa"].map(secretPlayer) }),
    });
    assert.equal(res.status, 201, `team ${number} registered`);
    await res.body?.cancel();
  }
  await moveTo(call, "privacy-pools-extra", "pools");
  const drawn = await call("POST", seriesPath("privacy-pools-extra", "/draw"), {
    token: ORGANIZER_TOKEN,
    body: { pools: 1 },
  });
  assert.equal(drawn.status, 201, "the four teams play in one pool");
  await drawn.body?.cancel();
  const result = await recordResult(call, "privacy-pools-extra", "A", 1, { teams: [101, 104], score: [40, 33] });
  assert.equal(result.status, 200, "a result gives every read some data");
  await result.body?.cancel();
});

after(async () => {
  await mf.dispose();
});

const asOrganizer = { token: ORGANIZER_TOKEN };
const slug = "privacy-pools-extra";

function assertNoSecret(text, where) {
  for (const secret of SECRETS) {
    assert.ok(!text.includes(secret), `${where} holds no licence or contact (${secret.slice(0, 10)}…)`);
  }
}

test("no licence or contact on a pool, match or standings read, for anyone", async () => {
  for (const options of [{}, asOrganizer]) {
    for (const rest of ["/pools", "/matches", "/standings", "/pools/A/matches/1"]) {
      const res = await call("GET", seriesPath(slug, rest), options);
      assert.equal(res.status, 200, `GET ${rest}`);
      const text = await res.text();
      assert.match(text, /"first_name":"Ane"/, `GET ${rest}: the names of the players are public`);
      assertNoSecret(text, `GET ${rest}${options.token ? " as the organizer" : ""}`);
      assert.doesNotMatch(text, /"(licence|contact|registered_on|fee_paid)"/, `GET ${rest}: no organizer-only field`);
    }
  }
});

test("no problem about a draw, a move, a tiebreak or a result quotes a licence or a contact", async () => {
  const sends = [
    [
      "a draw seed that is a contact",
      () => call("POST", seriesPath(slug, "/draw"), { ...asOrganizer, body: { pool_size: 4, seeds: [CONTACT] } }),
    ],
    [
      "a field named after a licence in a draw",
      () => call("POST", seriesPath(slug, "/draw"), { ...asOrganizer, body: { pool_size: 4, [LICENCE]: 1 } }),
    ],
    [
      "a move number that is a contact",
      () => call("POST", seriesPath(slug, "/pools/A/teams"), { ...asOrganizer, body: { number: CONTACT } }),
    ],
    [
      "a tiebreak team that is a contact",
      () => call("PUT", seriesPath(slug, "/pools/A/tiebreak"), { ...asOrganizer, body: { teams: [CONTACT] } }),
    ],
    [
      "a contact sent as a team of a result",
      () => call("PUT", seriesPath(slug, "/pools/A/matches/1/result"), {
        ...asOrganizer,
        body: { teams: [CONTACT, 104], score: [40, 0] },
      }),
    ],
    [
      "a contact sent as the forfeiting team",
      () => call("PUT", seriesPath(slug, "/pools/A/matches/1/result"), {
        ...asOrganizer,
        body: { teams: [101, 104], forfeited_by: CONTACT },
      }),
    ],
  ];
  for (const [what, send] of sends) {
    const res = await send();
    const text = await res.clone().text();
    const problem = await assertProblem(res, 422, "invalid-request");
    assertNoSecret(text, what);
    assertNoSecret(JSON.stringify(problem), `${what}: the problem document`);
  }
});
