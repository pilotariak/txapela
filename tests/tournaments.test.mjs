// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// The tournaments API (src/api/tournaments.rs): each route, for an organizer and without a
// token, against a migrated D1 in Miniflare.

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import {
  ORGANIZER_TOKEN,
  assertProblem,
  createTournament,
  moveTo,
  startApi,
  tournamentBody,
} from "./worker.mjs";

let mf;
let db;
let call;

before(async () => {
  ({ mf, db, call } = await startApi());
});

after(async () => {
  await mf.dispose();
});

const asOrganizer = { token: ORGANIZER_TOKEN };

// A fresh slug for each tournament of these tests.
let lastSlug = 0;
function slug(prefix = "tournoi") {
  lastSlug += 1;
  return `${prefix}-${lastSlug}`;
}

async function rows(table, slugValue) {
  const row = await db
    .prepare(`SELECT count(*) AS n FROM ${table} WHERE tournament_id = (SELECT id FROM tournaments WHERE slug = ?)`)
    .bind(slugValue)
    .first();
  return row.n;
}

async function tournamentCount() {
  return (await db.prepare("SELECT count(*) AS n FROM tournaments").first()).n;
}

// Checks a 422 whose detail names the rule.
async function assertInvalid(res, detail) {
  const body = await assertProblem(res, 422, "invalid-request");
  assert.match(body.detail ?? "", detail, `the detail names the rule: ${body.detail}`);
}

