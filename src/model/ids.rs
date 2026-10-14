//! Human-readable, investigation-scoped references such as `S-12` or `E-4`.
//!
//! Every record has a UUID primary key and a per-investigation sequence number. The short
//! reference is what analysts and agents cite in notes, findings, and reports.

use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Page,
    Source,
    Finding,
    Entity,
    Relationship,
    Note,
    Artifact,
    Timeline,
    Job,
    AgentRun,
    Proposal,
    Resolution,
}

impl RefKind {
    pub const ALL: [RefKind; 12] = [
        RefKind::Page,
        RefKind::Source,
        RefKind::Finding,
        RefKind::Entity,
        RefKind::Relationship,
        RefKind::Note,
        RefKind::Artifact,
        RefKind::Timeline,
        RefKind::Job,
        RefKind::AgentRun,
        RefKind::Proposal,
        RefKind::Resolution,
    ];

    pub fn prefix(self) -> &'static str {
        match self {
            RefKind::Page => "P",
            RefKind::Source => "S",
            RefKind::Finding => "F",
            RefKind::Entity => "E",
            RefKind::Relationship => "R",
            RefKind::Note => "N",
            RefKind::Artifact => "A",
            RefKind::Timeline => "T",
            RefKind::Job => "J",
            RefKind::AgentRun => "Q",
            RefKind::Proposal => "X",
            RefKind::Resolution => "V",
        }
    }

    /// Uppercase noun used in agent context headings (`SOURCE S-104`).
    pub fn noun(self) -> &'static str {
        match self {
            RefKind::Page => "PAGE",
            RefKind::Source => "SOURCE",
            RefKind::Finding => "FINDING",
            RefKind::Entity => "ENTITY",
            RefKind::Relationship => "RELATIONSHIP",
            RefKind::Note => "NOTE",
            RefKind::Artifact => "ARTIFACT",
            RefKind::Timeline => "EVENT",
            RefKind::Job => "JOB",
            RefKind::AgentRun => "AGENT-RUN",
            RefKind::Proposal => "PROPOSAL",
            RefKind::Resolution => "RESOLUTION",
        }
    }

    pub fn from_prefix(prefix: &str) -> Option<RefKind> {
        RefKind::ALL
            .into_iter()
            .find(|kind| kind.prefix().eq_ignore_ascii_case(prefix))
    }
}

/// A short reference like `S-12`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShortRef {
    pub kind: RefKind,
    pub seq: i64,
}

impl ShortRef {
    pub fn new(kind: RefKind, seq: i64) -> Self {
        Self { kind, seq }
    }
}

impl fmt::Display for ShortRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}-{}", self.kind.prefix(), self.seq)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseRefError(pub String);

impl fmt::Display for ParseRefError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "not a valid reference: {:?} (expected something like S-12)",
            self.0
        )
    }
}

impl std::error::Error for ParseRefError {}

impl FromStr for ShortRef {
    type Err = ParseRefError;

    /// Accepts `S-12`, `s12`, `[S-12]`, and `S 12`.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let trimmed = input
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim();
        let split_at = trimmed
            .find(|c: char| c.is_ascii_digit() || c == '-' || c == ' ')
            .ok_or_else(|| ParseRefError(input.to_string()))?;
        let (prefix, rest) = trimmed.split_at(split_at);
        let kind = RefKind::from_prefix(prefix).ok_or_else(|| ParseRefError(input.to_string()))?;
        let number = rest.trim_start_matches(['-', ' ']);
        let seq = number
            .parse::<i64>()
            .ok()
            .filter(|seq| *seq > 0)
            .ok_or_else(|| ParseRefError(input.to_string()))?;
        Ok(ShortRef { kind, seq })
    }
}

static REF_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b([PSFENRATJQXV])-(\d{1,7})\b").expect("reference pattern is valid")
});

/// Finds every short reference cited in free text, in order of first appearance, without
/// duplicates. Used to link agent responses back to the evidence they cite.
pub fn find_refs(text: &str) -> Vec<ShortRef> {
    let mut found: Vec<ShortRef> = Vec::new();
    for captures in REF_PATTERN.captures_iter(text) {
        let kind = captures
            .get(1)
            .and_then(|m| RefKind::from_prefix(m.as_str()));
        let seq = captures.get(2).and_then(|m| m.as_str().parse::<i64>().ok());
        if let (Some(kind), Some(seq)) = (kind, seq) {
            let reference = ShortRef { kind, seq };
            if !found.contains(&reference) {
                found.push(reference);
            }
        }
    }
    found
}

/// Parses a comma/space separated list of references (`S-1, S-2 S-3`).
pub fn parse_ref_list(input: &str) -> Result<Vec<ShortRef>, ParseRefError> {
    input
        .split([',', ' ', ';'])
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ShortRef::from_str)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_with_prefix() {
        let input = ShortRef::new(RefKind::Source, 104);
        assert_eq!(input.to_string(), "S-104");
    }

    #[test]
    fn parses_common_spellings() {
        let expected = ShortRef::new(RefKind::Entity, 22);
        for input in ["E-22", "e22", "[E-22]", " E 22 "] {
            assert_eq!(input.parse::<ShortRef>(), Ok(expected), "input {input:?}");
        }
    }

    #[test]
    fn rejects_invalid_refs() {
        for input in ["", "Z-1", "S-", "S-0", "S-abc", "12"] {
            assert!(input.parse::<ShortRef>().is_err(), "input {input:?}");
        }
    }

    #[test]
    fn finds_refs_in_text_once() {
        let input = "Cloudflare fronts the site [S-31][s-32]; see f-2 and again S-31.";
        let actual = find_refs(input);
        let expected = vec![
            ShortRef::new(RefKind::Source, 31),
            ShortRef::new(RefKind::Source, 32),
            ShortRef::new(RefKind::Finding, 2),
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn parses_reference_lists() {
        let actual = parse_ref_list("S-1, S-2 s3").expect("valid list");
        assert_eq!(actual.len(), 3);
        assert!(parse_ref_list("S-1, nope").is_err());
    }
}
