// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! A pool: teams of a series that play each other once, as stored in `pools` and `pool_teams`,
//! and the rules that change the pools of a series once drawn: a move and a tiebreak.
//!
//! A pool is named by a letter within its series, A for the first: its public key. Frontis's
//! `pool` field numbers the pools of a pool phase from 1, so pool A is Frontis's pool 1, and a
//! series plays a single pool phase, which Frontis writes as a null `poolPhase`.
//!
//! Every change of the pools happens while the tournament is `pools`, and a draw or a move only
//! before the first result of the series: matches are generated with the pools, so a move
//! generates again the matches of the two pools it touches.

use core::fmt;
use core::ops::RangeInclusive;

use super::matches::{Match, MatchState, Phase};
use super::round_robin::pairings;
use super::rule::{Invalid, invalid};
use super::status::{NotInPools, Status};

/// The most pools of a series: one per letter, A to Z.
pub(crate) const MAX_POOLS: usize = 26;

/// The sizes a move can leave: one team fewer or more than a draw allows (4 to 6), so that two
/// teams swap pools in two moves.
pub(crate) const MOVE_SIZES: RangeInclusive<usize> = 3..=7;

/// The name of a pool within its series: A to Z. It is stored as its number, 1 to 26.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PoolName(u8);

impl PoolName {
    /// The pool of an index from 0: 0 is A.
    pub(crate) fn from_index(index: usize) -> Option<Self> {
        u8::try_from(index)
            .ok()
            .filter(|index| usize::from(*index) < MAX_POOLS)
            .map(Self)
    }

    /// The pool of a stored number from 1: 1 is A.
    pub(crate) fn from_number(number: i64) -> Option<Self> {
        usize::try_from(number)
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(Self::from_index)
    }

    /// The pool of a path: one uppercase letter, A to Z. `None` for anything else, which no pool
    /// is named.
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s.as_bytes() {
            [letter @ b'A'..=b'Z'] => Some(Self(letter - b'A')),
            _ => None,
        }
    }

    /// Its stored number: 1 for A.
    pub(crate) fn number(self) -> i64 {
        i64::from(self.0) + 1
    }
}

impl fmt::Display for PoolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", char::from(b'A' + self.0))
    }
}

/// A team in its pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PoolTeam {
    pub(crate) number: i64,
    /// Its place in the seed order of the draw, from 1: it orders the teams of a pool, and keeps
    /// its value when the team moves.
    pub(crate) seed: i64,
    /// Its place in the organizer's order of the pool, which separates the teams still level on
    /// points and goal average: set for every team of the pool, or for none.
    pub(crate) tiebreak: Option<i64>,
}

/// A pool of a series, its teams in seed order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pool {
    pub(crate) name: PoolName,
    pub(crate) teams: Vec<PoolTeam>,
}

impl Pool {
    /// The matches of the pool, all scheduled: every team against every other one once, in the
    /// rounds of the circle method over the seed order, numbered from 1 within the pool.
    pub(crate) fn matches(&self) -> Vec<Match> {
        pairings(self.teams.len())
            .into_iter()
            .zip(1_i64..)
            .map(|(pairing, number)| Match {
                phase: Phase::Pool,
                pool: self.name,
                number,
                round: pairing.round,
                teams: [self.teams[pairing.a].number, self.teams[pairing.b].number],
                state: MatchState::Scheduled,
            })
            .collect()
    }

    fn has(&self, team: i64) -> bool {
        self.teams.iter().any(|member| member.number == team)
    }

    /// The organizer's order of the pool's teams, which separates the teams still level on
    /// points and goal average: every team of the pool once, or none to remove the order.
    /// Checked in this order: the status (409), then the list (422). Each team number with its
    /// place, from 1.
    pub(crate) fn tiebreak(
        &self,
        status: Status,
        order: &[i64],
    ) -> Result<Vec<(i64, i64)>, PoolError> {
        status.check_pools()?;
        let every_team_once = order.len() == self.teams.len()
            && self.teams.iter().all(|team| order.contains(&team.number));
        if !order.is_empty() && !every_team_once {
            return Err(invalid(
                "teams",
                format!(
                    "lists every team of pool {} once, in the organizer's order, or none",
                    self.name
                ),
            )
            .into());
        }
        Ok(order.iter().copied().zip(1_i64..).collect())
    }
}

