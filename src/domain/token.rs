// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! The organizer token of the interim check (`api/auth.rs`, until the ADR of #5): a bearer token
//! compared by its SHA-256 with the `ORGANIZER_TOKEN_SHA256` Worker secret. txapela never stores
//! or logs a token or its hash.

use core::fmt;

use sha2::{Digest as _, Sha256};

/// Length of a SHA-256 in hexadecimal.
const SHA256_HEX_CHARS: usize = 64;

/// SHA-256 of `bytes`.
pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Lowercase hexadecimal of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    out
}

/// Whether `s` is a SHA-256 in lowercase hexadecimal, the only form of the secret that the check
/// accepts.
pub(crate) fn is_sha256_hex(s: &str) -> bool {
    s.len() == SHA256_HEX_CHARS
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether two strings are equal, in a time that depends on their length only.
fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0_u8, |diff, (x, y)| diff | (x ^ y))
            == 0
}

/// A token sent as `Authorization: Bearer <token>`.
///
/// `Debug` hides the value, so that a token never reaches a log by accident.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct OrganizerToken(String);

impl OrganizerToken {
    /// The token of an `Authorization` header: the `Bearer` scheme (any case, RFC 9110), one
    /// space, and a non-empty `b64token` (RFC 6750). `None` for any other value, which the caller
    /// answers like a wrong token, without hashing it.
    pub(crate) fn from_authorization(header: &str) -> Option<Self> {
        let (scheme, token) = header.split_once(' ')?;
        if !scheme.eq_ignore_ascii_case("Bearer") {
            return None;
        }
        let body = token.trim_end_matches('=');
        let valid = !body.is_empty()
            && body
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~+/".contains(&b));
        valid.then(|| Self(token.to_owned()))
    }

    /// Whether the SHA-256 of the token, in lowercase hexadecimal, is `expected`.
    pub(crate) fn matches(&self, expected: &str) -> bool {
        is_sha256_hex(expected) && constant_time_eq(&hex(&sha256(self.0.as_bytes())), expected)
    }
}

impl fmt::Debug for OrganizerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OrganizerToken(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::{OrganizerToken, constant_time_eq, hex, is_sha256_hex, sha256};

    #[test]
    fn sha256_matches_the_fips_180_2_vector() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "SHA-256 of abc"
        );
    }

    #[test]
    fn reads_a_bearer_token() {
        let token = OrganizerToken::from_authorization("Bearer abc.DEF-123_~+/==")
            .expect("should read a b64token");
        let hash = hex(&sha256(b"abc.DEF-123_~+/=="));
        assert!(token.matches(&hash), "the token is compared by its SHA-256");
        assert_eq!(
            OrganizerToken::from_authorization("bearer abc"),
            OrganizerToken::from_authorization("Bearer abc"),
            "the scheme is case-insensitive"
        );
        assert_eq!(
            format!("{token:?}"),
            "OrganizerToken(<redacted>)",
            "Debug never shows a token"
        );
    }

    #[test]
    fn rejects_other_authorization_values() {
        for invalid in [
            "",
            "Bearer",
            "Bearer ",
            "Bearer  abc",
            "Bearer ===",
            "Bearer a b",
            "Bearer a=b",
            "Bearer abc\t",
            "Basic YWJjOmRlZg==",
            "Token abc",
            "abc",
        ] {
            assert_eq!(
                OrganizerToken::from_authorization(invalid),
                None,
                "{invalid:?} carries no bearer token"
            );
        }
    }

    #[test]
    fn a_wrong_token_or_a_malformed_secret_does_not_match() {
        let token = OrganizerToken::from_authorization("Bearer fictional-organizer-token")
            .expect("should read the token");
        let expected = hex(&sha256(b"fictional-organizer-token"));
        assert!(token.matches(&expected), "the right hash");
        assert!(
            !token.matches(&hex(&sha256(b"another-token"))),
            "the hash of another token"
        );
        assert!(
            !token.matches(&expected.to_uppercase()),
            "the secret is lowercase hexadecimal"
        );
        assert!(!token.matches(""), "an empty secret never matches");
        assert!(
            !token.matches(&format!("{expected}\n")),
            "a secret with a newline is malformed"
        );
        let empty_hash = hex(&sha256(b""));
        assert!(
            OrganizerToken::from_authorization("Bearer ").is_none() && is_sha256_hex(&empty_hash),
            "no header value hashes to the SHA-256 of an empty token"
        );
    }

    #[test]
    fn a_b64token_keeps_its_padding_when_hashed() {
        let token = OrganizerToken::from_authorization("Bearer aB3-._~+/xy==")
            .expect("should read a padded b64token");
        assert!(
            token.matches(&hex(&sha256(b"aB3-._~+/xy=="))),
            "the padding and the punctuation are part of the token"
        );
        assert!(
            !token.matches(&hex(&sha256(b"aB3-._~+/xy"))),
            "the padding is not stripped before hashing"
        );
    }

    #[test]
    fn recognizes_lowercase_sha256_hex() {
        let hash = hex(&sha256(b"fictional-organizer-token"));
        assert!(is_sha256_hex(&hash), "64 lowercase hex digits");
        assert!(!is_sha256_hex(&hash[1..]), "63 digits");
        assert!(!is_sha256_hex(&hash.to_uppercase()), "uppercase");
        assert!(!is_sha256_hex(&"g".repeat(64)), "not hexadecimal");
    }

    #[test]
    fn compares_in_constant_time_by_length_only() {
        assert!(constant_time_eq("abc", "abc"), "equal");
        assert!(!constant_time_eq("abc", "abd"), "last byte differs");
        assert!(!constant_time_eq("abc", "abcd"), "lengths differ");
    }
}
