// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Extra match tests through the Worker: scores against several targets, the order in which the
// body lists the teams, forfeits, and the order of checks. Players and clubs are fictional.

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import {
  ORGANIZER_TOKEN,
  assertProblem,
  drawnTournament,
  recordResult,
  seriesPath,
  startApi,
} from "./worker.mjs";

let mf;
let call;

before(async () => {
  ({ mf, call } = await startApi());
});

after(async () => {
  await mf.dispose();
});

const asOrganizer = { token: ORGANIZER_TOKEN };

let lastSlug = 0;
function nextSlug() {
  lastSlug += 1;
  return `matches-scores-${lastSlug}`;
}

// A tournament in pools with a drawn series whose specialty plays to `target`.
async function drawn(target = 40) {
  const slug = nextSlug();
  await drawnTournament(call, slug, {
    tournament: { specialties: [{ specialty: "paleta_gomme_creuse", target_score: target }] },
  });
  return slug;
}

async function invalid(res, detail) {
  const body = await assertProblem(res, 422, "invalid-request");
  assert.match(body.detail ?? "", detail, `the detail names the rule: ${body.detail}`);
}

describe("a result against the target score of the series", () => {
  test("exactly one team at the target, whether it is listed first or second", async () => {
    for (const target of [15, 40]) {
      const slug = await drawn(target);
      const detail = new RegExp(
        `^score: one team reaches ${target}, the target score of the series, and the other stays from 0 to ${target - 1}$`,
      );
      // Neither team reaches it, both reach it, one goes beyond it, or a score is negative.
      for (const score of [
        [target, target],
        [target + 1, 0],
        [target - 1, target - 2],
        [0, 0],
        [target, -1],
        [-1, target],
        [target + 1, target],
      ]) {
        await invalid(await recordResult(call, slug, "A", 1, { teams: [101, 108], score }), detail);
      }
      for (const score of [[target, target], [target - 1, target - 2], [target + 1, 0]]) {
        await invalid(await recordResult(call, slug, "A", 1, { teams: [108, 101], score }), detail);
      }
      const stored = await (await call("GET", seriesPath(slug, "/pools/A/matches/1"))).json();
      assert.equal(stored.state, "scheduled", `target ${target}: no invalid result stored`);
      assert.equal(stored.score, undefined, `target ${target}: no invalid score stored`);

      const played = await recordResult(call, slug, "A", 1, { teams: [108, 101], score: [target - 1, target] });
      assert.equal(played.status, 200, `target ${target}: the second team of the body reached it`);
      const body = await played.json();
      assert.equal(body.state, "played", "a score is a played match");
      // The body lists the teams in reverse; the response lists them in match order (101 first).
      assert.deepEqual(body.score, [target, target - 1], "the score is stored in the order of the match");
    }
  });

  test("a forfeit follows the forfeiting team, whatever the order of teams in the body", async () => {
    const slug = await drawn(30);
    const first = await recordResult(call, slug, "A", 1, { teams: [108, 101], forfeited_by: 108 });
    assert.equal(first.status, 200, "108 forfeits, listed first");
    const body = await first.json();
    assert.equal(body.state, "forfeit", "a forfeit is its own state");
    assert.deepEqual(body.score, [30, 0], "the other team gets the target score, 108 gets 0");
    assert.equal(body.forfeited_by, 108, "the forfeiting team of the match");

    const corrected = await recordResult(call, slug, "A", 1, { teams: [108, 101], forfeited_by: 101 });
    assert.equal(corrected.status, 200, "the forfeit is corrected to the other team");
    const after = await corrected.json();
    assert.deepEqual(after.score, [0, 30], "the score follows the teams of the match, not the body");
    assert.equal(after.forfeited_by, 101, "101 forfeits now");
  });
});

describe("order of checks of a result", () => {
  test("401, then 404, then the body, then the status, then the score", async () => {
    const slug = await drawn();
    const path = seriesPath(slug, "/pools/A/matches/1/result");
    await assertProblem(await call("PUT", path, { body: "{" }), 401, "unauthorized");
    await assertProblem(await call("PUT", path, { token: "wrong-token", body: "{" }), 401, "unauthorized");
    await assertProblem(
      await call("PUT", "/tournaments/no-such-tournament/series/1re-serie-a/pools/A/matches/1/result", {
        ...asOrganizer,
        body: "{",
      }),
      404,
      "not-found",
    );
    await assertProblem(
      await call("PUT", `/tournaments/${slug}/series/no-such-series/pools/A/matches/1/result`, {
        ...asOrganizer,
        body: "{",
      }),
      404,
      "not-found",
    );
    await assertProblem(
      await call("PUT", seriesPath(slug, "/pools/a/matches/1/result"), { ...asOrganizer, body: "{" }),
      404,
      "not-found",
    );
    for (const number of ["0", "01", "x", "7"]) {
      await assertProblem(
        await call("PUT", seriesPath(slug, `/pools/A/matches/${number}/result`), { ...asOrganizer, body: "{" }),
        404,
        "not-found",
      );
    }
    await invalid(await call("PUT", path, { ...asOrganizer, body: "{" }), /^the body is not JSON$/);
    await invalid(
      await call("PUT", path, { ...asOrganizer, body: { teams: [101, 108], score: [0, 0] } }),
      /^score: one team reaches/,
    );
    for (const rest of ["/matches", "/pools/A/matches/1"]) {
      await assertProblem(await call("GET", seriesPath(slug, rest), { token: "wrong-token" }), 401, "unauthorized");
    }

    // In another status, a broken body still answers 422 before the 409 about the status.
    const other = await drawn();
    const close = await call("PATCH", `/tournaments/${other}`, { ...asOrganizer, body: { status: "finals" } });
    assert.equal(close.status, 200, "the tournament moved to finals");
    await close.body?.cancel();
    const finalsPath = seriesPath(other, "/pools/A/matches/1/result");
    await invalid(await call("PUT", finalsPath, { ...asOrganizer, body: { teams: [101] } }), /^teams: holds/);
    await assertProblem(
      await call("PUT", finalsPath, { ...asOrganizer, body: { teams: [101, 108], score: [0, 0] } }),
      409,
      "not-in-pools",
    );
  });
});
