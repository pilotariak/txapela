// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! A tournament, as stored in `tournaments`, `courts` and `tournament_specialties`, and the rules
//! on its values: the ones checked at creation, and the ones of a change.
//!
//! Aire, specialty, ranking and status are TEXT columns without `CHECK`: these enums parse them,
//! so that a new variant (main nue, pala…) is a code change, not a table rebuild.

use core::fmt;
use core::str::FromStr;

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use super::slug::Slug;
use super::status::{InvalidTransition, Status};

/// The largest integer that D1 stores exactly: values travel as JavaScript numbers.
pub(crate) const MAX_INTEGER: i64 = (1 << 53) - 1;

/// Defaults of the points of a match: a win, a loss and a forfeit.
const DEFAULT_POINTS: Points = Points {
    win: 3,
    loss: 1,
    forfeit: 0,
};

/// The fewest teams a tournament can take.
const MIN_TEAMS: i64 = 2;

/// Where the tournament is played: `tournaments.aire`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(crate) enum Aire {
    Fronton,
    MurAGauche,
    Trinquet,
}

impl Aire {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fronton => "fronton",
            Self::MurAGauche => "mur_a_gauche",
            Self::Trinquet => "trinquet",
        }
    }
}

impl FromStr for Aire {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "fronton" => Ok(Self::Fronton),
            "mur_a_gauche" => Ok(Self::MurAGauche),
            "trinquet" => Ok(Self::Trinquet),
            _ => Err(()),
        }
    }
}

/// A specialty played in the tournament: `tournament_specialties.specialty`. The order of the
/// variants is the order of a tournament's specialties.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(crate) enum Specialty {
    PaletaGommeCreuse,
    PaletaGommePleine,
}

impl Specialty {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PaletaGommeCreuse => "paleta_gomme_creuse",
            Self::PaletaGommePleine => "paleta_gomme_pleine",
        }
    }
}

impl FromStr for Specialty {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "paleta_gomme_creuse" => Ok(Self::PaletaGommeCreuse),
            "paleta_gomme_pleine" => Ok(Self::PaletaGommePleine),
            _ => Err(()),
        }
    }
}

/// How pool teams are ranked: `tournaments.ranking`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(crate) enum Ranking {
    /// Points first, then goal average: goals for minus goals against.
    #[default]
    PointsThenGoalAverage,
}

impl Ranking {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PointsThenGoalAverage => "points_then_goal_average",
        }
    }
}

impl FromStr for Ranking {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "points_then_goal_average" => Ok(Self::PointsThenGoalAverage),
            _ => Err(()),
        }
    }
}

/// A specialty of the tournament and the score a team must reach to win a match of it. Series
/// inherit it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpecialtyTarget {
    pub(crate) specialty: Specialty,
    pub(crate) target_score: i64,
}

/// Points of a match in the pool ranking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Points {
    pub(crate) win: i64,
    pub(crate) loss: i64,
    pub(crate) forfeit: i64,
}

impl Default for Points {
    /// 3 for a win, 1 for a loss, 0 for a forfeit.
    fn default() -> Self {
        DEFAULT_POINTS
    }
}

/// Points of a request: an absent value keeps the one of `base`, the defaults at creation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PointsChanges {
    pub(crate) win: Option<i64>,
    pub(crate) loss: Option<i64>,
    pub(crate) forfeit: Option<i64>,
}

impl PointsChanges {
    pub(crate) fn apply_to(self, base: Points) -> Points {
        Points {
            win: self.win.unwrap_or(base.win),
            loss: self.loss.unwrap_or(base.loss),
            forfeit: self.forfeit.unwrap_or(base.forfeit),
        }
    }
}

/// The four dates of a tournament, civil dates in Europe/Paris.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Dates {
    pub(crate) registration_opens: Date,
    pub(crate) registration_closes: Date,
    pub(crate) first_match: Date,
    pub(crate) finals: Date,
}

/// Dates of a `PATCH`: an absent date keeps the stored one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DatesChanges {
    pub(crate) registration_opens: Option<Date>,
    pub(crate) registration_closes: Option<Date>,
    pub(crate) first_match: Option<Date>,
    pub(crate) finals: Option<Date>,
}

