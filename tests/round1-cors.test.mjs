// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Round 1: CORS preflights of the Worker run in Miniflare with the [vars] of wrangler.toml.
// The allowed origin is read from the same ALLOWED_ORIGINS value the Worker is given.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { allowedOrigins, assertProblem, startWorker } from "./worker.mjs";

let mf;

before(() => {
  mf = startWorker();
});

after(async () => {
  await mf.dispose();
});

function preflight(origin) {
  return mf.dispatchFetch("http://localhost/api/v1/health", {
    method: "OPTIONS",
    headers: {
      Origin: origin,
      "Access-Control-Request-Method": "GET",
      "Access-Control-Request-Headers": "content-type",
    },
  });
}

function assertNoCors(res, why) {
  for (const name of [
    "access-control-allow-origin",
    "access-control-allow-credentials",
    "access-control-allow-methods",
    "access-control-allow-headers",
    "access-control-max-age",
  ]) {
    assert.equal(res.headers.get(name), null, `${why}: no ${name}`);
  }
}

test("a preflight from an allowed origin answers 204 with the CORS headers", async () => {
  assert.ok(allowedOrigins.length > 0, "wrangler.toml lists at least one allowed origin");
  const allowed = allowedOrigins[0];

  const res = await preflight(allowed);

  assert.equal(res.status, 204, "the preflight is answered before the router's 405");
  assert.equal(await res.text(), "", "a 204 preflight has no body");
  assert.equal(res.headers.get("access-control-allow-origin"), allowed, "the origin is echoed, not *");
  assert.equal(res.headers.get("access-control-allow-credentials"), "true", "credentials are allowed");
  assert.match(res.headers.get("access-control-allow-methods") ?? "", /\bGET\b/, "GET is allowed");
  assert.match(res.headers.get("access-control-allow-headers") ?? "", /content-type/i, "Content-Type is allowed");
  assert.equal(res.headers.get("access-control-max-age"), "7200", "the preflight may be cached");
  assert.match(res.headers.get("vary") ?? "", /\bOrigin\b/, "the response varies on Origin");
});

test("a preflight from a rejected origin answers 405 without CORS headers", async () => {
  assert.ok(allowedOrigins.length > 0, "wrangler.toml lists at least one allowed origin");
  const rejected = `${allowedOrigins[0]}.evil.example`;
  assert.ok(!allowedOrigins.includes(rejected), "the origin is not in ALLOWED_ORIGINS");

  const res = await preflight(rejected);

  assert.equal(res.status, 405, "an unlisted origin does not get a preflight");
  assertNoCors(res, rejected);
  assert.match(res.headers.get("vary") ?? "", /\bOrigin\b/, "the response still varies on Origin");
  await assertProblem(res, 405, "method-not-allowed");
});

test("a preflight without an Origin answers 405 without CORS headers", async () => {
  const res = await mf.dispatchFetch("http://localhost/api/v1/health", {
    method: "OPTIONS",
    headers: { "Access-Control-Request-Method": "GET" },
  });

  assert.equal(res.status, 405, "no Origin means no preflight");
  assertNoCors(res, "no Origin");
  await assertProblem(res, 405, "method-not-allowed");
});
