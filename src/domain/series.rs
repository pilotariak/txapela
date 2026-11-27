// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! A series: the teams of a tournament that play one of its specialties at one level, such as
//! "1re Série A", as stored in `series`.
//!
//! Its slug is its public key within the tournament, chosen at creation and immutable. Its
//! number block holds the numbers of its teams: a multiple of 100, and the 100 numbers from it,
//! so that 100 holds teams 100 to 199 and 200 holds teams 200 to 299. Two series of a tournament
//! never share a block, so two blocks never overlap.

use core::fmt;
use core::ops::RangeInclusive;

use super::rule::{Invalid, integer, invalid, required_text};
use super::slug::Slug;
use super::status::RegistrationClosed;
use super::tournament::{Specialty, Tournament};

/// The numbers of a block: from the block to the block + 99.
pub(crate) const BLOCK_SIZE: i64 = 100;

/// The last block: team numbers keep four digits at most.
const LAST_BLOCK: i64 = 9_900;

/// The fewest teams a series can take.
const MIN_TEAMS: i64 = 2;

/// A series of a tournament.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Series {
    /// The public key within the tournament, immutable.
    pub(crate) slug: Slug,
    pub(crate) name: String,
    /// One of the tournament's specialties, whose target score the series plays to.
    pub(crate) specialty: Specialty,
    pub(crate) max_teams: i64,
    /// Display order: series are listed by position, then slug.
    pub(crate) position: i64,
    /// The first number of the block of its teams.
    pub(crate) number_block: i64,
}

/// A `PATCH` of a series: each field is `Some` when the body sets it. The slug never changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SeriesChanges {
    pub(crate) name: Option<String>,
    pub(crate) specialty: Option<Specialty>,
    pub(crate) max_teams: Option<i64>,
    pub(crate) position: Option<i64>,
    pub(crate) number_block: Option<i64>,
}

impl SeriesChanges {
    /// The first field present that changes only while registration is open.
    fn registration_only_field(&self) -> Option<&'static str> {
        [
            ("specialty", self.specialty.is_some()),
            ("max_teams", self.max_teams.is_some()),
            ("number_block", self.number_block.is_some()),
        ]
        .into_iter()
        .find_map(|(field, present)| present.then_some(field))
    }
}

/// The teams of a series: all of them, withdrawn ones included, and the registered ones, which
/// its maximum counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SeriesTeams {
    pub(crate) teams: i64,
    pub(crate) registered: i64,
}

/// A change of a series that the tournament or its teams forbid, or that breaks a rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SeriesError {
    /// The tournament is past registration.
    RegistrationClosed(RegistrationClosed),
    /// The series breaks a rule.
    Invalid(Invalid),
    /// `max_teams` below the teams registered in the series.
    BelowRegistered { registered: i64 },
    /// A new number block for a series that has teams, whose numbers are in the old one.
    BlockHasTeams { teams: i64 },
    /// Deleting a series that has teams.
    DeleteHasTeams { teams: i64 },
}

/// `1 team`, `2 teams`.
fn teams(count: i64) -> String {
    if count == 1 {
        "1 team".to_owned()
    } else {
        format!("{count} teams")
    }
}

impl fmt::Display for SeriesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegistrationClosed(error) => error.fmt(f),
            Self::Invalid(error) => error.fmt(f),
            Self::BelowRegistered { registered } => write!(
                f,
                "max_teams: the series has {} registered, max_teams cannot be below {registered}",
                teams(*registered)
            ),
            Self::BlockHasTeams { teams: count } => write!(
                f,
                "number_block: the series has {}, whose numbers are in its block: it cannot move",
                teams(*count)
            ),
            Self::DeleteHasTeams { teams: count } => write!(
                f,
                "teams: the series has {}: delete its teams first",
                teams(*count)
            ),
        }
    }
}

impl std::error::Error for SeriesError {}

impl From<RegistrationClosed> for SeriesError {
    fn from(error: RegistrationClosed) -> Self {
        Self::RegistrationClosed(error)
    }
}

impl Series {
    /// The team numbers of the series: its block and the 99 numbers after it.
    pub(crate) fn numbers(&self) -> RangeInclusive<i64> {
        self.number_block..=self.number_block + BLOCK_SIZE - 1
    }

    /// The score a team must reach to win a match of the series: the target score of its
    /// specialty in the tournament.
    pub(crate) fn target_score(&self, tournament: &Tournament) -> Option<i64> {
        tournament
            .specialties
            .iter()
            .find(|specialty| specialty.specialty == self.specialty)
            .map(|specialty| specialty.target_score)
    }