/// The pools of a series, with their matches, and the revision that every draw and move adds one
/// to: a write that depends on the pools carries the revision it read, so that D1 refuses it
/// when another draw or move came in between.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SeriesPools {
    pub(crate) revision: i64,
    /// By name.
    pub(crate) pools: Vec<Pool>,
    /// By pool, then number.
    pub(crate) matches: Vec<Match>,
}

/// A move of a team to another pool, and the matches of the two pools after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Move {
    pub(crate) team: i64,
    pub(crate) from: PoolName,
    pub(crate) to: PoolName,
    /// The matches of `from` and `to` after the move, all scheduled.
    pub(crate) matches: Vec<Match>,
}

impl SeriesPools {
    pub(crate) fn pool(&self, name: PoolName) -> Option<&Pool> {
        self.pools.iter().find(|pool| pool.name == name)
    }

    /// The match of a number in a pool.
    pub(crate) fn find_match(&self, pool: PoolName, number: i64) -> Option<&Match> {
        self.matches
            .iter()
            .find(|game| game.pool == pool && game.number == number)
    }

    /// The matches with a result, played or forfeit.
    pub(crate) fn results(&self) -> usize {
        self.matches
            .iter()
            .filter(|game| game.state.is_result())
            .count()
    }

    /// Checks that the series has no result yet: once it has one, its pools are final.
    pub(crate) fn check_no_results(&self) -> Result<(), PoolError> {
        match self.results() {
            0 => Ok(()),
            results => Err(PoolError::HasResults { results }),
        }
    }

    /// Moves the team `team` to the pool `to`, checked in this order: the status (409), the team
    /// and the sizes the move leaves (422), then the results of the series (409). `None` when
    /// the team is in `to` already: nothing changes.
    pub(crate) fn move_team(
        &self,
        status: Status,
        team: i64,
        to: PoolName,
    ) -> Result<Option<Move>, PoolError> {
        status.check_pools()?;
        let from = self
            .pools
            .iter()
            .find(|pool| pool.has(team))
            .ok_or_else(|| invalid("number", format!("the series has no team {team} in a pool")))?;
        let target = self
            .pool(to)
            .ok_or_else(|| invalid("pool", format!("the series has no pool {to}")))?;
        if from.name == to {
            self.check_no_results()?;
            return Ok(None);
        }
        let mut source = from.clone();
        let mut destination = target.clone();
        let moved = source
            .teams
            .iter()
            .position(|member| member.number == team)
            .map(|index| source.teams.remove(index))
            .ok_or_else(|| invalid("number", format!("the series has no team {team} in a pool")))?;
        destination.teams.push(moved);
        destination.teams.sort_by_key(|member| member.seed);
        for pool in [&source, &destination] {
            if !MOVE_SIZES.contains(&pool.teams.len()) {
                return Err(invalid(
                    "pool",
                    format!(
                        "the move leaves {} teams in pool {}: a move leaves {} to {} teams in each \
                         pool",
                        pool.teams.len(),
                        pool.name,
                        MOVE_SIZES.start(),
                        MOVE_SIZES.end()
                    ),
                )
                .into());
            }
        }
        self.check_no_results()?;
        let mut matches = source.matches();
        matches.extend(destination.matches());
        Ok(Some(Move {
            team,
            from: from.name,
            to,
            matches,
        }))
    }
}

/// A change of the pools or a result that the tournament or the series forbid, or that breaks a
/// rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PoolError {
    /// The tournament is not in its pools.
    NotInPools(NotInPools),
    /// The request breaks a rule.
    Invalid(Invalid),
    /// A draw or a move once the series has results.
    HasResults { results: usize },
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInPools(error) => error.fmt(f),
            Self::Invalid(error) => error.fmt(f),
            Self::HasResults { results } => write!(
                f,
                "matches: the series has {results} result{}: its pools no longer change",
                if *results == 1 { "" } else { "s" }
            ),
        }
    }
}

