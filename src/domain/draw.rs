// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The draw of a series: its registered teams split into pools of 4 to 6, in the snake order of
//! their seeds, away from the teams of their club when the draw can.
//!
//! The seed order is the order of the organizer's list, then the other registered teams by
//! number. The teams go in pots of one team per pool, in seed order: the first pot fills the
//! pools A, B, C…, the next one C, B, A…, and so on (the snake), the last pot maybe short. Within
//! a pot, a team that would join a pool holding a team of its club takes another pool of the
//! pot: the draw finds the most teams of the pot it can keep away from their clubs (a maximum
//! matching between the teams and the pools), each team trying its snake pool first. Teams that
//! no pool can keep away from their club take the pools left, in snake order. A team's clubs are
//! the clubs of its players, compared without case: two teams share a club when one of their
//! players does, and a team without a club shares none.
//!
//! The draw is the same for the same teams and request: it reads no clock and no randomness.

use core::ops::{Range, RangeInclusive};

use super::pool::{MAX_POOLS, Pool, PoolError, PoolName, PoolTeam};
use super::rule::{Invalid, invalid};
use super::status::Status;

/// The sizes of the pools of a draw.
pub(crate) const POOL_SIZES: RangeInclusive<usize> = 4..=6;

/// How the organizer splits the teams: the size of the largest pool, or the number of pools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawSize {
    /// 4, 5 or 6: as few pools as this size allows, as even as can be.
    PoolSize(i64),
    /// This many pools, as even as can be.
    Pools(i64),
}

/// A draw as the organizer asks for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DrawRequest {
    pub(crate) size: DrawSize,
    /// The team numbers of the first seeds, in order: the other teams follow by number.
    pub(crate) seeds: Vec<i64>,
}

/// A registered team of the series and its clubs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entrant {
    pub(crate) number: i64,
    /// Trimmed, lowercase, without blanks or duplicates.
    clubs: Vec<String>,
}

impl Entrant {
    /// A team and the clubs of its players, absent or blank ones left out.
    pub(crate) fn new<'a>(number: i64, clubs: impl IntoIterator<Item = &'a str>) -> Self {
        let mut keys: Vec<String> = clubs
            .into_iter()
            .map(|club| club.trim().to_lowercase())
            .filter(|club| !club.is_empty())
            .collect();
        keys.sort();
        keys.dedup();
        Self {
            number,
            clubs: keys,
        }
    }

    fn shares_a_club(&self, other: &Self) -> bool {
        self.clubs.iter().any(|club| other.clubs.contains(club))
    }
}

/// `1 team`, `2 teams`.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Draws the pools of a series from its registered teams, checked in this order: the status
/// (409), the seeds and the sizes (422), then the results of the series (409), which a draw
/// replaces only while there are none.
pub(crate) fn draw(
    status: Status,
    request: &DrawRequest,
    entrants: Vec<Entrant>,
    results: usize,
) -> Result<Vec<Pool>, PoolError> {
    status.check_pools()?;
    let seeded = seed_order(entrants, &request.seeds)?;
    let pools = pool_count(seeded.len(), request.size)?;
    if results > 0 {
        return Err(PoolError::HasResults { results });
    }
    let seeds: Vec<i64> = (1_i64..).take(seeded.len()).collect();
    snake(&seeded, pools)
        .into_iter()
        .enumerate()
        .map(|(index, members)| {
            let name = PoolName::from_index(index)
                .ok_or_else(|| invalid("pools", format!("is at most {MAX_POOLS}")))?;
            Ok(Pool {
                name,
                teams: members
                    .into_iter()
                    .map(|member| PoolTeam {
                        number: seeded[member].number,
                        seed: seeds[member],
                        tiebreak: None,
                    })
                    .collect(),
            })
        })
        .collect()
}