impl DatesChanges {
    fn apply_to(self, base: Dates) -> Dates {
        Dates {
            registration_opens: self.registration_opens.unwrap_or(base.registration_opens),
            registration_closes: self.registration_closes.unwrap_or(base.registration_closes),
            first_match: self.first_match.unwrap_or(base.first_match),
            finals: self.finals.unwrap_or(base.finals),
        }
    }
}

/// Where the tournament is played.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Venue {
    pub(crate) name: String,
    pub(crate) address: String,
    pub(crate) latitude: f64,
    pub(crate) longitude: f64,
}

/// The venue of a `PATCH`: an absent value keeps the stored one.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct VenueChanges {
    pub(crate) name: Option<String>,
    pub(crate) address: Option<String>,
    pub(crate) latitude: Option<f64>,
    pub(crate) longitude: Option<f64>,
}

impl VenueChanges {
    fn apply_to(self, base: Venue) -> Venue {
        Venue {
            name: self.name.unwrap_or(base.name),
            address: self.address.unwrap_or(base.address),
            latitude: self.latitude.unwrap_or(base.latitude),
            longitude: self.longitude.unwrap_or(base.longitude),
        }
    }
}

/// A tournament: a `tournaments` row with its courts and its specialties.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tournament {
    /// The public key, immutable.
    pub(crate) slug: Slug,
    pub(crate) name: String,
    /// Free text until clubs have their own records.
    pub(crate) organizing_club: String,
    pub(crate) venue: Venue,
    pub(crate) aire: Aire,
    /// Names of the courts of the venue, in the organizer's order.
    pub(crate) courts: Vec<String>,
    /// In the order of [`Specialty`].
    pub(crate) specialties: Vec<SpecialtyTarget>,
    pub(crate) points: Points,
    pub(crate) ranking: Ranking,
    pub(crate) dates: Dates,
    /// Fee per team in euro cents, shown only: payment is out of scope.
    pub(crate) fee_cents: i64,
    pub(crate) max_teams: i64,
    pub(crate) status: Status,
}

/// A value that breaks a rule of a tournament: the API answers 422 with this text, which names
/// the field and the rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InvalidTournament {
    pub(crate) field: &'static str,
    pub(crate) reason: String,
}

impl fmt::Display for InvalidTournament {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}

impl std::error::Error for InvalidTournament {}

fn invalid(field: &'static str, reason: impl Into<String>) -> InvalidTournament {
    InvalidTournament {
        field,
        reason: reason.into(),
    }
}

/// Text trimmed of surrounding blanks, which cannot be empty.
fn required_text(field: &'static str, value: &str) -> Result<String, InvalidTournament> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid(field, "cannot be empty"));
    }
    Ok(value.to_owned())
}

/// An integer from `min` to [`MAX_INTEGER`].
fn integer(field: &'static str, value: i64, min: i64) -> Result<(), InvalidTournament> {
    if (min..=MAX_INTEGER).contains(&value) {
        Ok(())
    } else {
        Err(invalid(
            field,
            format!("is an integer from {min} to {MAX_INTEGER}"),
        ))
    }
}

/// Dates in order: registration opens ≤ registration closes ≤ first match ≤ finals.
fn dates_in_order(dates: &Dates) -> Result<(), InvalidTournament> {
    let order = [
        ("registration_opens", dates.registration_opens),
        ("registration_closes", dates.registration_closes),
        ("first_match", dates.first_match),
        ("finals", dates.finals),
    ];
    for pair in order.windows(2) {
        let [(earlier, before), (later, after)] = pair else {
            continue;
        };
        if after < before {
            return Err(invalid(
                "dates",
                format!(
                    "{later} cannot be before {earlier}: registration_opens <= \
                     registration_closes <= first_match <= finals"
                ),
            ));
        }
    }
    Ok(())
}

/// A change of a tournament that its status forbids, or that breaks a rule.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ChangeError {
    /// The change touches a value that only a draft can change: the field.
    NotADraft(&'static str),
    /// Only a draft can be deleted: the status of the tournament.
    NotADraftToDelete(Status),
    /// A status change other than one step forward.
    InvalidTransition(InvalidTransition),
    /// The tournament after the change breaks a rule.
    Invalid(InvalidTournament),
}

