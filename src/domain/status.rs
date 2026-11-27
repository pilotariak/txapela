// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The status of a tournament and its transitions: `draft` → `registration` → `pools` →
//! `finals` → `finished`, one step forward at a time.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Serialize};

/// `tournaments.status`. A new tournament is a draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(utoipa::ToSchema))]
pub(crate) enum Status {
    /// Being set up: only organizers see it.
    Draft,
    /// Teams register.
    Registration,
    /// Pool matches.
    Pools,
    /// The final phase.
    Finals,
    /// Over: the results are final.
    Finished,
}

/// A status change other than one step forward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InvalidTransition {
    pub(crate) from: Status,
    pub(crate) to: Status,
}

impl fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.from.next() {
            Some(next) => write!(
                f,
                "status: a {} tournament moves to {} only, not to {}",
                self.from.as_str(),
                next.as_str(),
                self.to.as_str()
            ),
            None => write!(
                f,
                "status: a {} tournament cannot move to {}",
                self.from.as_str(),
                self.to.as_str()
            ),
        }
    }
}

impl std::error::Error for InvalidTransition {}

/// A change to the series or teams of a tournament whose registration is over: its status is
/// past `registration`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RegistrationClosed(pub(crate) Status);

impl fmt::Display for RegistrationClosed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "status: series and teams change only while the tournament is draft or \
             registration, not {}",
            self.0.as_str()
        )
    }
}

impl std::error::Error for RegistrationClosed {}

/// A draw, a move, a tiebreak or a result while the tournament is not in its pools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotInPools(pub(crate) Status);

impl fmt::Display for NotInPools {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "status: pools are drawn and played while the tournament is pools, not {}",
            self.0.as_str()
        )
    }
}

impl std::error::Error for NotInPools {}

impl Status {
    /// The statuses in which series and teams change. Once the tournament moves to `pools`, the
    /// registered teams are fixed: a team that withdraws then stays in its pool, and the
    /// organizer records its remaining matches as forfeits.
    pub(crate) const REGISTRATION_OPEN: [Self; 2] = [Self::Draft, Self::Registration];

    /// Checks that series and teams can still change: in [`Status::REGISTRATION_OPEN`] only.
    pub(crate) fn check_registration_open(self) -> Result<(), RegistrationClosed> {
        if Self::REGISTRATION_OPEN.contains(&self) {
            Ok(())
        } else {
            Err(RegistrationClosed(self))
        }
    }

    /// Checks that pools can be drawn, changed and played: in `pools` only. Registration is
    /// closed by then, so the teams of a draw are fixed, and the final phase (#10) starts from
    /// standings that no longer change.
    pub(crate) fn check_pools(self) -> Result<(), NotInPools> {
        if self == Self::Pools {
            Ok(())
        } else {
            Err(NotInPools(self))
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Registration => "registration",
            Self::Pools => "pools",
            Self::Finals => "finals",
            Self::Finished => "finished",
        }
    }

    /// The status one step forward, `None` once finished.
    pub(crate) fn next(self) -> Option<Self> {
        match self {
            Self::Draft => Some(Self::Registration),
            Self::Registration => Some(Self::Pools),
            Self::Pools => Some(Self::Finals),
            Self::Finals => Some(Self::Finished),
            Self::Finished => None,
        }
    }

    /// Checks a move to `to`: only the next status is allowed. Staying on the same status is not
    /// a step forward either.
    pub(crate) fn move_to(self, to: Self) -> Result<Self, InvalidTransition> {
        if self.next() == Some(to) {
            Ok(to)
        } else {
            Err(InvalidTransition { from: self, to })
        }
    }
}

impl FromStr for Status {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        match s {
            "draft" => Ok(Self::Draft),
            "registration" => Ok(Self::Registration),
            "pools" => Ok(Self::Pools),
            "finals" => Ok(Self::Finals),
            "finished" => Ok(Self::Finished),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InvalidTransition, NotInPools, RegistrationClosed, Status};

    const ALL: [Status; 5] = [
        Status::Draft,
        Status::Registration,
        Status::Pools,
        Status::Finals,
        Status::Finished,
    ];

    #[test]
    fn moves_one_step_forward_only() {
        let steps = [
            (Status::Draft, Status::Registration),
            (Status::Registration, Status::Pools),
            (Status::Pools, Status::Finals),
            (Status::Finals, Status::Finished),
        ];
        for from in ALL {
            for to in ALL {
                let allowed = steps.contains(&(from, to));
                let result = from.move_to(to);
                if allowed {
                    assert_eq!(result, Ok(to), "{from:?} moves to {to:?}");
                } else {
                    assert_eq!(
                        result,
                        Err(InvalidTransition { from, to }),
                        "{from:?} cannot move to {to:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn staying_on_the_same_status_is_not_a_move() {
        for status in ALL {
            assert!(
                status.move_to(status).is_err(),
                "{status:?} to itself is not a step forward"
            );
        }
    }

    #[test]
    fn the_detail_names_the_allowed_step() {
        assert_eq!(
            Status::Draft
                .move_to(Status::Pools)
                .map_err(|e| e.to_string()),
            Err("status: a draft tournament moves to registration only, not to pools".to_owned()),
            "a skipped step"
        );
        assert_eq!(
            Status::Finished
                .move_to(Status::Draft)
                .map_err(|e| e.to_string()),
            Err("status: a finished tournament cannot move to draft".to_owned()),
            "the last status"
        );
    }

    #[test]
    fn series_and_teams_change_in_draft_and_registration_only() {
        for status in ALL {
            let open = matches!(status, Status::Draft | Status::Registration);
            assert_eq!(
                status.check_registration_open(),
                if open {
                    Ok(())
                } else {
                    Err(RegistrationClosed(status))
                },
                "{status:?}"
            );
        }
        assert_eq!(
            RegistrationClosed(Status::Pools).to_string(),
            "status: series and teams change only while the tournament is draft or \
             registration, not pools",
            "the detail names the status"
        );
    }

    #[test]
    fn pools_change_and_play_in_pools_only() {
        for status in ALL {
            assert_eq!(
                status.check_pools(),
                if status == Status::Pools {
                    Ok(())
                } else {
                    Err(NotInPools(status))
                },
                "{status:?}"
            );
        }
        assert_eq!(
            NotInPools(Status::Finals).to_string(),
            "status: pools are drawn and played while the tournament is pools, not finals",
            "the detail names the status"
        );
    }

    #[test]
    fn text_form_round_trips() {
        for status in ALL {
            assert_eq!(
                status.as_str().parse::<Status>(),
                Ok(status),
                "{status:?} round trips through its text form"
            );
        }
        assert_eq!(
            "Draft".parse::<Status>(),
            Err(()),
            "the text form is lowercase"
        );
        assert_eq!("".parse::<Status>(), Err(()), "an empty status");
    }
}
