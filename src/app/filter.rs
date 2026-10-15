//! `/` filter grammar for evidence tables: `kind:ip src:ct prov:ai` plus free text.

use crate::model::{Entity, EntityKind, Finding, Provenance, Source, Workspace};

use super::EvidenceRow;

/// One parsed `/` query. Tokens of the form `key:value` are structured; the rest is free text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableFilter {
    pub kind: Option<String>,
    pub src: Option<String>,
    pub prov: Option<Provenance>,
    pub sort: Option<(SortKey, bool)>,
    pub text: String,
}

/// Column a table can sort by. `sort:seen` / `sort:-kind` in the filter grammar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Kind,
    Value,
    Prov,
    Src,
    Seen,
    Status,
    Name,
}

impl SortKey {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "kind" | "type" => Some(Self::Kind),
            "value" | "name" | "title" => Some(Self::Value),
            "prov" | "provenance" => Some(Self::Prov),
            "src" | "source" | "sources" => Some(Self::Src),
            "seen" | "when" | "time" => Some(Self::Seen),
            "status" | "state" => Some(Self::Status),
            _ => None,
        }
    }

    pub fn header(self) -> &'static str {
        match self {
            Self::Kind => "KIND",
            Self::Value => "VALUE",
            Self::Prov => "PROV",
            Self::Src => "SRC",
            Self::Seen => "SEEN",
            Self::Status => "STATUS",
            Self::Name => "NAME",
        }
    }
}

impl TableFilter {
    pub fn parse(raw: &str) -> Self {
        let mut filter = Self::default();
        let mut text = Vec::new();
        for token in raw.split_whitespace() {
            let Some((key, value)) = token.split_once(':') else {
                text.push(token);
                continue;
            };
            match key.to_ascii_lowercase().as_str() {
                "kind" | "k" => filter.kind = Some(value.to_ascii_lowercase()),
                "src" | "source" => filter.src = Some(value.to_ascii_lowercase()),
                "prov" | "provenance" => filter.prov = parse_prov(value),
                "sort" => {
                    let (name, descending) = if let Some(rest) = value.strip_prefix('-') {
                        (rest, true)
                    } else {
                        (value, false)
                    };
                    if let Some(key) = SortKey::parse(name) {
                        filter.sort = Some((key, descending));
                    }
                }
                _ => text.push(token),
            }
        }
        filter.text = text.join(" ").to_ascii_lowercase();
        filter
    }

    pub fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.src.is_none()
            && self.prov.is_none()
            && self.sort.is_none()
            && self.text.is_empty()
    }

    pub fn matches_entity(&self, entity: &Entity, workspace: &Workspace) -> bool {
        if let Some(kind) = &self.kind
            && !kind_matches_entity(&entity.kind, kind)
        {
            return false;
        }
        if let Some(want) = self.prov
            && !entity_provenances(entity, workspace).contains(&want)
        {
            return false;
        }
        if let Some(src) = &self.src
            && !entity_has_source(entity, workspace, src)
        {
            return false;
        }
        if self.text.is_empty() {
            return true;
        }
        let hay = format!(
            "{} {} {} {}",
            entity.display_name(),
            entity.value,
            entity.kind.label(),
            entity.short_ref()
        );
        hay.to_ascii_lowercase().contains(&self.text)
    }

    pub fn matches_finding(&self, finding: &Finding, workspace: &Workspace) -> bool {
        if let Some(kind) = &self.kind
            && !kind_token_matches(kind, "finding")
            && !kind_token_matches(kind, "f")
        {
            return false;
        }
        if let Some(want) = self.prov
            && finding.provenance != want
        {
            return false;
        }
        if let Some(src) = &self.src
            && !ids_have_source(&finding.source_ids, workspace, src)
        {
            return false;
        }
        if self.text.is_empty() {
            return true;
        }
        let hay = format!(
            "{} {} {}",
            finding.title,
            finding.details,
            finding.short_ref()
        );
        hay.to_ascii_lowercase().contains(&self.text)
    }

    pub fn matches_source(&self, source: &Source) -> bool {
        if let Some(kind) = &self.kind
            && !kind_token_matches(kind, "source")
            && !kind_token_matches(kind, "s")
            && !kind_token_matches(kind, source.kind.as_str())
        {
            return false;
        }
        if let Some(want) = self.prov
            && source.provenance != want
        {
            return false;
        }
        if let Some(src) = &self.src
            && !source_matches_token(source, src)
        {
            return false;
        }
        if self.text.is_empty() {
            return true;
        }
        let hay = format!(
            "{} {} {}",
            source.title,
            source.kind.label(),
            source.short_ref()
        );
        hay.to_ascii_lowercase().contains(&self.text)
    }

    pub fn matches_text(&self, hay: &str) -> bool {
        if !self.text.is_empty() && !hay.to_ascii_lowercase().contains(&self.text) {
            return false;
        }
        true
    }
}