/// The teams in seed order: the numbers of `seeds` first, then the others by number. Each seed
/// is a registered team of the series, listed once.
fn seed_order(mut entrants: Vec<Entrant>, seeds: &[i64]) -> Result<Vec<Entrant>, Invalid> {
    entrants.sort_by_key(|entrant| entrant.number);
    let mut seeded = Vec::with_capacity(entrants.len());
    for &number in seeds {
        if seeded
            .iter()
            .any(|entrant: &Entrant| entrant.number == number)
        {
            return Err(invalid("seeds", format!("{number} appears twice")));
        }
        let index = entrants
            .iter()
            .position(|entrant| entrant.number == number)
            .ok_or_else(|| {
                invalid(
                    "seeds",
                    format!("{number} is not a registered team of the series"),
                )
            })?;
        seeded.push(entrants.remove(index));
    }
    seeded.extend(entrants);
    Ok(seeded)
}

/// The number of pools for `teams` teams: every pool holds 4 to 6 teams, and pools differ by one
/// team at most.
fn pool_count(teams: usize, size: DrawSize) -> Result<usize, Invalid> {
    if teams < *POOL_SIZES.start() {
        return Err(invalid(
            "teams",
            format!(
                "the series has {} registered: a draw needs {} at least",
                count(teams, "team"),
                POOL_SIZES.start()
            ),
        ));
    }
    let (field, pools) = match size {
        DrawSize::PoolSize(size) => {
            let size = usize::try_from(size)
                .ok()
                .filter(|size| POOL_SIZES.contains(size))
                .ok_or_else(|| invalid("pool_size", "is 4, 5 or 6"))?;
            ("pool_size", teams.div_ceil(size))
        }
        DrawSize::Pools(pools) => {
            let pools = usize::try_from(pools)
                .ok()
                .filter(|pools| (1..=MAX_POOLS).contains(pools))
                .ok_or_else(|| invalid("pools", format!("is an integer from 1 to {MAX_POOLS}")))?;
            ("pools", pools)
        }
    };
    let (smallest, largest) = (teams / pools, teams.div_ceil(pools));
    if !POOL_SIZES.contains(&smallest) || !POOL_SIZES.contains(&largest) {
        let sizes = if smallest == largest {
            count(smallest, "team")
        } else {
            format!("{smallest} to {largest} teams")
        };
        return Err(invalid(
            field,
            format!(
                "{} in {} make pools of {sizes}: a pool holds {} to {} teams",
                count(teams, "team"),
                count(pools, "pool"),
                POOL_SIZES.start(),
                POOL_SIZES.end()
            ),
        ));
    }
    Ok(pools)
}

/// The members of each pool, as indexes into `seeded`, in seed order: one team of each pot per
/// pool, in the snake order, moved within its pot away from the teams of its club when it can.
fn snake(seeded: &[Entrant], pools: usize) -> Vec<Vec<usize>> {
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); pools];
    let forward: Vec<usize> = (0..pools).collect();
    let backward: Vec<usize> = (0..pools).rev().collect();
    for (pot, start) in (0..seeded.len()).step_by(pools).enumerate() {
        let order = if pot % 2 == 0 { &forward } else { &backward };
        let end = (start + pools).min(seeded.len());
        let assigned = assign_pot(seeded, &members, start..end, order);
        for (team, pool) in (start..end).zip(assigned) {
            members[pool].push(team);
        }
    }
    members
}

/// The pool of each team of a pot, in seed order. `order` lists the pools in the snake direction
/// of the pot, and the `i`-th team of the pot prefers `order[i]`.
fn assign_pot(
    seeded: &[Entrant],
    members: &[Vec<usize>],
    pot: Range<usize>,
    order: &[usize],
) -> Vec<usize> {
    let teams: Vec<usize> = pot.collect();
    let preference = |position: usize| {
        let preferred = order[position];
        core::iter::once(preferred).chain(order.iter().copied().filter(move |&p| p != preferred))
    };
    // The pools that keep each team away from its clubs, its snake pool first.
    let choices: Vec<Vec<usize>> = teams
        .iter()
        .enumerate()
        .map(|(position, &team)| {
            preference(position)
                .filter(|&pool| {
                    !members[pool]
                        .iter()
                        .any(|&other| seeded[other].shares_a_club(&seeded[team]))
                })
                .collect()
        })
        .collect();
    let mut holder: Vec<Option<usize>> = vec![None; members.len()];
    for position in 0..teams.len() {
        let mut visited = vec![false; members.len()];
        augment(position, &choices, &mut holder, &mut visited);
    }
    let mut assigned: Vec<Option<usize>> = vec![None; teams.len()];
    for (pool, position) in holder.iter().enumerate() {
        if let Some(position) = position {
            assigned[*position] = Some(pool);
        }
    }
    // The teams that no pool keeps away from their clubs take the pools left.
    for (position, slot) in assigned.iter_mut().enumerate() {
        if slot.is_none()
            && let Some(pool) = preference(position).find(|&pool| holder[pool].is_none())
        {
            holder[pool] = Some(position);
            *slot = Some(pool);
        }
    }
    assigned
        .into_iter()
        .zip(order)
        .map(|(pool, &snake_pool)| pool.unwrap_or(snake_pool))
        .collect()
}

