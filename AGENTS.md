# AGENTS.md — txapela

**txapela** manages the Basque pelota tournaments that a club runs: registrations, series, pools, a calendar on the
courts, a final phase and public results. Organizers write the data, and the results are public.

txapela is a Cloudflare Worker written in Rust with [workers-rs](https://github.com/cloudflare/workers-rs), and a D1
database, `pilotariak-txapela`, served at `https://txapela.pilotariak.com`. Clients call txapela directly: tournament
data does not go through Frontis.

## Decisions

- [ADR-0001](docs/adr/0001-keep-tournament-data-out-of-frontis.md): why tournament data lives in its own D1 database,
  outside Frontis.
- [pilotariak/pik](https://github.com/pilotariak/pik) is the reference: txapela follows its layout, tooling and rules.

## Architecture

```text
txapela/
├── src/
│   ├── lib.rs              # Worker entry: #[event(fetch)] answers preflights, runs the router, then adds what every
│   │                       # response needs
│   ├── api/
│   │   ├── mod.rs          # routes(): the /api/v1 prefix (v1! macro), finish(): problem 404/405, Cache-Control, CORS
│   │   ├── health.rs       # GET /api/v1/health
│   │   ├── cors.rs         # ALLOWED_ORIGINS, the preflight, the CORS headers of /api/ responses
│   │   └── problem.rs      # RFC 9457 errors, one stable type per error
│   ├── db/
│   │   └── mod.rs          # D1 queries, one file per table, returning domain types
│   ├── domain/
│   │   └── mod.rs          # Pure logic and types, no worker import: unit tests run by cargo test on the host
│   └── web/
│       ├── mod.rs          # routes(): the public pages outside /api/
│       ├── landing.rs      # GET /: version, CSP and Cache-Control
│       └── landing.html    # The landing page, embedded with include_str!
├── database/
│   └── migrations/         # D1 migrations, applied in order by wrangler
├── docs/adr/               # Architecture decision records
├── tests/
│   ├── worker.mjs          # Starts build/ in Miniflare with the bindings of wrangler.toml; applies the migrations
│   ├── health.test.mjs     # Worker tests, run with node:test: health route and security headers
│   ├── problem.test.mjs    # 404 and 405 problems under /api/
│   ├── cors.test.mjs       # CORS of the API
│   ├── landing.test.mjs    # GET /
│   └── database.test.mjs   # D1 binding and migrations
├── wrangler.toml           # Worker config: build command, ALLOWED_ORIGINS, Custom Domain, D1 binding, logs
├── Cargo.toml              # Crate (version managed by release-please)
├── deny.toml               # cargo-deny: advisories, licenses, bans and sources of the crates
├── rust-toolchain.toml     # Pinned Rust toolchain and the wasm32-unknown-unknown target
├── package.json            # Pinned wrangler and miniflare, bun version, dev, deploy, migration and query scripts
└── bun.lock                # bun lock file of the JavaScript dependencies
```

`worker-build` compiles the crate to `wasm32-unknown-unknown` and writes `build/index.js` and `build/index_bg.wasm`.
`wrangler dev` and `wrangler deploy` run it through the `build` command of `wrangler.toml`.

## Module layout

One crate split into modules, in the `mod.rs` style. Modules are private or `pub(crate)`: the crate is a `cdylib`, so
nothing needs `pub`. New code goes here:

- **`api/`**: one file per resource. A file holds the resource's handlers and its request and response types.
  `api/mod.rs` registers them with `v1!("/path")`, the only place the version appears; a breaking change adds
  `api/v2/` for the handlers that change only. Code shared by the routes gets its own file: `cors.rs` (allowed
  origins, preflight, CORS headers), `problem.rs` (RFC 9457 errors).
- **`db/`**: D1 queries, one file per table, returning `domain` types.
- **`domain/`**: pure logic and types. It never imports `worker`, so `cargo test` runs it on the host. It reads no
  clock: the routes read the Worker's clock (`worker::Date::now()`) and pass the time in.
- **`web/`**: the public pages outside `/api/`.

Dependencies point one way: `api` uses `db` and `domain`, `db` uses `domain`, `domain` uses neither. workers-rs has
no middleware: what every response needs (security headers, `Cache-Control: private, no-store` on `/api/*`,
problem+json instead of the Router's plain-text 404 and 405 on `/api/*`, the CORS headers) goes after `router.run()`
in `fetch`, and the CORS preflight before it.

Rust tests live in `#[cfg(test)]` modules inside `src/`: a `cdylib` crate has no library for `tests/*.rs` to link
against. `tests/` holds the Worker tests, which run the built Worker in Miniflare.

## Routes

Every API route lives under `/api/v1/`. Installed app versions stay in use for months, so a breaking change goes to
`/api/v2/` while `/api/v1/` keeps working. Only the landing page lives outside `/api/`. The workers-rs `Router` answers
405 to any other method on a registered path, except a CORS preflight, answered before it.

| Method | Path             | Description                                              |
| ------ | ---------------- | -------------------------------------------------------- |
| `GET`  | `/`              | Landing page (HTML): what txapela is, live status        |
| `GET`  | `/api/v1/health` | Liveness check: `{"status":"ok"}`                        |

The tournament resources come with the next issues ([#6](https://github.com/pilotariak/txapela/issues/6) to
[#11](https://github.com/pilotariak/txapela/issues/11)), and how organizers authenticate is still open
([#5](https://github.com/pilotariak/txapela/issues/5)).

- **Errors** are `application/problem+json` (RFC 9457) with a stable `type`,
  `https://txapela.pilotariak.com/problems/<name>`: the list is the `ProblemType` enum of `problem.rs`. Never rename
  one: clients map them to messages. Any unknown path under `/api/` answers the `not-found` problem, and another
  method on a known path the `method-not-allowed` problem.
- **CORS** (`cors.rs`): one list, the `ALLOWED_ORIGINS` Worker variable (origins separated by commas), drives CORS.
  Production allows `https://kancha.pilotariak.com` only (`[vars]` of `wrangler.toml`, checked by
  `tests/cors.test.mjs`); `make dev` replaces it with `--var` for the Expo dev server, `http://localhost:8081`. Never
  put it in `.dev.vars` or a secret. Every `/api/` response sends `Vary: Origin`, and to an allowed origin
  `Access-Control-Allow-Origin` (the origin echoed, never `*`) and `Access-Control-Allow-Credentials: true`, problems
  included. `OPTIONS` from an allowed origin answers 204 with the methods, the request headers and
  `Access-Control-Max-Age: 7200` (the cap of Chromium); any other `OPTIONS` gets the router's 405. A request header
  that Kancha starts sending goes in `ALLOW_HEADERS`; a response header it starts reading needs
  `Access-Control-Expose-Headers`.

### Landing page

`src/web/landing.html` is one HTML document with inline CSS and script, no build step, in the style of the other
Pilotariak landing pages (design tokens from frontis `DESIGN.md`).

- `__TXAPELA_VERSION__` is replaced with `CARGO_PKG_VERSION` when the page is served.
- `GET /` sends `Content-Security-Policy` (Azkena's policy plus `connect-src 'self'` for the status script) and
  `Cache-Control: public, max-age=300`. A new origin on the page needs a CSP change.
- Never put player contact details, or real player names, on the page.

## Rules the next issues must keep

- **Personal data.** Organizers write the contact details of players. Never log them, never return them on a public
  route, and never paste remote rows into an issue, a pull request or a log.
- **No clock in `domain`.** Routes read the Worker's clock and pass the time in, so that every rule is tested on the
  host with fixed instants.
- **API routes under `/api/v1/`**, as above. Only the public pages live outside it.
- **No tournament data in Frontis** ([ADR-0001](docs/adr/0001-keep-tournament-data-out-of-frontis.md)).

## Dev Setup

Requirements:

- [rustup](https://rustup.rs). `rust-toolchain.toml` pins the toolchain, rustfmt, clippy and the wasm target.
- [Node.js](https://nodejs.org) ≥ 22.12 (required by Wrangler). Wrangler and the Worker tests run on Node.js.
- [bun](https://bun.sh), the version of the `packageManager` field of `package.json`. bun installs the JavaScript
  dependencies and runs the `package.json` scripts; `bun.lock` is the lock file (no `package-lock.json`).
- [worker-build](https://crates.io/crates/worker-build) 0.9.0, the same version as the `worker` crate.
- [cargo-deny](https://crates.io/crates/cargo-deny), the version of `CARGO_DENY_VERSION` in the Makefile, for
  `make deny`.
- [prek](https://github.com/j178/prek) ≥ the `minimum_prek_version` of `.pre-commit-config.yaml`, with hawkeye and
  taplo, for `make lint`.

```bash
make install         # rustup toolchain install, cargo install worker-build and cargo-deny, bun install
```

## Running Locally

```bash
make migrate-local   # bun run migrate:local: apply the migrations to the local D1
make dev             # bun run dev: wrangler dev, ALLOWED_ORIGINS=http://localhost:8081  →  http://localhost:8793
```

```bash
curl http://localhost:8793/api/v1/health
```

`wrangler.toml` pins the HTTP port to 8793 and the inspector port to 9244, so txapela runs beside the other Pilotariak
services. When a port is taken, `make dev` stops instead of moving to another one.

## Database

- Binding `DB`, database `pilotariak-txapela` (named like the other Pilotariak databases, `pilotariak-<name>`),
  migrations in `database/migrations/`. Commands call the database by its name.
- **The remote database does not exist yet.** `database_id` in `wrangler.toml` is a placeholder,
  `00000000-0000-0000-0000-000000000000`, which local development accepts. Before the first deploy, the maintainer
  runs `bunx wrangler d1 create pilotariak-txapela`, then pastes the `database_id` it prints into `wrangler.toml`.
  `make migrate-remote`, `make db-remote` and `make deploy` need the real id.
- `wrangler dev`, the local migrations and the local queries share one local state, `.wrangler/state/`. The local
  database has no tables until a migration creates them.
- `make migrate-local`, `make migrate-remote`, `make migrate-list`, `make db-local SQL="..."` and
  `make db-remote SQL="..."` wrap the `package.json` scripts of the same names (`migrate:local`, …).
- Create a migration with `bunx wrangler d1 migrations create pilotariak-txapela "<description>"`, then add the
  `-- SPDX` header from `hack/header.txt` at the top of the file.
- Never edit a migration that has been applied remotely: add a new one.

## Testing

```bash
make check           # cargo fmt --check, cargo clippy on wasm32-unknown-unknown
make doc             # cargo doc --no-deps --document-private-items on wasm32, rustdoc warnings denied
make test            # worker-build, cargo test, then the Worker tests (bun run test)
make deny            # cargo deny check: RustSec advisories, licenses, bans and sources (deny.toml)
make lint            # prek run -a: license headers, formatting, YAML/TOML/JSON, secrets, Rust hooks
```

- Pure Rust logic gets unit tests in its module, run by `cargo test` on the host.
- Behavior that needs the runtime (routes, D1) gets a test in `tests/`, which loads `build/` in Miniflare, as the
  workers-rs README recommends. Start the Worker with `startWorker()` from `tests/worker.mjs`, and declare a new
  binding there, so that every test file runs the same Worker. Check an error with `assertProblem()`.
- Time-dependent rules are tested by passing instants to `domain`, or by writing past instants in D1 through the
  binding, never by waiting.

CI runs all of this on every pull request:

- `ci.yml`, job `test`: rustfmt, clippy, `cargo doc`, `cargo test`, `worker-build`, `bun install --frozen-lockfile`, a
  check that `miniflare` matches the version wrangler depends on, the migrations on a local D1, then the Worker tests.
  The Rust cache (`Swatinem/rust-cache`) also holds `~/.cargo/bin`, so `cargo install worker-build` does nothing on a
  cache hit. Only `main` saves the cache.
- `ci.yml`, job `deny`: `cargo deny check`, with the cargo-deny release binary.
- `prek.yml`: the prek hooks.

## Deployment

```bash
make deploy          # make migrate-remote, then bun run deploy (wrangler deploy)
```

Only the maintainer deploys, once the D1 database exists (see Database). `wrangler deploy` attaches the Custom Domain
`txapela.pilotariak.com` declared in `routes`, then `https://txapela.pilotariak.com/api/v1/health` answers. On the
pilotariak.com zone, it creates the DNS record and the certificate. A Custom Domain takes a bare hostname, without a
path or a wildcard.

## Key Conventions

- Keep versions in step, by hand: the `worker` crate and `worker-build` (`Cargo.toml`, `wrangler.toml`, Makefile, CI);
  `wrangler` and `miniflare` at the versions wrangler bundles, both exact; `compatibility_date` no later than the
  workerd that wrangler bundles; `cargo-deny` (Makefile, CI). txapela pins the same versions as pik.
- GitHub Actions are pinned by commit SHA with the version in a comment.
- JavaScript dependencies go through bun: `bun add -d -E <package>`, `bun.lock` committed, never `npm install`.
- Do not bump `version` in `Cargo.toml` by hand: release-please does it from conventional commits.
- `Cargo.toml` forbids `unsafe` code and turns on `clippy::pedantic`, and `make check` fails on any warning. Silence a
  lint with `#[expect(lint, reason = "...")]`, never `#[allow]`. In tests, import items by name (no `use super::*`) and
  give every assertion a message.
- Commits follow [Conventional Commits](https://www.conventionalcommits.org) and carry a DCO `Signed-off-by`
  (`git commit -s`, see CONTRIBUTING.md).
- License headers are required on every source and config file (checked by `licenserc.toml`): `//` for Rust and
  JavaScript, `#` for TOML, YAML and the Makefile, `--` for SQL, `<!-- -->` after the doctype for HTML.