impl fmt::Display for ChangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotADraft(field) => write!(
                f,
                "{field}: only a draft tournament can change its aire, specialties, points, \
                 ranking and courts"
            ),
            Self::NotADraftToDelete(status) => write!(
                f,
                "status: a {} tournament cannot be deleted, only a draft can",
                status.as_str()
            ),
            Self::InvalidTransition(error) => error.fmt(f),
            Self::Invalid(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ChangeError {}

/// A `PATCH` of a tournament: each field is `Some` when the body sets it. The slug never changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TournamentChanges {
    pub(crate) name: Option<String>,
    pub(crate) organizing_club: Option<String>,
    pub(crate) venue: Option<VenueChanges>,
    pub(crate) aire: Option<Aire>,
    pub(crate) courts: Option<Vec<String>>,
    pub(crate) specialties: Option<Vec<SpecialtyTarget>>,
    pub(crate) points: Option<PointsChanges>,
    pub(crate) ranking: Option<Ranking>,
    pub(crate) dates: Option<DatesChanges>,
    pub(crate) fee_cents: Option<i64>,
    pub(crate) max_teams: Option<i64>,
    pub(crate) status: Option<Status>,
}

impl TournamentChanges {
    /// The first field present that only a draft can change.
    fn draft_only_field(&self) -> Option<&'static str> {
        [
            ("aire", self.aire.is_some()),
            ("specialties", self.specialties.is_some()),
            ("points", self.points.is_some()),
            ("ranking", self.ranking.is_some()),
            ("courts", self.courts.is_some()),
        ]
        .into_iter()
        .find_map(|(field, present)| present.then_some(field))
    }
}

impl Tournament {
    /// The rules of a tournament, checked at creation and on the state after every change.
    /// Text values are trimmed, and specialties put in the order of [`Specialty`].
    pub(crate) fn validate(&mut self) -> Result<(), InvalidTournament> {
        self.name = required_text("name", &self.name)?;
        self.organizing_club = required_text("organizing_club", &self.organizing_club)?;
        self.venue.name = required_text("venue.name", &self.venue.name)?;
        self.venue.address = required_text("venue.address", &self.venue.address)?;
        if !(-90.0..=90.0).contains(&self.venue.latitude) {
            return Err(invalid("venue.latitude", "is between -90 and 90"));
        }
        if !(-180.0..=180.0).contains(&self.venue.longitude) {
            return Err(invalid("venue.longitude", "is between -180 and 180"));
        }
        self.validate_courts()?;
        self.validate_specialties()?;
        self.validate_points()?;
        dates_in_order(&self.dates)?;
        integer("fee_cents", self.fee_cents, 0)?;
        integer("max_teams", self.max_teams, MIN_TEAMS)?;
        Ok(())
    }

    /// At least one court, each name trimmed, not empty and unique within the tournament. The
    /// order stays the organizer's.
    fn validate_courts(&mut self) -> Result<(), InvalidTournament> {
        if self.courts.is_empty() {
            return Err(invalid("courts", "holds at least one court"));
        }
        let mut names: Vec<String> = Vec::with_capacity(self.courts.len());
        for court in &self.courts {
            let name = court.trim();
            if name.is_empty() {
                return Err(invalid("courts", "a court name cannot be empty"));
            }
            if names.iter().any(|other| other == name) {
                return Err(invalid(
                    "courts",
                    format!("the court name {name:?} appears twice"),
                ));
            }
            names.push(name.to_owned());
        }
        self.courts = names;
        Ok(())
    }

