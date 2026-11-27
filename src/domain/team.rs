// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! A team: two players that the organizer registers in a series of a tournament, as stored in
//! `teams` and `players`.
//!
//! Its number is its public key within the tournament: chosen at creation in the block of its
//! series, unique, immutable, like its series. txapela sets its registration date: the date in
//! Europe/Paris when the organizer registers it, from the Worker's clock that the route passes
//! in. The organizer says whether the fee is paid. A team is `registered` or `withdrawn`: a
//! withdrawn team keeps its number, and no maximum counts it. Withdrawing a team once the pools
//! are drawn is a forfeit, left to the pools (#8).

use core::fmt;
use core::str::FromStr;

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use super::player::{PLAYERS, Player, pair};
use super::rule::{Invalid, invalid};
use super::series::Series;
use super::slug::Slug;
use super::status::RegistrationClosed;
use super::tournament::Tournament;

/// `teams.status`. A new team is registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(crate) enum TeamStatus {
    /// Counted by the maximums of its series and tournament.
    Registered,
    /// Out of the tournament before the pools: it keeps its number, and no maximum counts it.
    Withdrawn,
}

impl TeamStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::Withdrawn => "withdrawn",
        }
    }
}

impl FromStr for TeamStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "registered" => Ok(Self::Registered),
            "withdrawn" => Ok(Self::Withdrawn),
            _ => Err(()),
        }
    }
}

/// A team number as a path writes it: decimal digits without a leading zero, such as `101`.
/// `None` for anything else, which no team has.
pub(crate) fn parse_number(s: &str) -> Option<i64> {
    let canonical = !s.is_empty() && !s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit());
    if canonical { s.parse().ok() } else { None }
}

/// A team: a `teams` row with its two players.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Team {
    /// The public key within the tournament, in the block of its series, immutable.
    pub(crate) number: i64,
    /// The slug of its series, immutable.
    pub(crate) series: Slug,
    /// In the organizer's order.
    pub(crate) players: [Player; PLAYERS],
    /// The civil date in Europe/Paris when the organizer registered it, set by txapela.
    pub(crate) registered_on: Date,
    /// Set by the organizer: payment itself is out of scope.
    pub(crate) fee_paid: bool,
    pub(crate) status: TeamStatus,
}

/// A team to register, as the organizer enters it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NewTeam {
    pub(crate) number: i64,
    pub(crate) players: Vec<Player>,
    pub(crate) fee_paid: bool,
}

/// A `PATCH` of a team: each field is `Some` when the body sets it. The number, the series and
/// the registration date never change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TeamChanges {
    /// Replaces both players.
    pub(crate) players: Option<Vec<Player>>,
    pub(crate) fee_paid: Option<bool>,
    pub(crate) status: Option<TeamStatus>,
}

/// The registered teams of the series and of the tournament, the team being changed left out:
/// what their maximums count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Registered {
    pub(crate) series: i64,
    pub(crate) tournament: i64,
}

/// A registration or a change of a team that the tournament or a maximum forbids, or that
/// breaks a rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TeamError {
    /// The tournament is past registration.
    RegistrationClosed(RegistrationClosed),
    /// The team breaks a rule.
    Invalid(Invalid),
    /// The series already has its maximum of registered teams.
    SeriesFull { series: Slug, max_teams: i64 },
    /// The tournament already has its maximum of registered teams.
    TournamentFull { max_teams: i64 },
}

impl fmt::Display for TeamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegistrationClosed(error) => error.fmt(f),
            Self::Invalid(error) => error.fmt(f),
            Self::SeriesFull { series, max_teams } => write!(
                f,
                "series: {series} is full, with its {max_teams} teams registered"
            ),
            Self::TournamentFull { max_teams } => write!(
                f,
                "max_teams: the tournament is full, with its {max_teams} teams registered"
            ),
        }
    }
}

impl std::error::Error for TeamError {}

impl From<RegistrationClosed> for TeamError {
    fn from(error: RegistrationClosed) -> Self {
        Self::RegistrationClosed(error)
    }
}

