// SPDX-License-Identifier: AGPL-3.0-or-later
//! Textual device IDs (protocol.md §2.2). Derivation lives in `nestwatch-crypto`.

use core::fmt;
use core::str::FromStr;

use thiserror::Error;

/// Number of characters in a device ID (protocol.md §2.2).
pub const DEVICE_ID_LEN: usize = 16;

/// Error for a malformed device ID.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid device id: expected 16 lower-case base32 characters")]
pub struct DeviceIdError;

/// A device ID: 16 lower-case RFC 4648 base32 characters (protocol.md §2.2).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(String);

impl DeviceId {
    /// Validates the canonical 16-character form.
    pub fn parse(s: &str) -> Result<Self, DeviceIdError> {
        let ok = s.len() == DEVICE_ID_LEN
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b));
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            Err(DeviceIdError)
        }
    }

    /// Parses user input in grouped form (`abcd-efgh-ijkl-mnop`), ignoring
    /// hyphens, spaces and case.
    pub fn parse_grouped(s: &str) -> Result<Self, DeviceIdError> {
        let compact: String = s
            .chars()
            .filter(|c| *c != '-' && !c.is_whitespace())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        Self::parse(&compact)
    }

    /// The canonical 16-character form.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The grouped display form `abcd-efgh-ijkl-mnop` (protocol.md §2.2).
    pub fn grouped(&self) -> String {
        let b = self.0.as_str();
        format!("{}-{}-{}-{}", &b[0..4], &b[4..8], &b[8..12], &b[12..16])
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for DeviceId {
    type Err = DeviceIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_canonical() {
        let id = DeviceId::parse("abcdefghijklmn27").unwrap();
        assert_eq!(id.as_str(), "abcdefghijklmn27");
        assert_eq!(id.grouped(), "abcd-efgh-ijkl-mn27");
    }

    #[test]
    fn rejects_bad_ids() {
        for bad in [
            "",
            "abcdefghijklmn2",
            "abcdefghijklmn278",
            "ABCDEFGHIJKLMN27",
            "abcdefghijklmn18",
            "abcd-efgh-ijkl-m",
        ] {
            assert_eq!(DeviceId::parse(bad), Err(DeviceIdError), "{bad}");
        }
    }

    #[test]
    fn parse_grouped_input() {
        let id = DeviceId::parse_grouped(" ABCD-efgh ijkl-MN27 ").unwrap();
        assert_eq!(id.as_str(), "abcdefghijklmn27");
    }
}
