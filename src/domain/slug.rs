// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The slug of a tournament: its public key, in URLs and in the API. The organizer chooses it at
//! creation, and it never changes.

use core::fmt;

/// Shortest slug, in characters.
const MIN_CHARS: usize = 3;
/// Longest slug, in characters.
const MAX_CHARS: usize = 64;

/// What a slug looks like: the `detail` of the 422 answered to another value.
pub(crate) const SLUG_RULE: &str = "slug: 3 to 64 characters among a-z, 0-9 and single hyphens, \
    starting and ending with a letter or a digit";

/// A tournament slug: 3 to 64 lowercase ASCII letters, digits and single hyphens, starting and
/// ending with a letter or a digit, such as `open-trinquet-2027`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Slug(String);

impl Slug {
    /// A slug, or `None` when `s` breaks the rule ([`SLUG_RULE`]).
    pub(crate) fn parse(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        let alphanumeric = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
        let valid = (MIN_CHARS..=MAX_CHARS).contains(&bytes.len())
            && bytes.first().is_some_and(alphanumeric)
            && bytes.last().is_some_and(alphanumeric)
            && bytes.iter().all(|b| alphanumeric(b) || *b == b'-')
            && !s.contains("--");
        valid.then(|| Self(s.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::Slug;

    #[test]
    fn accepts_lowercase_letters_digits_and_single_hyphens() {
        for valid in [
            "abc",
            "open-trinquet-2027",
            "2027",
            "a-b",
            "x1y",
            "tournoi-d-ete",
            &"a".repeat(64),
        ] {
            assert_eq!(
                Slug::parse(valid).map(|slug| slug.as_str().to_owned()),
                Some(valid.to_owned()),
                "{valid:?} is a slug"
            );
        }
    }

    #[test]
    fn rejects_other_shapes() {
        for invalid in [
            "",
            "ab",
            &"a".repeat(65),
            "-abc",
            "abc-",
            "ab--c",
            "Open-2027",
            "open_2027",
            "open 2027",
            "open.2027",
            "café-2027",
            "ab/c",
        ] {
            assert_eq!(Slug::parse(invalid), None, "{invalid:?} is not a slug");
        }
    }
}
