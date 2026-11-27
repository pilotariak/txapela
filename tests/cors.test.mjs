// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// CORS of the API for the Kancha website (src/api/cors.rs). The Worker runs with the [vars] of
// wrangler.toml, the production list, so the development origin is an unlisted one here.

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import { ORGANIZER_TOKEN, allowedOrigins, applyMigrations, assertProblem, startWorker } from "./worker.mjs";

const ORIGIN = "https://kancha.pilotariak.com";
const DEV_ORIGIN = "http://localhost:8081";
const UNLISTED = [
  "https://evil.example",
  "null",
  DEV_ORIGIN,
  `${ORIGIN}.evil.example`,
  "http://kancha.pilotariak.com",
];
const CORS_HEADERS = [
  "access-control-allow-origin",
  "access-control-allow-credentials",
  "access-control-allow-methods",
  "access-control-allow-headers",
  "access-control-expose-headers",
  "access-control-max-age",
];

let mf;

before(async () => {
  mf = startWorker();
  await applyMigrations(await mf.getD1Database("DB"));
});

after(async () => {
  await mf.dispose();
});

function call(method, path, headers = {}) {
  return mf.dispatchFetch(`http://localhost/api/v1${path}`, { method, headers });
}

function assertVary(res) {
  assert.match(res.headers.get("vary") ?? "", /\bOrigin\b/, "the response varies on Origin");
}

function assertCors(res, origin) {
  assert.equal(res.headers.get("access-control-allow-origin"), origin, "the origin is echoed, never *");
  assert.equal(res.headers.get("access-control-allow-credentials"), "true", "credentials are allowed");
  assertVary(res);
}

function assertNoCors(res, why) {
  for (const name of CORS_HEADERS) {
    assert.equal(res.headers.get(name), null, `${why}: no ${name}`);
  }
}

// A preflight as a browser sends it.
function preflight(path, origin, method = "POST", requestHeaders = "content-type") {
  return call("OPTIONS", path, {
    Origin: origin,
    "Access-Control-Request-Method": method,
    "Access-Control-Request-Headers": requestHeaders,
  });
}

describe("allowed origins", () => {
  test("production allows only https://kancha.pilotariak.com", () => {
    assert.deepEqual(allowedOrigins, [ORIGIN], "ALLOWED_ORIGINS of wrangler.toml");
  });
});

describe("responses", () => {
  test("an allowed origin is echoed, with credentials", async () => {
    const res = await call("GET", "/health", { Origin: ORIGIN });
    assert.equal(res.status, 200);
    assertCors(res, ORIGIN);
    assert.equal(res.headers.get("access-control-allow-methods"), null, "preflight headers only on a preflight");
    assert.equal(res.headers.get("access-control-expose-headers"), null, "no header to expose yet");
    await res.body?.cancel();
  });

  test("an unlisted origin gets no CORS header", async () => {
    for (const origin of UNLISTED) {
      const res = await call("GET", "/health", { Origin: origin });
      assert.equal(res.status, 200);
      assertNoCors(res, origin);
      assertVary(res);
      await res.body?.cancel();
    }
  });

  test("a request without Origin, like the native app's, gets no CORS header", async () => {
    const res = await call("GET", "/health");
    assert.equal(res.status, 200);
    assertNoCors(res, "no Origin");
    assertVary(res);
    await res.body?.cancel();
  });

  test("outside /api/, the landing page sends no CORS header", async () => {
    const res = await mf.dispatchFetch("http://localhost/", { headers: { Origin: ORIGIN } });
    assert.equal(res.status, 200);
    assertNoCors(res, "landing page");
    await res.body?.cancel();
  });
});

describe("problems", () => {
  test("a 401 of the tournaments API carries the CORS headers", async () => {
    const res = await call("POST", "/tournaments", { Origin: ORIGIN, Authorization: "Bearer wrong-token" });
    assertCors(res, ORIGIN);
    await assertProblem(res, 401, "unauthorized");
    const ok = await call("GET", "/tournaments", { Origin: ORIGIN, Authorization: `Bearer ${ORGANIZER_TOKEN}` });
    assertCors(ok, ORIGIN);
    assert.equal(ok.status, 200, "an organizer read from the website");
    await ok.body?.cancel();
  });

  test("the router's 404 and 405 carry the CORS headers, so the page can read the problem", async () => {
    const notFound = await call("GET", "/nope", { Origin: ORIGIN });
    assertCors(notFound, ORIGIN);
    await assertProblem(notFound, 404, "not-found");
    const notAllowed = await call("POST", "/health", { Origin: ORIGIN });
    assertCors(notAllowed, ORIGIN);
    await assertProblem(notAllowed, 405, "method-not-allowed");
  });
});

describe("preflight", () => {
  test("a preflight from an allowed origin answers 204 before the router's 405", async () => {
    for (const method of ["GET", "POST", "PUT", "PATCH", "DELETE"]) {
      const res = await preflight("/health", ORIGIN, method);
      assert.equal(res.status, 204, `${method} /health`);
      assert.equal(await res.text(), "", "a preflight has no body");
      assertCors(res, ORIGIN);
      assert.equal(res.headers.get("access-control-allow-methods"), "GET, POST, PUT, PATCH, DELETE");
      assert.equal(res.headers.get("access-control-allow-headers"), "Content-Type, Authorization");
      assert.equal(res.headers.get("access-control-max-age"), "7200");
      assert.equal(res.headers.get("cache-control"), "private, no-store");
      assert.equal(res.headers.get("x-content-type-options"), "nosniff");
    }
  });

  test("the tournaments routes allow Authorization, which carries the organizer token", async () => {
    for (const [path, method] of [
      ["/tournaments", "GET"],
      ["/tournaments", "POST"],
      ["/tournaments/open-2027", "PATCH"],
      ["/tournaments/open-2027", "DELETE"],
    ]) {
      const res = await preflight(path, ORIGIN, method, "authorization, content-type");
      assert.equal(res.status, 204, `${method} ${path}`);
      assertCors(res, ORIGIN);
      assert.match(res.headers.get("access-control-allow-headers") ?? "", /\bAuthorization\b/, "Authorization");
      assert.match(res.headers.get("access-control-allow-methods") ?? "", new RegExp(`\\b${method}\\b`), method);
    }
  });

  test("a preflight from an unlisted origin, or without one, gets the router's 405 without CORS headers", async () => {
    for (const origin of UNLISTED) {
      const res = await preflight("/health", origin);
      assertNoCors(res, origin);
      assertVary(res);
      await assertProblem(res, 405, "method-not-allowed");
    }
    const bare = await call("OPTIONS", "/health");
    assertNoCors(bare, "no Origin");
    await assertProblem(bare, 405, "method-not-allowed");
  });
});
