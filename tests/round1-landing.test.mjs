// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Round 1: the public landing page of the Worker built in build/, run in Miniflare.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { after, before, test } from "node:test";
import { startWorker } from "./worker.mjs";

let mf;

before(() => {
  mf = startWorker();
});

after(async () => {
  await mf.dispose();
});

test("GET / serves an HTML landing page with the crate version and its headers", async () => {
  const version = readFileSync("Cargo.toml", "utf8").match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  assert.ok(version, "Cargo.toml declares a crate version");

  const res = await mf.dispatchFetch("http://localhost/");

  assert.equal(res.status, 200, "the landing page is public");
  assert.match(
    res.headers.get("content-type") ?? "",
    /^text\/html;\s*charset=utf-8$/i,
    "the landing page is HTML",
  );
  assert.equal(res.headers.get("cache-control"), "public, max-age=300", "the page may be cached");

  const csp = res.headers.get("content-security-policy") ?? "";
  assert.match(csp, /default-src 'none'/, "the default policy denies everything");
  assert.match(csp, /connect-src 'self'/, "the status script may call the same origin");

  const html = await res.text();
  assert.match(html, /<!doctype html>/i, "the page is an HTML document");
  assert.match(html, /<title>[^<]*txapela/i, "the page is txapela's");
  assert.ok(html.includes(`v${version}`), `the page shows v${version}`);
  assert.ok(!html.includes("__TXAPELA_VERSION__"), "the version placeholder was replaced");
  assert.ok(html.includes("/api/v1/health"), "the page references the health route");
});
