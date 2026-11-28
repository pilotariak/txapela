// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

// Extra standings tests through the Worker: shared ranks, the tournament's points, corrections of
// forfeits, and a draw clearing the tiebreak. Players and clubs are fictional.

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import {
  ORGANIZER_TOKEN,
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
  return `standings-points-${lastSlug}`;
}

async function drawn(tournament = {}) {
  const slug = nextSlug();
  await drawnTournament(call, slug, { tournament });
  return slug;
}

async function record(slug, number, body) {
  const res = await recordResult(call, slug, "A", number, body);
  assert.equal(res.status, 200, `A${number} recorded`);
  await res.body?.cancel();
}

async function standingsOf(slug) {
  const res = await call("GET", seriesPath(slug, "/standings"));
  assert.equal(res.status, 200);
  return (await res.json()).pools.find((pool) => pool.pool === "A").standings;
}

// Each line as [team, rank, played, wins, losses, forfeits, points, for, against, average].
function lines(standings) {
  return standings.map((line) => [
    line.team.number,
    line.rank,
    line.played,
    line.wins,
    line.losses,
    line.forfeits,
    line.points,
    line.goals_for,
    line.goals_against,
    line.goal_average,
  ]);
}

function ranks(standings) {
  return standings.map((line) => [line.team.number, line.rank]);
}

function tiebreak(slug, teams) {
  return call("PUT", seriesPath(slug, "/pools/A/tiebreak"), { ...asOrganizer, body: { teams } });
}

describe("the ranks of the standings", () => {
  test("teams level on points and goal average share their rank: 1, 2, 2, 4", async () => {
    const slug = await drawn();
    await record(slug, 1, { teams: [101, 108], score: [40, 30] });
    await record(slug, 2, { teams: [104, 105], score: [40, 30] });
    await record(slug, 5, { teams: [101, 104], score: [40, 30] });
    await record(slug, 6, { teams: [105, 108], score: [30, 40] });
    assert.deepEqual(
      lines(await standingsOf(slug)),
      [
        [101, 1, 2, 2, 0, 0, 6, 80, 60, 20],
        [104, 2, 2, 1, 1, 0, 4, 70, 70, 0],
        [108, 2, 2, 1, 1, 0, 4, 70, 70, 0],
        [105, 4, 2, 0, 2, 0, 2, 60, 80, -20],
      ],
      "104 and 108 have the same points and goal average, so both are second; 105 is fourth",
    );
  });

  test("the points of the tournament give wins, losses and forfeits their value", async () => {
    const slug = await drawn({ points: { win: 5, loss: 2, forfeit: 1 } });
    await record(slug, 1, { teams: [101, 108], score: [40, 30] });
    await record(slug, 2, { teams: [104, 105], forfeited_by: 105 });
    assert.deepEqual(
      lines(await standingsOf(slug)),
      [
        [104, 1, 1, 1, 0, 0, 5, 40, 0, 40],
        [101, 2, 1, 1, 0, 0, 5, 40, 30, 10],
        [108, 3, 1, 0, 1, 0, 2, 30, 40, -10],
        [105, 4, 1, 0, 1, 1, 1, 0, 40, -40],
      ],
      "a forfeit gives the forfeit point, not the loss point, and 0–40 counts in the average",
    );
  });

  test("a corrected forfeit updates the points, the forfeits and the ranks again", async () => {
    const slug = await drawn();
    await record(slug, 1, { teams: [101, 108], score: [40, 30] });
    await record(slug, 1, { teams: [108, 101], forfeited_by: 101 });
    assert.deepEqual(
      ranks(await standingsOf(slug)),
      [[108, 1], [104, 2], [105, 2], [101, 4]],
      "101 forfeited: it falls behind the teams that did not play",
    );
    const forfeited = (await standingsOf(slug)).find((line) => line.team.number === 101);
    assert.deepEqual(
      [forfeited.forfeits, forfeited.losses, forfeited.points, forfeited.goal_average],
      [1, 1, 0, -40],
      "the forfeit counts as a loss with the forfeit points and a 0–40 score",
    );

    await record(slug, 1, { teams: [101, 108], score: [38, 40] });
    assert.deepEqual(
      ranks(await standingsOf(slug)),
      [[108, 1], [101, 2], [104, 3], [105, 3]],
      "recorded again as a played score: 101 keeps the loss points and takes second",
    );
  });
});

describe("a draw clears the manual order", () => {
  test("the new draw replaces the pools, their matches and their tiebreak", async () => {
    const slug = await drawn();
    const ordered = await tiebreak(slug, [108, 105, 104, 101]);
    assert.equal(ordered.status, 200, "the organizer orders pool A");
    assert.deepEqual(
      ranks((await ordered.json()).pools[0].standings),
      [[108, 1], [105, 2], [104, 3], [101, 4]],
      "all level: the order decides",
    );

    const again = await call("POST", seriesPath(slug, "/draw"), {
      ...asOrganizer,
      body: { pool_size: 4, seeds: [108, 107] },
    });
    assert.equal(again.status, 201, "no result yet, so the series is drawn again");
    await again.body?.cancel();
    assert.deepEqual(
      ranks(await standingsOf(slug)),
      [[102, 1], [103, 1], [106, 1], [108, 1]],
      "the tiebreak went with the old pools: every team of A is level again, by number",
    );
    const { matches } = await (await call("GET", seriesPath(slug, "/matches"))).json();
    assert.equal(matches.length, 12, "the new pools have their own round robins");
  });
});
