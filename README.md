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
- Tournaments: an organizer creates a tournament as a draft (aire, courts, specialties and their target scores,
  points, dates, venue), changes it, moves its status one step forward, and deletes a draft; anyone reads the
  tournaments that are not drafts
- Series and teams: an organizer splits a tournament into series (one specialty at one level, a maximum of teams, a
  block of 100 team numbers) and registers teams of two players in them, until the pools are drawn; anyone reads the
  team numbers, player names and clubs, while licence numbers and contacts stay with the organizer
- Pools, matches and standings: once the tournament is in pools, an organizer draws each series into pools A, B, C…
  of 4 to 6 teams, in snake order of their seeds and away from the teams of their club when the draw can, moves a team
  until the first result, and records and corrects scores and forfeits (the target score to 0); every pool plays a
  round robin, and anyone reads the pools, the matches and the standings, computed from the matches on every read
- OpenAPI 3.1 contract in [`openapi.json`](openapi.json), checked against the code by `cargo test` and against the base
  branch by `oasdiff breaking` in CI, and served unchanged at
  [`https://txapela.pilotariak.com/api/v1/openapi.json`](https://txapela.pilotariak.com/api/v1/openapi.json)
- Landing page at `/`, health check at `/api/v1/health`
- Rust unit tests with `cargo test`, Worker tests in Miniflare, `cargo deny` for the dependencies

## API

| Method   | Path                                                                         | Who       |
| -------- | ---------------------------------------------------------------------------- | --------- |
| `GET`    | `/api/v1/health`                                                             | public    |
| `GET`    | `/api/v1/openapi.json`                                                       | public    |
| `GET`    | `/api/v1/tournaments`                                                        | public    |
| `POST`   | `/api/v1/tournaments`                                                        | organizer |
| `GET`    | `/api/v1/tournaments/:slug`                                                  | public    |
| `PATCH`  | `/api/v1/tournaments/:slug`                                                  | organizer |
| `DELETE` | `/api/v1/tournaments/:slug`                                                  | organizer |
| `GET`    | `/api/v1/tournaments/:slug/series`                                           | public    |
| `POST`   | `/api/v1/tournaments/:slug/series`                                           | organizer |
| `GET`    | `/api/v1/tournaments/:slug/series/:series`                                   | public    |
| `PATCH`  | `/api/v1/tournaments/:slug/series/:series`                                   | organizer |
| `DELETE` | `/api/v1/tournaments/:slug/series/:series`                                   | organizer |
| `GET`    | `/api/v1/tournaments/:slug/teams`                                            | public    |
| `POST`   | `/api/v1/tournaments/:slug/teams`                                            | organizer |
| `GET`    | `/api/v1/tournaments/:slug/teams/:number`                                    | public    |
| `PATCH`  | `/api/v1/tournaments/:slug/teams/:number`                                    | organizer |
| `DELETE` | `/api/v1/tournaments/:slug/teams/:number`                                    | organizer |
| `POST`   | `/api/v1/tournaments/:slug/series/:series/draw`                              | organizer |
| `GET`    | `/api/v1/tournaments/:slug/series/:series/pools`                             | public    |
| `POST`   | `/api/v1/tournaments/:slug/series/:series/pools/:pool/teams`                 | organizer |
| `PUT`    | `/api/v1/tournaments/:slug/series/:series/pools/:pool/tiebreak`              | organizer |
| `GET`    | `/api/v1/tournaments/:slug/series/:series/matches`                           | public    |
| `GET`    | `/api/v1/tournaments/:slug/series/:series/pools/:pool/matches/:match`        | public    |
| `PUT`    | `/api/v1/tournaments/:slug/series/:series/pools/:pool/matches/:match/result` | organizer |
| `GET`    | `/api/v1/tournaments/:slug/series/:series/standings`                         | public    |

Organizer routes take `Authorization: Bearer <token>`. Until organizer authentication is settled
([#5](https://github.com/pilotariak/txapela/issues/5)), txapela compares the SHA-256 of the token with the
`ORGANIZER_TOKEN_SHA256` Worker secret, and answers 401 when the secret is missing. A draft tournament, with its series
and teams, is seen only with the token, and so are the licence numbers and contacts of players, which the pools,
matches and standings never show. See
[`openapi.json`](openapi.json) for the bodies and errors, and [AGENTS.md](AGENTS.md) for the rules. The Worker serves
that contract, the committed file unchanged, at `GET /api/v1/openapi.json`, without a token: point an OpenAPI tool
(Bruno, Postman, Scalar, openapi-typescript) at `https://txapela.pilotariak.com/api/v1/openapi.json`.

## Layout

```text
src/lib.rs      Worker entry point
src/api/        REST API under /api/v1: routes, organizer check, problems, CORS, OpenAPI
src/db/         D1 queries, one file per table
src/domain/     Pure logic and types, tested on the host
src/web/        Public pages outside /api/
database/       D1 migrations
tests/          Worker tests in Miniflare
openapi.json    OpenAPI 3.1 contract, generated by make openapi, served at /api/v1/openapi.json
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
curl http://localhost:8793/api/v1/openapi.json
```

The organizer routes need the `ORGANIZER_TOKEN_SHA256` secret. Locally, hash a throwaway token with
`printf %s "$TOKEN" | shasum -a 256` and put `ORGANIZER_TOKEN_SHA256="<hash>"` in `.dev.vars`, which git ignores. Never
commit a token or its hash.

Then check and test:

```sh
make check doc deny test lint
```

Run `make help` for the full list of targets.

## Deployment

The D1 database `pilotariak-txapela` lives in the EU jurisdiction, and its id is the `database_id` of `wrangler.toml`.
Before the first deploy, the maintainer sets the organizer secret with
`bunx wrangler secret put ORGANIZER_TOKEN_SHA256`, then runs `make deploy`, which applies the migrations and deploys
the Worker to `https://txapela.pilotariak.com`.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
See [CONTRIBUTING](CONTRIBUTING.md).

## License

See [LICENSE](LICENSE).