impl From<Invalid> for TeamError {
    fn from(error: Invalid) -> Self {
        Self::Invalid(error)
    }
}

impl Team {
    /// Registers a team in `series` on `today`, the date in Europe/Paris that the route reads
    /// from the Worker's clock. Checked in this order: a tournament past registration (409),
    /// the rules (422: a number in the block of the series, two players), then the maximums of
    /// the series and of the tournament (409).
    pub(crate) fn register(
        new: NewTeam,
        tournament: &Tournament,
        series: &Series,
        registered: Registered,
        today: Date,
    ) -> Result<Self, TeamError> {
        tournament.status.check_registration_open()?;
        let numbers = series.numbers();
        if !numbers.contains(&new.number) {
            return Err(invalid(
                "number",
                format!(
                    "is from {} to {}, the block of the series {}",
                    numbers.start(),
                    numbers.end(),
                    series.slug
                ),
            )
            .into());
        }
        let team = Self {
            number: new.number,
            series: series.slug.clone(),
            players: pair(new.players)?,
            registered_on: today,
            fee_paid: new.fee_paid,
            status: TeamStatus::Registered,
        };
        Self::check_room(tournament, series, registered)?;
        Ok(team)
    }

    /// Room for one more registered team in the series and in the tournament.
    fn check_room(
        tournament: &Tournament,
        series: &Series,
        registered: Registered,
    ) -> Result<(), TeamError> {
        if registered.series >= series.max_teams {
            return Err(TeamError::SeriesFull {
                series: series.slug.clone(),
                max_teams: series.max_teams,
            });
        }
        if registered.tournament >= tournament.max_teams {
            return Err(TeamError::TournamentFull {
                max_teams: tournament.max_teams,
            });
        }
        Ok(())
    }

    /// The team after a `PATCH`, checked in this order: a status change past registration (409,
    /// even to the current status), the players (422), then, for a withdrawn team registered
    /// again, the maximums (409). Players and the fee change in every status.
    pub(crate) fn change(
        &self,
        changes: TeamChanges,
        tournament: &Tournament,
        series: &Series,
        registered: Registered,
    ) -> Result<Self, TeamError> {
        if changes.status.is_some() {
            tournament.status.check_registration_open()?;
        }
        let players = match changes.players {
            Some(players) => pair(players)?,
            None => self.players.clone(),
        };
        let next = Self {
            number: self.number,
            series: self.series.clone(),
            players,
            registered_on: self.registered_on,
            fee_paid: changes.fee_paid.unwrap_or(self.fee_paid),
            status: changes.status.unwrap_or(self.status),
        };
        if self.status == TeamStatus::Withdrawn && next.status == TeamStatus::Registered {
            Self::check_room(tournament, series, registered)?;
        }
        Ok(next)
    }

