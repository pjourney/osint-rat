//! Confidentiality markings (Traffic Light Protocol) on investigations and objects.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Reserved property-bag key for an optional object-level marking.
pub const OBJECT_MARKING_FIELD: &str = "marking";

/// Traffic Light Protocol label, or unmarked.
///
/// Stored as a stable lowercase key (`tlp:amber`). Display uses the `TLP:` prefix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Marking {
    #[default]
    #[serde(rename = "unmarked")]
    Unmarked,
    #[serde(rename = "tlp:clear")]
    TlpClear,
    #[serde(rename = "tlp:green")]
    TlpGreen,
    #[serde(rename = "tlp:amber")]
    TlpAmber,
    #[serde(rename = "tlp:amber+strict")]
    TlpAmberStrict,
    #[serde(rename = "tlp:red")]
    TlpRed,
}

impl Marking {
    pub const ALL: [Marking; 6] = [
        Marking::Unmarked,
        Marking::TlpClear,
        Marking::TlpGreen,
        Marking::TlpAmber,
        Marking::TlpAmberStrict,
        Marking::TlpRed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Marking::Unmarked => "unmarked",
            Marking::TlpClear => "tlp:clear",
            Marking::TlpGreen => "tlp:green",
            Marking::TlpAmber => "tlp:amber",
            Marking::TlpAmberStrict => "tlp:amber+strict",
            Marking::TlpRed => "tlp:red",
        }
    }

    /// Analyst-facing label shown in the workbench and export listings.
    pub fn label(self) -> &'static str {
        match self {
            Marking::Unmarked => "unmarked",
            Marking::TlpClear => "TLP:CLEAR",
            Marking::TlpGreen => "TLP:GREEN",
            Marking::TlpAmber => "TLP:AMBER",
            Marking::TlpAmberStrict => "TLP:AMBER+STRICT",
            Marking::TlpRed => "TLP:RED",
        }
    }

    /// Parses a storage key, `TLP:` label, or short colour name.
    pub fn parse(value: &str) -> Result<Marking, MarkingError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(Marking::Unmarked);
        }
        let normalized = trimmed
            .trim_start_matches(['(', '['])
            .trim_end_matches([')', ']'])
            .trim()
            .to_ascii_lowercase()
            .replace('_', "-")
            .replace(' ', "");
        let key = normalized.strip_prefix("tlp:").unwrap_or(&normalized);
        let key = key.strip_prefix("tlp-").unwrap_or(key);
        match key {
            "unmarked" | "none" | "clearance-none" => Ok(Marking::Unmarked),
            "clear" | "white" => Ok(Marking::TlpClear),
            "green" => Ok(Marking::TlpGreen),
            "amber" | "yellow" => Ok(Marking::TlpAmber),
            "amber+strict" | "amber-strict" | "amberstrict" => Ok(Marking::TlpAmberStrict),
            "red" => Ok(Marking::TlpRed),
            _ => Err(MarkingError(trimmed.to_string())),
        }
    }
}

impl fmt::Display for Marking {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

impl From<Marking> for String {
    fn from(value: Marking) -> Self {
        value.as_str().to_string()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkingError(pub String);

impl fmt::Display for MarkingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unknown marking {:?}; use unmarked, TLP:CLEAR, TLP:GREEN, TLP:AMBER, TLP:AMBER+STRICT, or TLP:RED",
            self.0
        )
    }
}

impl std::error::Error for MarkingError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tlp_spellings() {
        assert_eq!(
            Marking::parse("TLP:AMBER").expect("amber"),
            Marking::TlpAmber
        );
        assert_eq!(
            Marking::parse("tlp-green").expect("green"),
            Marking::TlpGreen
        );
        assert_eq!(Marking::parse("white").expect("clear"), Marking::TlpClear);
        assert_eq!(
            Marking::parse("AMBER+STRICT").expect("strict"),
            Marking::TlpAmberStrict
        );
        assert_eq!(Marking::parse("").expect("empty"), Marking::Unmarked);
        assert_eq!(Marking::parse("unmarked").expect("none"), Marking::Unmarked);
        assert!(Marking::parse("secret").is_err());
    }

    #[test]
    fn round_trips_storage_keys() {
        for marking in Marking::ALL {
            let parsed = Marking::parse(marking.as_str()).expect("key");
            assert_eq!(parsed, marking);
            let json = serde_json::to_string(&marking).expect("json");
            let decoded: Marking = serde_json::from_str(&json).expect("decode");
            assert_eq!(decoded, marking);
        }
    }
}
