// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! A player of a team, as the organizer enters it: names, club, licence number and contact.
//!
//! Names and club are public. The licence number and the contact (phone or email) are personal
//! data, for the organizer only: the API leaves them out of public responses, and no rule here
//! quotes a value in its message, so that none reaches a problem detail or a log.

use super::rule::{Invalid, invalid, optional_text, required_text};

/// The players of a team.
pub(crate) const PLAYERS: usize = 2;

/// The names of a player's fields in a problem detail.
struct Fields {
    first_name: &'static str,
    last_name: &'static str,
    club: &'static str,
    licence: &'static str,
    contact: &'static str,
}

/// The fields of the first and the second player, as `players[0].first_name`.
const FIELDS: [Fields; PLAYERS] = [
    Fields {
        first_name: "players[0].first_name",
        last_name: "players[0].last_name",
        club: "players[0].club",
        licence: "players[0].licence",
        contact: "players[0].contact",
    },
    Fields {
        first_name: "players[1].first_name",
        last_name: "players[1].last_name",
        club: "players[1].club",
        licence: "players[1].licence",
        contact: "players[1].contact",
    },
];

/// A player: a `players` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Player {
    pub(crate) first_name: String,
    pub(crate) last_name: String,
    /// Free text until clubs have their own records.
    pub(crate) club: Option<String>,
    /// Organizer only.
    pub(crate) licence: Option<String>,
    /// A phone number or an email address, organizer only.
    pub(crate) contact: Option<String>,
}

impl Player {
    /// The player with its text values trimmed: names cannot be empty, and an optional value,
    /// when given, cannot be empty either.
    fn checked(self, fields: &Fields) -> Result<Self, Invalid> {
        Ok(Self {
            first_name: required_text(fields.first_name, &self.first_name)?,
            last_name: required_text(fields.last_name, &self.last_name)?,
            club: optional_text(fields.club, self.club.as_deref())?,
            licence: optional_text(fields.licence, self.licence.as_deref())?,
            contact: optional_text(fields.contact, self.contact.as_deref())?,
        })
    }
}

/// The two players of a team, checked, in the given order.
pub(crate) fn pair(players: Vec<Player>) -> Result<[Player; PLAYERS], Invalid> {
    let [first, second]: [Player; PLAYERS] = players
        .try_into()
        .map_err(|_| invalid("players", "holds exactly two players"))?;
    let [first_fields, second_fields] = &FIELDS;
    Ok([first.checked(first_fields)?, second.checked(second_fields)?])
}

#[cfg(test)]
mod tests {
    use super::{Player, pair};

    /// A fictional player.
    fn player(first_name: &str, last_name: &str) -> Player {
        Player {
            first_name: first_name.to_owned(),
            last_name: last_name.to_owned(),
            club: Some("Club Exemple de Pelote".to_owned()),
            licence: Some("LIC-EXEMPLE-0001".to_owned()),
            contact: Some("joueur.exemple@example.org".to_owned()),
        }
    }

    fn field_of(players: Vec<Player>) -> Option<&'static str> {
        pair(players).err().map(|error| error.field)
    }

    #[test]
    fn a_team_has_exactly_two_players_in_the_given_order() {
        let [first, second] = pair(vec![player("Ane", "Exemple"), player("Beñat", "Fictif")])
            .expect("two valid players");
        assert_eq!(first.first_name, "Ane", "the first player stays first");
        assert_eq!(second.last_name, "Fictif", "the second player stays second");
        for count in [0, 1, 3] {
            let players = vec![player("Ane", "Exemple"); count];
            assert_eq!(
                pair(players).map_err(|error| error.to_string()),
                Err("players: holds exactly two players".to_owned()),
                "{count} players"
            );
        }
    }

    #[test]
    fn values_are_trimmed_names_are_required_and_optional_values_absent_or_not_empty() {
        let mut first = player("  Ane ", " Exemple ");
        first.contact = Some(" +00 0 00 00 00 00 ".to_owned());
        let mut second = player("Beñat", "Fictif");
        second.club = None;
        second.licence = None;
        second.contact = None;
        let [first, second] = pair(vec![first, second]).expect("valid players");
        assert_eq!(
            (first.first_name.as_str(), first.last_name.as_str()),
            ("Ane", "Exemple"),
            "names are trimmed"
        );
        assert_eq!(
            first.contact.as_deref(),
            Some("+00 0 00 00 00 00"),
            "the contact is trimmed"
        );
        assert_eq!(
            (second.club, second.licence, second.contact),
            (None, None, None),
            "optional values may be absent"
        );

        let mut blank = player("Ane", "Exemple");
        blank.first_name = " ".to_owned();
        assert_eq!(
            field_of(vec![blank, player("Beñat", "Fictif")]),
            Some("players[0].first_name"),
            "a first name is required"
        );
        let mut blank = player("Beñat", "Fictif");
        blank.last_name = String::new();
        assert_eq!(
            field_of(vec![player("Ane", "Exemple"), blank]),
            Some("players[1].last_name"),
            "a last name is required, and the detail names the second player"
        );
        for (edit, field) in [
            (
                (|p: &mut Player| p.club = Some(" ".to_owned())) as fn(&mut Player),
                "players[1].club",
            ),
            (|p| p.licence = Some(String::new()), "players[1].licence"),
            (|p| p.contact = Some("\t".to_owned()), "players[1].contact"),
        ] {
            let mut second = player("Beñat", "Fictif");
            edit(&mut second);
            assert_eq!(
                field_of(vec![player("Ane", "Exemple"), second]),
                Some(field),
                "{field} cannot be empty when given"
            );
        }
    }

    #[test]
    fn no_message_quotes_a_value() {
        let mut first = player("Ane", "Exemple");
        first.first_name = "   ".to_owned();
        first.contact = Some("secret.exemple@example.org".to_owned());
        let detail = pair(vec![first, player("Beñat", "Fictif")])
            .map_err(|error| error.to_string())
            .expect_err("should refuse the blank first name");
        assert_eq!(
            detail, "players[0].first_name: cannot be empty",
            "field and rule only"
        );
        assert!(
            !detail.contains("example.org"),
            "the contact is never quoted"
        );
    }
}
