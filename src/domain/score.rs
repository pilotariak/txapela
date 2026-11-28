// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The result of a match as the organizer records it, and the rule of a valid score.
//!
//! A score is valid when one team reaches exactly the target score of the series' specialty and
//! the other stays below it, from 0. A forfeit is a state of the match: the team that forfeits
//! gets 0 and the other team the target score, such as 40–0, which counts in the goal average;
//! never 0–0. Recording a result again corrects it. The result names the two teams of the match,
//! so that it never lands on another pair.

use super::matches::{Match, MatchState, Side};
use super::pool::PoolError;
use super::rule::{Invalid, invalid};
use super::status::Status;

/// What the organizer records for a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entered {
    /// The scores of the two teams, in the order the entry lists the teams.
    Score([i64; 2]),
    /// The number of the team that forfeits.
    ForfeitedBy(i64),
}

/// A result as the organizer enters it: the numbers of the two teams of the match, in either
/// order, and what happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResultEntry {
    pub(crate) teams: [i64; 2],
    pub(crate) entered: Entered,
}

/// Checks a score against the target score of the series: one team has exactly `target`, the
/// other from 0 to `target - 1`.
pub(crate) fn check_score(score: [i64; 2], target: i64) -> Result<(), Invalid> {
    let below = 0..target;
    let [first, second] = score;
    if (first == target && below.contains(&second)) || (second == target && below.contains(&first))
    {
        Ok(())
    } else {
        Err(invalid(
            "score",
            format!(
                "one team reaches {target}, the target score of the series, and the other stays \
                 from 0 to {}",
                target - 1
            ),
        ))
    }
}

/// The state of `game` with the result of `entry`, checked in this order: the status (409), the
/// teams, then the score (422). `target` is the target score of the series.
pub(crate) fn record(
    status: Status,
    game: &Match,
    entry: ResultEntry,
    target: i64,
) -> Result<MatchState, PoolError> {
    status.check_pools()?;
    let [a, b] = game.teams;
    let reversed = match entry.teams {
        teams if teams == [a, b] => false,
        teams if teams == [b, a] => true,
        _ => {
            return Err(invalid(
                "teams",
                format!(
                    "match {} of pool {} is team {a} against team {b}",
                    game.number, game.pool
                ),
            )
            .into());
        }
    };
    match entry.entered {
        Entered::Score([first, second]) => {
            let score = if reversed {
                [second, first]
            } else {
                [first, second]
            };
            check_score(score, target)?;
            Ok(MatchState::Played { score })
        }
        Entered::ForfeitedBy(team) => {
            let by = if team == a {
                Side::A
            } else if team == b {
                Side::B
            } else {
                return Err(invalid(
                    "forfeited_by",
                    format!("is team {a} or team {b}, the teams of the match"),
                )
                .into());
            };
            let score = match by {
                Side::A => [0, target],
                Side::B => [target, 0],
            };
            Ok(MatchState::Forfeit { by, score })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Entered, ResultEntry, check_score, record};
    use crate::domain::matches::{Match, MatchState, Phase, Side};
    use crate::domain::pool::{PoolError, PoolName};
    use crate::domain::status::{NotInPools, Status};

    /// Match 3 of pool B: team 104 against team 102.
    fn game() -> Match {
        Match {
            phase: Phase::Pool,
            pool: PoolName::parse("B").expect("should be a pool name"),
            number: 3,
            round: 2,
            teams: [104, 102],
            state: MatchState::Scheduled,
        }
    }

    fn played(teams: [i64; 2], score: [i64; 2]) -> ResultEntry {
        ResultEntry {
            teams,
            entered: Entered::Score(score),
        }
    }

    #[test]
    fn a_valid_score_has_exactly_one_team_at_the_target() {
        for target in [15, 30, 35, 40] {
            for score in [
                [target, 0],
                [0, target],
                [target, target - 1],
                [target - 1, target],
                [target, target / 2],
            ] {
                assert_eq!(check_score(score, target), Ok(()), "{score:?} to {target}");
            }
            for score in [
                [target, target],
                [target + 1, 0],
                [target - 1, target - 2],
                [0, 0],
                [target, -1],
                [-1, target],
                [target + 1, target],
            ] {
                assert_eq!(
                    check_score(score, target).map_err(|e| e.to_string()),
                    Err(format!(
                        "score: one team reaches {target}, the target score of the series, and \
                         the other stays from 0 to {}",
                        target - 1
                    )),
                    "{score:?} to {target}"
                );
            }
        }
    }

    #[test]
    fn a_target_of_one_takes_one_to_zero_only() {
        assert_eq!(check_score([1, 0], 1), Ok(()), "1–0");
        assert!(check_score([1, 1], 1).is_err(), "1–1");
    }

    #[test]
    fn a_score_follows_the_order_the_entry_lists_the_teams() {
        assert_eq!(
            record(Status::Pools, &game(), played([104, 102], [40, 31]), 40),
            Ok(MatchState::Played { score: [40, 31] }),
            "the order of the match"
        );
        assert_eq!(
            record(Status::Pools, &game(), played([102, 104], [40, 31]), 40),
            Ok(MatchState::Played { score: [31, 40] }),
            "the reverse order: 102 reached 40"
        );
    }

    #[test]
    fn a_forfeit_gives_the_target_score_to_the_other_team() {
        for (team, by, score) in [(104, Side::A, [0, 30]), (102, Side::B, [30, 0])] {
            assert_eq!(
                record(
                    Status::Pools,
                    &game(),
                    ResultEntry {
                        teams: [102, 104],
                        entered: Entered::ForfeitedBy(team),
                    },
                    30
                ),
                Ok(MatchState::Forfeit { by, score }),
                "{team} forfeits: target to 0, never 0–0"
            );
        }
        assert_eq!(
            record(
                Status::Pools,
                &game(),
                ResultEntry {
                    teams: [102, 104],
                    entered: Entered::ForfeitedBy(101),
                },
                30
            )
            .map_err(|e| e.to_string()),
            Err("forfeited_by: is team 104 or team 102, the teams of the match".to_owned()),
            "a team of another match"
        );
    }

    #[test]
    fn a_result_names_the_teams_of_the_match() {
        for teams in [[104, 101], [102, 102], [101, 103]] {
            assert_eq!(
                record(Status::Pools, &game(), played(teams, [40, 31]), 40)
                    .map_err(|e| e.to_string()),
                Err("teams: match 3 of pool B is team 104 against team 102".to_owned()),
                "{teams:?}"
            );
        }
    }

    #[test]
    fn a_result_checks_the_status_then_the_teams_then_the_score() {
        for status in [Status::Registration, Status::Finals, Status::Finished] {
            assert_eq!(
                record(status, &game(), played([1, 2], [0, 0]), 40),
                Err(PoolError::NotInPools(NotInPools(status))),
                "{status:?}: results are recorded and corrected in pools only"
            );
        }
        assert!(
            record(Status::Pools, &game(), played([1, 2], [0, 0]), 40)
                .is_err_and(|e| e.to_string().starts_with("teams:")),
            "then the teams"
        );
        assert!(
            record(Status::Pools, &game(), played([104, 102], [0, 0]), 40)
                .is_err_and(|e| e.to_string().starts_with("score:")),
            "then the score"
        );
    }
}
