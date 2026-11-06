use std::fmt;
use std::str::FromStr;

/// A release's place in the `YYYY.MM.DD-N` sequence: the UTC date it was built and its ordinal
/// among that day's releases.
///
/// Orders chronologically, and `-10` follows `-9`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReleaseVersion {
    year: u16,
    month: u8,
    day: u8,
    build: u32,
}

impl fmt::Display for ReleaseVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}.{:02}.{:02}-{}",
            self.year, self.month, self.day, self.build
        )
    }
}

/// A string that is not a `YYYY.MM.DD-N` release version.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{input:?} is not a YYYY.MM.DD-N release version")]
pub struct ParseVersionError {
    input: String,
}

impl FromStr for ReleaseVersion {
    type Err = ParseVersionError;

    /// Parse `YYYY.MM.DD-N`, with or without a leading `v`.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let invalid = || ParseVersionError {
            input: input.to_string(),
        };
        let unprefixed = input.strip_prefix('v').unwrap_or(input);
        let (date, build) = unprefixed.split_once('-').ok_or_else(invalid)?;
        let mut fields = date.split('.');
        let (Some(year), Some(month), Some(day), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(invalid());
        };
        let version = ReleaseVersion {
            year: number(year).ok_or_else(invalid)?,
            month: number(month).ok_or_else(invalid)?,
            day: number(day).ok_or_else(invalid)?,
            build: number(build).ok_or_else(invalid)?,
        };
        let in_calendar = (1..=12).contains(&version.month) && (1..=31).contains(&version.day);
        if in_calendar {
            Ok(version)
        } else {
            Err(invalid())
        }
    }
}

/// Parse a non-empty run of ASCII digits, rejecting the signs `str::parse` would accept.
fn number<T: FromStr>(digits: &str) -> Option<T> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> ReleaseVersion {
        s.parse().unwrap()
    }

    #[test]
    fn parses_displays_and_rejects() {
        assert_eq!(v("2026.09.09-1"), v("v2026.09.09-1"));
        assert_eq!(v("2026.09.09-1").to_string(), "2026.09.09-1");
        assert_eq!(v("2026.9.9-12").to_string(), "2026.09.09-12");

        for bad in [
            "test",
            "2026.10.17",
            "2026.10.17-",
            "2026.10-1",
            "2026.10.17.1-1",
            "2026.13.01-1",
            "2026.00.01-1",
            "2026.10.32-1",
            "2026.10.17-+1",
            "vv2026.10.17-1",
            "",
        ] {
            assert_eq!(
                bad.parse::<ReleaseVersion>(),
                Err(ParseVersionError {
                    input: bad.to_string()
                }),
                "{bad:?} should not parse",
            );
        }
    }

    #[test]
    fn orders_chronologically() {
        let ascending = [
            "2025.12.31-3",
            "2026.01.01-1",
            "2026.01.09-1",
            "2026.01.10-1",
            "2026.10.17-9",
            "2026.10.17-10",
            "2026.10.18-1",
        ];
        for pair in ascending.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
    }
}