    /// The rules of a series, checked at creation and on the state after every change. The name
    /// is trimmed.
    pub(crate) fn validate(&mut self, tournament: &Tournament) -> Result<(), Invalid> {
        self.name = required_text("name", &self.name)?;
        if self.target_score(tournament).is_none() {
            return Err(invalid(
                "specialty",
                format!(
                    "{} is not a specialty of the tournament",
                    self.specialty.as_str()
                ),
            ));
        }
        if !(MIN_TEAMS..=BLOCK_SIZE).contains(&self.max_teams) {
            return Err(invalid(
                "max_teams",
                format!("is an integer from {MIN_TEAMS} to {BLOCK_SIZE}, the numbers of a block"),
            ));
        }
        integer("position", self.position, 1)?;
        if !(BLOCK_SIZE..=LAST_BLOCK).contains(&self.number_block)
            || self.number_block % BLOCK_SIZE != 0
        {
            return Err(invalid(
                "number_block",
                format!("is a multiple of {BLOCK_SIZE} from {BLOCK_SIZE} to {LAST_BLOCK}"),
            ));
        }
        Ok(())
    }

    /// A new series of `tournament`, checked: 409 `registration-closed` past registration, then
    /// the rules (422).
    pub(crate) fn create(mut self, tournament: &Tournament) -> Result<Self, SeriesError> {
        tournament.status.check_registration_open()?;
        self.validate(tournament).map_err(SeriesError::Invalid)?;
        Ok(self)
    }

    /// The series after a `PATCH`, checked in this order: specialty, maximum or block changed
    /// past registration (409 `registration-closed`), then the rules on the merged state (422),
    /// then its teams: a block that moves under them, or a maximum below the registered ones
    /// (409). Name and position change in every status.
    pub(crate) fn change(
        &self,
        changes: SeriesChanges,
        tournament: &Tournament,
        teams: SeriesTeams,
    ) -> Result<Self, SeriesError> {
        if changes.registration_only_field().is_some() {
            tournament.status.check_registration_open()?;
        }
        let mut next = Self {
            slug: self.slug.clone(),
            name: changes.name.unwrap_or_else(|| self.name.clone()),
            specialty: changes.specialty.unwrap_or(self.specialty),
            max_teams: changes.max_teams.unwrap_or(self.max_teams),
            position: changes.position.unwrap_or(self.position),
            number_block: changes.number_block.unwrap_or(self.number_block),
        };
        next.validate(tournament).map_err(SeriesError::Invalid)?;
        if next.number_block != self.number_block && teams.teams > 0 {
            return Err(SeriesError::BlockHasTeams { teams: teams.teams });
        }
        if next.max_teams < teams.registered {
            return Err(SeriesError::BelowRegistered {
                registered: teams.registered,
            });
        }
        Ok(next)
    }

    /// Checks a deletion: while registration is open, and only without teams, withdrawn ones
    /// included.
    pub(crate) fn check_delete(
        tournament: &Tournament,
        teams: SeriesTeams,
    ) -> Result<(), SeriesError> {
        tournament.status.check_registration_open()?;
        if teams.teams > 0 {
            return Err(SeriesError::DeleteHasTeams { teams: teams.teams });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Series, SeriesChanges, SeriesError, SeriesTeams};
    use crate::domain::rule::Invalid;
    use crate::domain::slug::Slug;
    use crate::domain::status::{RegistrationClosed, Status};
    use crate::domain::tournament::{Specialty, SpecialtyTarget, Tournament, example_draft};

    /// A draft playing both specialties.
    fn tournament() -> Tournament {
        let mut tournament = example_draft();
        tournament.specialties = vec![
            SpecialtyTarget {
                specialty: Specialty::PaletaGommeCreuse,
                target_score: 40,
            },
            SpecialtyTarget {
                specialty: Specialty::PaletaGommePleine,
                target_score: 30,
            },
        ];
        tournament
    }

    fn in_status(status: Status) -> Tournament {
        let mut tournament = tournament();
        tournament.status = status;
        tournament
    }

    fn series() -> Series {
        Series {
            slug: Slug::parse("1re-serie-a").expect("should be a slug"),
            name: "1re Série A".to_owned(),
            specialty: Specialty::PaletaGommeCreuse,
            max_teams: 8,
            position: 1,
            number_block: 100,
        }
    }

    fn rule(edit: impl FnOnce(&mut Series)) -> Result<Series, Invalid> {
        let mut series = series();
        edit(&mut series);
        series.validate(&tournament()).map(|()| series)
    }

    fn field_of(result: Result<Series, Invalid>) -> Option<&'static str> {
        result.err().map(|error| error.field)
    }

