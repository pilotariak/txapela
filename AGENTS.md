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
│   │   ├── mod.rs          # routes(): the /api/v1 prefix (v1! macro), finish(): problem 404/405, Cache-Control, CORS;
│   │   │                   # body reading (size, JSON, no null, no value quoted), the :slug, :series, :number,
│   │   │                   # :pool and :match parameters; ATTEMPTS of a write of the pools
│   │   ├── health.rs       # GET /api/v1/health
│   │   ├── tournaments.rs  # GET|POST /tournaments, GET|PATCH|DELETE /tournaments/:slug
│   │   ├── series.rs       # GET|POST /tournaments/:slug/series, GET|PATCH|DELETE /tournaments/:slug/series/:series
│   │   ├── teams.rs        # GET|POST /tournaments/:slug/teams, GET|PATCH|DELETE /tournaments/:slug/teams/:number;
│   │   │                   # the public and organizer views of a player
│   │   ├── pools.rs        # POST …/series/:series/draw, GET …/series/:series/pools, POST …/pools/:pool/teams (move);
│   │   │                   # the public view of a team in a pool, a match or a standings line
│   │   ├── matches.rs      # GET …/series/:series/matches, GET …/pools/:pool/matches/:match, PUT …/:match/result;
│   │   │                   # the extensible phase and state of a match
│   │   ├── standings.rs    # GET …/series/:series/standings, PUT …/pools/:pool/tiebreak
│   │   ├── auth.rs         # Interim organizer check: Bearer token vs the ORGANIZER_TOKEN_SHA256 secret
│   │   ├── cors.rs         # ALLOWED_ORIGINS, the preflight, the CORS headers of /api/ responses
│   │   ├── problem.rs      # RFC 9457 errors, one stable type per error; the closed and extensible type lists
│   │   └── openapi.rs      # cfg(test) only: builds openapi.json and checks it is up to date
│   ├── db/
│   │   ├── mod.rs          # Parameter binding (numbers, text, dates, booleans, JSON rows), batch, the Selection a
│   │   │                   # read loads, written and written_last
│   │   ├── tournaments.rs  # tournaments, and a whole tournament read or written with its courts and specialties
│   │   ├── courts.rs       # courts
│   │   ├── tournament_specialties.rs # tournament_specialties
│   │   ├── series.rs       # series, the teams of a series, what a tournament change must keep
│   │   ├── teams.rs        # teams, and a whole team read or written with its players; the registered teams; the
│   │   │                   # entrants of a draw
│   │   ├── players.rs      # players; their public fields only for the pools, matches and standings
│   │   ├── pools.rs        # pools, the guard of a write of the pools, the pools of a series read with their teams,
│   │   │                   # players and matches, a draw and a move written in one batch
│   │   ├── pool_teams.rs   # pool_teams: the teams of a pool, a move, the tiebreak
│   │   └── matches.rs      # matches: the matches of the pools, a result
│   ├── domain/
│   │   ├── mod.rs          # Pure logic and types, no worker import: unit tests run by cargo test on the host
│   │   ├── rule.rs         # A broken rule (422) and the checks shared by every resource: text, integers
│   │   ├── tournament.rs   # Aire, Specialty, Ranking, the rules of a tournament and of a change
│   │   ├── status.rs       # draft → registration → pools → finals → finished, one step forward; registration open;
│   │   │                   # pools
│   │   ├── series.rs       # A series: its specialty, maximum, position and number block, and the rules of a change
│   │   ├── team.rs         # A team: number, players, registration date, fee, status, and the maximums
│   │   ├── player.rs       # The two players of a team; licence and contact are organizer only
│   │   ├── draw.rs         # The draw: seed order, pools of 4 to 6, snake order, the same-club rule
│   │   ├── pool.rs         # A pool named A to Z, its teams and matches, the revision, a move, a tiebreak
│   │   ├── round_robin.rs  # The circle method, with a bye per round for an odd number of teams
│   │   ├── matches.rs      # A match: phase, pool, number, round, teams, state and score
│   │   ├── score.rs        # A result: the teams of the match, a valid score, a forfeit as the target score to 0
│   │   ├── standings.rs    # The standings of a pool, computed from its matches: points, goal average, tiebreak
│   │   ├── slug.rs         # The public key of a tournament, and of a series within it
│   │   ├── civil.rs        # YYYY-MM-DD civil dates, the date in Europe/Paris of an instant
│   │   └── token.rs        # SHA-256 of the organizer token, constant-time comparison
│   └── web/
│       ├── mod.rs          # routes(): the public pages outside /api/
│       ├── landing.rs      # GET /: version, CSP and Cache-Control
│       └── landing.html    # The landing page, embedded with include_str!
├── database/
│   └── migrations/         # D1 migrations, applied in order by wrangler
├── docs/adr/               # Architecture decision records
├── tests/
│   ├── worker.mjs          # Starts build/ in Miniflare with the bindings of wrangler.toml and the test organizer
│   │                       # token's hash, optionally capturing its logs; applies the migrations; request helper;
│   │                       # tournament, series, team and player fixtures; a drawn tournament, a result
│   ├── health.test.mjs     # Worker tests, run with node:test: health route and security headers
│   ├── problem.test.mjs    # 404 and 405 problems under /api/
│   ├── cors.test.mjs       # CORS of the API
│   ├── landing.test.mjs    # GET /
│   ├── tournaments.test.mjs # Each tournament route, its rules and its problems
│   ├── series.test.mjs     # Each series route, its rules and its problems
│   ├── teams.test.mjs      # Each team route, its rules, its problems, and the public view of a team
│   ├── pools.test.mjs      # The draw, the pools and a move: rules, order of checks, concurrent draws, moves, results
│   ├── matches.test.mjs    # The matches and their results: scores, forfeits, corrections, results against finals
│   ├── standings.test.mjs  # The standings, forfeits and corrections, the tiebreak, tiebreaks against moves
│   ├── privacy.test.mjs    # No licence or contact in a public response, a problem detail or a log line
│   ├── auth.test.mjs       # The organizer check, including a Worker without the secret
│   └── database.test.mjs   # D1 binding, migrations, tables, foreign keys and the cascade on delete
├── openapi.json            # OpenAPI 3.1 contract, generated by `make openapi`
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