pub fn entity_provenances(entity: &Entity, workspace: &Workspace) -> Vec<Provenance> {
    let mut seen = Vec::new();
    push_unique(&mut seen, entity.provenance);
    for id in &entity.source_ids {
        if let Some(source) = workspace.source(*id) {
            push_unique(&mut seen, source.provenance);
        }
    }
    seen
}

pub fn entity_seen_at(entity: &Entity, workspace: &Workspace) -> Option<crate::model::Timestamp> {
    entity
        .source_ids
        .iter()
        .filter_map(|id| workspace.source(*id))
        .map(|source| source.last_seen_at.unwrap_or(source.retrieved_at))
        .max()
        .or(Some(entity.updated_at))
}

pub fn cmp_entities(
    left: &Entity,
    right: &Entity,
    workspace: &Workspace,
    sort: (SortKey, bool),
) -> std::cmp::Ordering {
    let (key, descending) = sort;
    let ordering = match key {
        SortKey::Kind => left.kind.as_str().cmp(right.kind.as_str()),
        SortKey::Value | SortKey::Name => left.display_name().cmp(right.display_name()),
        SortKey::Prov => left.provenance.cmp(&right.provenance),
        SortKey::Src => left.source_ids.len().cmp(&right.source_ids.len()),
        SortKey::Seen => entity_seen_at(left, workspace).cmp(&entity_seen_at(right, workspace)),
        SortKey::Status => left.kind.as_str().cmp(right.kind.as_str()),
    }
    .then_with(|| left.display_name().cmp(right.display_name()))
    .then_with(|| left.id.cmp(&right.id));
    if descending {
        ordering.reverse()
    } else {
        ordering
    }
}

pub fn cmp_evidence(
    left: EvidenceRow,
    right: EvidenceRow,
    workspace: &Workspace,
    sort: (SortKey, bool),
) -> std::cmp::Ordering {
    let (key, descending) = sort;
    let ordering = match key {
        SortKey::Kind => evidence_kind(left).cmp(evidence_kind(right)),
        SortKey::Value | SortKey::Name => {
            evidence_title(left, workspace).cmp(&evidence_title(right, workspace))
        }
        SortKey::Prov => evidence_prov(left, workspace).cmp(&evidence_prov(right, workspace)),
        SortKey::Src => evidence_src(left, workspace).cmp(&evidence_src(right, workspace)),
        SortKey::Seen => evidence_seen(left, workspace).cmp(&evidence_seen(right, workspace)),
        SortKey::Status => evidence_kind(left).cmp(evidence_kind(right)),
    }
    .then_with(|| evidence_title(left, workspace).cmp(&evidence_title(right, workspace)));
    if descending {
        ordering.reverse()
    } else {
        ordering
    }
}

fn evidence_kind(row: EvidenceRow) -> &'static str {
    match row {
        EvidenceRow::Finding(_) => "F",
        EvidenceRow::Source(_) => "S",
    }
}

fn evidence_title(row: EvidenceRow, workspace: &Workspace) -> String {
    match row {
        EvidenceRow::Finding(id) => workspace
            .finding(id)
            .map(|finding| finding.title.clone())
            .unwrap_or_default(),
        EvidenceRow::Source(id) => workspace
            .source(id)
            .map(|source| source.title.clone())
            .unwrap_or_default(),
    }
}

fn evidence_prov(row: EvidenceRow, workspace: &Workspace) -> Provenance {
    match row {
        EvidenceRow::Finding(id) => workspace
            .finding(id)
            .map(|finding| finding.provenance)
            .unwrap_or(Provenance::Collected),
        EvidenceRow::Source(id) => workspace
            .source(id)
            .map(|source| source.provenance)
            .unwrap_or(Provenance::Collected),
    }
}

fn evidence_src(row: EvidenceRow, workspace: &Workspace) -> usize {
    match row {
        EvidenceRow::Finding(id) => workspace
            .finding(id)
            .map(|finding| finding.source_ids.len())
            .unwrap_or(0),
        EvidenceRow::Source(_) => 1,
    }
}