    const NO_TEAMS: SeriesTeams = SeriesTeams {
        teams: 0,
        registered: 0,
    };

    #[test]
    fn a_valid_series_passes_and_plays_to_its_specialty_target() {
        assert_eq!(rule(|_| {}), Ok(series()), "the fixture is valid");
        assert_eq!(
            series().target_score(&tournament()),
            Some(40),
            "the target score of gomme creuse in the tournament"
        );
        let mut pleine = series();
        pleine.specialty = Specialty::PaletaGommePleine;
        assert_eq!(
            pleine.target_score(&tournament()),
            Some(30),
            "the target score of gomme pleine"
        );
    }

    #[test]
    fn the_specialty_is_one_the_tournament_declares() {
        let mut creuse_only = tournament();
        creuse_only.specialties.truncate(1);
        let mut pleine = series();
        pleine.specialty = Specialty::PaletaGommePleine;
        assert_eq!(
            pleine.validate(&creuse_only).map_err(|e| e.to_string()),
            Err("specialty: paleta_gomme_pleine is not a specialty of the tournament".to_owned()),
            "a specialty the tournament does not play"
        );
    }

    #[test]
    fn the_name_is_trimmed_and_required() {
        let trimmed = rule(|s| s.name = "  2e Série ".to_owned()).expect("blanks are fine");
        assert_eq!(trimmed.name, "2e Série", "the name is trimmed");
        assert_eq!(field_of(rule(|s| s.name = " ".to_owned())), Some("name"));
    }

    #[test]
    fn the_maximum_fits_in_a_block_and_the_position_is_positive() {
        for max_teams in [2, 100] {
            assert!(
                rule(|s| s.max_teams = max_teams).is_ok(),
                "{max_teams} teams"
            );
        }
        for max_teams in [1, 101, -1] {
            assert_eq!(
                field_of(rule(|s| s.max_teams = max_teams)),
                Some("max_teams"),
                "{max_teams} teams"
            );
        }
        assert_eq!(field_of(rule(|s| s.position = 0)), Some("position"));
        assert!(rule(|s| s.position = 12).is_ok(), "any position from 1");
    }

    #[test]
    fn a_block_is_a_multiple_of_100_from_100_to_9900() {
        for block in [100, 200, 9_900] {
            assert!(rule(|s| s.number_block = block).is_ok(), "block {block}");
        }
        for block in [0, 50, 150, 10_000, -100] {
            assert_eq!(
                rule(|s| s.number_block = block).map_err(|e| e.to_string()),
                Err("number_block: is a multiple of 100 from 100 to 9900".to_owned()),
                "block {block}"
            );
        }
        let mut second = series();
        second.number_block = 200;
        assert_eq!(second.numbers(), 200..=299, "a block holds 100 numbers");
    }

    #[test]
    fn creation_needs_an_open_registration_before_the_rules() {
        assert!(
            series().create(&in_status(Status::Registration)).is_ok(),
            "in registration"
        );
        let mut broken = series();
        broken.name = String::new();
        for status in [Status::Pools, Status::Finals, Status::Finished] {
            assert_eq!(
                broken.clone().create(&in_status(status)),
                Err(SeriesError::RegistrationClosed(RegistrationClosed(status))),
                "{status:?}: the status first"
            );
        }
        assert!(
            matches!(
                broken.create(&tournament()),
                Err(SeriesError::Invalid(Invalid { field: "name", .. }))
            ),
            "then the rules"
        );
    }

    #[test]
    fn a_change_merges_the_patch_into_the_stored_values() {
        let changed = series()
            .change(
                SeriesChanges {
                    name: Some("1re Série".to_owned()),
                    specialty: Some(Specialty::PaletaGommePleine),
                    max_teams: Some(12),
                    ..SeriesChanges::default()
                },
                &tournament(),
                NO_TEAMS,
            )
            .expect("should change");
        let mut expected = series();
        expected.name = "1re Série".to_owned();
        expected.specialty = Specialty::PaletaGommePleine;
        expected.max_teams = 12;
        assert_eq!(changed, expected, "absent fields keep their value");
        assert_eq!(
            series().change(SeriesChanges::default(), &tournament(), NO_TEAMS),
            Ok(series()),
            "an empty patch changes nothing"
        );
    }