- **`api/`**: one file per resource. A file holds the resource's handlers, its request and response types, and its
  `#[utoipa::path]` annotations. `api/mod.rs` registers them with `v1!("/path")`, the only place the version appears;
  a breaking change adds `api/v2/` for the handlers that change only. Code shared by the routes gets its own file:
  `auth.rs` (organizer check), `cors.rs` (allowed origins, preflight, CORS headers), `problem.rs` (RFC 9457 errors),
  `openapi.rs`.
- **`api/openapi.rs`** and every `#[utoipa::path]` and `ToSchema` exist under `cfg(test)` only
  (`#[cfg_attr(test, …)]`, utoipa is a dev-dependency): txapela does not serve the document, so the wasm build
  carries no schema code. Write a `#[schema(...)]` field attribute as `#[cfg_attr(test, schema(...))]` too, and give
  an `Option` field of a request `nullable = false`: the API answers 422 to a `null`.
- **`db/`**: D1 queries, one file per table, returning `domain` types. No INTEGER PRIMARY KEY leaves D1: statements
  find a tournament by its slug, a series by the slug of its tournament and its own, a team by the slug of its
  tournament and its number, a pool by its series and its number, and a match by its pool and its number. Child rows
  are inserted with `(SELECT id FROM tournaments WHERE slug = ?)` (or the same for a team) in the batch that inserts
  their parent.
- **`domain/`**: pure logic and types: the rules of a tournament, a series, a team and its players, the status
  transitions, slugs, civil dates, the organizer token's hash, the draw, the round robin, results and standings. It
  reads no randomness either: a draw is the same for the same teams. `rule.rs` holds the shape of a broken rule and the
  checks every resource shares. It never imports `worker`, so `cargo test` runs it on the host. It reads no clock: the
  routes read the Worker's clock (`worker::Date::now()`) and pass the time in, as the registration date of a team.
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

