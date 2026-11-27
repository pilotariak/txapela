// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! A broken rule, and the checks that tournaments, series, teams and players share.
//!
//! The API answers 422 `invalid-request` with the text of [`Invalid`], which names the field and
//! the rule. It never quotes the value: a player's name or contact never reaches a problem detail.

use core::fmt;

/// The largest integer that D1 stores exactly: values travel as JavaScript numbers.
pub(crate) const MAX_INTEGER: i64 = (1 << 53) - 1;

/// A value that breaks a rule: the API answers 422 with this text, which names the field and the
/// rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Invalid {
    pub(crate) field: &'static str,
    pub(crate) reason: String,
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}

impl std::error::Error for Invalid {}

pub(crate) fn invalid(field: &'static str, reason: impl Into<String>) -> Invalid {
    Invalid {
        field,
        reason: reason.into(),
    }
}

/// Text trimmed of surrounding blanks, which cannot be empty.
pub(crate) fn required_text(field: &'static str, value: &str) -> Result<String, Invalid> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid(field, "cannot be empty"));
    }
    Ok(value.to_owned())
}

/// Optional text: absent stays absent, and a value present is trimmed and cannot be empty.
pub(crate) fn optional_text(
    field: &'static str,
    value: Option<&str>,
) -> Result<Option<String>, Invalid> {
    value.map(|value| required_text(field, value)).transpose()
}

/// An integer from `min` to [`MAX_INTEGER`].
pub(crate) fn integer(field: &'static str, value: i64, min: i64) -> Result<(), Invalid> {
    if (min..=MAX_INTEGER).contains(&value) {
        Ok(())
    } else {
        Err(invalid(
            field,
            format!("is an integer from {min} to {MAX_INTEGER}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{Invalid, MAX_INTEGER, integer, invalid, optional_text, required_text};

    #[test]
    fn the_text_names_the_field_and_the_rule() {
        assert_eq!(
            invalid("name", "cannot be empty").to_string(),
            "name: cannot be empty",
            "field, then rule"
        );
    }

    #[test]
    fn required_text_is_trimmed_and_not_empty() {
        assert_eq!(
            required_text("name", "  Open \n"),
            Ok("Open".to_owned()),
            "trimmed"
        );
        assert_eq!(
            required_text("name", " \t"),
            Err(Invalid {
                field: "name",
                reason: "cannot be empty".to_owned(),
            }),
            "blanks only"
        );
    }

    #[test]
    fn optional_text_is_absent_or_not_empty() {
        assert_eq!(optional_text("club", None), Ok(None), "absent stays absent");
        assert_eq!(
            optional_text("club", Some(" Club Exemple ")),
            Ok(Some("Club Exemple".to_owned())),
            "trimmed"
        );
        assert_eq!(
            optional_text("club", Some(" ")).map_err(|error| error.field),
            Err("club"),
            "present but empty"
        );
    }

    #[test]
    fn integers_stay_within_what_d1_stores_exactly() {
        assert_eq!(integer("n", 0, 0), Ok(()), "the minimum");
        assert_eq!(integer("n", MAX_INTEGER, 0), Ok(()), "the maximum");
        assert!(integer("n", -1, 0).is_err(), "below the minimum");
        assert!(
            integer("n", MAX_INTEGER + 1, 0).is_err(),
            "above what D1 stores exactly"
        );
    }
}
