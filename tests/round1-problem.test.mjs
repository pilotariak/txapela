// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Round 1: an unknown /api/v1 path answers an RFC 9457 problem, not the router's plain text.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { PROBLEM, startWorker } from "./worker.mjs";

let mf;

before(() => {
  mf = startWorker();
});

after(async () => {
  await mf.dispose();
});

for (const path of ["/api/v1/no-such-resource", "/api/v1/health/extra"]) {
  test(`GET ${path} answers a 404 problem+json with the not-found type`, async () => {
    const res = await mf.dispatchFetch(`http://localhost${path}`);

    assert.equal(res.status, 404, "an unknown path is not found");
    assert.match(
      res.headers.get("content-type") ?? "",
      /^application\/problem\+json(?:\s*;.*)?$/i,
      "the error is a problem+json document",
    );

    const body = await res.json();
    assert.deepEqual(
      body,
      {
        type: `${PROBLEM}not-found`,
        title: "Not found",
        status: 404,
      },
      "the problem carries the stable type, title and status",
    );
  });
}
