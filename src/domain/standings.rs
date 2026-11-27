// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The standings of a pool: computed from its matches on every read and never stored, so that a
//! corrected score updates them.
//!
//! A played match gives the tournament's win points to the team that reached the target score,
//! and its loss points to the other. A forfeit gives the win points to the other team and the
//! forfeit points to the team that forfeited, which counts as a loss; its score, the target
//! score to 0, counts in the goal average like any other. Scheduled and postponed matches count
//! for nothing.
//!
//! Teams rank by points, then goal average (goals for minus goals against). Teams still level
//! share their rank (1, 2, 2, 4), unless the organizer ordered the pool: the tiebreak then
//! separates them, and only them, since it comes after points and goal average.

use core::cmp::Ordering;

use super::matches::{Match, MatchState, Side};
use super::pool::Pool;
use super::tournament::Points;

/// The line of a team in the standings of its pool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Standing {
    pub(crate) team: i64,
    /// Matches with a result, played or forfeit.
    pub(crate) played: i64,
    pub(crate) wins: i64,
    /// Forfeits included.
    pub(crate) losses: i64,
    /// The matches the team forfeited.
    pub(crate) forfeits: i64,
    pub(crate) points: i64,
    pub(crate) goals_for: i64,
    pub(crate) goals_against: i64,
    /// Goals for minus goals against.
    pub(crate) goal_average: i64,
    /// From 1; teams still level share it.
    pub(crate) rank: i64,
}

/// What separates two lines: points, goal average, then the organizer's order.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    points: i64,
    goal_average: i64,
    tiebreak: Option<i64>,
}