    /// At least one specialty, none twice, each with a target score above zero.
    fn validate_specialties(&mut self) -> Result<(), InvalidTournament> {
        if self.specialties.is_empty() {
            return Err(invalid("specialties", "holds at least one specialty"));
        }
        self.specialties
            .sort_by_key(|specialty| specialty.specialty);
        for pair in self.specialties.windows(2) {
            if pair[0].specialty == pair[1].specialty {
                return Err(invalid(
                    "specialties",
                    format!("{} appears twice", pair[0].specialty.as_str()),
                ));
            }
        }
        for specialty in &self.specialties {
            if !(1..=MAX_INTEGER).contains(&specialty.target_score) {
                return Err(invalid(
                    "specialties",
                    format!(
                        "the target_score of {} is an integer from 1 to {MAX_INTEGER}",
                        specialty.specialty.as_str()
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Non-negative integers with win > loss >= forfeit.
    fn validate_points(&self) -> Result<(), InvalidTournament> {
        let Points { win, loss, forfeit } = self.points;
        integer("points.win", win, 0)?;
        integer("points.loss", loss, 0)?;
        integer("points.forfeit", forfeit, 0)?;
        if win <= loss {
            return Err(invalid("points", "win must be above loss"));
        }
        if loss < forfeit {
            return Err(invalid("points", "loss cannot be below forfeit"));
        }
        Ok(())
    }

    /// Whether the tournament is seen without an organizer token: a draft is not.
    pub(crate) fn is_public(&self) -> bool {
        self.status != Status::Draft
    }

    /// The tournament after a `PATCH`, checked in this order: a value that only a draft can
    /// change (409 `not-a-draft`), the status step (409 `invalid-transition`), then every rule
    /// of creation on the merged state (422). Whether a field is a draft-only change depends on
    /// its presence, not on its value, and on the status before the patch.
    pub(crate) fn change(&self, changes: TournamentChanges) -> Result<Self, ChangeError> {
        if self.status != Status::Draft
            && let Some(field) = changes.draft_only_field()
        {
            return Err(ChangeError::NotADraft(field));
        }
        let status = match changes.status {
            Some(to) => self
                .status
                .move_to(to)
                .map_err(ChangeError::InvalidTransition)?,
            None => self.status,
        };
        let mut next = Self {
            slug: self.slug.clone(),
            name: changes.name.unwrap_or_else(|| self.name.clone()),
            organizing_club: changes
                .organizing_club
                .unwrap_or_else(|| self.organizing_club.clone()),
            venue: changes
                .venue
                .unwrap_or_default()
                .apply_to(self.venue.clone()),
            aire: changes.aire.unwrap_or(self.aire),
            courts: changes.courts.unwrap_or_else(|| self.courts.clone()),
            specialties: changes
                .specialties
                .unwrap_or_else(|| self.specialties.clone()),
            points: changes.points.unwrap_or_default().apply_to(self.points),
            ranking: changes.ranking.unwrap_or(self.ranking),
            dates: changes.dates.unwrap_or_default().apply_to(self.dates),
            fee_cents: changes.fee_cents.unwrap_or(self.fee_cents),
            max_teams: changes.max_teams.unwrap_or(self.max_teams),
            status,
        };
        next.validate().map_err(ChangeError::Invalid)?;
        Ok(next)
    }

    /// Checks a deletion: only a draft can be deleted.
    pub(crate) fn check_delete(&self) -> Result<(), ChangeError> {
        if self.status == Status::Draft {
            Ok(())
        } else {
            Err(ChangeError::NotADraftToDelete(self.status))
        }
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::{
        Aire, ChangeError, Dates, DatesChanges, InvalidTournament, MAX_INTEGER, Points,
        PointsChanges, Ranking, Specialty, SpecialtyTarget, Tournament, TournamentChanges, Venue,
        VenueChanges,
    };
    use crate::domain::slug::Slug;
    use crate::domain::status::{InvalidTransition, Status};

    /// A valid draft at a fictional club and venue.
    fn draft() -> Tournament {
        Tournament {
            slug: Slug::parse("open-de-printemps-2027").expect("should be a slug"),
            name: "Open de printemps".to_owned(),
            organizing_club: "Club Exemple de Pelote".to_owned(),
            venue: Venue {
                name: "Trinquet de l'Exemple".to_owned(),
                address: "1 place de l'Exemple, Ville-Exemple".to_owned(),
                latitude: 43.0,
                longitude: -1.0,
            },
            aire: Aire::Trinquet,
            courts: vec!["Trinquet A".to_owned(), "Trinquet B".to_owned()],
            specialties: vec![SpecialtyTarget {
                specialty: Specialty::PaletaGommeCreuse,
                target_score: 40,
            }],
            points: Points::default(),
            ranking: Ranking::default(),
            dates: Dates {
                registration_opens: date(2027, 3, 1),
                registration_closes: date(2027, 3, 31),
                first_match: date(2027, 4, 10),
                finals: date(2027, 6, 19),
            },
            fee_cents: 2000,
            max_teams: 16,
            status: Status::Draft,
        }
    }

    /// The rule a tournament changed by `edit` breaks, if any.
    fn rule(edit: impl FnOnce(&mut Tournament)) -> Result<Tournament, InvalidTournament> {
        let mut tournament = draft();
        edit(&mut tournament);
        tournament.validate().map(|()| tournament)
    }

    fn field_of(result: Result<Tournament, InvalidTournament>) -> Option<&'static str> {
        result.err().map(|error| error.field)
    }

    #[test]
    fn a_valid_tournament_passes() {
        assert_eq!(rule(|_| {}), Ok(draft()), "the fixture is valid");
    }

    #[test]
    fn enums_round_trip_through_their_text_form() {
        for aire in [Aire::Fronton, Aire::MurAGauche, Aire::Trinquet] {
            assert_eq!(aire.as_str().parse::<Aire>(), Ok(aire), "{aire:?}");
        }
        assert_eq!(
            Aire::MurAGauche.as_str(),
            "mur_a_gauche",
            "the column holds snake_case"
        );
        assert_eq!("pelote".parse::<Aire>(), Err(()), "an unknown aire");
        for specialty in [Specialty::PaletaGommeCreuse, Specialty::PaletaGommePleine] {
            assert_eq!(
                specialty.as_str().parse::<Specialty>(),
                Ok(specialty),
                "{specialty:?}"
            );
        }
        assert_eq!(
            "main_nue".parse::<Specialty>(),
            Err(()),
            "a specialty txapela does not know yet"
        );
        assert_eq!(
            Ranking::default().as_str().parse::<Ranking>(),
            Ok(Ranking::PointsThenGoalAverage),
            "the default ranking"
        );
    }

    #[test]
    fn text_values_are_trimmed_and_cannot_be_empty() {
        let tournament = rule(|t| {
            t.name = "  Open de printemps \n".to_owned();
            t.venue.address = " 1 place de l'Exemple ".to_owned();
        })
        .expect("blanks around values are fine");
        assert_eq!(tournament.name, "Open de printemps", "name is trimmed");
        assert_eq!(
            tournament.venue.address, "1 place de l'Exemple",
            "venue.address is trimmed"
        );
        assert_eq!(field_of(rule(|t| t.name = " ".to_owned())), Some("name"));
        assert_eq!(
            field_of(rule(|t| t.organizing_club = String::new())),
            Some("organizing_club"),
            "the organizing club is required"
        );
        assert_eq!(
            field_of(rule(|t| t.venue.name = "\t".to_owned())),
            Some("venue.name"),
            "the venue name is required"
        );
        assert_eq!(
            field_of(rule(|t| t.venue.address = String::new())),
            Some("venue.address"),
            "the venue address is required"
        );
    }

    #[test]
    fn coordinates_stay_in_range() {
        for (latitude, longitude) in [(-90.0, -180.0), (90.0, 180.0), (0.0, 0.0)] {
            assert!(
                rule(|t| {
                    t.venue.latitude = latitude;
                    t.venue.longitude = longitude;
                })
                .is_ok(),
                "({latitude}, {longitude}) is on the globe"
            );
        }
        assert_eq!(
            field_of(rule(|t| t.venue.latitude = 90.5)),
            Some("venue.latitude"),
            "latitude above 90"
        );
        assert_eq!(
            field_of(rule(|t| t.venue.latitude = -91.0)),
            Some("venue.latitude"),
            "latitude below -90"
        );
        assert_eq!(
            field_of(rule(|t| t.venue.longitude = 180.1)),
            Some("venue.longitude"),
            "longitude above 180"
        );
        assert_eq!(
            field_of(rule(|t| t.venue.longitude = -200.0)),
            Some("venue.longitude"),
            "longitude below -180"
        );
    }

    #[test]
    fn courts_are_trimmed_unique_and_kept_in_order() {
        let tournament = rule(|t| {
            t.courts = vec![" Trinquet B ".to_owned(), "Trinquet A".to_owned()];
        })
        .expect("two distinct courts");
        assert_eq!(
            tournament.courts,
            vec!["Trinquet B".to_owned(), "Trinquet A".to_owned()],
            "names are trimmed, in the given order"
        );
        assert_eq!(
            rule(|t| t.courts = Vec::new()).map_err(|e| e.to_string()),
            Err("courts: holds at least one court".to_owned()),
            "at least one court"
        );
        assert_eq!(
            field_of(rule(|t| t.courts = vec!["  ".to_owned()])),
            Some("courts"),
            "an empty name"
        );
        assert_eq!(
            rule(|t| t.courts = vec!["Trinquet A".to_owned(), " Trinquet A".to_owned()])
                .map_err(|e| e.to_string()),
            Err("courts: the court name \"Trinquet A\" appears twice".to_owned()),
            "names are unique once trimmed"
        );
    }

    #[test]
    fn specialties_need_one_at_least_none_twice_and_a_target_above_zero() {
        let both = rule(|t| {
            t.specialties = vec![
                SpecialtyTarget {
                    specialty: Specialty::PaletaGommePleine,
                    target_score: 30,
                },
                SpecialtyTarget {
                    specialty: Specialty::PaletaGommeCreuse,
                    target_score: 40,
                },
            ];
        })
        .expect("two specialties");
        assert_eq!(
            both.specialties
                .iter()
                .map(|s| s.specialty)
                .collect::<Vec<_>>(),
            vec![Specialty::PaletaGommeCreuse, Specialty::PaletaGommePleine],
            "specialties are in a fixed order"
        );
        assert_eq!(
            rule(|t| t.specialties = Vec::new()).map_err(|e| e.to_string()),
            Err("specialties: holds at least one specialty".to_owned()),
            "at least one specialty"
        );
        assert_eq!(
            rule(|t| {
                let creuse = t.specialties[0];
                t.specialties = vec![creuse, creuse];
            })
            .map_err(|e| e.to_string()),
            Err("specialties: paleta_gomme_creuse appears twice".to_owned()),
            "no duplicate"
        );
        for target_score in [0, -1, MAX_INTEGER + 1] {
            assert_eq!(
                field_of(rule(|t| t.specialties[0].target_score = target_score)),
                Some("specialties"),
                "target score {target_score}"
            );
        }
        assert!(
            rule(|t| t.specialties[0].target_score = 1).is_ok(),
            "a target score of 1"
        );
    }

    #[test]
    fn points_are_non_negative_with_win_above_loss_at_least_forfeit() {
        assert_eq!(
            Points::default(),
            Points {
                win: 3,
                loss: 1,
                forfeit: 0
            },
            "defaults 3, 1, 0"
        );
        assert_eq!(
            PointsChanges {
                win: Some(4),
                ..PointsChanges::default()
            }
            .apply_to(Points::default()),
            Points {
                win: 4,
                loss: 1,
                forfeit: 0
            },
            "an absent value keeps its default"
        );
        for (win, loss, forfeit) in [(3, 1, 0), (2, 1, 1), (1, 0, 0), (5, 2, 2)] {
            assert!(
                rule(|t| t.points = Points { win, loss, forfeit }).is_ok(),
                "{win}, {loss}, {forfeit} are valid points"
            );
        }
        for (win, loss, forfeit, field) in [
            (1, 1, 0, "points"),
            (1, 2, 0, "points"),
            (3, 0, 1, "points"),
            (3, 1, -1, "points.forfeit"),
            (3, -1, -2, "points.loss"),
            (-1, -2, -3, "points.win"),
            (MAX_INTEGER + 1, 1, 0, "points.win"),
        ] {
            assert_eq!(
                field_of(rule(|t| t.points = Points { win, loss, forfeit })),
                Some(field),
                "{win}, {loss}, {forfeit} break {field}"
            );
        }
    }

    #[test]
    fn dates_are_in_order() {
        assert!(
            rule(|t| {
                let day = date(2027, 5, 1);
                t.dates = Dates {
                    registration_opens: day,
                    registration_closes: day,
                    first_match: day,
                    finals: day,
                };
            })
            .is_ok(),
            "four dates on one day are in order"
        );
        assert_eq!(
            rule(|t| t.dates.registration_closes = date(2027, 2, 28)).map_err(|e| e.to_string()),
            Err(
                "dates: registration_closes cannot be before registration_opens: \
                 registration_opens <= registration_closes <= first_match <= finals"
                    .to_owned()
            ),
            "registration closes before it opens"
        );
        assert_eq!(
            field_of(rule(|t| t.dates.first_match = date(2027, 3, 30))),
            Some("dates"),
            "first match before registration closes"
        );
        assert_eq!(
            field_of(rule(|t| t.dates.finals = date(2027, 4, 9))),
            Some("dates"),
            "finals before the first match"
        );
    }

    #[test]
    fn fee_and_maximum_teams_stay_in_range() {
        assert!(rule(|t| t.fee_cents = 0).is_ok(), "a free tournament");
        assert_eq!(
            field_of(rule(|t| t.fee_cents = -1)),
            Some("fee_cents"),
            "a negative fee"
        );
        assert_eq!(
            field_of(rule(|t| t.fee_cents = MAX_INTEGER + 1)),
            Some("fee_cents"),
            "a fee D1 cannot store exactly"
        );
        assert!(rule(|t| t.max_teams = 2).is_ok(), "two teams");
        assert_eq!(
            field_of(rule(|t| t.max_teams = 1)),
            Some("max_teams"),
            "one team is not a tournament"
        );
    }

    #[test]
    fn only_drafts_are_public_and_deletable() {
        let mut tournament = draft();
        assert!(!tournament.is_public(), "a draft is hidden");
        tournament.status = Status::Pools;
        assert_eq!(
            tournament.check_delete().map_err(|e| e.to_string()),
            Err("status: a pools tournament cannot be deleted, only a draft can".to_owned()),
            "the detail names the status"
        );
        tournament.status = Status::Draft;
        assert_eq!(tournament.check_delete(), Ok(()), "a draft can be deleted");
        for status in [
            Status::Registration,
            Status::Pools,
            Status::Finals,
            Status::Finished,
        ] {
            tournament.status = status;
            assert!(tournament.is_public(), "{status:?} is public");
            assert_eq!(
                tournament.check_delete(),
                Err(ChangeError::NotADraftToDelete(status)),
                "{status:?} cannot be deleted"
            );
        }
    }

    #[test]
    fn a_change_merges_the_patch_into_the_stored_values() {
        let changed = draft()
            .change(TournamentChanges {
                name: Some(" Open d'été ".to_owned()),
                venue: Some(VenueChanges {
                    latitude: Some(43.5),
                    ..VenueChanges::default()
                }),
                points: Some(PointsChanges {
                    loss: Some(2),
                    ..PointsChanges::default()
                }),
                dates: Some(DatesChanges {
                    finals: Some(date(2027, 6, 26)),
                    ..DatesChanges::default()
                }),
                ..TournamentChanges::default()
            })
            .expect("should change a draft");
        let mut expected = draft();
        expected.name = "Open d'été".to_owned();
        expected.venue.latitude = 43.5;
        expected.points.loss = 2;
        expected.dates.finals = date(2027, 6, 26);
        assert_eq!(changed, expected, "absent fields keep their stored value");
        assert_eq!(
            draft().change(TournamentChanges::default()),
            Ok(draft()),
            "an empty patch changes nothing"
        );
    }

    #[test]
    fn a_change_keeps_the_four_dates_in_order() {
        assert_eq!(
            draft().change(TournamentChanges {
                dates: Some(DatesChanges {
                    first_match: Some(date(2027, 7, 1)),
                    ..DatesChanges::default()
                }),
                ..TournamentChanges::default()
            }),
            Err(ChangeError::Invalid(InvalidTournament {
                field: "dates",
                reason: "finals cannot be before first_match: registration_opens <= \
                         registration_closes <= first_match <= finals"
                    .to_owned(),
            })),
            "a first match after the stored finals"
        );
    }

    #[test]
    fn rules_change_only_in_draft() {
        let mut registration = draft();
        registration.status = Status::Registration;
        let draft_only = [
            (
                "aire",
                TournamentChanges {
                    aire: Some(Aire::Fronton),
                    ..TournamentChanges::default()
                },
            ),
            (
                "specialties",
                TournamentChanges {
                    specialties: Some(draft().specialties),
                    ..TournamentChanges::default()
                },
            ),
            (
                "points",
                TournamentChanges {
                    points: Some(PointsChanges::default()),
                    ..TournamentChanges::default()
                },
            ),
            (
                "ranking",
                TournamentChanges {
                    ranking: Some(Ranking::PointsThenGoalAverage),
                    ..TournamentChanges::default()
                },
            ),
            (
                "courts",
                TournamentChanges {
                    courts: Some(vec!["Trinquet C".to_owned()]),
                    ..TournamentChanges::default()
                },
            ),
        ];
        for (field, changes) in draft_only {
            assert!(
                draft().change(changes.clone()).is_ok(),
                "a draft changes {field}"
            );
            assert_eq!(
                registration.change(changes),
                Err(ChangeError::NotADraft(field)),
                "{field} is fixed once the tournament leaves draft, even to its current value"
            );
        }
        let renamed = registration
            .change(TournamentChanges {
                name: Some("Open d'automne".to_owned()),
                organizing_club: Some("Club Exemple".to_owned()),
                venue: Some(VenueChanges {
                    name: Some("Fronton de l'Exemple".to_owned()),
                    ..VenueChanges::default()
                }),
                dates: Some(DatesChanges {
                    finals: Some(date(2027, 6, 20)),
                    ..DatesChanges::default()
                }),
                fee_cents: Some(2500),
                max_teams: Some(24),
                ..TournamentChanges::default()
            })
            .expect("the other fields stay editable");
        assert_eq!(renamed.status, Status::Registration, "status unchanged");
        assert_eq!(renamed.max_teams, 24, "max_teams changed");
    }

    #[test]
    fn status_moves_one_step_forward_through_a_change() {
        let moved = draft()
            .change(TournamentChanges {
                status: Some(Status::Registration),
                aire: Some(Aire::MurAGauche),
                ..TournamentChanges::default()
            })
            .expect("a draft opens registration and changes its aire on the way");
        assert_eq!(moved.status, Status::Registration, "one step forward");
        assert_eq!(
            moved.aire,
            Aire::MurAGauche,
            "the status before the patch was draft"
        );
        assert_eq!(
            draft().change(TournamentChanges {
                status: Some(Status::Pools),
                ..TournamentChanges::default()
            }),
            Err(ChangeError::InvalidTransition(InvalidTransition {
                from: Status::Draft,
                to: Status::Pools,
            })),
            "a skipped step"
        );
        assert_eq!(
            draft().change(TournamentChanges {
                status: Some(Status::Draft),
                ..TournamentChanges::default()
            }),
            Err(ChangeError::InvalidTransition(InvalidTransition {
                from: Status::Draft,
                to: Status::Draft,
            })),
            "the current status is not a step"
        );
    }

    #[test]
    fn change_errors_come_in_order() {
        let mut pools = draft();
        pools.status = Status::Pools;
        assert_eq!(
            pools.change(TournamentChanges {
                aire: Some(Aire::Fronton),
                status: Some(Status::Draft),
                name: Some(String::new()),
                ..TournamentChanges::default()
            }),
            Err(ChangeError::NotADraft("aire")),
            "not-a-draft first"
        );
        assert!(
            matches!(
                pools.change(TournamentChanges {
                    status: Some(Status::Registration),
                    name: Some(String::new()),
                    ..TournamentChanges::default()
                }),
                Err(ChangeError::InvalidTransition(_))
            ),
            "then the transition"
        );
        assert!(
            matches!(
                pools.change(TournamentChanges {
                    status: Some(Status::Finals),
                    name: Some(String::new()),
                    ..TournamentChanges::default()
                }),
                Err(ChangeError::Invalid(InvalidTournament {
                    field: "name",
                    ..
                }))
            ),
            "then the rules on the merged state"
        );
    }
}