describe("POST /tournaments", () => {
  test("creates a draft with the default points and ranking", async () => {
    const s = slug("open");
    const res = await call("POST", "/tournaments", {
      ...asOrganizer,
      body: tournamentBody({
        slug: s,
        name: "  Open de printemps ",
        courts: [{ name: " Trinquet B" }, { name: "Trinquet A " }],
        specialties: [
          { specialty: "paleta_gomme_pleine", target_score: 30 },
          { specialty: "paleta_gomme_creuse", target_score: 40 },
        ],
      }),
    });
    assert.equal(res.status, 201);
    assert.equal(res.headers.get("location"), `/api/v1/tournaments/${s}`, "Location of the new tournament");
    assert.equal(res.headers.get("cache-control"), "private, no-store");
    assert.deepEqual(await res.json(), {
      slug: s,
      name: "Open de printemps",
      organizing_club: "Club Exemple de Pelote",
      venue: {
        name: "Trinquet de l'Exemple",
        address: "1 place de l'Exemple, Ville-Exemple",
        latitude: 43,
        longitude: -1,
      },
      aire: "trinquet",
      courts: [{ name: "Trinquet B" }, { name: "Trinquet A" }],
      specialties: [
        { specialty: "paleta_gomme_creuse", target_score: 40 },
        { specialty: "paleta_gomme_pleine", target_score: 30 },
      ],
      points: { win: 3, loss: 1, forfeit: 0 },
      ranking: "points_then_goal_average",
      dates: {
        registration_opens: "2027-03-01",
        registration_closes: "2027-03-31",
        first_match: "2027-04-10",
        finals: "2027-06-19",
      },
      fee_cents: 2000,
      max_teams: 16,
      status: "draft",
    });
    assert.equal(await rows("courts", s), 2, "two courts stored");
    assert.equal(await rows("tournament_specialties", s), 2, "two specialties stored");
    const stored = await call("GET", `/tournaments/${s}`, asOrganizer);
    assert.deepEqual(
      (await stored.json()).courts,
      [{ name: "Trinquet B" }, { name: "Trinquet A" }],
      "courts are read back in the given order",
    );
  });

  test("takes explicit points, keeping the defaults of the others", async () => {
    const body = await createTournament(call, {
      slug: slug(),
      points: { win: 4, forfeit: 1 },
      ranking: "points_then_goal_average",
    });
    assert.deepEqual(body.points, { win: 4, loss: 1, forfeit: 1 }, "loss keeps its default");
  });

  test("a duplicate slug answers 409 slug-taken and writes nothing", async () => {
    const s = slug();
    await createTournament(call, { slug: s });
    const count = await tournamentCount();
    const res = await call("POST", "/tournaments", {
      ...asOrganizer,
      body: tournamentBody({ slug: s, name: "Autre", courts: [{ name: "Fronton" }] }),
    });
    const problem = await assertProblem(res, 409, "slug-taken");
    assert.match(problem.detail, new RegExp(s), "the detail names the slug");
    assert.equal(await tournamentCount(), count, "no tournament added");
    assert.equal(await rows("courts", s), 2, "the first tournament keeps its courts only");
  });

  test("without the organizer token, answers 401 before reading the body", async () => {
    const count = await tournamentCount();
    await assertProblem(await call("POST", "/tournaments", { body: tournamentBody({ slug: slug() }) }), 401, "unauthorized");
    await assertProblem(await call("POST", "/tournaments", { body: "not json" }), 401, "unauthorized");
    assert.equal(await tournamentCount(), count, "nothing written");
  });

  const invalid = [
    ["a slug with capitals", { slug: "Open-2027" }, /^slug: 3 to 64 characters/],
    ["a slug too short", { slug: "ab" }, /^slug:/],
    ["a slug with a double hyphen", { slug: "open--2027" }, /^slug:/],
    ["an unknown aire", { aire: "pelote" }, /unknown variant `pelote`/],
    ["no specialty", { specialties: [] }, /^specialties: holds at least one specialty$/],
    [
      "a specialty twice",
      {
        specialties: [
          { specialty: "paleta_gomme_creuse", target_score: 40 },
          { specialty: "paleta_gomme_creuse", target_score: 30 },
        ],
      },
      /^specialties: paleta_gomme_creuse appears twice$/,
    ],
    ["an unknown specialty", { specialties: [{ specialty: "main_nue", target_score: 40 }] }, /unknown variant `main_nue`/],
    ["a target score of zero", { specialties: [{ specialty: "paleta_gomme_creuse", target_score: 0 }] }, /target_score/],
    ["a target score that is not an integer", { specialties: [{ specialty: "paleta_gomme_creuse", target_score: 40.5 }] }, /i64/],
    ["no court", { courts: [] }, /^courts: holds at least one court$/],
    ["an empty court name", { courts: [{ name: "  " }] }, /^courts: a court name cannot be empty$/],
    ["a court twice", { courts: [{ name: "A" }, { name: " A " }] }, /^courts: the court name "A" appears twice$/],
    ["win not above loss", { points: { win: 1, loss: 1 } }, /^points: win must be above loss$/],
    ["loss below forfeit", { points: { win: 3, loss: 0, forfeit: 1 } }, /^points: loss cannot be below forfeit$/],
    ["a negative forfeit", { points: { forfeit: -1 } }, /^points.forfeit: /],
    [
      "registration closing before it opens",
      { dates: { ...tournamentBody().dates, registration_closes: "2027-02-28" } },
      /^dates: registration_closes cannot be before registration_opens/,
    ],
    [
      "finals before the first match",
      { dates: { ...tournamentBody().dates, finals: "2027-04-09" } },
      /^dates: finals cannot be before first_match/,
    ],
    ["a date that is not YYYY-MM-DD", { dates: { ...tournamentBody().dates, first_match: "10/04/2027" } }, /^dates.first_match is a YYYY-MM-DD date$/],
    ["a day that does not exist", { dates: { ...tournamentBody().dates, finals: "2027-02-30" } }, /^dates.finals is a YYYY-MM-DD date$/],
    ["a missing date", { dates: { registration_opens: "2027-03-01" } }, /missing field `registration_closes`/],
    ["an empty name", { name: " " }, /^name: cannot be empty$/],
    ["an empty organizing club", { organizing_club: "" }, /^organizing_club: cannot be empty$/],
    ["an empty venue name", { venue: { ...tournamentBody().venue, name: "" } }, /^venue.name: cannot be empty$/],
    ["an empty venue address", { venue: { ...tournamentBody().venue, address: " " } }, /^venue.address: cannot be empty$/],
    ["a latitude above 90", { venue: { ...tournamentBody().venue, latitude: 91 } }, /^venue.latitude: is between -90 and 90$/],
    ["a longitude below -180", { venue: { ...tournamentBody().venue, longitude: -181 } }, /^venue.longitude: is between -180 and 180$/],
    ["a negative fee", { fee_cents: -1 }, /^fee_cents: is an integer from 0/],
    ["a fee D1 cannot store exactly", { fee_cents: 2 ** 53 }, /^fee_cents: is an integer from 0 to 9007199254740991$/],
    ["a single team", { max_teams: 1 }, /^max_teams: is an integer from 2/],
    ["a status", { status: "registration" }, /unknown field `status`/],
    ["an unknown field", { colour: "red" }, /unknown field `colour`/],
    ["an unknown field of the venue", { venue: { ...tournamentBody().venue, city: "X" } }, /unknown field `city`/],
    ["a null", { ranking: null }, /^ranking cannot be null$/],
    ["a nested null", { points: { win: null } }, /^points.win cannot be null$/],
  ];
  for (const [what, changes, detail] of invalid) {
    test(`${what} answers 422 invalid-request`, async () => {
      const count = await tournamentCount();
      const res = await call("POST", "/tournaments", { ...asOrganizer, body: tournamentBody({ slug: slug(), ...changes }) });
      await assertInvalid(res, detail);
      assert.equal(await tournamentCount(), count, "nothing written");
    });
  }

  test("a missing field, a body that is not JSON or too large answers 422", async () => {
    const { venue: _venue, ...noVenue } = tournamentBody({ slug: slug() });
    await assertInvalid(await call("POST", "/tournaments", { ...asOrganizer, body: noVenue }), /missing field `venue`/);
    await assertInvalid(await call("POST", "/tournaments", { ...asOrganizer, body: "{" }), /^the body is not JSON$/);
    await assertInvalid(
      await call("POST", "/tournaments", { ...asOrganizer, body: tournamentBody({ slug: slug(), name: "x".repeat(17 * 1024) }) }),
      /^the body is larger than 16384 bytes$/,
    );
  });
});