impl std::error::Error for PoolError {}

impl From<NotInPools> for PoolError {
    fn from(error: NotInPools) -> Self {
        Self::NotInPools(error)
    }
}

impl From<Invalid> for PoolError {
    fn from(error: Invalid) -> Self {
        Self::Invalid(error)
    }
}

/// Pools of the given team numbers, seeded in that order from 1, for the tests of `domain`.
#[cfg(test)]
pub(crate) fn example_pools(pools: &[&[i64]]) -> Vec<Pool> {
    let mut seed = 0;
    pools
        .iter()
        .enumerate()
        .map(|(index, numbers)| Pool {
            name: PoolName::from_index(index).expect("should be a pool name"),
            teams: numbers
                .iter()
                .map(|&number| {
                    seed += 1;
                    PoolTeam {
                        number,
                        seed,
                        tiebreak: None,
                    }
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{MAX_POOLS, PoolError, PoolName, SeriesPools, example_pools};
    use crate::domain::matches::{MatchState, Side};
    use crate::domain::rule::Invalid;
    use crate::domain::status::{NotInPools, Status};

    fn name(letter: &str) -> PoolName {
        PoolName::parse(letter).expect("should be a pool name")
    }

    /// Pools A (101 to 105) and B (106 to 110), with their matches.
    fn drawn() -> SeriesPools {
        let pools = example_pools(&[&[101, 102, 103, 104, 105], &[106, 107, 108, 109, 110]]);
        let matches = pools.iter().flat_map(super::Pool::matches).collect();
        SeriesPools {
            revision: 1,
            pools,
            matches,
        }
    }

    fn field_of<T>(result: &Result<T, PoolError>) -> Option<&'static str> {
        match result {
            Err(PoolError::Invalid(Invalid { field, .. })) => Some(*field),
            _ => None,
        }
    }

    #[test]
    fn pools_are_named_by_a_letter_and_stored_by_a_number_from_1() {
        let a = PoolName::from_index(0).expect("should be pool A");
        assert_eq!(a.to_string(), "A", "the first pool");
        assert_eq!(a.number(), 1, "is Frontis's pool 1");
        let z = PoolName::from_number(26).expect("should be pool Z");
        assert_eq!(z.to_string(), "Z", "the last pool");
        assert_eq!(PoolName::parse("Z"), Some(z), "a path letter");
        assert_eq!(PoolName::from_index(MAX_POOLS), None, "no 27th pool");
        for number in [0, 27, -1] {
            assert_eq!(PoolName::from_number(number), None, "no pool {number}");
        }
        for path in ["a", "AA", "", "1", "É"] {
            assert_eq!(PoolName::parse(path), None, "{path:?} names no pool");
        }
    }

    #[test]
    fn a_pool_generates_every_match_of_its_teams_in_seed_order() {
        let pools = example_pools(&[&[104, 101, 103, 102]]);
        let matches = pools[0].matches();
        assert_eq!(matches.len(), 6, "every pair of four teams");
        assert_eq!(
            matches.iter().map(|game| game.number).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6],
            "numbered from 1 within the pool"
        );
        assert_eq!(
            matches[0].teams,
            [104, 102],
            "the first seed meets the last one in round 1"
        );
        assert!(
            matches
                .iter()
                .all(|game| game.pool == name("A") && game.state == MatchState::Scheduled),
            "scheduled matches of pool A"
        );
    }

    #[test]
    fn a_move_regenerates_the_matches_of_both_pools() {
        let moved = drawn()
            .move_team(Status::Pools, 103, name("B"))
            .expect("should move")
            .expect("should change the pools");
        assert_eq!((moved.from, moved.to), (name("A"), name("B")), "A to B");
        let pairs = |pool: PoolName| -> BTreeSet<[i64; 2]> {
            moved
                .matches
                .iter()
                .filter(|game| game.pool == pool)
                .map(|game| {
                    let [a, b] = game.teams;
                    [a.min(b), a.max(b)]
                })
                .collect()
        };
        let all_pairs = |teams: &[i64]| -> BTreeSet<[i64; 2]> {
            teams
                .iter()
                .flat_map(|a| teams.iter().filter(move |b| a < *b).map(move |b| [*a, *b]))
                .collect()
        };
        assert_eq!(
            pairs(name("A")),
            all_pairs(&[101, 102, 104, 105]),
            "pool A without 103"
        );
        assert_eq!(
            pairs(name("B")),
            all_pairs(&[103, 106, 107, 108, 109, 110]),
            "pool B with 103"
        );
        assert_eq!(moved.matches.len(), 6 + 15, "4 and 6 teams");
    }

    #[test]
    fn a_move_to_the_pool_of_the_team_changes_nothing() {
        assert_eq!(
            drawn().move_team(Status::Pools, 103, name("A")),
            Ok(None),
            "103 is in A already"
        );
    }

    #[test]
    fn a_move_leaves_3_to_7_teams_in_each_pool() {
        let pools = example_pools(&[&[101, 102, 103], &[104, 105, 106, 107, 108, 109, 110]]);
        let matches = pools.iter().flat_map(super::Pool::matches).collect();
        let pools = SeriesPools {
            revision: 3,
            pools,
            matches,
        };
        assert_eq!(
            pools
                .move_team(Status::Pools, 101, name("B"))
                .map_err(|e| e.to_string()),
            Err(
                "pool: the move leaves 2 teams in pool A: a move leaves 3 to 7 teams in each pool"
                    .to_owned()
            ),
            "pool A too small and pool B too large"
        );
        assert!(
            pools
                .move_team(Status::Pools, 104, name("A"))
                .is_ok_and(|moved| moved.is_some()),
            "4 and 6 teams"
        );
    }

    #[test]
    fn a_move_checks_the_status_then_the_team_then_the_results() {
        let mut pools = drawn();
        pools.matches[0].state = MatchState::Forfeit {
            by: Side::A,
            score: [0, 40],
        };
        assert_eq!(
            pools.move_team(Status::Finals, 999, name("B")),
            Err(PoolError::NotInPools(NotInPools(Status::Finals))),
            "the status first"
        );
        assert_eq!(
            field_of(&pools.move_team(Status::Pools, 999, name("B"))),
            Some("number"),
            "then a team that no pool holds"
        );
        assert_eq!(
            pools
                .move_team(Status::Pools, 101, name("B"))
                .map_err(|e| e.to_string()),
            Err("matches: the series has 1 result: its pools no longer change".to_owned()),
            "then the results"
        );
        assert_eq!(
            pools.move_team(Status::Pools, 101, name("A")),
            Err(PoolError::HasResults { results: 1 }),
            "even to its own pool"
        );
    }

    #[test]
    fn a_tiebreak_lists_every_team_of_the_pool_once_or_none() {
        let pools = drawn();
        let pool = &pools.pools[0];
        assert_eq!(
            pool.tiebreak(Status::Pools, &[105, 101, 103, 102, 104]),
            Ok(vec![(105, 1), (101, 2), (103, 3), (102, 4), (104, 5)]),
            "a place from 1 for each team"
        );
        assert_eq!(
            pool.tiebreak(Status::Pools, &[]),
            Ok(Vec::new()),
            "none removes the order"
        );
        for order in [
            &[101, 102, 103, 104][..],
            &[101, 102, 103, 104, 104],
            &[101, 102, 103, 104, 106],
            &[101, 102, 103, 104, 105, 106],
        ] {
            assert_eq!(
                pool.tiebreak(Status::Pools, order)
                    .map_err(|e| e.to_string()),
                Err(
                    "teams: lists every team of pool A once, in the organizer's order, or none"
                        .to_owned()
                ),
                "{order:?}"
            );
        }
        assert_eq!(
            pool.tiebreak(Status::Registration, &[1]),
            Err(PoolError::NotInPools(NotInPools(Status::Registration))),
            "the status first"
        );
    }
}