fn evidence_seen(row: EvidenceRow, workspace: &Workspace) -> Option<crate::model::Timestamp> {
    match row {
        EvidenceRow::Finding(id) => workspace.finding(id).map(|finding| finding.updated_at),
        EvidenceRow::Source(id) => workspace
            .source(id)
            .map(|source| source.last_seen_at.unwrap_or(source.retrieved_at)),
    }
}

pub fn evidence_row_matches(filter: &TableFilter, row: EvidenceRow, workspace: &Workspace) -> bool {
    match row {
        EvidenceRow::Finding(id) => workspace
            .finding(id)
            .is_some_and(|finding| filter.matches_finding(finding, workspace)),
        EvidenceRow::Source(id) => workspace
            .source(id)
            .is_some_and(|source| filter.matches_source(source)),
    }
}

fn parse_prov(raw: &str) -> Option<Provenance> {
    let trimmed = raw.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "col" | "c" | "collected" | "collector" => Some(Provenance::Collected),
        "ext" | "x" | "external" | "original" => Some(Provenance::External),
        "anl" | "a" | "analyst" | "human" => Some(Provenance::Analyst),
        "ai" | "agent" | "*" => Some(Provenance::Agent),
        other => Provenance::parse(other),
    }
}

fn kind_matches_entity(kind: &EntityKind, token: &str) -> bool {
    let tag = entity_kind_tag(kind);
    kind_token_matches(token, tag) || kind_token_matches(token, kind.as_str())
}

fn entity_kind_tag(kind: &EntityKind) -> &'static str {
    match kind {
        EntityKind::Domain | EntityKind::Hostname => "dom",
        EntityKind::Ip => "ip",
        EntityKind::Url => "url",
        EntityKind::Email => "eml",
        EntityKind::Username => "usr",
        EntityKind::Repository => "rep",
        EntityKind::Asn => "asn",
        EntityKind::File => "fil",
        EntityKind::Organization => "org",
        EntityKind::Person => "per",
        EntityKind::Location => "loc",
        EntityKind::Phone => "tel",
        EntityKind::Certificate => "crt",
        EntityKind::Unknown | EntityKind::Other(_) => "unk",
    }
}

fn kind_token_matches(token: &str, candidate: &str) -> bool {
    let token = token.trim().trim_end_matches(' ');
    let candidate = candidate.trim().to_ascii_lowercase();
    token.eq_ignore_ascii_case(&candidate) || candidate.starts_with(token)
}

fn entity_has_source(entity: &Entity, workspace: &Workspace, token: &str) -> bool {
    ids_have_source(&entity.source_ids, workspace, token)
}

fn ids_have_source(ids: &[uuid::Uuid], workspace: &Workspace, token: &str) -> bool {
    ids.iter().any(|id| {
        workspace
            .source(*id)
            .is_some_and(|source| source_matches_token(source, token))
    })
}

fn source_matches_token(source: &Source, token: &str) -> bool {
    let kind = source.kind.as_str().to_ascii_lowercase();
    if kind_token_matches(token, &kind) || kind_token_matches(token, source.kind.label()) {
        return true;
    }
    source
        .collector
        .as_deref()
        .is_some_and(|collector| collector.to_ascii_lowercase().contains(token))
}

fn push_unique(seen: &mut Vec<Provenance>, value: Provenance) {
    if !seen.contains(&value) {
        seen.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_grammar_tokens_and_free_text() {
        let filter = TableFilter::parse("kind:ip src:ct prov:ai login sort:-seen");
        assert_eq!(filter.kind.as_deref(), Some("ip"));
        assert_eq!(filter.src.as_deref(), Some("ct"));
        assert_eq!(filter.prov, Some(Provenance::Agent));
        assert_eq!(filter.sort, Some((SortKey::Seen, true)));
        assert_eq!(filter.text, "login");
    }

    #[test]
    fn parse_empty_is_empty() {
        assert!(TableFilter::parse("").is_empty());
        assert!(TableFilter::parse("   ").is_empty());
    }

    #[test]
    fn provenance_aliases() {
        assert_eq!(parse_prov("col"), Some(Provenance::Collected));
        assert_eq!(parse_prov("◆".trim()), None);
        assert_eq!(parse_prov("ai"), Some(Provenance::Agent));
        assert_eq!(parse_prov("X"), Some(Provenance::External));
    }

    #[test]
    fn kind_token_matches_prefix() {
        assert!(kind_token_matches("dom", "domain"));
        assert!(kind_token_matches("ip", "ip"));
        assert!(!kind_token_matches("url", "domain"));
    }
}
