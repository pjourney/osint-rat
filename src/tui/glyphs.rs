//! Unicode glyphs for the cockpit, plus the `ui.ascii` fallback.
//!
//! No Nerd Fonts. Colour never carries meaning alone: every state also has a
//! glyph (and, in ASCII mode, a letter or short word).

use crate::model::{EntityKind, Provenance};

/// One cell (or short sequence) per meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyphs {
    /// Collected provenance (`●` / `C`).
    pub collected: &'static str,
    /// External / imported provenance (`◇` / `X`).
    pub external: &'static str,
    /// Analyst provenance (`✎` / `A`).
    pub analyst: &'static str,
    /// Accepted AI provenance (`◆` / `*`).
    pub ai: &'static str,
    /// Pending AI proposal (`◆?` / `*?`).
    pub ai_pending: &'static str,
    /// Spinner frames. ASCII is a single `"..."` frame.
    pub spinner: &'static [&'static str],
    /// Job succeeded (`✓` / `ok`).
    pub done: &'static str,
    /// Job failed (`✗` / `x`).
    pub failed: &'static str,
    /// Active tool refused (`⊘` / `no`).
    pub refused: &'static str,
    /// Focus bar on the focused pane title (`▌` / `>`).
    pub focus: &'static str,
    /// Outgoing graph edge (`─▶` / `->`).
    pub edge_out: &'static str,
    /// Incoming graph edge (`◀─` / `<-`).
    pub edge_in: &'static str,
    /// `true` when this set is the ASCII fallback.
    pub ascii: bool,
}

const UNICODE_SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ASCII_SPINNER: &[&str] = &["..."];

impl Glyphs {
    pub fn unicode() -> Self {
        Self {
            collected: "●",
            external: "◇",
            analyst: "✎",
            ai: "◆",
            ai_pending: "◆?",
            spinner: UNICODE_SPINNER,
            done: "✓",
            failed: "✗",
            refused: "⊘",
            focus: "▌",
            edge_out: "─▶",
            edge_in: "◀─",
            ascii: false,
        }
    }

    pub fn ascii() -> Self {
        Self {
            collected: "C",
            external: "X",
            analyst: "A",
            ai: "*",
            ai_pending: "*?",
            spinner: ASCII_SPINNER,
            done: "ok",
            failed: "x",
            refused: "no",
            focus: ">",
            edge_out: "->",
            edge_in: "<-",
            ascii: true,
        }
    }

    pub fn from_ascii_flag(ascii: bool) -> Self {
        if ascii {
            Self::ascii()
        } else {
            Self::unicode()
        }
    }

    pub fn provenance(self, provenance: Provenance) -> &'static str {
        match provenance {
            Provenance::Collected => self.collected,
            Provenance::External => self.external,
            Provenance::Analyst => self.analyst,
            Provenance::Agent => self.ai,
        }
    }

    /// Fixed-width 3-character entity kind tags. Same in Unicode and ASCII.
    pub fn entity_tag(kind: &EntityKind) -> &'static str {
        match kind {
            EntityKind::Domain | EntityKind::Hostname => "DOM",
            EntityKind::Ip => "IP ",
            EntityKind::Url => "URL",
            EntityKind::Email => "EML",
            EntityKind::Username => "USR",
            EntityKind::Repository => "REP",
            EntityKind::Asn => "ASN",
            EntityKind::File => "FIL",
            EntityKind::Organization => "ORG",
            EntityKind::Person => "PER",
            EntityKind::Location => "LOC",
            EntityKind::Phone => "TEL",
            EntityKind::Certificate => "CRT",
            EntityKind::Unknown | EntityKind::Other(_) => "UNK",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_fallback_is_plain_us_ascii() {
        let glyphs = Glyphs::ascii();
        for sample in [
            glyphs.collected,
            glyphs.external,
            glyphs.analyst,
            glyphs.ai,
            glyphs.ai_pending,
            glyphs.done,
            glyphs.failed,
            glyphs.refused,
            glyphs.focus,
            glyphs.edge_out,
            glyphs.edge_in,
            glyphs.spinner[0],
        ] {
            assert!(
                sample.bytes().all(|byte| byte.is_ascii()),
                "{sample:?} must be ASCII"
            );
        }
    }

    #[test]
    fn entity_tags_are_three_cells() {
        for kind in [
            EntityKind::Domain,
            EntityKind::Ip,
            EntityKind::Url,
            EntityKind::Email,
            EntityKind::Username,
            EntityKind::Repository,
            EntityKind::Asn,
            EntityKind::File,
            EntityKind::Organization,
        ] {
            assert_eq!(Glyphs::entity_tag(&kind).chars().count(), 3, "{kind:?}");
        }
    }
}
