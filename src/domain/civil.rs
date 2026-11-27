// SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
// SPDX-License-Identifier: Apache-2.0

//! Civil dates as text: `YYYY-MM-DD`, as stored in D1 and sent in bodies. They are dates in
//! Europe/Paris, without a time of day: times come with the calendar (#9).

use jiff::civil::Date;

/// The two-digit number at `bytes[at..at + 2]`.
fn two_digits(bytes: &[u8], at: usize) -> Option<i8> {
    let tens = bytes.get(at).filter(|b| b.is_ascii_digit())?;
    let units = bytes.get(at + 1).filter(|b| b.is_ascii_digit())?;
    i8::try_from((tens - b'0') * 10 + (units - b'0')).ok()
}

/// A `YYYY-MM-DD` date. Any other form, or a day that does not exist, is `None`.
pub(crate) fn parse_date(s: &str) -> Option<Date> {
    let bytes = s.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year = s.get(0..4)?;
    if !year.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: i16 = year.parse().ok()?;
    Date::new(year, two_digits(bytes, 5)?, two_digits(bytes, 8)?).ok()
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::parse_date;

    #[test]
    fn dates_are_yyyy_mm_dd_only() {
        assert_eq!(
            parse_date("2027-05-14"),
            Some(date(2027, 5, 14)),
            "a valid date"
        );
        assert_eq!(
            date(2027, 5, 14).to_string(),
            "2027-05-14",
            "jiff writes the same form"
        );
        assert_eq!(
            parse_date("2028-02-29"),
            Some(date(2028, 2, 29)),
            "a leap day"
        );
        for invalid in [
            "2027-02-29",
            "2027-02-30",
            "2027-13-01",
            "20270514",
            "2027-05-14T00:00",
            "27-05-14",
            "2027-5-14",
            "+002027-05-14",
            "2027/05/14",
            "",
        ] {
            assert_eq!(parse_date(invalid), None, "{invalid:?} is not a date");
        }
        assert_eq!(
            parse_date("9999-12-31"),
            Some(date(9999, 12, 31)),
            "the last day jiff can represent"
        );
    }

    #[test]
    fn only_ascii_digits_form_the_year_month_and_day() {
        for invalid in [
            "a027-05-14",
            "2027-a5-14",
            "2027-05-a4",
            "２０２７-０５-１４",
            "2027-05-14\n",
        ] {
            assert_eq!(parse_date(invalid), None, "{invalid:?} is not a date");
        }
    }
}
