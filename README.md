# txapela

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://github.com/pilotariak/txapela/blob/main/LICENSE)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/pilotariak/txapela/badge)](https://scorecard.dev/viewer/?uri=github.com/pilotariak/txapela)

Pelota tournaments run by a club: a Cloudflare Worker in Rust and a D1 database for the registrations, series, pools,
matches on the courts, final phase and public results.

## Features

- Cloudflare Worker written in Rust with [workers-rs](https://github.com/cloudflare/workers-rs), compiled to
  `wasm32-unknown-unknown` by `worker-build`
- D1 database `pilotariak-txapela`, with migrations in `database/migrations/`
  ([ADR-0001](docs/adr/0001-keep-tournament-data-out-of-frontis.md))
- REST API under `/api/v1/`, with RFC 9457 `application/problem+json` errors and CORS for the Kancha website
- Landing page at `/`, health check at `/api/v1/health`
- Rust unit tests with `cargo test`, Worker tests in Miniflare, `cargo deny` for the dependencies

## Layout

```text
src/lib.rs      Worker entry point
src/api/        REST API under /api/v1: routes, problems, CORS
src/db/         D1 queries, one file per table
src/domain/     Pure logic and types, tested on the host
src/web/        Public pages outside /api/
database/       D1 migrations
tests/          Worker tests in Miniflare
```

`api` uses `db` and `domain`, `db` uses `domain`, and `domain` uses neither. See [AGENTS.md](AGENTS.md) for the
rules.

## Quick start

Requires [rustup](https://rustup.rs), [Node.js](https://nodejs.org) ≥ 22.12 and [bun](https://bun.sh).

```sh
make install         # Rust toolchain, worker-build, cargo-deny, JavaScript dependencies
make migrate-local   # apply the D1 migrations to the local database
make dev             # wrangler dev on http://localhost:8793
curl http://localhost:8793/api/v1/health
```

Then check and test:

```sh
make check doc deny test lint
```

Run `make help` for the full list of targets.

## Deployment

The D1 database does not exist yet: `database_id` in `wrangler.toml` is a placeholder. Before the first deploy, the
maintainer runs `bunx wrangler d1 create pilotariak-txapela`, pastes the `database_id` it prints into `wrangler.toml`,
then runs `make deploy`, which applies the migrations and deploys the Worker to `https://txapela.pilotariak.com`.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
See [CONTRIBUTING](CONTRIBUTING.md).

## License

See [LICENSE](LICENSE).