| Method   | Path                                                                         | Who       | Description                                                                              |
| -------- | ---------------------------------------------------------------------------- | --------- | ---------------------------------------------------------------------------------------- |
| `GET`    | `/`                                                                          | public    | Landing page (HTML): what txapela is, live status                                        |
| `GET`    | `/api/v1/health`                                                             | public    | Liveness check: `{"status":"ok"}`                                                        |
| `GET`    | `/api/v1/tournaments`                                                        | public    | Tournaments by first match date, then slug; drafts with the organizer token only         |
| `POST`   | `/api/v1/tournaments`                                                        | organizer | Create a tournament, as a draft: 201, `Location`, the tournament                         |
| `GET`    | `/api/v1/tournaments/:slug`                                                  | public    | A tournament; a draft answers 404 without the organizer token                            |
| `PATCH`  | `/api/v1/tournaments/:slug`                                                  | organizer | Change a tournament: partial body, checked on the merged state                           |
| `DELETE` | `/api/v1/tournaments/:slug`                                                  | organizer | Delete a draft with its courts, specialties, series, teams and players: 204              |
| `GET`    | `/api/v1/tournaments/:slug/series`                                           | public    | Series by position, then slug; 404 for a draft without the organizer token               |
| `POST`   | `/api/v1/tournaments/:slug/series`                                           | organizer | Create a series, in draft or registration: 201, `Location`, the series                   |
| `GET`    | `/api/v1/tournaments/:slug/series/:series`                                   | public    | A series, with the target score of its specialty                                         |
| `PATCH`  | `/api/v1/tournaments/:slug/series/:series`                                   | organizer | Change a series: partial body, checked on the merged state and its teams                 |
| `DELETE` | `/api/v1/tournaments/:slug/series/:series`                                   | organizer | Delete a series without teams, in draft or registration: 204                             |
| `GET`    | `/api/v1/tournaments/:slug/teams`                                            | public    | Teams by number; licences, contacts, dates and fees for the organizer only               |
| `POST`   | `/api/v1/tournaments/:slug/teams`                                            | organizer | Register a team, in draft or registration: 201, `Location`, the team                     |
| `GET`    | `/api/v1/tournaments/:slug/teams/:number`                                    | public    | A team, in the same two views                                                            |
| `PATCH`  | `/api/v1/tournaments/:slug/teams/:number`                                    | organizer | Change the players, fee or status of a team                                              |
| `DELETE` | `/api/v1/tournaments/:slug/teams/:number`                                    | organizer | Delete a team with its players, in draft or registration: 204                            |
| `POST`   | `/api/v1/tournaments/:slug/series/:series/draw`                              | organizer | Draw the pools of a series, in pools, until its first result: 201, `Location`, the pools |
| `GET`    | `/api/v1/tournaments/:slug/series/:series/pools`                             | public    | Pools by name, their teams in seed order; empty until the draw                           |
| `POST`   | `/api/v1/tournaments/:slug/series/:series/pools/:pool/teams`                 | organizer | Move a team into the pool, until the first result: the pools                             |
| `PUT`    | `/api/v1/tournaments/:slug/series/:series/pools/:pool/tiebreak`              | organizer | Order the teams of a pool, for teams level on points and goal average                    |
| `GET`    | `/api/v1/tournaments/:slug/series/:series/matches`                           | public    | Matches by pool, then number; empty until the draw                                       |
| `GET`    | `/api/v1/tournaments/:slug/series/:series/pools/:pool/matches/:match`        | public    | A match, its teams, state and score                                                      |
| `PUT`    | `/api/v1/tournaments/:slug/series/:series/pools/:pool/matches/:match/result` | organizer | Record or correct the result of a match, in pools: the match                             |
| `GET`    | `/api/v1/tournaments/:slug/series/:series/standings`                         | public    | The standings of each pool, computed from its matches                                    |

