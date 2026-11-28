// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The round robin of a pool by the circle method: every team plays every other one once.
//!
//! The teams sit around a circle, the first one fixed, and each round pairs them across the
//! circle before the others turn one place. A pool with an odd number of teams gets an empty
//! place, a bye: the team paired with it rests that round, and each team rests once.

/// A match of the round robin: its round, from 1, and its two teams as indexes into the teams of
/// the pool, `a` before `b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pairing {
    pub(crate) round: i64,
    pub(crate) a: usize,
    pub(crate) b: usize,
}

/// The matches of a round robin between `teams` teams, by round, then by place on the circle:
/// `teams × (teams - 1) / 2` matches in `teams - 1` rounds, or `teams` rounds for an odd number of
/// teams. None for fewer than two teams.
pub(crate) fn pairings(teams: usize) -> Vec<Pairing> {
    if teams < 2 {
        return Vec::new();
    }
    // The place `teams`, for an odd number of teams, is the bye.
    let places = teams + teams % 2;
    let mut circle: Vec<usize> = (0..places).collect();
    let mut pairings = Vec::with_capacity(teams * (teams - 1) / 2);
    for round in (1_i64..).take(places - 1) {
        for across in 0..places / 2 {
            let (first, second) = (circle[across], circle[places - 1 - across]);
            if first < teams && second < teams {
                pairings.push(Pairing {
                    round,
                    a: first.min(second),
                    b: first.max(second),
                });
            }
        }
        // The first place stays, the others turn one place.
        circle[1..].rotate_right(1);
    }
    pairings
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{Pairing, pairings};

    /// The largest pool the tests cover: a series takes 100 teams at most, so no pool is larger.
    const LARGEST: usize = 100;

    #[test]
    fn fewer_than_two_teams_play_no_match() {
        assert_eq!(pairings(0), Vec::new(), "no team");
        assert_eq!(pairings(1), Vec::new(), "one team");
    }

    #[test]
    fn four_teams_play_three_rounds_of_two_matches() {
        let rounds: Vec<(i64, usize, usize)> = pairings(4)
            .into_iter()
            .map(|Pairing { round, a, b }| (round, a, b))
            .collect();
        assert_eq!(
            rounds,
            vec![
                (1, 0, 3),
                (1, 1, 2),
                (2, 0, 2),
                (2, 1, 3),
                (3, 0, 1),
                (3, 2, 3),
            ],
            "the circle method: the first team stays, the others turn"
        );
    }

    #[test]
    fn five_teams_play_five_rounds_with_one_bye_each() {
        let matches = pairings(5);
        assert_eq!(matches.len(), 10, "every pair once");
        assert_eq!(
            matches.iter().map(|pairing| pairing.round).max(),
            Some(5),
            "five rounds"
        );
        for round in 1..=5 {
            assert_eq!(
                matches
                    .iter()
                    .filter(|pairing| pairing.round == round)
                    .count(),
                2,
                "round {round}: two matches, one team rests"
            );
        }
    }

    /// The property of the round robin, checked for every pool size from 2 to [`LARGEST`]: the
    /// function depends on the size only, so these sizes are every input a series can give it.
    /// Every pair of teams meets exactly once, no team plays twice in a round, there are
    /// `size - 1` rounds for an even size and `size` for an odd one, and with an odd size exactly
    /// one team rests each round and each team rests exactly once.
    #[test]
    fn every_pair_meets_exactly_once_with_a_bye_per_round_for_odd_sizes() {
        for size in 2..=LARGEST {
            let matches = pairings(size);
            let mut pairs = BTreeSet::new();
            let mut rounds: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
            for Pairing { round, a, b } in &matches {
                assert!(
                    a < b && *b < size,
                    "size {size}: teams {a} and {b} are in the pool"
                );
                assert!(
                    pairs.insert((*a, *b)),
                    "size {size}: {a} and {b} meet more than once"
                );
                rounds.entry(*round).or_default().extend([*a, *b]);
            }
            assert_eq!(
                pairs.len(),
                size * (size - 1) / 2,
                "size {size}: every pair meets"
            );
            let expected_rounds = if size % 2 == 0 { size - 1 } else { size };
            assert_eq!(
                rounds.keys().copied().collect::<Vec<_>>(),
                (1_i64..).take(expected_rounds).collect::<Vec<_>>(),
                "size {size}: rounds numbered from 1, without a gap"
            );
            let mut byes = vec![0_usize; size];
            for (round, playing) in &rounds {
                let distinct: BTreeSet<_> = playing.iter().copied().collect();
                assert_eq!(
                    distinct.len(),
                    playing.len(),
                    "size {size}, round {round}: no team plays twice"
                );
                assert_eq!(
                    playing.len(),
                    size - size % 2,
                    "size {size}, round {round}: every team plays but the one with the bye"
                );
                for (team, rests) in byes.iter_mut().enumerate() {
                    if !distinct.contains(&team) {
                        *rests += 1;
                    }
                }
            }
            let expected_byes = usize::from(size % 2 != 0);
            assert!(
                byes.iter().all(|&rests| rests == expected_byes),
                "size {size}: each team rests {expected_byes} time(s): {byes:?}"
            );
            assert!(
                matches
                    .windows(2)
                    .all(|pair| pair[0].round <= pair[1].round),
                "size {size}: matches by round"
            );
        }
    }
}