describe("GET /tournaments", () => {
  test("lists public tournaments by first match date then slug, and drafts for an organizer only", async () => {
    const dates = (first) => ({
      registration_opens: "2028-01-01",
      registration_closes: "2028-01-15",
      first_match: first,
      finals: "2028-12-31",
    });
    const later = await createTournament(call, { slug: "liste-c", dates: dates("2028-05-01") });
    const tieB = await createTournament(call, { slug: "liste-b", dates: dates("2028-04-01") });
    const tieA = await createTournament(call, { slug: "liste-a", dates: dates("2028-04-01") });
    const draft = await createTournament(call, { slug: "liste-0-draft", dates: dates("2028-03-01") });
    for (const { slug: s } of [later, tieB, tieA]) {
      await moveTo(call, s, "registration");
    }
    const ours = (list) => list.tournaments.map((t) => t.slug).filter((s) => s.startsWith("liste-"));

    const anonymous = await call("GET", "/tournaments");
    assert.equal(anonymous.status, 200);
    assert.equal(anonymous.headers.get("cache-control"), "private, no-store");
    const publicList = await anonymous.json();
    assert.deepEqual(ours(publicList), ["liste-a", "liste-b", "liste-c"], "no draft, by first match then slug");
    assert.ok(
      publicList.tournaments.every((t) => t.status !== "draft"),
      "the public list holds no draft at all",
    );
    const listed = publicList.tournaments.find((t) => t.slug === "liste-a");
    assert.deepEqual(listed.courts, [{ name: "Trinquet A" }, { name: "Trinquet B" }], "with its courts");
    assert.deepEqual(listed.specialties, [{ specialty: "paleta_gomme_creuse", target_score: 40 }], "and specialties");

    const organizer = await (await call("GET", "/tournaments", asOrganizer)).json();
    assert.deepEqual(ours(organizer), [draft.slug, "liste-a", "liste-b", "liste-c"], "drafts included, same order");
  });
});