/// The standings of `pool`, from the matches of the series and the points of the tournament, by
/// rank, then team number among teams that share a rank.
pub(crate) fn standings(pool: &Pool, matches: &[Match], points: Points) -> Vec<Standing> {
    let mut lines: Vec<(Standing, Option<i64>)> = pool
        .teams
        .iter()
        .map(|team| {
            (
                Standing {
                    team: team.number,
                    ..Standing::default()
                },
                team.tiebreak,
            )
        })
        .collect();
    for game in matches.iter().filter(|game| game.pool == pool.name) {
        let (score, forfeit) = match game.state {
            MatchState::Played { score } => (score, None),
            MatchState::Forfeit { by, score } => (score, Some(by)),
            MatchState::Scheduled | MatchState::Postponed => continue,
        };
        let loser = match forfeit {
            Some(by) => Some(by),
            None => match score[0].cmp(&score[1]) {
                Ordering::Greater => Some(Side::B),
                Ordering::Less => Some(Side::A),
                Ordering::Equal => None,
            },
        };
        for side in [Side::A, Side::B] {
            let Some((line, _)) = lines
                .iter_mut()
                .find(|(line, _)| line.team == game.teams[side.index()])
            else {
                continue;
            };
            let other = 1 - side.index();
            line.played += 1;
            line.goals_for += score[side.index()];
            line.goals_against += score[other];
            match loser {
                Some(lost) if lost == side => {
                    line.losses += 1;
                    if forfeit.is_some() {
                        line.forfeits += 1;
                        line.points += points.forfeit;
                    } else {
                        line.points += points.loss;
                    }
                }
                Some(_) => {
                    line.wins += 1;
                    line.points += points.win;
                }
                None => {}
            }
        }
    }
    for (line, _) in &mut lines {
        line.goal_average = line.goals_for - line.goals_against;
    }
    let key = |(line, tiebreak): &(Standing, Option<i64>)| Key {
        points: line.points,
        goal_average: line.goal_average,
        tiebreak: *tiebreak,
    };
    lines.sort_by(|x, y| {
        let (kx, ky) = (key(x), key(y));
        ky.points
            .cmp(&kx.points)
            .then(ky.goal_average.cmp(&kx.goal_average))
            .then(kx.tiebreak.cmp(&ky.tiebreak))
            .then(x.0.team.cmp(&y.0.team))
    });
    let mut previous: Option<(Key, i64)> = None;
    lines
        .iter()
        .zip(1_i64..)
        .map(|(entry, place)| {
            let rank = match previous {
                Some((before, rank)) if before == key(entry) => rank,
                _ => place,
            };
            previous = Some((key(entry), rank));
            Standing { rank, ..entry.0 }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Standing, standings};
    use crate::domain::matches::{Match, MatchState, Side};
    use crate::domain::pool::{Pool, example_pools};
    use crate::domain::tournament::Points;

    /// Pool A: teams 101 to 104, with the matches of the circle method.
    fn pool() -> (Pool, Vec<Match>) {
        let pool = example_pools(&[&[101, 102, 103, 104]]).remove(0);
        let matches = pool.matches();
        (pool, matches)
    }

    /// Sets the state of the match between two teams, its score in the order `teams` names them.
    fn set(matches: &mut [Match], teams: [i64; 2], state: MatchState) {
        let game = matches
            .iter_mut()
            .find(|game| game.teams == teams || game.teams == [teams[1], teams[0]])
            .expect("should be a match of the pool");
        let reversed = game.teams != teams;
        game.state = match (state, reversed) {
            (MatchState::Played { score: [x, y] }, true) => MatchState::Played { score: [y, x] },
            (MatchState::Forfeit { by, score: [x, y] }, true) => MatchState::Forfeit {
                by: if by == Side::A { Side::B } else { Side::A },
                score: [y, x],
            },
            (state, _) => state,
        };
    }

    fn played(score: [i64; 2]) -> MatchState {
        MatchState::Played { score }
    }

    fn line(standings: &[Standing], team: i64) -> Standing {
        *standings
            .iter()
            .find(|line| line.team == team)
            .expect("should be a team of the pool")
    }

    fn ranks(standings: &[Standing]) -> Vec<(i64, i64)> {
        standings
            .iter()
            .map(|line| (line.team, line.rank))
            .collect()
    }

    #[test]
    fn before_any_result_every_team_shares_the_first_rank() {
        let (pool, matches) = pool();
        let table = standings(&pool, &matches, Points::default());
        assert_eq!(
            ranks(&table),
            vec![(101, 1), (102, 1), (103, 1), (104, 1)],
            "all level, by number"
        );
        assert!(
            table
                .iter()
                .all(|line| line.played == 0 && line.points == 0),
            "nothing played"
        );
    }

    #[test]
    fn points_then_goal_average_rank_the_teams() {
        let (pool, mut matches) = pool();
        set(&mut matches, [101, 102], played([40, 30]));
        set(&mut matches, [101, 103], played([40, 35]));
        set(&mut matches, [104, 101], played([40, 20]));
        set(&mut matches, [102, 103], played([40, 10]));
        set(&mut matches, [102, 104], played([40, 38]));
        set(&mut matches, [103, 104], played([40, 39]));
        let table = standings(&pool, &matches, Points::default());
        assert_eq!(
            line(&table, 101),
            Standing {
                team: 101,
                played: 3,
                wins: 2,
                losses: 1,
                forfeits: 0,
                points: 2 * 3 + 1,
                goals_for: 100,
                goals_against: 105,
                goal_average: -5,
                rank: 2,
            },
            "two wins, a loss"
        );
        assert_eq!(
            ranks(&table),
            vec![(102, 1), (101, 2), (104, 3), (103, 4)],
            "102 and 101 have 7 points, goal average +22 and -5; 104 and 103 have 5, +17 and -34"
        );
    }

    #[test]
    fn a_forfeit_counts_the_target_score_to_zero_and_the_forfeit_points() {
        let (pool, mut matches) = pool();
        let points = Points {
            win: 3,
            loss: 1,
            forfeit: 0,
        };
        set(
            &mut matches,
            [101, 102],
            MatchState::Forfeit {
                by: Side::B,
                score: [40, 0],
            },
        );
        let table = standings(&pool, &matches, points);
        assert_eq!(
            line(&table, 102),
            Standing {
                team: 102,
                played: 1,
                wins: 0,
                losses: 1,
                forfeits: 1,
                points: 0,
                goals_for: 0,
                goals_against: 40,
                goal_average: -40,
                rank: 4,
            },
            "the forfeit points and 0–40 in the goal average"
        );
        assert_eq!(
            line(&table, 101),
            Standing {
                team: 101,
                played: 1,
                wins: 1,
                points: 3,
                goals_for: 40,
                goal_average: 40,
                rank: 1,
                ..Standing::default()
            },
            "the win points and 40–0"
        );
        let generous = Points {
            win: 3,
            loss: 2,
            forfeit: 1,
        };
        assert_eq!(
            line(&standings(&pool, &matches, generous), 102).points,
            1,
            "the forfeit points of the tournament, not the loss points"
        );
    }

    #[test]
    fn a_corrected_score_updates_the_standings() {
        let (pool, mut matches) = pool();
        set(&mut matches, [101, 102], played([40, 30]));
        let before = standings(&pool, &matches, Points::default());
        assert_eq!(line(&before, 101).wins, 1, "101 won");
        set(&mut matches, [101, 102], played([38, 40]));
        let after = standings(&pool, &matches, Points::default());
        assert_eq!(
            (line(&after, 101).wins, line(&after, 101).losses),
            (0, 1),
            "the correction gives the win to 102"
        );
        assert_eq!(
            (line(&after, 102).points, line(&after, 102).goal_average),
            (3, 2),
            "and its points and goal average"
        );
        assert_eq!(
            ranks(&after),
            vec![(102, 1), (101, 2), (103, 3), (104, 3)],
            "computed again from the matches: 101 keeps its loss point"
        );
    }

    #[test]
    fn scheduled_and_postponed_matches_count_for_nothing() {
        let (pool, mut matches) = pool();
        set(&mut matches, [101, 102], MatchState::Postponed);
        assert!(
            standings(&pool, &matches, Points::default())
                .iter()
                .all(|line| line.played == 0),
            "no result yet"
        );
    }

    #[test]
    fn the_tiebreak_separates_level_teams_only() {
        let (mut pool, mut matches) = pool();
        set(&mut matches, [101, 102], played([40, 30]));
        set(&mut matches, [103, 104], played([40, 30]));
        let level = standings(&pool, &matches, Points::default());
        assert_eq!(
            ranks(&level),
            vec![(101, 1), (103, 1), (102, 3), (104, 3)],
            "same points, same goal average: shared ranks"
        );
        // The organizer's order puts 104 first and 101 last: it decides between level teams,
        // never against points or goal average.
        for (team, place) in [(104, 1), (103, 2), (102, 3), (101, 4)] {
            if let Some(member) = pool.teams.iter_mut().find(|member| member.number == team) {
                member.tiebreak = Some(place);
            }
        }
        assert_eq!(
            ranks(&standings(&pool, &matches, Points::default())),
            vec![(103, 1), (101, 2), (104, 3), (102, 4)],
            "103 before 101, 104 before 102: winners stay above losers"
        );
    }
}