`openapi.json` describes every route, its bodies and errors. The next resources come with the next issues
([#9](https://github.com/pilotariak/txapela/issues/9) to [#11](https://github.com/pilotariak/txapela/issues/11)), and
how organizers and players authenticate is still open ([#5](https://github.com/pilotariak/txapela/issues/5)): until
then, the organizer enters every registration.

- **Organizer check** (`auth.rs`, interim until the ADR of #5): `auth::organizer` is the only place the write routes
  check the caller. It reads `Authorization: Bearer <token>` (RFC 6750 `b64token`, scheme in any case), hashes the
  token with SHA-256 and compares the lowercase hexadecimal, in constant time, with the `ORGANIZER_TOKEN_SHA256`
  Worker secret (surrounding blanks ignored). It fails closed: a missing secret, a secret that is not 64 lowercase
  hex digits, a missing or malformed header, or a wrong token answers 401 `unauthorized` with
  `WWW-Authenticate: Bearer`. No cookie, no
  account, no organizers table, no per-tournament ownership. Never log a token or its hash. The maintainer sets the
  secret with `wrangler secret put ORGANIZER_TOKEN_SHA256`; for `make dev`, see Running Locally.
- **Reads** (`auth::viewer`): without `Authorization`, the public view, without drafts; with the organizer token,
  drafts included; with any other `Authorization`, 401, so that an organizer with a wrong token is told so. The series,
  teams, pools, matches and standings of a draft answer 404 without the token, like the draft.
- **Order of checks**: 401, then 404 (unknown slug, series, team number, pool or match, or a draft without the token:
  the same problem), then the body (422), then the rules of `domain`: the status first (409 `not-a-draft`,
  `invalid-transition`, `registration-closed` or `not-in-pools`), then 422 on the merged state, then the other rows
  (409 `slug-taken`, `block-taken`, `number-taken`, `series-full`, `tournament-full`, `series-has-teams`,
  `series-has-results`).
- **Slug**: chosen at creation, immutable (a `PATCH` with `slug` answers 422): 3 to 64 characters among a-z, 0-9 and
  single hyphens, starting and ending with a letter or a digit. A duplicate answers 409 `slug-taken`. A path that
  cannot be a slug answers 404. A series has a slug of the same form, unique within its tournament: its public key in
  `/tournaments/:slug/series/:series`.
- **Team number**: the public key of a team in `/tournaments/:slug/teams/:number`, chosen at creation, unique within
  the tournament (409 `number-taken`), inside the number block of its series, immutable like its series (a `PATCH`
  with `number`, `series` or `registered_on` answers 422; to move a team, delete it and register it again). A path
  that is not decimal digits without a leading zero answers 404.
- **Values**: aire `fronton`, `mur_a_gauche` or `trinquet`; at least one specialty among `paleta_gomme_creuse` and
  `paleta_gomme_pleine`, none twice, each with an integer `target_score` above zero; at least one court, names
  trimmed, not empty and unique, in the given order; points (defaults 3, 1, 0) with win > loss >= forfeit >= 0;
  ranking `points_then_goal_average` (default); four civil dates `YYYY-MM-DD` in Europe/Paris, all required, with
  registration opens <= registration closes <= first match <= finals; venue name and address not empty, latitude in
  [-90, 90], longitude in [-180, 180]; organizing club not empty; `fee_cents` >= 0, shown only; `max_teams` >= 2.
  Integers stay at most 2^53 - 1, which D1 stores exactly. Text values are trimmed.
- **Series values**: name not empty; a specialty that the tournament declares (422 otherwise), whose target score
  the series plays to (`target_score`, read from the tournament, never copied); `max_teams` from 2 to 100; `position`
  >= 1, the display order (then slug); `number_block` a multiple of 100 from 100 to 9900. A block holds the 100
  numbers from it (100 holds 100 to 199), and two series of a tournament never share a block (409 `block-taken`), so
  blocks never overlap.
- **Team values**: exactly two players, each with a first and last name not empty, and an optional club, licence
  number and contact (phone or email), not empty when given; `fee_paid` (default `false`), set by the organizer; a
  status `registered` or `withdrawn`. txapela sets `registered_on`, the date in Europe/Paris of the request: the
  route reads the Worker's clock and passes the instant to `domain`. The maximums of the series and of the
  tournament count registered teams only: a team beyond either answers 409 `series-full` or `tournament-full`.
  A registration, and a withdrawn team registered again, carry that room in their SQL (`ROOM` in `db/teams.rs`),
  with the block of the series for a registration: D1 checks it in the statement that writes, so that concurrent
  requests never take a place twice. The writes of a series (open registration, declared specialty, maximum at
  least the registered teams, no team under a moving block) and a tournament `PATCH` that touches its series and
  teams are guarded the same way (see Guarded writes).
- **Pools**: drawn, changed and played while the tournament is `pools` only (409 `not-in-pools` before and after):
  registration is closed by then, so the teams of a draw are fixed, and the final phase (#10) starts from standings
  that no longer change. A pool is named by a letter, its public key within the series: A, B, C…, up to Z, stored as
  its number from 1. Frontis's `pool` (an `Int` that starts again in each `poolPhase`) is that number: pool A is
  Frontis's pool 1, and a series plays a single pool phase (Frontis's `poolPhase` null). A path that is not one
  uppercase letter answers 404.
- **Draw** (`POST …/series/:series/draw`): the registered teams of the series, withdrawn ones left out; at least 4
  (422). The body gives exactly one of `pool_size` (4, 5 or 6: the largest pool, in as few pools as it allows) or
  `pools` (1 to 26); every pool then holds 4 to 6 teams and pools differ by one team at most (422 otherwise). The
  seed order is the optional `seeds` list of team numbers (registered teams of the series, none twice, 422
  otherwise), then the other teams by number: teams have no seed column. Pots of one team per pool fill the pools in
  snake order (A, B, C, then C, B, A…); within a pot, a team takes another pool of the pot when its own already holds
  a team of its club, and the draw keeps the most teams away from their clubs that it can (a maximum matching, each
  team trying its snake pool first). A team's clubs are the clubs of its players, trimmed and compared without case;
  two teams share a club when one of their players does. The draw generates the matches of every pool. A series is
  drawn again while it has no result, played or forfeit (409 `series-has-results` after): the new draw replaces the
  pools, their matches and their tiebreak. 201 with `Location: …/pools`.
- **Move** (`POST …/pools/:pool/teams {"number": …}`): a team of another pool joins this one, keeping its seed, until
  the first result of the series (409 `series-has-results`). A move leaves 3 to 7 teams in each pool (422 otherwise):
  one more or less than a draw, so that two teams swap in two moves. It generates again the matches of both pools in
  the batch that moves the team, and removes their tiebreak. A team already in the pool stays: 200, nothing changes.
- **Matches**: every team of a pool against every other one once, by the circle method over the seed order; a pool
  with an odd number of teams gets a bye each round, and each team rests once. A match has a `phase` (`pool`, as
  Frontis names it; the final phase comes with #10), its pool, a number within the pool from 1 (its public key with the
  pool, in `…/pools/:pool/matches/:match`, a path of decimal digits without a leading zero), a round, its two teams
  (the better seed first), a `state` (`scheduled`, `played`, `forfeit`, `postponed`) and, with a result, its score.
  `phase` and `state` are `x-extensible-enum` lists in the contract: #9 sets `postponed` and #10 adds phases. The slot
  of a match (date, time, court) comes with the calendar (#9).
- **Results** (`PUT …/matches/:match/result`): recorded and corrected (recorded again) while the tournament is `pools`.
  The body names the two teams of the match in either order (`teams`, 422 for another pair) and gives exactly one of
  `score` (in the order of `teams`) or `forfeited_by` (a team of the match). A score is valid when one team reaches
  exactly the target score of the series and the other stays from 0 below it (422). A forfeit records the target
  score to 0 for the other team, never 0–0, and it counts in the goal average; the team that forfeits gets the forfeit
  points of the tournament, the other team the win points. Pools, matches and standings show the public view of a
  team to every caller, the organizer included: its number and its players' names and clubs.
- **Standings** (`GET …/standings`): computed from the matches on every read, never stored, so a corrected score
  updates them. Played (results, forfeits included), wins, losses (forfeits included), forfeits, points, goals for,
  goals against, goal average (for minus against) and rank: by points, then goal average; teams still level share
  their rank (1, 2, 2, 4), by number. The organizer's tiebreak (`PUT …/pools/:pool/tiebreak {"teams": […]}`, every
  team of the pool once, or none to remove it, 422 otherwise) is the last sort key: it separates teams level on
  points and goal average, and only them.
- **Status**: `draft` → `registration` → `pools` → `finals` → `finished`, one step forward only, through
  `PATCH {"status": ...}`; any other value, the current one included, answers 409 `invalid-transition`. A new
  tournament is a draft. A draft is never public.
- **Registration open**: while the tournament is `draft` or `registration`, the organizer creates and deletes series
  and teams, changes the specialty, maximum and block of a series, and withdraws a team or registers it again (when
  its series and tournament have room). From `pools` on, these answer 409 `registration-closed`, even with the
  current value: a team that withdraws then stays in its pool, with its status `registered`, and the organizer
  records each of its remaining matches as a forfeit. The name and position of a series, and the players and fee of a
  team, stay editable in every status.
- **PATCH**: absent fields keep their value, nested objects (`venue`, `points`, `dates`) included; `courts` and
  `specialties` are replaced whole. No field takes `null` (422). Aire, specialties, points, ranking and courts change
  only while the status before the patch is `draft`: once present in the body, even with their current value, they
  answer 409 `not-a-draft` on another status. The other fields stay editable in every status. A tournament `PATCH`
  that removes a specialty a series plays, or sets `max_teams` below the registered teams, answers 422
  `invalid-request`, after the other rules: the route loads its series and teams only when the specialties change or
  `max_teams` goes down. A series `PATCH` cannot set `max_teams` below its registered teams (409 `series-full`) nor
  move the block of a series that has teams, withdrawn ones included (409 `series-has-teams`). A team `PATCH`
  replaces both players.
- **DELETE**: a tournament in draft only, otherwise 409 `not-a-draft`; its courts, specialties, series, teams,
  players, pools and matches go with it (ON DELETE CASCADE). A series without teams only (409 `series-has-teams`; the foreign key of
  `teams` refuses it too). A team with its players.
- **Errors** are `application/problem+json` (RFC 9457) with a stable `type`,
  `https://txapela.pilotariak.com/problems/<name>`: the list is the `ProblemType` enum of `problem.rs`. Never rename
  one: clients map them to messages. The `detail` names the field and the rule, in English, for developers; it never
  holds a token, a hash or a player's data, and never quotes a value of the body: `from_value` drops it from serde's
  messages, and names an unknown field or variant only when it looks like a name of the API (a letter, then
  letters, digits or underscores), never an email address or a phone number.

  | Type                  | Status | When                                                                                                |
  | --------------------- | ------ | --------------------------------------------------------------------------------------------------- |
  | `unauthorized`        | 401    | No organizer token, a malformed or wrong one, or no valid secret                                    |
  | `not-found`           | 404    | Unknown path under `/api/`, unknown slug, series, team, pool or match, or a draft without the token |
  | `method-not-allowed`  | 405    | Another method on a known path                                                                      |
  | `slug-taken`          | 409    | Another tournament has this slug, or another series of the tournament                               |
  | `not-a-draft`         | 409    | Changing aire, specialties, points, ranking or courts, or deleting, outside draft                   |
  | `invalid-transition`  | 409    | A status change other than one step forward                                                         |
  | `invalid-request`     | 422    | Body too large, not JSON, unknown field, `null`, or a broken rule                                   |
  | `internal`            | 500    | txapela failed                                                                                      |
  | `registration-closed` | 409    | A series or team change that needs draft or registration, from `pools` on                           |
  | `series-full`         | 409    | A team beyond the maximum of its series, or a series maximum below its registered teams             |
  | `tournament-full`     | 409    | A team beyond the maximum teams of the tournament                                                   |
  | `number-taken`        | 409    | Another team of the tournament has this number                                                      |
  | `block-taken`         | 409    | Another series of the tournament has this number block                                              |
  | `series-has-teams`    | 409    | Deleting a series that has teams, or moving its number block                                        |
  | `not-in-pools`        | 409    | A draw, a move, a tiebreak or a result while the tournament is not `pools`                          |
  | `series-has-results`  | 409    | A draw or a move once the series has a result, played or forfeit                                    |

- **OpenAPI**: `openapi.json` is the contract, OpenAPI 3.1, generated by utoipa from the `#[utoipa::path]`
  annotations of the handlers. `cargo test` fails when it differs from the code: run `make openapi` and commit it.
  The write routes declare the `bearer` security scheme; the reads declare it as optional. CI runs
  `oasdiff breaking` against the base branch (job `contract`): a breaking change of `/api/v1` fails, and goes to
  `/api/v2` instead.
- **Problem types in the contract**: the `ProblemType` schema is the closed `enum` of the 8 types of the health and
  tournaments routes (`FROZEN` in `problem.rs`), which every error response of those routes refers to. oasdiff rates a
  value added to a response `enum` as breaking (`response-property-enum-value-added`, ERR), and an `enum` turned into
  an open list too (`response-property-became-not-enum`, ERR): `FROZEN` never changes, and those routes send no other
  type (a tournament `PATCH` refused by its series and teams answers 422). Every type is listed again in
  `ExtensibleProblemType`, an `x-extensible-enum` that clients read as open: the 409 of the series and teams routes
  (`SeriesAndTeamsConflict`) and of the pools, matches and standings routes (`PoolsConflict`) refers to it, so a new
  type stays additive there. Their 401, 404 and 422 reuse the closed responses. A new route with a new problem type
  refers to the extensible list. A response value whose list will grow, such as the `phase` and `state` of a match,
  is an `x-extensible-enum` too, never a closed `enum`.
- **CORS** (`cors.rs`): one list, the `ALLOWED_ORIGINS` Worker variable (origins separated by commas), drives CORS.
  Production allows `https://kancha.pilotariak.com` only (`[vars]` of `wrangler.toml`, checked by
  `tests/cors.test.mjs`); `make dev` replaces it with `--var` for the Expo dev server, `http://localhost:8081`. Never
  put it in `.dev.vars` or a secret. Every `/api/` response sends `Vary: Origin`, and to an allowed origin
  `Access-Control-Allow-Origin` (the origin echoed, never `*`) and `Access-Control-Allow-Credentials: true`, problems
  included. `OPTIONS` from an allowed origin answers 204 with the methods, the request headers and
  `Access-Control-Max-Age: 7200` (the cap of Chromium); any other `OPTIONS` gets the router's 405. A request header
  that Kancha starts sending goes in `ALLOW_HEADERS` (`Content-Type, Authorization`); a response header it starts
  reading needs `Access-Control-Expose-Headers`.

### Landing page

`src/web/landing.html` is one HTML document with inline CSS and script, no build step, in the style of the other
Pilotariak landing pages (design tokens from frontis `DESIGN.md`).

- `__TXAPELA_VERSION__` is replaced with `CARGO_PKG_VERSION` when the page is served.
- `GET /` sends `Content-Security-Policy` (Azkena's policy plus `connect-src 'self'` for the status script) and
  `Cache-Control: public, max-age=300`. A new origin on the page needs a CSP change.
- Never put player contact details, or real player names, on the page.

## Rules the next issues must keep

- **Personal data.** Organizers write the contact details and licence numbers of players. Never log them, never
  return them on a public route, never quote them in a problem detail (a rule's message names the field, not the
  value), and never paste remote rows into an issue, a pull request or a log. `tests/privacy.test.mjs` reads every
  GET route of `openapi.json` without the token: a new public route is checked by it.
- **No clock in `domain`.** Routes read the Worker's clock and pass the time in, so that every rule is tested on the
  host with fixed instants.
- **API routes under `/api/v1/`**, as above. Only the public pages live outside it.
- **No tournament data in Frontis** ([ADR-0001](docs/adr/0001-keep-tournament-data-out-of-frontis.md)).
- **One organizer check.** Until the ADR of #5, `auth::organizer` is the only place a write route checks the caller.
  Never log a token or its hash, and keep it failing closed.
- **Enums, not CHECK.** A value with a fixed list (aire, specialty, status, ranking, team status, match phase, match
  state, forfeit side) is a TEXT column without `CHECK`, parsed by a Rust enum of `domain/`: a new variant is a code
  change, not a table rebuild.
- **Guarded writes.** A write that takes a limited place (a registered team), lowers a limit (a maximum), or
  removes or moves what other rows depend on (a specialty, a number block, a series) carries its condition in the
  SQL that writes, and the route reads whether it applied (`db::written`, `db::write`): a check in `domain` followed
  by a separate write lets a concurrent request slip between them. A batch carries the condition in every
  statement (`db::Guard`), so that it applies whole or not at all. When D1 refuses, the route runs the checks of
  `domain` again on the new state to word the problem (`api::refusal`). The writes of the pools guard the status
  `pools` and, for a draw or a move, a series without result (`db::pools::guard`); a move and a tiebreak also carry
  `series.pools_revision`, which every draw and move adds one to in the last statement of its batch, so that a move
  never writes matches computed from pools that changed meanwhile (`db::written_last` reads that statement). A result
  carries the two teams of its match. Those routes read, check and write up to `api::ATTEMPTS` times: a refused write
  reads the new state, and answers the problem of a rule it now breaks. Many rows go in one statement as a JSON
  array read with `json_each(?)`: D1 caps the parameters of a statement. The tests of `concurrent …` in
  `tests/teams.test.mjs`, `tests/series.test.mjs`, `tests/pools.test.mjs`, `tests/matches.test.mjs` and
  `tests/standings.test.mjs` check the rows left by concurrent requests.
- **Frozen problem types.** Never add a type to `FROZEN` in `problem.rs`, nor send a new type from the health and
  tournaments routes: their contract lists a closed `enum`. A new type goes to the end of `ProblemType`, and only
  routes whose error responses refer to `ExtensibleProblemType` send it.

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
curl http://localhost:8793/api/v1/tournaments
```

The write routes need the organizer token, whose SHA-256 the Worker reads from the `ORGANIZER_TOKEN_SHA256` secret.
Without it, every organizer call answers 401. For local development, pick a throwaway token and give wrangler its
hash, never a real one:

```bash
TOKEN="local-organizer-token"                          # throwaway, local only
printf %s "$TOKEN" | shasum -a 256 | cut -d' ' -f1     # printf, not echo: a newline changes the hash
```

Then either write `ORGANIZER_TOKEN_SHA256="<hash>"` in `.dev.vars` (git-ignored, read by `wrangler dev`), or pass it
on the command line with `bun run dev --var ORGANIZER_TOKEN_SHA256:<hash>`. Call the write routes with
`Authorization: Bearer $TOKEN`. Never commit a token or its hash.

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
make test            # worker-build, cargo test (openapi.json check included), then the Worker tests (bun run test)
make openapi         # regenerate openapi.json from the #[utoipa::path] annotations (TXAPELA_WRITE_OPENAPI=1)
make deny            # cargo deny check: RustSec advisories, licenses, bans and sources (deny.toml)
make lint            # prek run -a: license headers, formatting, YAML/TOML/JSON, secrets, Rust hooks
```

- Pure Rust logic gets unit tests in its module, run by `cargo test` on the host.
- Behavior that needs the runtime (routes, D1) gets a test in `tests/`, which loads `build/` in Miniflare, as the
  workers-rs README recommends. Start the Worker with `startWorker()` from `tests/worker.mjs`, or `startApi()` for a
  migrated D1 and a request helper, and declare a new binding there, so that every test file runs the same Worker.
  Check an error with `assertProblem()`. `worker.mjs` declares `ORGANIZER_TOKEN_SHA256` with the hash of a fictional
  token, `ORGANIZER_TOKEN`, computed at run time; `startWorker({ bindings })` replaces a binding, or leaves it out
  when set to `undefined`, and `startWorker({ logs })` pushes every line the Worker logs into the `logs` array.
  `createTeams()` registers numbered teams of invented players, `drawnTournament()` a tournament in pools with a drawn
  series, and `recordResult()` records the result of a match.
- Fixtures are fictional: invented players, clubs, venues and addresses, licence numbers like `LIC-EXEMPLE-…`, and
  contacts on `example.org` (reserved for examples, RFC 2606); never real player names or contact details.
- A property of a pure function whose inputs are few is checked for every input rather than sampled: the round robin
  for every pool size from 2 to 100, and the draw for every number of teams from 4 to 100, every size and clubs
  spread four ways. No property-testing crate is needed.
- Time-dependent rules are tested by passing instants to `domain`, or by writing past instants in D1 through the
  binding, never by waiting.

CI runs all of this on every pull request:

- `ci.yml`, job `test`: rustfmt, clippy, `cargo doc`, `cargo test`, `worker-build`, `bun install --frozen-lockfile`, a
  check that `miniflare` matches the version wrangler depends on, the migrations on a local D1, then the Worker tests.
  The Rust cache (`Swatinem/rust-cache`) also holds `~/.cargo/bin`, so `cargo install worker-build` does nothing on a
  cache hit. Only `main` saves the cache.
- `ci.yml`, job `contract` (pull requests): `oasdiff breaking` between the `openapi.json` of the base branch and the
  one of the pull request; an ERR-level change fails.
- `ci.yml`, job `deny`: `cargo deny check`, with the cargo-deny release binary.
- `prek.yml`: the prek hooks.

## Deployment

```bash
make deploy          # make migrate-remote, then bun run deploy (wrangler deploy)
```

Only the maintainer deploys, once the D1 database exists (see Database), and sets the organizer secret, which no
file holds: `bunx wrangler secret put ORGANIZER_TOKEN_SHA256`, then the lowercase SHA-256 of the organizer token.
Until it is set, every organizer call answers 401. `wrangler deploy` attaches the Custom Domain
`txapela.pilotariak.com` declared in `routes`, then `https://txapela.pilotariak.com/api/v1/health` answers. On the
pilotariak.com zone, it creates the DNS record and the certificate. A Custom Domain takes a bare hostname, without a
path or a wildcard.

## Key Conventions

- Keep versions in step, by hand: the `worker` crate and `worker-build` (`Cargo.toml`, `wrangler.toml`, Makefile, CI);
  `wrangler` and `miniflare` at the versions wrangler bundles, both exact; `compatibility_date` no later than the
  workerd that wrangler bundles; `cargo-deny` (Makefile, CI). txapela pins the same versions as pik, `jiff`, `sha2`
  and `utoipa` included.
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