describe("GET /tournaments/:slug", () => {
  test("a draft is invisible without the organizer token: 404 as for an unknown slug", async () => {
    const { slug: s } = await createTournament(call, { slug: slug("brouillon") });
    const hidden = await call("GET", `/tournaments/${s}`);
    const unknown = await call("GET", "/tournaments/no-such-tournament");
    const hiddenBody = await assertProblem(hidden, 404, "not-found");
    const unknownBody = await assertProblem(unknown, 404, "not-found");
    assert.deepEqual(hiddenBody, unknownBody, "a draft and an unknown slug answer the same problem");

    const seen = await call("GET", `/tournaments/${s}`, asOrganizer);
    assert.equal(seen.status, 200);
    assert.equal((await seen.json()).status, "draft", "the organizer sees the draft");
  });

  test("a published tournament is public", async () => {
    const created = await createTournament(call, { slug: slug("public") });
    await moveTo(call, created.slug, "registration");
    const res = await call("GET", `/tournaments/${created.slug}`);
    assert.equal(res.status, 200);
    assert.deepEqual(await res.json(), { ...created, status: "registration" }, "the whole tournament");
  });

  test("a path that cannot be a slug answers 404", async () => {
    for (const path of ["/tournaments/Open-2027", "/tournaments/ab", "/tournaments/a--b"]) {
      await assertProblem(await call("GET", path, asOrganizer), 404, "not-found");
    }
  });
});

