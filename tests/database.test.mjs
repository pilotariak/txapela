// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The DB binding of wrangler.toml and the migrations of database/migrations/, in Miniflare.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { applyMigrations, startWorker } from "./worker.mjs";

let mf;

before(() => {
  mf = startWorker();
});

after(async () => {
  await mf.dispose();
});

test("the migrations apply to the DB binding, which then answers queries", async () => {
  const db = await mf.getD1Database("DB");
  await applyMigrations(db);
  const row = await db.prepare("SELECT 1 AS one").first();
  assert.deepEqual(row, { one: 1 }, "the database answers after the migrations");
});
