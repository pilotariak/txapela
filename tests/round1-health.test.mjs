// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Round 1: the liveness route of the Worker built in build/, run in Miniflare.

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

test("GET /api/v1/health answers 200 with a JSON status of ok", async () => {
  const res = await mf.dispatchFetch("http://localhost/api/v1/health");

  assert.equal(res.status, 200, "the liveness route is available");
  assert.match(
    res.headers.get("content-type") ?? "",
    /^application\/json(?:\s*;.*)?$/i,
    "the health route answers JSON",
  );
  assert.deepEqual(await res.json(), { status: "ok" }, "the exact health document");
});