    #[test]
    fn past_registration_only_the_name_and_position_change() {
        let pools = in_status(Status::Pools);
        for (field, changes) in [
            (
                "specialty",
                SeriesChanges {
                    specialty: Some(Specialty::PaletaGommeCreuse),
                    ..SeriesChanges::default()
                },
            ),
            (
                "max_teams",
                SeriesChanges {
                    max_teams: Some(8),
                    ..SeriesChanges::default()
                },
            ),
            (
                "number_block",
                SeriesChanges {
                    number_block: Some(100),
                    ..SeriesChanges::default()
                },
            ),
        ] {
            assert_eq!(
                series().change(changes, &pools, NO_TEAMS),
                Err(SeriesError::RegistrationClosed(RegistrationClosed(
                    Status::Pools
                ))),
                "{field} is fixed past registration, even to its value"
            );
        }
        let renamed = series()
            .change(
                SeriesChanges {
                    name: Some("Première série".to_owned()),
                    position: Some(2),
                    ..SeriesChanges::default()
                },
                &pools,
                NO_TEAMS,
            )
            .expect("name and position stay editable");
        assert_eq!(
            (renamed.name.as_str(), renamed.position),
            ("Première série", 2),
            "renamed and moved"
        );
    }

    #[test]
    fn teams_keep_their_block_and_the_maximum_stays_at_least_the_registered_teams() {
        let teams = SeriesTeams {
            teams: 5,
            registered: 4,
        };
        let moved = |block| {
            series().change(
                SeriesChanges {
                    number_block: Some(block),
                    ..SeriesChanges::default()
                },
                &tournament(),
                teams,
            )
        };
        assert_eq!(
            moved(200).map_err(|e| e.to_string()),
            Err(
                "number_block: the series has 5 teams, whose numbers are in its block: it \
                 cannot move"
                    .to_owned()
            ),
            "a block cannot move under its teams"
        );
        assert!(moved(100).is_ok(), "the same block is not a move");
        assert!(
            series()
                .change(
                    SeriesChanges {
                        number_block: Some(300),
                        ..SeriesChanges::default()
                    },
                    &tournament(),
                    NO_TEAMS,
                )
                .is_ok(),
            "an empty series moves its block"
        );
        let maximum = |max_teams| {
            series().change(
                SeriesChanges {
                    max_teams: Some(max_teams),
                    ..SeriesChanges::default()
                },
                &tournament(),
                teams,
            )
        };
        assert!(maximum(4).is_ok(), "down to the registered teams");
        assert_eq!(
            maximum(3).map_err(|e| e.to_string()),
            Err(
                "max_teams: the series has 4 teams registered, max_teams cannot be below 4"
                    .to_owned()
            ),
            "withdrawn teams do not count"
        );
    }

    #[test]
    fn change_errors_come_in_order() {
        let teams = SeriesTeams {
            teams: 3,
            registered: 3,
        };
        let changes = SeriesChanges {
            name: Some(String::new()),
            max_teams: Some(2),
            number_block: Some(200),
            ..SeriesChanges::default()
        };
        assert!(
            matches!(
                series().change(changes.clone(), &in_status(Status::Finals), teams),
                Err(SeriesError::RegistrationClosed(_))
            ),
            "registration-closed first"
        );
        assert!(
            matches!(
                series().change(changes.clone(), &tournament(), teams),
                Err(SeriesError::Invalid(_))
            ),
            "then the rules"
        );
        let changes = SeriesChanges {
            name: None,
            ..changes
        };
        assert_eq!(
            series().change(changes, &tournament(), teams),
            Err(SeriesError::BlockHasTeams { teams: 3 }),
            "then the teams"
        );
    }

    #[test]
    fn only_a_series_without_teams_is_deleted_while_registration_is_open() {
        assert_eq!(
            Series::check_delete(&tournament(), NO_TEAMS),
            Ok(()),
            "an empty series of a draft"
        );
        assert_eq!(
            Series::check_delete(
                &tournament(),
                SeriesTeams {
                    teams: 1,
                    registered: 0
                }
            )
            .map_err(|e| e.to_string()),
            Err("teams: the series has 1 team: delete its teams first".to_owned()),
            "a withdrawn team still holds the series"
        );
        assert_eq!(
            Series::check_delete(&in_status(Status::Pools), NO_TEAMS),
            Err(SeriesError::RegistrationClosed(RegistrationClosed(
                Status::Pools
            ))),
            "past registration"
        );
    }
}
