// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The interim organizer check (src/api/auth.rs): `Authorization: Bearer <token>`, whose SHA-256
// must be the ORGANIZER_TOKEN_SHA256 secret. It fails closed: no secret, a malformed secret, no
// header, a malformed header or a wrong token answer 401 unauthorized.

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import {
  ORGANIZER_TOKEN,
  assertProblem,
  createTournament,
  sha256Hex,
  startApi,
  tournamentBody,
} from "./worker.mjs";

let mf;
let call;
let draft;

before(async () => {
  ({ mf, call } = await startApi());
  draft = await createTournament(call, { slug: "auth-draft" });
});

after(async () => {
  await mf.dispose();
});

// Every write route, with a body that would succeed for an organizer.
const writes = [
  ["POST", "/tournaments", tournamentBody({ slug: "auth-new" })],
  ["PATCH", "/tournaments/auth-draft", { name: "Autre nom" }],
  ["DELETE", "/tournaments/auth-draft", undefined],
];

async function assertUnauthorized(res, why) {
  const text = await res.clone().text();
  await assertProblem(res, 401, "unauthorized");
  assert.equal(res.headers.get("www-authenticate"), "Bearer", `${why}: WWW-Authenticate`);
  assert.ok(!text.includes(ORGANIZER_TOKEN), `${why}: the token is never echoed`);
  assert.ok(!text.includes(sha256Hex(ORGANIZER_TOKEN)), `${why}: neither is its hash`);
}

describe("write routes", () => {
  test("need the organizer token", async () => {
    for (const [method, path, body] of writes) {
      await assertUnauthorized(await call(method, path, { body }), `${method} ${path} without a token`);
    }
  });

  test("refuse a malformed Authorization header or a wrong token", async () => {
    const headers = [
      "Basic dXNlcjpwYXNz",
      "Bearer",
      "Bearer ",
      `Token ${ORGANIZER_TOKEN}`,
      `Bearer  ${ORGANIZER_TOKEN}`,
      `Bearer ${ORGANIZER_TOKEN} extra`,
      ORGANIZER_TOKEN,
      `Bearer ${ORGANIZER_TOKEN}x`,
      `Bearer ${ORGANIZER_TOKEN.toUpperCase()}`,
      `Bearer ${sha256Hex(ORGANIZER_TOKEN)}`,
    ];
    for (const [method, path, body] of writes) {
      for (const authorization of headers) {
        await assertUnauthorized(
          await call(method, path, { body, headers: { Authorization: authorization } }),
          `${method} ${path} with ${JSON.stringify(authorization)}`,
        );
      }
    }
    const stored = await call("GET", `/tournaments/${draft.slug}`, { token: ORGANIZER_TOKEN });
    assert.deepEqual(await stored.json(), draft, "no refused write changed the draft");
  });

  test("accept the organizer token, whatever the case of the scheme", async () => {
    const res = await call("PATCH", "/tournaments/auth-draft", {
      body: { max_teams: 12 },
      headers: { Authorization: `bearer ${ORGANIZER_TOKEN}` },
    });
    assert.equal(res.status, 200, "the scheme is case-insensitive");
    assert.equal((await res.json()).max_teams, 12);
  });
});

describe("read routes", () => {
  test("are public without a header, and refuse a wrong token instead of hiding drafts", async () => {
    const list = await call("GET", "/tournaments");
    assert.equal(list.status, 200, "no header: the public list");
    assert.ok(
      !(await list.json()).tournaments.some((t) => t.slug === draft.slug),
      "the public list leaves the draft out",
    );
    for (const path of ["/tournaments", `/tournaments/${draft.slug}`]) {
      await assertUnauthorized(await call("GET", path, { token: "wrong-token" }), `GET ${path} with a wrong token`);
      await assertUnauthorized(
        await call("GET", path, { headers: { Authorization: "Basic dXNlcjpwYXNz" } }),
        `GET ${path} with another scheme`,
      );
      const res = await call("GET", path, { token: ORGANIZER_TOKEN });
      assert.equal(res.status, 200, `GET ${path} with the token`);
      await res.body?.cancel();
    }
  });

  test("the health route needs no token", async () => {
    const res = await call("GET", "/health", { token: "wrong-token" });
    assert.equal(res.status, 200, "the health route ignores Authorization");
    await res.body?.cancel();
  });
});

