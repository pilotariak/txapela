// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Runs the Worker built by `worker-build --release` (build/) in Miniflare, as the workers-rs
// README recommends for testing a Rust Worker. Run `make build` first. Shared by the
// *.test.mjs files, so that each binding is declared once.

import assert from "node:assert/strict";
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

export function startWorker() {
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
          env: {
            DB: { type: "d1", id: "DB" },
            ...vars,
          },
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
