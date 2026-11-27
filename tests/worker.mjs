// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Runs the Worker built by `worker-build --release` (build/) in Miniflare, as the workers-rs
// README recommends for testing a Rust Worker. Run `make build` first. Shared by the
// *.test.mjs files, so that each binding is declared once.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { Miniflare } from "miniflare";
import { experimental_readRawConfig as readRawConfig, unstable_splitSqlQuery as splitSqlQuery } from "wrangler";

const { rawConfig } = readRawConfig({ config: "wrangler.toml" });

// Same compatibility date as the deployed Worker.
const compatibilityDate = rawConfig.compatibility_date;

// The [vars] of wrangler.toml, as text bindings: `wrangler dev --var` passes strings only, so a
// variable that is not a string would behave differently in development.
const vars = Object.fromEntries(
  Object.entries(rawConfig.vars ?? {}).map(([name, value]) => {
    if (typeof value !== "string") {
      throw new Error(`wrangler.toml: vars.${name} must be a string`);
    }
    return [name, { type: "text", value }];
  }),
);

// The origins of ALLOWED_ORIGINS in wrangler.toml, split as src/api/cors.rs splits them.
export const allowedOrigins = (rawConfig.vars?.ALLOWED_ORIGINS ?? "")
  .split(",")
  .map((origin) => origin.trim())
  .filter((origin) => origin !== "");

// The prefix of every problem type (src/api/problem.rs).
export const PROBLEM = "https://txapela.pilotariak.com/problems/";

// The organizer token of the tests: fictional, and never a real secret. The Worker gets its
// SHA-256 in the ORGANIZER_TOKEN_SHA256 secret, computed here so that no hash is committed.
export const ORGANIZER_TOKEN = "txapela-test-organizer-token";

export function sha256Hex(text) {
  return createHash("sha256").update(text).digest("hex");
}

// Starts the Worker. `bindings` adds or replaces bindings; a binding set to undefined is left
// out, such as the secret in the tests of a Worker without it.
export function startWorker({ bindings = {} } = {}) {
  const env = Object.fromEntries(
    Object.entries({
      DB: { type: "d1", id: "DB" },
      ORGANIZER_TOKEN_SHA256: { type: "text", value: sha256Hex(ORGANIZER_TOKEN) },
      ...vars,
      ...bindings,
    }).filter(([, binding]) => binding !== undefined),
  );
  return new Miniflare({
    workers: [
      {
        config: {
          name: "txapela",
          compatibilityDate,
          manifest: {
            mainModule: "build/index.js",
            modules: {
              "build/index.js": {
                type: "esm",
                contents: readFileSync("build/index.js", "utf8"),
              },
              "build/index_bg.wasm": {
                type: "wasm",
                contents: new Uint8Array(readFileSync("build/index_bg.wasm")),
              },
            },
          },
          env,
        },
      },
    ],
  });
}

// Checks an RFC 9457 problem response, and returns its document.
export async function assertProblem(res, status, type) {
  assert.equal(res.status, status, `status ${status}`);
  assert.match(res.headers.get("content-type") ?? "", /^application\/problem\+json/);
  const body = await res.json();
  assert.equal(body.type, `${PROBLEM}${type}`, `problem type ${type}`);
  assert.equal(body.status, status, "the problem repeats the status");
  assert.equal(typeof body.title, "string", "the problem has a title");
  return body;
}

const migrations = "database/migrations";

// Applies database/migrations/*.sql to a D1 binding the way `wrangler d1 migrations apply
// --local` does: wrangler's SQL splitter, then one batch per file, without the d1_migrations
// bookkeeping.
export async function applyMigrations(db) {
  const files = readdirSync(migrations)
    .filter((file) => file.endsWith(".sql"))
    .sort();
  for (const file of files) {
    const statements = splitSqlQuery(readFileSync(`${migrations}/${file}`, "utf8")).filter(
      // 0001 holds comments only.
      (statement) => statement.replace(/--.*$/gm, "").trim() !== "",
    );
    if (statements.length > 0) {
      await db.batch(statements.map((statement) => db.prepare(statement)));
    }
  }
}

// A Worker with a migrated D1, and a request helper for /api/v1. `token` goes in
// `Authorization: Bearer`; `body` is sent as JSON unless it is a string.
export async function startApi(options) {
  const mf = startWorker(options);
  const db = await mf.getD1Database("DB");
  await applyMigrations(db);

  function call(method, path, { token, body, headers = {} } = {}) {
    const sent = { ...headers };
    if (token !== undefined) sent.Authorization = `Bearer ${token}`;
    if (body !== undefined) sent["Content-Type"] = "application/json";
    return mf.dispatchFetch(`http://localhost/api/v1${path}`, {
      method,
      headers: sent,
      body: body === undefined || typeof body === "string" ? body : JSON.stringify(body),
    });
  }

  return { mf, db, call };
}

// A valid tournament at a fictional club and venue, with `changes` on top.
export function tournamentBody(changes = {}) {
  return {
    slug: "open-de-printemps-2027",
    name: "Open de printemps",
    organizing_club: "Club Exemple de Pelote",
    venue: {
      name: "Trinquet de l'Exemple",
      address: "1 place de l'Exemple, Ville-Exemple",
      latitude: 43.0,
      longitude: -1.0,
    },
    aire: "trinquet",
    courts: [{ name: "Trinquet A" }, { name: "Trinquet B" }],
    specialties: [{ specialty: "paleta_gomme_creuse", target_score: 40 }],
    dates: {
      registration_opens: "2027-03-01",
      registration_closes: "2027-03-31",
      first_match: "2027-04-10",
      finals: "2027-06-19",
    },
    fee_cents: 2000,
    max_teams: 16,
    ...changes,
  };
}

// Creates a tournament as the organizer, and returns its body.
export async function createTournament(call, changes = {}) {
  const res = await call("POST", "/tournaments", { token: ORGANIZER_TOKEN, body: tournamentBody(changes) });
  assert.equal(res.status, 201, `tournament created: ${res.status}`);
  return res.json();
}

// Moves a tournament one step at a time until it reaches `status`.
export async function moveTo(call, slug, status) {
  const steps = ["draft", "registration", "pools", "finals", "finished"];
  for (const next of steps.slice(1, steps.indexOf(status) + 1)) {
    const res = await call("PATCH", `/tournaments/${slug}`, { token: ORGANIZER_TOKEN, body: { status: next } });
    assert.equal(res.status, 200, `${slug} moved to ${next}`);
    await res.body?.cancel();
  }
}