// A Worker whose secret is missing or malformed: every organizer call answers 401, and public
// reads keep working.
for (const [what, secret] of [
  ["without the ORGANIZER_TOKEN_SHA256 secret", undefined],
  ["with an empty secret", { type: "text", value: "" }],
  ["with an uppercase secret", { type: "text", value: sha256Hex(ORGANIZER_TOKEN).toUpperCase() }],
  ["with a secret that is not a SHA-256", { type: "text", value: ORGANIZER_TOKEN }],
  ["with the SHA-256 of an empty token", { type: "text", value: sha256Hex("") }],
]) {
  describe(`a Worker ${what}`, () => {
    let closed;

    before(async () => {
      closed = await startApi({ bindings: { ORGANIZER_TOKEN_SHA256: secret } });
    });

    after(async () => {
      await closed.mf.dispose();
    });

    test("fails closed: the organizer token answers 401", async () => {
      for (const [method, path, body] of writes) {
        await assertUnauthorized(
          await closed.call(method, path, { token: ORGANIZER_TOKEN, body }),
          `${method} ${path}`,
        );
      }
      for (const authorization of ["Bearer", "Bearer ", "Bearer ="]) {
        await assertUnauthorized(
          await closed.call("POST", "/tournaments", {
            body: tournamentBody({ slug: "auth-empty" }),
            headers: { Authorization: authorization },
          }),
          `an empty token, ${JSON.stringify(authorization)}`,
        );
      }
      await assertUnauthorized(
        await closed.call("GET", "/tournaments", { token: ORGANIZER_TOKEN }),
        "a read with the token",
      );
    });

    test("public reads keep working", async () => {
      const res = await closed.call("GET", "/tournaments");
      assert.equal(res.status, 200);
      assert.deepEqual(await res.json(), { tournaments: [] });
    });
  });
}

// Round-1 checks of the token syntax and of the secret's exact accepted form.

describe("the secret", () => {
  test("surrounding blanks around a valid SHA-256 are ignored", async () => {
    const open = await startApi({
      bindings: {
        ORGANIZER_TOKEN_SHA256: { type: "text", value: ` \t${sha256Hex(ORGANIZER_TOKEN)}\n ` },
      },
    });
    try {
      const created = await open.call("POST", "/tournaments", {
        token: ORGANIZER_TOKEN,
        body: tournamentBody({ slug: "blanks-around-secret" }),
      });
      assert.equal(created.status, 201, "the trimmed hash is the secret");
      await created.body?.cancel();
      const hidden = await open.call("GET", "/tournaments/blanks-around-secret");
      await assertProblem(hidden, 404, "not-found");
    } finally {
      await open.mf.dispose();
    }
  });
});

describe("bearer token syntax", () => {
  test("an RFC 6750 b64token with padding and punctuation is compared by its SHA-256", async () => {
    const token = "aB3-._~+/xy==";
    const open = await startApi({
      bindings: { ORGANIZER_TOKEN_SHA256: { type: "text", value: sha256Hex(token) } },
    });
    try {
      for (const [index, scheme] of ["Bearer", "bearer", "BeArEr"].entries()) {
        const created = await open.call("POST", "/tournaments", {
          body: tournamentBody({ slug: `b64token-${index}` }),
          headers: { Authorization: `${scheme} ${token}` },
        });
        assert.equal(created.status, 201, `${scheme}: the scheme is case-insensitive`);
        await created.body?.cancel();
      }
      await assertUnauthorized(
        await open.call("GET", "/tournaments", { token: "aB3-._~+/xy=" }),
        "a token without one padding character is a different token",
      );
      await assertUnauthorized(
        await open.call("GET", "/tournaments", { token: `${token}=` }),
        "a token with extra padding is a different token",
      );
      const organizer = await open.call("GET", "/tournaments", { token });
      assert.equal(organizer.status, 200, "the exact token opens the organizer view");
      await organizer.body?.cancel();
    } finally {
      await open.mf.dispose();
    }
  });
});
