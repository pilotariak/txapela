---
adr: 0001
status: ✅ Accepted
deciders: nlamirault
consulted:
informed:
date: 2026-11-27
spdx-license: Apache-2.0
---

# ADR-0001: Keep tournament data in its own D1 database, out of Frontis

## Context

txapela manages Basque pelota tournaments run by a club: registrations, series, pools, a calendar on the courts, a
final phase and public results. Organizers write this data. They create tournaments, enter registrations and scores,
handle the entry fees and see the contact details of the players. Results are public.

Frontis ([`pilotariak/frontis`](https://github.com/pilotariak/frontis)) is the GraphQL gateway that the other
Pilotariak services read competition data from. It holds league championships, which the scraper of its scheduler
Worker loads from the results site of each league. Each supported league has a D1 database of its own, and all of them
share the schema in Frontis's [`database/migrations`](https://github.com/pilotariak/frontis/tree/main/database/migrations)
(see its [supported leagues](https://github.com/pilotariak/frontis/blob/main/docs/reference/leagues.md)).
The gateway is public and read-only: its supergraph declares a `Query` type and no `Mutation` type, and it
authenticates no end user.

Tournament data is not like championship data. People write it instead of a scraper, part of it is personal, and it
needs write routes and a way to authenticate organizers.

pik ([`pilotariak/pik`](https://github.com/pilotariak/pik)) is the precedent. The slots of Kancha's Créneaux tab are
written by players and live in pik, outside Frontis. pik is a Cloudflare Worker written in Rust with
[workers-rs](https://github.com/cloudflare/workers-rs), with a D1 database of its own, and Kancha calls it directly.
[pilotariak/pik#2](https://github.com/pilotariak/pik/issues/2) set up the Worker and its D1 database.
[pilotariak/pik#4](https://github.com/pilotariak/pik/issues/4) exposed its REST API under `/api/v1`, described by an
OpenAPI contract.

## Considered Options

1. Tournament tables in Frontis, behind a federated `tournaments` subgraph with mutations.
2. A separate Worker and D1 database: txapela.
3. Tournament tables in the league databases of Frontis.

## Pros and Cons

### 1. Tournament tables in Frontis, behind a subgraph with mutations

**Pros:**

- ✅ One endpoint and one GraphQL client for championships and tournaments.
- ✅ Azkena and Xilo, which already read Frontis, could answer questions about tournaments at once.

**Cons:**

- ❌ It adds mutations and end-user authentication to a public gateway whose supergraph only has queries.
- ❌ The contact details of players would sit behind a gateway built to serve public data to anyone.
- ❌ Organizer routes are mostly writes. pik#4 chose REST over GraphQL for such routes, where GraphQL adds mutations,
  per-field authorization and `POST`-only caching for no gain.

### 2. A separate Worker and D1 database

**Pros:**

- ✅ Frontis does not change: no mutations, no end-user authentication, no tournament tables.
- ✅ Tournament data, personal data included, has one owner and one access path. It can be migrated, exported or
  deleted without touching league data.
- ✅ txapela follows pik's layout and tooling: a workers-rs Worker, D1 migrations, a REST API under `/api/v1`.
- ✅ txapela can serve the public tournament pages itself, so a shared link works without the app.

**Cons:**

- ❌ One more Worker, D1 database and CI pipeline to deploy and watch.
- ❌ Kancha calls one more backend, with its own base URL and its own network errors.
- ❌ Azkena and Xilo do not see tournaments until a read path is added for them.

### 3. Tournament tables in the league databases of Frontis

**Pros:**

- ✅ No new database: tournaments could join the clubs and categories already loaded for a league.

**Cons:**

- ❌ All league databases share one schema, so the tournament tables would land in every league database through the
  shared migrations.
- ❌ A tournament is run by a club, not by a league. A club in a league that Frontis does not support has no database
  to write to.
- ❌ Frontis treats league databases as scraped data, which it resets
  ([`database/reset.sql`](https://github.com/pilotariak/frontis/blob/main/database/reset.sql)) and reloads. Organizer
  data, which cannot be scraped again, would sit next to it.
- ❌ Writes still need a path into Frontis, so the cons of option 1 remain.

## Decision

We will use *option 2*: tournament data lives in a D1 database of its own, `pilotariak-txapela`, served by txapela, a
Cloudflare Worker written in Rust with workers-rs, like pik.

- Frontis stores, serves and proxies no tournament data. Its gateway keeps no mutations and no end-user
  authentication.
- Clients call txapela directly. Kancha holds the organizer screens. txapela serves the public pages itself.

## Consequences

### Positive

- ✅ Frontis and the services that read it do not change.
- ✅ The tournament schema can evolve, and its data can be removed, without touching league data.
- ✅ txapela and pik share a layout, so their code, tests and tooling look alike.

### Negative

- ❌ txapela must be rebuilt as a workers-rs Worker bound to the `pilotariak-txapela` database
  ([#4](https://github.com/pilotariak/txapela/issues/4)): the crate is a binary packaged as a container image today.
- ❌ Kancha talks to three backends: Frontis, pik and txapela.
- ❌ Azkena and Xilo cannot answer questions about tournaments yet.
- ❌ Organizers write personal data, so txapela must authenticate them before Kancha writes to it. How is still open
  ([#5](https://github.com/pilotariak/txapela/issues/5)).

### Neutral

- ↔️ Results are public, unlike pik's slot data. The public pages need no token and never show contact details
  ([#11](https://github.com/pilotariak/txapela/issues/11)).
- ↔️ A read-only `tournaments` subgraph in Frontis, for Azkena and Xilo, stays possible later. It would make Frontis
  serve tournament data, so it needs an ADR of its own. It is out of scope here.

## References

- [pik: the slots service](https://github.com/pilotariak/pik)
- [pilotariak/pik#2: add the pik Worker skeleton and its D1 database](https://github.com/pilotariak/pik/issues/2)
- [pilotariak/pik#4: expose the slots API with player tokens and organizer roles](https://github.com/pilotariak/pik/issues/4)
- [workers-rs: Cloudflare Workers in Rust](https://github.com/cloudflare/workers-rs)
- [Frontis: supported leagues and their D1 databases](https://github.com/pilotariak/frontis/blob/main/docs/reference/leagues.md)
- [#4: re-bootstrap as a workers-rs Worker with D1 `pilotariak-txapela`](https://github.com/pilotariak/txapela/issues/4)
- [#5: decide how organizers authenticate](https://github.com/pilotariak/txapela/issues/5)
- [#11: public tournament and results pages](https://github.com/pilotariak/txapela/issues/11)
