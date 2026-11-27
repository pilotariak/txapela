// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Errors of the API as RFC 9457 problems (src/api/problem.rs): the router's plain-text 404 and
// 405 become application/problem+json under /api/, with a stable type.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { assertProblem, startWorker } from "./worker.mjs";

let mf;

before(() => {
  mf = startWorker();
});

after(async () => {
  await mf.dispose();
});

for (const path of ["/api/v1/nope", "/api/v1/health/nope", "/api/v2/health", "/api/nope"]) {
  test(`GET ${path} answers a 404 problem`, async () => {
    const res = await mf.dispatchFetch(`http://localhost${path}`);
    const body = await assertProblem(res, 404, "not-found");
    assert.deepEqual(body, {
      type: "https://txapela.pilotariak.com/problems/not-found",
      title: "Not found",
      status: 404,
    });
    assert.equal(res.headers.get("cache-control"), "private, no-store");
  });
}

for (const method of ["POST", "PUT", "PATCH", "DELETE"]) {
  test(`${method} /api/v1/health answers a 405 problem`, async () => {
    const res = await mf.dispatchFetch("http://localhost/api/v1/health", { method });
    const body = await assertProblem(res, 405, "method-not-allowed");
    assert.equal(body.title, "Method not allowed");
  });
}

test("outside /api/, an unknown path keeps the router's plain 404", async () => {
  const res = await mf.dispatchFetch("http://localhost/nope");
  assert.equal(res.status, 404);
  assert.doesNotMatch(res.headers.get("content-type") ?? "", /problem\+json/, "not a problem");
  await res.body?.cancel();
});
