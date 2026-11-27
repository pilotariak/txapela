// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { after, before, test } from "node:test";
import { startWorker } from "./worker.mjs";

// The version the page shows comes from Cargo.toml (env!("CARGO_PKG_VERSION")).
const version = readFileSync("Cargo.toml", "utf8").match(/^version = "([^"]+)"/m)[1];

// Azkena's policy plus connect-src 'self' for the status script.
const csp =
  "default-src 'none'; script-src 'unsafe-inline'; " +
  "style-src 'unsafe-inline' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; " +
  "img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'";

let mf;

before(() => {
  mf = startWorker();
});

after(async () => {
  await mf.dispose();
});

test("GET / answers the landing page with its headers", async () => {
  const res = await mf.dispatchFetch("http://localhost/");
  assert.equal(res.status, 200);
  assert.equal(res.headers.get("content-type"), "text/html; charset=utf-8");
  assert.equal(res.headers.get("content-security-policy"), csp);
  assert.equal(res.headers.get("cache-control"), "public, max-age=300");

  const html = await res.text();
  assert.match(html, /<!doctype html>/i);
  assert.ok(html.includes("<title>txapela"), "the page is txapela's");
  assert.ok(html.includes(`v${version}`), `the page shows v${version}`);
  assert.ok(!html.includes("__TXAPELA_VERSION__"), "no version placeholder is left");
  assert.ok(html.includes("fetch('/api/v1/health'"), "the status script fetches the health route");
});

for (const method of ["POST", "PUT", "PATCH", "DELETE"]) {
  test(`${method} / answers 405`, async () => {
    const res = await mf.dispatchFetch("http://localhost/", { method });
    assert.equal(res.status, 405);
    await res.body?.cancel();
  });
}