    /// Checks a deletion: while registration is open only.
    pub(crate) fn check_delete(tournament: &Tournament) -> Result<(), TeamError> {
        tournament.status.check_registration_open()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::{Date, date};

    use super::{NewTeam, Registered, Team, TeamChanges, TeamError, TeamStatus, parse_number};
    use crate::domain::player::Player;
    use crate::domain::rule::Invalid;
    use crate::domain::series::Series;
    use crate::domain::slug::Slug;
    use crate::domain::status::{RegistrationClosed, Status};
    use crate::domain::tournament::{Specialty, Tournament, example_draft};

    const TODAY: Date = date(2027, 3, 5);

    const ROOM: Registered = Registered {
        series: 0,
        tournament: 0,
    };

    fn tournament(status: Status) -> Tournament {
        let mut tournament = example_draft();
        tournament.status = status;
        tournament.max_teams = 10;
        tournament
    }

    fn series() -> Series {
        Series {
            slug: Slug::parse("1re-serie-a").expect("should be a slug"),
            name: "1re Série A".to_owned(),
            specialty: Specialty::PaletaGommeCreuse,
            max_teams: 4,
            position: 1,
            number_block: 100,
        }
    }

    /// A fictional player.
    fn player(first_name: &str) -> Player {
        Player {
            first_name: first_name.to_owned(),
            last_name: "Exemple".to_owned(),
            club: Some("Club Exemple de Pelote".to_owned()),
            licence: None,
            contact: Some(format!("{}@example.org", first_name.to_lowercase())),
        }
    }

    fn new_team(number: i64) -> NewTeam {
        NewTeam {
            number,
            players: vec![player("Ane"), player("Beñat")],
            fee_paid: false,
        }
    }

    /// Team 101, registered in a draft.
    fn registered_team() -> Team {
        Team::register(
            new_team(101),
            &tournament(Status::Draft),
            &series(),
            ROOM,
            TODAY,
        )
        .expect("should register")
    }

    #[test]
    fn a_registered_team_is_dated_today_and_unpaid_by_default() {
        let team = registered_team();
        assert_eq!(team.number, 101, "its number");
        assert_eq!(team.series.as_str(), "1re-serie-a", "its series");
        assert_eq!(team.registered_on, TODAY, "the date passed in");
        assert!(!team.fee_paid, "unpaid");
        assert_eq!(team.status, TeamStatus::Registered, "registered");
        assert_eq!(
            team.players[1].first_name, "Beñat",
            "players in the given order"
        );
    }

    #[test]
    fn the_number_is_in_the_block_of_the_series() {
        for number in [100, 199] {
            assert!(
                Team::register(
                    new_team(number),
                    &tournament(Status::Registration),
                    &series(),
                    ROOM,
                    TODAY
                )
                .is_ok(),
                "{number} is in the block 100"
            );
        }
        for number in [99, 200, 0, -101] {
            assert_eq!(
                Team::register(
                    new_team(number),
                    &tournament(Status::Registration),
                    &series(),
                    ROOM,
                    TODAY
                )
                .map_err(|e| e.to_string()),
                Err("number: is from 100 to 199, the block of the series 1re-serie-a".to_owned()),
                "{number} is outside the block"
            );
        }
    }

    #[test]
    fn a_team_has_two_valid_players() {
        let mut team = new_team(101);
        team.players.pop();
        assert_eq!(
            Team::register(team, &tournament(Status::Draft), &series(), ROOM, TODAY),
            Err(TeamError::Invalid(Invalid {
                field: "players",
                reason: "holds exactly two players".to_owned(),
            })),
            "one player"
        );
    }

    #[test]
    fn the_series_and_the_tournament_refuse_a_team_beyond_their_maximum() {
        let register = |registered| {
            Team::register(
                new_team(101),
                &tournament(Status::Registration),
                &series(),
                registered,
                TODAY,
            )
        };
        assert!(
            register(Registered {
                series: 3,
                tournament: 9
            })
            .is_ok(),
            "the last place of both"
        );
        assert_eq!(
            register(Registered {
                series: 4,
                tournament: 4
            })
            .map_err(|e| e.to_string()),
            Err("series: 1re-serie-a is full, with its 4 teams registered".to_owned()),
            "the series is full"
        );
        assert_eq!(
            register(Registered {
                series: 0,
                tournament: 10
            })
            .map_err(|e| e.to_string()),
            Err("max_teams: the tournament is full, with its 10 teams registered".to_owned()),
            "the tournament is full"
        );
    }

    #[test]
    fn registration_errors_come_in_order() {
        let full = Registered {
            series: 4,
            tournament: 10,
        };
        for status in [Status::Pools, Status::Finals, Status::Finished] {
            assert_eq!(
                Team::register(new_team(5), &tournament(status), &series(), full, TODAY),
                Err(TeamError::RegistrationClosed(RegistrationClosed(status))),
                "{status:?}: registration-closed first"
            );
        }
        assert!(
            matches!(
                Team::register(
                    new_team(5),
                    &tournament(Status::Registration),
                    &series(),
                    full,
                    TODAY
                ),
                Err(TeamError::Invalid(Invalid {
                    field: "number",
                    ..
                }))
            ),
            "then the rules"
        );
        assert!(
            matches!(
                Team::register(
                    new_team(101),
                    &tournament(Status::Registration),
                    &series(),
                    full,
                    TODAY
                ),
                Err(TeamError::SeriesFull { .. })
            ),
            "then the maximums"
        );
    }

    #[test]
    fn players_and_fee_change_in_every_status_and_the_status_while_registration_is_open() {
        let team = registered_team();
        let pools = tournament(Status::Pools);
        let changed = team
            .change(
                TeamChanges {
                    players: Some(vec![player("Ane"), player("Kepa")]),
                    fee_paid: Some(true),
                    ..TeamChanges::default()
                },
                &pools,
                &series(),
                ROOM,
            )
            .expect("players and fee stay editable");
        assert!(changed.fee_paid, "fee paid");
        assert_eq!(changed.players[1].first_name, "Kepa", "players replaced");
        assert_eq!(changed.registered_on, TODAY, "the date never changes");
        for status in [TeamStatus::Withdrawn, TeamStatus::Registered] {
            assert_eq!(
                team.change(
                    TeamChanges {
                        status: Some(status),
                        ..TeamChanges::default()
                    },
                    &pools,
                    &series(),
                    ROOM
                ),
                Err(TeamError::RegistrationClosed(RegistrationClosed(
                    Status::Pools
                ))),
                "{status:?} is a forfeit past registration (#8)"
            );
        }
        assert_eq!(
            team.change(TeamChanges::default(), &pools, &series(), ROOM),
            Ok(team.clone()),
            "an empty patch changes nothing"
        );
    }

    #[test]
    fn a_withdrawn_team_registered_again_needs_room() {
        let open = tournament(Status::Registration);
        let withdrawn = registered_team()
            .change(
                TeamChanges {
                    status: Some(TeamStatus::Withdrawn),
                    ..TeamChanges::default()
                },
                &open,
                &series(),
                Registered {
                    series: 4,
                    tournament: 10,
                },
            )
            .expect("a full series lets a team withdraw");
        assert_eq!(withdrawn.status, TeamStatus::Withdrawn, "withdrawn");
        let back = |registered| {
            withdrawn.change(
                TeamChanges {
                    status: Some(TeamStatus::Registered),
                    ..TeamChanges::default()
                },
                &open,
                &series(),
                registered,
            )
        };
        assert!(
            matches!(
                back(Registered {
                    series: 4,
                    tournament: 4
                }),
                Err(TeamError::SeriesFull { max_teams: 4, .. })
            ),
            "its series filled up meanwhile"
        );
        assert_eq!(
            back(Registered {
                series: 3,
                tournament: 9
            })
            .map(|team| team.status),
            Ok(TeamStatus::Registered),
            "a place left"
        );
    }

    #[test]
    fn a_team_is_deleted_while_registration_is_open_only() {
        for status in [Status::Draft, Status::Registration] {
            assert_eq!(
                Team::check_delete(&tournament(status)),
                Ok(()),
                "{status:?}"
            );
        }
        assert_eq!(
            Team::check_delete(&tournament(Status::Pools)),
            Err(TeamError::RegistrationClosed(RegistrationClosed(
                Status::Pools
            ))),
            "past registration"
        );
    }

    #[test]
    fn a_number_in_a_path_is_decimal_digits_without_a_leading_zero() {
        assert_eq!(parse_number("101"), Some(101), "a team number");
        assert_eq!(parse_number("9999"), Some(9_999), "the last number");
        for invalid in [
            "",
            "0",
            "0101",
            "+101",
            "-101",
            "10a",
            "1e2",
            " 101",
            "１０１",
            "99999999999999999999",
        ] {
            assert_eq!(
                parse_number(invalid),
                None,
                "{invalid:?} is not a team number"
            );
        }
    }

    #[test]
    fn status_round_trips_through_its_text_form() {
        for status in [TeamStatus::Registered, TeamStatus::Withdrawn] {
            assert_eq!(
                status.as_str().parse::<TeamStatus>(),
                Ok(status),
                "{status:?}"
            );
        }
        assert_eq!(
            "forfeit".parse::<TeamStatus>(),
            Err(()),
            "an unknown status"
        );
    }
}
