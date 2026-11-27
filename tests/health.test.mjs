// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { startWorker } from "./worker.mjs";

let mf;

before(() => {
  mf = startWorker();
});

after(async () => {
  await mf.dispose();
});

test("GET /api/v1/health answers 200 with status ok", async () => {
  const res = await mf.dispatchFetch("http://localhost/api/v1/health");
  assert.equal(res.status, 200);
  assert.match(res.headers.get("content-type"), /^application\/json/);
  assert.equal(res.headers.get("cache-control"), "private, no-store", "API responses are not cached");
  assert.deepEqual(await res.json(), { status: "ok" });
});

test("every response carries the security headers", async () => {
  for (const path of ["/api/v1/health", "/api/v1/nope", "/nope"]) {
    const res = await mf.dispatchFetch(`http://localhost${path}`);
    assert.equal(res.headers.get("x-content-type-options"), "nosniff", `${path}: nosniff`);
    assert.equal(res.headers.get("x-frame-options"), "DENY", `${path}: no framing`);
    assert.equal(res.headers.get("referrer-policy"), "no-referrer", `${path}: no referrer`);
    assert.equal(res.headers.get("cross-origin-resource-policy"), "same-origin", `${path}: CORP`);
    assert.match(res.headers.get("strict-transport-security") ?? "", /^max-age=31536000/, `${path}: HSTS`);
    assert.match(res.headers.get("content-security-policy") ?? "", /^default-src 'none'/, `${path}: CSP`);
    await res.body?.cancel();
  }
});

for (const path of ["/health", "/api/health"]) {
  test(`GET ${path} (outside /api/v1/) answers 404`, async () => {
    const res = await mf.dispatchFetch(`http://localhost${path}`);
    assert.equal(res.status, 404);
    await res.body?.cancel();
  });
}