describe("PATCH /tournaments/:slug", () => {
  test("changes the fields of the body and keeps the others", async () => {
    const created = await createTournament(call, { slug: slug() });
    const res = await call("PATCH", `/tournaments/${created.slug}`, {
      ...asOrganizer,
      body: {
        name: "Open d'été",
        venue: { latitude: 43.5 },
        points: { loss: 2 },
        dates: { finals: "2027-06-26" },
        courts: [{ name: "Trinquet C" }],
        specialties: [{ specialty: "paleta_gomme_pleine", target_score: 30 }],
      },
    });
    assert.equal(res.status, 200);
    const expected = {
      ...created,
      name: "Open d'été",
      venue: { ...created.venue, latitude: 43.5 },
      points: { win: 3, loss: 2, forfeit: 0 },
      dates: { ...created.dates, finals: "2027-06-26" },
      courts: [{ name: "Trinquet C" }],
      specialties: [{ specialty: "paleta_gomme_pleine", target_score: 30 }],
    };
    assert.deepEqual(await res.json(), expected, "the response is the tournament after the change");
    const stored = await call("GET", `/tournaments/${created.slug}`, asOrganizer);
    assert.deepEqual(await stored.json(), expected, "the change is stored");
    assert.equal(await rows("courts", created.slug), 1, "courts replaced");
    assert.equal(await rows("tournament_specialties", created.slug), 1, "specialties replaced");

    const empty = await call("PATCH", `/tournaments/${created.slug}`, { ...asOrganizer, body: {} });
    assert.equal(empty.status, 200);
    assert.deepEqual(await empty.json(), expected, "an empty body changes nothing");
  });

  test("validates the merged state: one date must keep all four in order", async () => {
    const created = await createTournament(call, { slug: slug() });
    await assertInvalid(
      await call("PATCH", `/tournaments/${created.slug}`, { ...asOrganizer, body: { dates: { first_match: "2027-07-01" } } }),
      /^dates: finals cannot be before first_match/,
    );
    await assertInvalid(
      await call("PATCH", `/tournaments/${created.slug}`, { ...asOrganizer, body: { points: { win: 1 } } }),
      /^points: win must be above loss$/,
    );
    await assertInvalid(
      await call("PATCH", `/tournaments/${created.slug}`, { ...asOrganizer, body: { courts: [] } }),
      /^courts: holds at least one court$/,
    );
    const stored = await call("GET", `/tournaments/${created.slug}`, asOrganizer);
    assert.deepEqual(await stored.json(), created, "nothing changed");
  });

  test("rejects the slug, unknown fields and nulls", async () => {
    const created = await createTournament(call, { slug: slug() });
    const path = `/tournaments/${created.slug}`;
    await assertInvalid(
      await call("PATCH", path, { ...asOrganizer, body: { slug: "autre-slug" } }),
      /^slug cannot change/,
    );
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: { slug: created.slug } }), /^slug cannot change/);
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: { colour: "red" } }), /unknown field `colour`/);
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: { name: null } }), /^name cannot be null$/);
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: { dates: { finals: "June" } } }), /^dates.finals is a YYYY-MM-DD date$/);
    await assertInvalid(await call("PATCH", path, { ...asOrganizer, body: { status: "open" } }), /unknown variant `open`/);
  });

  test("aire, specialties, points, ranking and courts change only in draft", async () => {
    const created = await createTournament(call, { slug: slug() });
    await moveTo(call, created.slug, "registration");
    const path = `/tournaments/${created.slug}`;
    for (const [field, value] of [
      ["aire", "fronton"],
      ["aire", "trinquet"],
      ["specialties", created.specialties],
      ["points", { win: 3 }],
      ["ranking", "points_then_goal_average"],
      ["courts", [{ name: "Trinquet C" }]],
    ]) {
      const problem = await assertProblem(
        await call("PATCH", path, { ...asOrganizer, body: { [field]: value } }),
        409,
        "not-a-draft",
      );
      assert.match(problem.detail, new RegExp(`^${field}: only a draft`), `${field} is fixed, even to its value`);
    }
    const res = await call("PATCH", path, {
      ...asOrganizer,
      body: {
        name: "Open d'automne",
        organizing_club: "Autre Club Exemple",
        venue: { name: "Fronton de l'Exemple" },
        dates: { finals: "2027-06-20" },
        fee_cents: 0,
        max_teams: 24,
      },
    });
    assert.equal(res.status, 200, "the other fields stay editable");
    const body = await res.json();
    assert.equal(body.status, "registration");
    assert.equal(body.max_teams, 24);
    assert.equal(body.aire, "trinquet", "aire unchanged");
  });

  test("status moves one step forward only", async () => {
    const created = await createTournament(call, { slug: slug() });
    const path = `/tournaments/${created.slug}`;
    const patch = (body) => call("PATCH", path, { ...asOrganizer, body });

    let problem = await assertProblem(await patch({ status: "pools" }), 409, "invalid-transition");
    assert.equal(problem.detail, "status: a draft tournament moves to registration only, not to pools");
    await assertProblem(await patch({ status: "draft" }), 409, "invalid-transition");

    const opened = await patch({ status: "registration", aire: "mur_a_gauche" });
    assert.equal(opened.status, 200, "a draft opens registration, changing its aire on the way");
    assert.deepEqual(
      [(await opened.clone().json()).status, (await opened.json()).aire],
      ["registration", "mur_a_gauche"],
    );

    await assertProblem(await patch({ status: "draft" }), 409, "invalid-transition");
    await assertProblem(await patch({ status: "registration" }), 409, "invalid-transition");
    for (const status of ["pools", "finals", "finished"]) {
      const res = await patch({ status });
      assert.equal(res.status, 200, `moves to ${status}`);
      assert.equal((await res.json()).status, status);
    }
    problem = await assertProblem(await patch({ status: "finished" }), 409, "invalid-transition");
    assert.equal(problem.detail, "status: a finished tournament cannot move to finished");
  });

  test("not-a-draft comes before invalid-transition, which comes before the rules", async () => {
    const created = await createTournament(call, { slug: slug() });
    await moveTo(call, created.slug, "pools");
    const path = `/tournaments/${created.slug}`;
    const patch = (body) => call("PATCH", path, { ...asOrganizer, body });
    await assertProblem(await patch({ aire: "fronton", status: "draft", name: "" }), 409, "not-a-draft");
    await assertProblem(await patch({ status: "registration", name: "" }), 409, "invalid-transition");
    await assertInvalid(await patch({ status: "finals", name: "" }), /^name: cannot be empty$/);
  });

  test("answers 401 without the token, then 404 for an unknown slug", async () => {
    const created = await createTournament(call, { slug: slug() });
    await assertProblem(await call("PATCH", `/tournaments/${created.slug}`, { body: { name: "X" } }), 401, "unauthorized");
    await assertProblem(await call("PATCH", "/tournaments/no-such-tournament", { body: { name: "X" } }), 401, "unauthorized");
    await assertProblem(
      await call("PATCH", "/tournaments/no-such-tournament", { ...asOrganizer, body: { name: "X" } }),
      404,
      "not-found",
    );
    await assertProblem(await call("PATCH", `/tournaments/${created.slug}`, { ...asOrganizer, body: "{" }), 422, "invalid-request");
  });
});

