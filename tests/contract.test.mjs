// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// GET /api/v1/openapi.json serves the committed openapi.json unchanged (src/api/contract.rs):
// public, with the headers of every API response.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { after, before, test } from "node:test";
import { assertProblem, startWorker } from "./worker.mjs";

const PATH = "/api/v1/openapi.json";
const ORIGIN = "https://kancha.pilotariak.com";
// The file as committed: the Worker embeds the one of the build, which `make test` rebuilds.
const COMMITTED = readFileSync("openapi.json", "utf8");

let mf;

before(() => {
  mf = startWorker();
});

after(async () => {
  await mf.dispose();
});

function get(headers = {}) {
  return mf.dispatchFetch(`http://localhost${PATH}`, { headers });
}

test("GET /api/v1/openapi.json answers the committed openapi.json, without a token", async () => {
  const res = await get();
  assert.equal(res.status, 200, "public, like /health");
  assert.equal(res.headers.get("content-type"), "application/json", "JSON, as Response::from_json sends it");
  assert.equal(await res.text(), COMMITTED, "the body is the committed file, byte for byte");
});

test("the document lists its own route, public", async () => {
  const contract = await (await get()).json();
  assert.equal(contract.openapi, "3.1.0", "OpenAPI 3.1");
  assert.equal(contract.paths[PATH]?.get?.operationId, "getContract", "the contract lists the route");
  assert.deepEqual(contract.paths[PATH].get.security, [{}], "the route takes no token");
});

test("it carries the headers of every API response", async () => {
  const res = await get();
  assert.equal(res.headers.get("cache-control"), "private, no-store", "like every API response, as the contract says");
  assert.equal(res.headers.get("x-content-type-options"), "nosniff", "nosniff");
  assert.equal(res.headers.get("x-frame-options"), "DENY", "no framing");
  assert.equal(res.headers.get("referrer-policy"), "no-referrer", "no referrer");
  assert.equal(res.headers.get("cross-origin-resource-policy"), "same-origin", "CORP");
  assert.match(res.headers.get("strict-transport-security") ?? "", /^max-age=31536000/, "HSTS");
  assert.match(res.headers.get("content-security-policy") ?? "", /^default-src 'none'/, "CSP");
  await res.body?.cancel();
});

test("the Kancha website reads it through CORS", async () => {
  const res = await get({ Origin: ORIGIN });
  assert.equal(res.status, 200, "a simple GET, without a preflight");
  assert.equal(res.headers.get("access-control-allow-origin"), ORIGIN, "the origin is echoed, never *");
  assert.equal(res.headers.get("access-control-allow-credentials"), "true", "credentials are allowed");
  assert.match(res.headers.get("vary") ?? "", /\bOrigin\b/, "the response varies on Origin");
  assert.equal(await res.text(), COMMITTED, "the same document for a cross-origin call");
});

test("another method answers 405", async () => {
  const res = await mf.dispatchFetch(`http://localhost${PATH}`, { method: "POST", body: "{}" });
  await assertProblem(res, 405, "method-not-allowed");
});