/// Kuhn's augmenting path: gives the team at `position` one of its choices, taking it from the
/// team that holds it when that team can take another of its own.
fn augment(
    position: usize,
    choices: &[Vec<usize>],
    holder: &mut [Option<usize>],
    visited: &mut [bool],
) -> bool {
    for &pool in &choices[position] {
        if visited[pool] {
            continue;
        }
        visited[pool] = true;
        let current = holder[pool];
        if current.is_none_or(|other| augment(other, choices, holder, visited)) {
            holder[pool] = Some(position);
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{DrawRequest, DrawSize, Entrant, POOL_SIZES, draw};
    use crate::domain::pool::{Pool, PoolError};
    use crate::domain::rule::Invalid;
    use crate::domain::status::{NotInPools, Status};

    /// Teams 101, 102… without a club.
    fn entrants(teams: i64) -> Vec<Entrant> {
        (101..101 + teams)
            .map(|number| Entrant::new(number, []))
            .collect()
    }

    fn by_size(size: i64) -> DrawRequest {
        DrawRequest {
            size: DrawSize::PoolSize(size),
            seeds: Vec::new(),
        }
    }

    /// The seeds of each pool.
    fn seeds(pools: &[Pool]) -> Vec<Vec<i64>> {
        pools
            .iter()
            .map(|pool| pool.teams.iter().map(|team| team.seed).collect())
            .collect()
    }

    fn numbers(pools: &[Pool]) -> Vec<Vec<i64>> {
        pools
            .iter()
            .map(|pool| pool.teams.iter().map(|team| team.number).collect())
            .collect()
    }

    fn drawn(request: &DrawRequest, entrants: Vec<Entrant>) -> Vec<Pool> {
        draw(Status::Pools, request, entrants, 0).expect("should draw")
    }

    fn detail(result: Result<Vec<Pool>, PoolError>) -> String {
        result
            .map(|_| ())
            .map_err(|e| e.to_string())
            .err()
            .unwrap_or_default()
    }

    #[test]
    fn teams_go_in_snake_order_of_their_seeds() {
        assert_eq!(
            seeds(&drawn(&by_size(4), entrants(8))),
            vec![vec![1, 4, 5, 8], vec![2, 3, 6, 7]],
            "8 teams in pools of 4: A, B then B, A"
        );
        assert_eq!(
            seeds(&drawn(&by_size(4), entrants(12))),
            vec![vec![1, 6, 7, 12], vec![2, 5, 8, 11], vec![3, 4, 9, 10]],
            "12 teams in 3 pools"
        );
        let pools = drawn(&by_size(5), entrants(13));
        assert_eq!(
            seeds(&pools),
            vec![vec![1, 6, 7, 12, 13], vec![2, 5, 8, 11], vec![3, 4, 9, 10]],
            "13 teams in pools of 5: the short last pot fills A first"
        );
        assert_eq!(
            pools
                .iter()
                .map(|pool| pool.name.to_string())
                .collect::<Vec<_>>(),
            vec!["A", "B", "C"],
            "pools are named A, B, C"
        );
        assert!(
            pools
                .iter()
                .flat_map(|pool| &pool.teams)
                .all(|team| team.tiebreak.is_none()),
            "no tiebreak after a draw"
        );
    }

    #[test]
    fn the_seed_order_is_the_list_of_the_organizer_then_the_team_numbers() {
        let request = DrawRequest {
            size: DrawSize::Pools(2),
            seeds: vec![105, 103],
        };
        let mut shuffled = entrants(8);
        shuffled.reverse();
        assert_eq!(
            numbers(&drawn(&request, shuffled)),
            vec![vec![105, 102, 104, 108], vec![103, 101, 106, 107]],
            "seeds 105, 103, then 101, 102, 104, 106, 107, 108"
        );
        for (seeds, message) in [
            (vec![105, 105], "seeds: 105 appears twice"),
            (
                vec![109],
                "seeds: 109 is not a registered team of the series",
            ),
        ] {
            assert_eq!(
                detail(draw(
                    Status::Pools,
                    &DrawRequest {
                        size: DrawSize::Pools(2),
                        seeds,
                    },
                    entrants(8),
                    0
                )),
                message,
                "{message}"
            );
        }
    }

    #[test]
    fn the_pool_size_or_the_number_of_pools_keeps_every_pool_at_4_to_6_teams() {
        assert_eq!(
            seeds(&drawn(&by_size(6), entrants(10)))
                .iter()
                .map(Vec::len)
                .collect::<Vec<_>>(),
            vec![5, 5],
            "10 teams in pools of 6 at most: 2 pools of 5"
        );
        assert_eq!(
            seeds(&drawn(
                &DrawRequest {
                    size: DrawSize::Pools(3),
                    seeds: Vec::new()
                },
                entrants(14)
            ))
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
            vec![5, 5, 4],
            "14 teams in 3 pools"
        );
        for (request, teams, message) in [
            (
                by_size(6),
                7,
                "pool_size: 7 teams in 2 pools make pools of 3 to 4 teams: a pool holds 4 to 6 teams",
            ),
            (
                DrawRequest {
                    size: DrawSize::Pools(1),
                    seeds: Vec::new(),
                },
                7,
                "pools: 7 teams in 1 pool make pools of 7 teams: a pool holds 4 to 6 teams",
            ),
            (by_size(3), 9, "pool_size: is 4, 5 or 6"),
            (by_size(7), 14, "pool_size: is 4, 5 or 6"),
            (
                DrawRequest {
                    size: DrawSize::Pools(0),
                    seeds: Vec::new(),
                },
                8,
                "pools: is an integer from 1 to 26",
            ),
            (
                by_size(4),
                3,
                "teams: the series has 3 teams registered: a draw needs 4 at least",
            ),
            (
                by_size(4),
                1,
                "teams: the series has 1 team registered: a draw needs 4 at least",
            ),
        ] {
            assert_eq!(
                detail(draw(Status::Pools, &request, entrants(teams), 0)),
                message,
                "{teams} teams"
            );
        }
    }

    #[test]
    fn a_draw_checks_the_status_then_the_rules_then_the_results() {
        assert_eq!(
            draw(Status::Registration, &by_size(3), entrants(2), 4),
            Err(PoolError::NotInPools(NotInPools(Status::Registration))),
            "the status first: registration is still open"
        );
        assert!(
            matches!(
                draw(Status::Pools, &by_size(3), entrants(8), 4),
                Err(PoolError::Invalid(Invalid {
                    field: "pool_size",
                    ..
                }))
            ),
            "then the rules"
        );
        assert_eq!(
            draw(Status::Pools, &by_size(4), entrants(8), 4),
            Err(PoolError::HasResults { results: 4 }),
            "then the results: a series with results is not drawn again"
        );
    }

    #[test]
    fn a_team_moves_within_its_pot_away_from_its_club() {
        // Seeds 1 and 4 share a club: the snake puts both in A, so 4 swaps with 3.
        let mut teams = entrants(8);
        teams[0] = Entrant::new(101, ["Club Bleu", "Club Exemple"]);
        teams[3] = Entrant::new(104, [" club bleu "]);
        assert_eq!(
            seeds(&drawn(&by_size(4), teams)),
            vec![vec![1, 3, 5, 8], vec![2, 4, 6, 7]],
            "seed 4 joins B, seed 3 A: clubs compare trimmed and without case"
        );
    }

    #[test]
    fn the_draw_keeps_the_most_teams_away_from_their_clubs() {
        // Pot 1 (seeds 4 to 6) goes C, B, A. Seed 4 shares a club with seed 3 (pool C) and with
        // seed 2 (pool B): only A keeps it away, so seeds 5 and 6 take B and C.
        let mut teams = entrants(12);
        teams[1] = Entrant::new(102, ["Club Vert"]);
        teams[2] = Entrant::new(103, ["Club Rouge"]);
        teams[3] = Entrant::new(104, ["Club Rouge", "Club Vert"]);
        let pools = drawn(&by_size(4), teams);
        assert_eq!(
            seeds(&pools),
            vec![vec![1, 4, 7, 12], vec![2, 5, 8, 11], vec![3, 6, 9, 10]],
            "seed 4 in A"
        );
    }

    #[test]
    fn a_club_the_draw_cannot_avoid_keeps_the_snake_order() {
        let teams: Vec<Entrant> = (101..109)
            .map(|number| Entrant::new(number, ["Club Exemple"]))
            .collect();
        assert_eq!(
            seeds(&drawn(&by_size(4), teams)),
            vec![vec![1, 4, 5, 8], vec![2, 3, 6, 7]],
            "every team of one club: the snake"
        );
    }

    /// A small deterministic generator for the clubs of the property test below: no randomness
    /// in the tests, and the same cases on every run.
    fn clubs_of(number: i64, salt: u64) -> Vec<&'static str> {
        const CLUBS: [&str; 4] = ["Club Bleu", "Club Rouge", "Club Vert", ""];
        let mut x = u64::try_from(number).unwrap_or(0) ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let first = usize::try_from(x % 4).unwrap_or(0);
        let second = usize::try_from((x >> 8) % 4).unwrap_or(0);
        vec![CLUBS[first], CLUBS[second]]
    }

    /// For every number of teams a series can draw, every size and pools count that the rules
    /// allow, and clubs spread in many ways: every team is in one pool, every pool holds 4 to 6
    /// teams and pools differ by one at most, and each pool takes at most one team of each pot,
    /// so that the club rule never breaks the seeding.
    #[test]
    fn every_valid_draw_is_balanced_whatever_the_clubs() {
        for teams in 4..=100_i64 {
            let requests = (4..=6)
                .map(DrawSize::PoolSize)
                .chain((1..=26).map(DrawSize::Pools));
            for size in requests {
                for salt in 0..4 {
                    let entrants: Vec<Entrant> = (101..101 + teams)
                        .map(|number| Entrant::new(number, clubs_of(number, salt)))
                        .collect();
                    let request = DrawRequest {
                        size,
                        seeds: Vec::new(),
                    };
                    let Ok(pools) = draw(Status::Pools, &request, entrants, 0) else {
                        continue;
                    };
                    let sizes: Vec<usize> = pools.iter().map(|pool| pool.teams.len()).collect();
                    assert!(
                        sizes.iter().all(|size| POOL_SIZES.contains(size)),
                        "{teams} teams, {size:?}: sizes {sizes:?}"
                    );
                    let (smallest, largest) = (
                        sizes.iter().min().copied().unwrap_or(0),
                        sizes.iter().max().copied().unwrap_or(0),
                    );
                    assert!(
                        largest - smallest <= 1,
                        "{teams} teams, {size:?}: sizes {sizes:?}"
                    );
                    let every: BTreeSet<i64> = pools
                        .iter()
                        .flat_map(|pool| pool.teams.iter().map(|team| team.number))
                        .collect();
                    assert_eq!(
                        every,
                        (101..101 + teams).collect(),
                        "{teams} teams, {size:?}: each team once"
                    );
                    let count = i64::try_from(pools.len()).unwrap_or(1);
                    for pool in &pools {
                        let pots: BTreeSet<i64> = pool
                            .teams
                            .iter()
                            .map(|team| (team.seed - 1) / count)
                            .collect();
                        assert_eq!(
                            pots.len(),
                            pool.teams.len(),
                            "{teams} teams, {size:?}: pool {} takes one team per pot",
                            pool.name
                        );
                        assert!(
                            pool.teams
                                .windows(2)
                                .all(|pair| pair[0].seed < pair[1].seed),
                            "{teams} teams, {size:?}: pool {} in seed order",
                            pool.name
                        );
                    }
                }
            }
        }
    }
}