describe("DELETE /tournaments/:slug", () => {
  test("deletes a draft with its courts and specialties", async () => {
    const created = await createTournament(call, { slug: slug() });
    const kept = await createTournament(call, { slug: slug() });
    const res = await call("DELETE", `/tournaments/${created.slug}`, asOrganizer);
    assert.equal(res.status, 204);
    assert.equal(await res.text(), "", "no body");
    await assertProblem(await call("GET", `/tournaments/${created.slug}`, asOrganizer), 404, "not-found");
    const orphans = await db
      .prepare(
        "SELECT (SELECT count(*) FROM courts WHERE tournament_id NOT IN (SELECT id FROM tournaments)) + " +
          "(SELECT count(*) FROM tournament_specialties WHERE tournament_id NOT IN (SELECT id FROM tournaments)) AS n",
      )
      .first();
    assert.equal(orphans.n, 0, "no court or specialty left without its tournament");
    assert.equal(await rows("courts", kept.slug), 2, "another tournament keeps its courts");
    await assertProblem(await call("DELETE", `/tournaments/${created.slug}`, asOrganizer), 404, "not-found");
  });

  test("a tournament that is not a draft answers 409 not-a-draft and stays", async () => {
    const created = await createTournament(call, { slug: slug() });
    await moveTo(call, created.slug, "registration");
    const problem = await assertProblem(await call("DELETE", `/tournaments/${created.slug}`, asOrganizer), 409, "not-a-draft");
    assert.equal(problem.detail, "status: a registration tournament cannot be deleted, only a draft can");
    const stored = await call("GET", `/tournaments/${created.slug}`);
    assert.equal(stored.status, 200, "still there");
    await stored.body?.cancel();
  });

  test("answers 401 without the token, then 404 for an unknown slug", async () => {
    const created = await createTournament(call, { slug: slug() });
    await assertProblem(await call("DELETE", `/tournaments/${created.slug}`), 401, "unauthorized");
    await assertProblem(await call("DELETE", "/tournaments/no-such-tournament", asOrganizer), 404, "not-found");
    const stored = await call("GET", `/tournaments/${created.slug}`, asOrganizer);
    assert.equal(stored.status, 200, "the draft is still there");
    await stored.body?.cancel();
  });
});

describe("other methods", () => {
  test("PUT on the tournaments paths answers 405", async () => {
    await assertProblem(await call("PUT", "/tournaments", asOrganizer), 405, "method-not-allowed");
    await assertProblem(await call("PUT", "/tournaments/some-slug", asOrganizer), 405, "method-not-allowed");
  });
});

// Round-1 checks of the binding decisions, against the API and the D1 binding.

describe("creation values", () => {
  test("all three aires and the single ranking round-trip through creation", async () => {
    for (const aire of ["fronton", "mur_a_gauche", "trinquet"]) {
      const created = await createTournament(call, {
        slug: slug("aire"),
        aire,
        ranking: "points_then_goal_average",
      });
      assert.equal(created.aire, aire, `${aire} is stored as its enum value`);
      assert.equal(created.ranking, "points_then_goal_average", "the only ranking");
      const stored = await call("GET", `/tournaments/${created.slug}`, asOrganizer);
      assert.equal((await stored.json()).aire, aire, `${aire} is read back`);
    }
  });

  test("a slug of 64 characters is accepted; longer and edge shapes are 422", async () => {
    const longest = "a".repeat(64);
    const created = await createTournament(call, { slug: longest });
    assert.equal(created.slug, longest, "the longest slug is valid");
    const count = await tournamentCount();
    for (const bad of ["a".repeat(65), "-abc", "abc-", "a--b", "ab_cd", "abc!", "ab c"]) {
      const res = await call("POST", "/tournaments", {
        ...asOrganizer,
        body: tournamentBody({ slug: bad }),
      });
      await assertInvalid(res, /^slug: 3 to 64 characters among a-z, 0-9 and single hyphens/);
    }
    assert.equal(await tournamentCount(), count, "invalid slugs do not add rows");
  });

  test("four equal dates are in order; first match before registration close is 422", async () => {
    const dates = {
      registration_opens: "2027-05-01",
      registration_closes: "2027-05-01",
      first_match: "2027-05-01",
      finals: "2027-05-01",
    };
    const created = await createTournament(call, { slug: slug("dates-eq"), dates });
    assert.deepEqual(created.dates, dates, "equal dates are allowed");

    const count = await tournamentCount();
    await assertInvalid(
      await call("POST", "/tournaments", {
        ...asOrganizer,
        body: tournamentBody({
          slug: slug("dates-bad"),
          dates: { ...dates, first_match: "2027-04-30" },
        }),
      }),
      /^dates: first_match cannot be before registration_closes/,
    );
    assert.equal(await tournamentCount(), count, "nothing written");
  });

  test("the largest integer D1 stores exactly is accepted, and one above is 422", async () => {
    const largest = 2 ** 53 - 1;
    const created = await createTournament(call, {
      slug: slug("big"),
      fee_cents: largest,
      max_teams: largest,
      points: { win: largest, loss: largest - 1, forfeit: 0 },
      specialties: [{ specialty: "paleta_gomme_pleine", target_score: largest }],
    });
    assert.equal(created.fee_cents, largest, "fee_cents survives the round trip");
    assert.equal(created.max_teams, largest, "max_teams survives the round trip");
    assert.equal(created.points.win, largest, "points.win survives the round trip");
    assert.equal(created.specialties[0].target_score, largest, "target_score survives the round trip");

    for (const field of ["fee_cents", "max_teams"]) {
      await assertInvalid(
        await call("POST", "/tournaments", {
          ...asOrganizer,
          body: tournamentBody({ slug: slug("big-bad"), [field]: 2 ** 53 }),
        }),
        new RegExp(`^${field}: is an integer from`),
      );
    }
    await assertInvalid(
      await call("POST", "/tournaments", {
        ...asOrganizer,
        body: tournamentBody({
          slug: slug("big-bad"),
          specialties: [{ specialty: "paleta_gomme_pleine", target_score: 2 ** 53 }],
        }),
      }),
      /^specialties: the target_score of paleta_gomme_pleine is an integer from 1/,
    );
  });
});

describe("D1 rows after a change or a delete", () => {
  test("a draft PATCH replaces courts, specialties and points in the stored rows", async () => {
    const created = await createTournament(call, { slug: slug("rows") });
    assert.equal(await rows("courts", created.slug), 2, "two courts to begin with");

    const res = await call("PATCH", `/tournaments/${created.slug}`, {
      ...asOrganizer,
      body: {
        courts: [{ name: " Trinquet C " }, { name: "Fronton" }],
        specialties: [
          { specialty: "paleta_gomme_pleine", target_score: 25 },
          { specialty: "paleta_gomme_creuse", target_score: 15 },
        ],
        points: { win: 5, forfeit: 1 },
      },
    });
    assert.equal(res.status, 200);
    assert.deepEqual(
      (await res.json()).specialties,
      [
        { specialty: "paleta_gomme_creuse", target_score: 15 },
        { specialty: "paleta_gomme_pleine", target_score: 25 },
      ],
      "specialties are returned in the fixed order",
    );

    const courtRows = await db
      .prepare(
        "SELECT position, name FROM courts WHERE tournament_id = " +
          "(SELECT id FROM tournaments WHERE slug = ?) ORDER BY position",
      )
      .bind(created.slug)
      .all();
    assert.deepEqual(
      courtRows.results.map((row) => [row.position, row.name]),
      [
        [1, "Trinquet C"],
        [2, "Fronton"],
      ],
      "court names are trimmed and keep the given order in D1",
    );
    const specialtyRows = await db
      .prepare(
        "SELECT specialty, target_score FROM tournament_specialties " +
          "WHERE tournament_id = (SELECT id FROM tournaments WHERE slug = ?) ORDER BY specialty",
      )
      .bind(created.slug)
      .all();
    assert.deepEqual(
      specialtyRows.results.map((row) => [row.specialty, row.target_score]),
      [
        ["paleta_gomme_creuse", 15],
        ["paleta_gomme_pleine", 25],
      ],
      "the two specialties are stored with their own target scores",
    );
    const stored = await db
      .prepare(
        "SELECT points_win, points_loss, points_forfeit FROM tournaments WHERE slug = ?",
      )
      .bind(created.slug)
      .first();
    assert.deepEqual(
      [stored.points_win, stored.points_loss, stored.points_forfeit],
      [5, 1, 1],
      "the partial points patch merged with the stored loss",
    );
  });

  test("DELETE removes only the deleted tournament's courts and specialties", async () => {
    const gone = await createTournament(call, { slug: slug("gone") });
    const kept = await createTournament(call, { slug: slug("kept") });
    assert.equal(await rows("courts", gone.slug), 2, "the doomed tournament has courts");
    assert.equal(await rows("tournament_specialties", gone.slug), 1, "and a specialty");
    const keptCourts = await rows("courts", kept.slug);
    const keptSpecialties = await rows("tournament_specialties", kept.slug);

    assert.equal((await call("DELETE", `/tournaments/${gone.slug}`, asOrganizer)).status, 204);
    assert.equal(
      (await db.prepare("SELECT count(*) AS n FROM tournaments WHERE slug = ?").bind(gone.slug).first()).n,
      0,
      "the row is gone",
    );
    assert.equal(await rows("courts", gone.slug), 0, "its courts went with it");
    assert.equal(await rows("tournament_specialties", gone.slug), 0, "its specialty went with it");
    assert.equal(await rows("courts", kept.slug), keptCourts, "the other tournament keeps its courts");
    assert.equal(
      await rows("tournament_specialties", kept.slug),
      keptSpecialties,
      "and its specialty",
    );
  });
});

describe("draft visibility and status rules", () => {
  test("a draft is hidden until the status leaves draft, then becomes public", async () => {
    const created = await createTournament(call, { slug: slug("visible") });
    const hidden = await call("GET", `/tournaments/${created.slug}`);
    await assertProblem(hidden, 404, "not-found");
    const before = await (await call("GET", "/tournaments")).json();
    assert.ok(
      !before.tournaments.some((t) => t.slug === created.slug),
      "the public list omits the draft",
    );

    await moveTo(call, created.slug, "registration");
    const published = await call("GET", `/tournaments/${created.slug}`);
    assert.equal(published.status, 200, "the public reads a non-draft");
    assert.equal((await published.json()).status, "registration");
    const after = await (await call("GET", "/tournaments")).json();
    assert.ok(
      after.tournaments.some((t) => t.slug === created.slug),
      "the list holds the published tournament",
    );

    for (const next of ["pools", "finals", "finished"]) {
      const moved = await call("PATCH", `/tournaments/${created.slug}`, {
        ...asOrganizer,
        body: { status: next },
      });
      assert.equal(moved.status, 200, `${created.slug} moves to ${next}`);
      await moved.body?.cancel();
    }
    const finished = await call("GET", `/tournaments/${created.slug}`);
    assert.equal((await finished.json()).status, "finished", "the public reads a finished tournament");
  });

  test("DELETE is refused in every non-draft status and keeps the rows", async () => {
    for (const status of ["registration", "pools", "finals", "finished"]) {
      const created = await createTournament(call, { slug: slug("notdel") });
      await moveTo(call, created.slug, status);
      const problem = await assertProblem(
        await call("DELETE", `/tournaments/${created.slug}`, asOrganizer),
        409,
        "not-a-draft",
      );
      assert.equal(
        problem.detail,
        `status: a ${status} tournament cannot be deleted, only a draft can`,
      );
      assert.equal(
        (await db.prepare("SELECT count(*) AS n FROM tournaments WHERE slug = ?").bind(created.slug).first()).n,
        1,
        `${status}: the tournament stays`,
      );
      assert.equal(await rows("courts", created.slug), 2, `${status}: its courts stay`);
    }
  });

  test("a PATCH of one date must keep the stored four dates in order", async () => {
    const created = await createTournament(call, { slug: slug("patch-date") });
    await assertInvalid(
      await call("PATCH", `/tournaments/${created.slug}`, {
        ...asOrganizer,
        body: { dates: { registration_opens: "2027-04-01" } },
      }),
      /^dates: registration_closes cannot be before registration_opens/,
    );
    const after = await call("GET", `/tournaments/${created.slug}`, asOrganizer);
    assert.deepEqual(await after.json(), created, "the failed patch changed nothing");
  });

  test("PATCH rejects a null body and a null item in an array", async () => {
    const created = await createTournament(call, { slug: slug("patch-null") });
    const path = `/tournaments/${created.slug}`;
    await assertInvalid(
      await call("PATCH", path, { ...asOrganizer, body: "null" }),
      /^the body cannot be null$/,
    );
    await assertInvalid(
      await call("PATCH", path, { ...asOrganizer, body: { courts: [{ name: "A" }, null] } }),
      /^courts\[1\] cannot be null$/,
    );
    const after = await call("GET", path, asOrganizer);
    assert.deepEqual(await after.json(), created, "the stored tournament is unchanged");
  });
});
