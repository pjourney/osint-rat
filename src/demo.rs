//! Bundled Example Corp investigation used as the first-run demo case.
//!
//! The archive under `examples/demo-case/` is this snapshot serialized. It is a
//! documentation-range fixture (`example.com`, `192.0.2.0/24`, `2001:db8::/32`) that
//! already contains a cited graph, a resolved identity, and playbook output so a new
//! user can open Graph and Search Around without collecting from the network.

use chrono::{TimeZone, Utc};
use uuid::Uuid;

use crate::exchange::{ArchiveManifest, FORMAT_VERSION};
use crate::model::{
    AuditEntry, AuditKind, Confidence, Entity, EntityKind, Event, Fields, Finding, Identity,
    Investigation, InvestigationData, Marking, Note, Page, PageEntityLink, PageKind, Provenance,
    Relationship, Resolution, ResolutionStatus, Source, SourceKind, TimelineEvent, TimelineKind,
    Timestamp, relation,
};

const INVESTIGATION: Uuid = uuid(1);
const PAGE: Uuid = uuid(2);
const SOURCE_DNS: Uuid = uuid(10);
const SOURCE_HTTP: Uuid = uuid(11);
const SOURCE_TLS: Uuid = uuid(12);
const SOURCE_RDAP: Uuid = uuid(13);
const FINDING_DNS: Uuid = uuid(20);
const FINDING_HTTP: Uuid = uuid(21);
const FINDING_TLS: Uuid = uuid(22);
const DOMAIN: Uuid = uuid(30);
const HOSTNAME: Uuid = uuid(31);
const WWW: Uuid = uuid(32);
const MAIL: Uuid = uuid(33);
const IP_V4: Uuid = uuid(34);
const IP_MAIL: Uuid = uuid(35);
const IP_V6: Uuid = uuid(36);
const NS_A: Uuid = uuid(37);
const NS_B: Uuid = uuid(38);
const ASN: Uuid = uuid(39);
const ORG: Uuid = uuid(40);
const REGISTRAR: Uuid = uuid(41);
const CERT: Uuid = uuid(42);
const URL: Uuid = uuid(43);
const EMAIL: Uuid = uuid(44);
const SPF: Uuid = uuid(45);
const FTP: Uuid = uuid(46);
const IP_FTP: Uuid = uuid(47);
const MX: Uuid = uuid(48);
const IP_MX: Uuid = uuid(49);
const BLOG: Uuid = uuid(78);
const RESOLUTION: Uuid = uuid(100);
const NOTE: Uuid = uuid(120);
const TIMELINE_OBSERVED: Uuid = uuid(80);
const TIMELINE_PLAYBOOK: Uuid = uuid(81);
const TIMELINE_RESOLVED: Uuid = uuid(82);
const EVENT_REG: Uuid = uuid(90);
const EVENT_CERT: Uuid = uuid(91);
const AUDIT_PLAYBOOK: Uuid = uuid(110);
const AUDIT_MERGE: Uuid = uuid(111);

const fn uuid(tag: u8) -> Uuid {
    Uuid::from_bytes([
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0x4c, 0xde, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        tag,
    ])
}

fn stamp() -> Timestamp {
    Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0)
        .single()
        .expect("demo timestamp")
}

fn earlier() -> Timestamp {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
        .single()
        .expect("demo earlier")
}

fn actor() -> Identity {
    Identity {
        user_id: "demo".into(),
        machine_name: "fixture".into(),
    }
}

/// Portable archive for `examples/demo-case/investigation.osint.json`.
pub fn demo_case_manifest() -> ArchiveManifest {
    ArchiveManifest {
        format: "os-int-investigation".into(),
        format_version: FORMAT_VERSION,
        exported_at: "2024-06-15T12:00:00.000+00:00".into(),
        data: demo_case_data(),
    }
}

/// In-memory snapshot of the Example Corp demo investigation.
pub fn demo_case_data() -> InvestigationData {
    let timestamp = stamp();
    let mut data = InvestigationData::empty(Investigation {
        id: INVESTIGATION,
        name: "Example Corp".into(),
        description: "Documentation-range fixture: example.com after a domain-infra playbook. Import this first to see Graph, a resolved identity, and cited findings.".into(),
        tags: vec!["demo".into(), "fixture".into()],
        marking: Marking::TlpClear,
        created_at: earlier(),
        updated_at: timestamp,
    });
    data.pages.push(page(timestamp));
    data.sources.extend(sources(timestamp));
    data.entities.extend(entities(timestamp));
    data.relationships.extend(relationships(timestamp));
    data.findings.extend(findings(timestamp));
    data.notes.push(Note {
        id: NOTE,
        investigation_id: INVESTIGATION,
        seq: 1,
        page_id: Some(PAGE),
        body: "Import path: open Graph (`g`) on example.com, Search Around (`s`) two hops, then press Enter on the canonical domain to open Object View. Constituent hostname example.com is merged. Playbook domain-infra is on the timeline as T-2.".into(),
        provenance: Provenance::Analyst,
        created_at: timestamp,
        updated_at: timestamp,
    });
    data.timeline.extend(timeline(timestamp));
    data.events.extend(events(timestamp));
    data.resolutions.push(resolution(timestamp));
    data.audit.extend(audit(timestamp));
    data.page_entities = vec![
        PageEntityLink {
            page_id: PAGE,
            entity_id: DOMAIN,
        },
        PageEntityLink {
            page_id: PAGE,
            entity_id: WWW,
        },
        PageEntityLink {
            page_id: PAGE,
            entity_id: IP_V4,
        },
    ];
    data
}

fn page(timestamp: Timestamp) -> Page {
    Page {
        id: PAGE,
        investigation_id: INVESTIGATION,
        seq: 1,
        title: "example.com".into(),
        kind: PageKind::Domain,
        target: Some("example.com".into()),
        description: "Public documentation domain (RFC 2606).".into(),
        notes: "Seed page for the bundled demo. Collectors are not required; the graph is already cited.".into(),
        entity_id: Some(DOMAIN),
        created_at: earlier(),
        updated_at: timestamp,
    }
}

fn sources(timestamp: Timestamp) -> Vec<Source> {
    vec![
        source(
            SOURCE_DNS,
            1,
            SourceKind::Dns,
            "dns",
            "DNS records for example.com",
            "example.com. 300 IN A 192.0.2.1\nexample.com. 300 IN AAAA 2001:db8::1\nwww.example.com. 300 IN CNAME example.com.\nmail.example.com. 300 IN A 192.0.2.10\nexample.com. 300 IN NS a.iana-servers.net.\nexample.com. 300 IN NS b.iana-servers.net.",
            timestamp,
        ),
        source(
            SOURCE_HTTP,
            2,
            SourceKind::Http,
            "http",
            "HTTP GET https://example.com/",
            "HTTP/1.1 200 OK\nserver: ExampleCDN",
            timestamp,
        ),
        source(
            SOURCE_TLS,
            3,
            SourceKind::Tls,
            "tls",
            "TLS certificate for example.com",
            "subject=CN=example.com\nSAN=example.com,www.example.com",
            timestamp,
        ),
        source(
            SOURCE_RDAP,
            4,
            SourceKind::Rdap,
            "rdap",
            "RDAP for example.com",
            "ldhName example.com\nregistrar Example Registrar\norg Example Organization",
            timestamp,
        ),
    ]
}

fn source(
    id: Uuid,
    seq: i64,
    kind: SourceKind,
    collector: &str,
    title: &str,
    content: &str,
    timestamp: Timestamp,
) -> Source {
    Source {
        id,
        investigation_id: INVESTIGATION,
        seq,
        kind,
        title: title.into(),
        url: None,
        retrieved_at: timestamp,
        last_seen_at: Some(timestamp),
        published_at: None,
        author: None,
        collector: Some(collector.into()),
        content_hash: Some(format!("{:064}", seq)),
        snapshot_path: None,
        content: content.into(),
        fields: Fields::new(),
        notes: String::new(),
        provenance: Provenance::Collected,
        page_id: Some(PAGE),
        job_id: None,
    }
}

fn entities(timestamp: Timestamp) -> Vec<Entity> {
    vec![
        entity(
            DOMAIN,
            1,
            EntityKind::Domain,
            "example.com",
            &[SOURCE_DNS, SOURCE_HTTP, SOURCE_TLS, SOURCE_RDAP],
            &[("registrar", "Example Registrar")],
            timestamp,
        ),
        entity(
            HOSTNAME,
            2,
            EntityKind::Hostname,
            "example.com",
            &[SOURCE_DNS],
            &[],
            timestamp,
        ),
        entity(
            WWW,
            3,
            EntityKind::Hostname,
            "www.example.com",
            &[SOURCE_DNS, SOURCE_TLS],
            &[],
            timestamp,
        ),
        entity(
            MAIL,
            4,
            EntityKind::Hostname,
            "mail.example.com",
            &[SOURCE_DNS],
            &[],
            timestamp,
        ),
        entity(
            IP_V4,
            5,
            EntityKind::Ip,
            "192.0.2.1",
            &[SOURCE_DNS],
            &[("prefix", "192.0.2.0/24"), ("country", "US")],
            timestamp,
        ),
        entity(
            IP_MAIL,
            6,
            EntityKind::Ip,
            "192.0.2.10",
            &[SOURCE_DNS],
            &[("prefix", "192.0.2.0/24")],
            timestamp,
        ),
        entity(
            IP_V6,
            7,
            EntityKind::Ip,
            "2001:db8::1",
            &[SOURCE_DNS],
            &[("prefix", "2001:db8::/32")],
            timestamp,
        ),
        entity(
            NS_A,
            8,
            EntityKind::Hostname,
            "a.iana-servers.net",
            &[SOURCE_DNS],
            &[],
            timestamp,
        ),
        entity(
            NS_B,
            9,
            EntityKind::Hostname,
            "b.iana-servers.net",
            &[SOURCE_DNS],
            &[],
            timestamp,
        ),
        entity(
            ASN,
            10,
            EntityKind::Asn,
            "AS64496",
            &[SOURCE_DNS],
            &[],
            timestamp,
        ),
        entity(
            ORG,
            11,
            EntityKind::Organization,
            "Example Organization",
            &[SOURCE_RDAP],
            &[],
            timestamp,
        ),
        entity(
            REGISTRAR,
            12,
            EntityKind::Organization,
            "Example Registrar",
            &[SOURCE_RDAP],
            &[],
            timestamp,
        ),
        entity(
            CERT,
            13,
            EntityKind::Certificate,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            &[SOURCE_TLS],
            &[("issuer", "Example CA")],
            timestamp,
        ),
        entity(
            URL,
            14,
            EntityKind::Url,
            "https://example.com/",
            &[SOURCE_HTTP],
            &[],
            timestamp,
        ),
        entity(
            EMAIL,
            15,
            EntityKind::Email,
            "admin@example.com",
            &[SOURCE_RDAP],
            &[],
            timestamp,
        ),
        entity(
            SPF,
            16,
            EntityKind::Hostname,
            "_spf.example.net",
            &[SOURCE_DNS],
            &[],
            timestamp,
        ),
        entity(
            FTP,
            17,
            EntityKind::Hostname,
            "ftp.example.com",
            &[SOURCE_DNS],
            &[],
            timestamp,
        ),
        entity(
            IP_FTP,
            18,
            EntityKind::Ip,
            "192.0.2.20",
            &[SOURCE_DNS],
            &[("prefix", "192.0.2.0/24")],
            timestamp,
        ),
        entity(
            MX,
            19,
            EntityKind::Hostname,
            "mx.example.com",
            &[SOURCE_DNS],
            &[],
            timestamp,
        ),
        entity(
            IP_MX,
            20,
            EntityKind::Ip,
            "192.0.2.25",
            &[SOURCE_DNS],
            &[("prefix", "192.0.2.0/24")],
            timestamp,
        ),
        entity(
            BLOG,
            21,
            EntityKind::Hostname,
            "blog.example.com",
            &[SOURCE_DNS],
            &[],
            timestamp,
        ),
    ]
}

fn entity(
    id: Uuid,
    seq: i64,
    kind: EntityKind,
    value: &str,
    sources: &[Uuid],
    fields: &[(&str, &str)],
    timestamp: Timestamp,
) -> Entity {
    let mut bag = Fields::new();
    for (key, value) in fields {
        bag.set(*key, *value);
    }
    Entity {
        id,
        investigation_id: INVESTIGATION,
        seq,
        kind,
        value: value.into(),
        label: String::new(),
        notes: String::new(),
        provenance: Provenance::Collected,
        fields: bag,
        source_ids: sources.to_vec(),
        created_at: timestamp,
        updated_at: timestamp,
    }
}

fn relationships(timestamp: Timestamp) -> Vec<Relationship> {
    let edges: &[(Uuid, &str, Uuid, Uuid)] = &[
        (uuid(50), relation::SUBDOMAIN_OF, WWW, DOMAIN),
        (uuid(51), relation::SUBDOMAIN_OF, MAIL, DOMAIN),
        (uuid(52), relation::RESOLVES_TO, DOMAIN, IP_V4),
        (uuid(53), relation::RESOLVES_TO, DOMAIN, IP_V6),
        (uuid(54), relation::RESOLVES_TO, WWW, IP_V4),
        (uuid(55), relation::RESOLVES_TO, MAIL, IP_MAIL),
        (uuid(56), relation::USES_NAMESERVER, DOMAIN, NS_A),
        (uuid(57), relation::USES_NAMESERVER, DOMAIN, NS_B),
        (uuid(58), relation::ANNOUNCED_BY, IP_V4, ASN),
        (uuid(59), relation::ANNOUNCED_BY, IP_MAIL, ASN),
        (uuid(60), relation::ANNOUNCED_BY, IP_V6, ASN),
        (uuid(61), relation::OPERATED_BY, IP_V4, ORG),
        (uuid(62), relation::REGISTERED_WITH, DOMAIN, REGISTRAR),
        (uuid(63), relation::PRESENTS_CERTIFICATE, DOMAIN, CERT),
        (uuid(64), relation::COVERS, CERT, DOMAIN),
        (uuid(65), relation::COVERS, CERT, WWW),
        (uuid(66), relation::SERVES, DOMAIN, URL),
        (uuid(67), relation::HOSTED_ON, EMAIL, DOMAIN),
        (uuid(68), relation::SPF_INCLUDES, DOMAIN, SPF),
        (uuid(69), relation::REVERSE_DNS, IP_V4, DOMAIN),
        (uuid(70), relation::ALIAS_OF, WWW, DOMAIN),
        (uuid(71), relation::SUBDOMAIN_OF, FTP, DOMAIN),
        (uuid(72), relation::RESOLVES_TO, FTP, IP_FTP),
        (uuid(73), relation::ANNOUNCED_BY, IP_FTP, ASN),
        (uuid(74), relation::SUBDOMAIN_OF, MX, DOMAIN),
        (uuid(75), relation::MAIL_HANDLED_BY, DOMAIN, MX),
        (uuid(76), relation::RESOLVES_TO, MX, IP_MX),
        (uuid(77), relation::ANNOUNCED_BY, IP_MX, ASN),
        (uuid(79), relation::SUBDOMAIN_OF, BLOG, DOMAIN),
        (uuid(83), relation::RESOLVES_TO, BLOG, IP_V4),
    ];
    edges
        .iter()
        .enumerate()
        .map(|(index, (id, kind, from, to))| Relationship {
            id: *id,
            investigation_id: INVESTIGATION,
            seq: (index as i64) + 1,
            from_id: *from,
            to_id: *to,
            kind: (*kind).into(),
            provenance: Provenance::Collected,
            confidence: Confidence::High,
            notes: String::new(),
            source_ids: vec![SOURCE_DNS],
            created_at: timestamp,
        })
        .collect()
}

fn findings(timestamp: Timestamp) -> Vec<Finding> {
    vec![
        Finding {
            id: FINDING_DNS,
            investigation_id: INVESTIGATION,
            seq: 1,
            page_id: Some(PAGE),
            title: "example.com resolves to documentation-range 192.0.2.1".into(),
            details: "A and AAAA land in TEST-NET-1 / documentation IPv6. Cited by S-1.".into(),
            confidence: Confidence::High,
            provenance: Provenance::Collected,
            source_ids: vec![SOURCE_DNS],
            created_at: timestamp,
            updated_at: timestamp,
        },
        Finding {
            id: FINDING_HTTP,
            investigation_id: INVESTIGATION,
            seq: 2,
            page_id: Some(PAGE),
            title: "HTTPS is served at the example.com apex".into(),
            details: "HTTP GET of https://example.com/ returned 200. Cited by S-2.".into(),
            confidence: Confidence::High,
            provenance: Provenance::Collected,
            source_ids: vec![SOURCE_HTTP],
            created_at: timestamp,
            updated_at: timestamp,
        },
        Finding {
            id: FINDING_TLS,
            investigation_id: INVESTIGATION,
            seq: 3,
            page_id: Some(PAGE),
            title: "Presented certificate covers example.com and www.example.com".into(),
            details: "TLS SAN list includes the apex and www. Cited by S-3.".into(),
            confidence: Confidence::High,
            provenance: Provenance::Collected,
            source_ids: vec![SOURCE_TLS],
            created_at: timestamp,
            updated_at: timestamp,
        },
    ]
}

fn timeline(timestamp: Timestamp) -> Vec<TimelineEvent> {
    vec![
        TimelineEvent {
            id: TIMELINE_OBSERVED,
            investigation_id: INVESTIGATION,
            seq: 1,
            occurred_at: timestamp,
            kind: TimelineKind::Observed,
            title: "Collected DNS, HTTP, TLS, and RDAP for example.com".into(),
            description: "Fixture observations, not a live collection.".into(),
            provenance: Provenance::Collected,
            source_id: Some(SOURCE_DNS),
            entity_id: Some(DOMAIN),
            page_id: Some(PAGE),
            created_at: timestamp,
        },
        TimelineEvent {
            id: TIMELINE_PLAYBOOK,
            investigation_id: INVESTIGATION,
            seq: 2,
            occurred_at: timestamp,
            kind: TimelineKind::PlaybookRun,
            title: "Playbook domain-infra".into(),
            description: "4 new source(s), 21 new entities (21 seen), 30 new relationships".into(),
            provenance: Provenance::Analyst,
            source_id: None,
            entity_id: Some(DOMAIN),
            page_id: Some(PAGE),
            created_at: timestamp,
        },
        TimelineEvent {
            id: TIMELINE_RESOLVED,
            investigation_id: INVESTIGATION,
            seq: 3,
            occurred_at: timestamp,
            kind: TimelineKind::EntitiesResolved,
            title: "Resolved example.com domain and hostname".into(),
            description: "Same canonical value; hostname absorbed.".into(),
            provenance: Provenance::Analyst,
            source_id: None,
            entity_id: Some(DOMAIN),
            page_id: Some(PAGE),
            created_at: timestamp,
        },
    ]
}

fn events(timestamp: Timestamp) -> Vec<Event> {
    vec![
        Event {
            id: EVENT_REG,
            investigation_id: INVESTIGATION,
            seq: 1,
            kind: TimelineKind::Registration,
            title: "example.com registered (fixture)".into(),
            description: "RDAP registration instant for the demo.".into(),
            started_at: earlier(),
            ended_at: None,
            provenance: Provenance::Collected,
            source_id: Some(SOURCE_RDAP),
            page_id: Some(PAGE),
            fields: Fields::new(),
            entity_ids: vec![DOMAIN],
            created_at: timestamp,
        },
        Event {
            id: EVENT_CERT,
            investigation_id: INVESTIGATION,
            seq: 2,
            kind: TimelineKind::CertificateIssued,
            title: "Certificate issued for example.com".into(),
            description: "Covers apex and www.".into(),
            started_at: earlier(),
            ended_at: None,
            provenance: Provenance::Collected,
            source_id: Some(SOURCE_TLS),
            page_id: Some(PAGE),
            fields: Fields::new(),
            entity_ids: vec![CERT, DOMAIN, WWW],
            created_at: timestamp,
        },
    ]
}

fn resolution(timestamp: Timestamp) -> Resolution {
    Resolution {
        id: RESOLUTION,
        investigation_id: INVESTIGATION,
        seq: 1,
        canonical_id: DOMAIN,
        constituent_ids: vec![DOMAIN, HOSTNAME],
        status: ResolutionStatus::Active,
        reason: "Same canonical value example.com".into(),
        created_by: actor().user_id,
        created_at: timestamp,
        unresolved_by: None,
        unresolved_at: None,
        property_writes: Vec::new(),
        source_writes: vec![SOURCE_DNS],
    }
}

fn audit(timestamp: Timestamp) -> Vec<AuditEntry> {
    let identity = actor();
    vec![
        AuditEntry {
            id: AUDIT_PLAYBOOK,
            investigation_id: INVESTIGATION,
            seq: 1,
            kind: AuditKind::PlaybookRun,
            actor: identity.user_id.clone(),
            machine: identity.machine_name.clone(),
            occurred_at: timestamp,
            summary: "Playbook domain-infra".into(),
            details: {
                let mut fields = Fields::new();
                fields.set("playbook_id", "domain-infra");
                fields
            },
            target_ids: Vec::new(),
        },
        AuditEntry {
            id: AUDIT_MERGE,
            investigation_id: INVESTIGATION,
            seq: 2,
            kind: AuditKind::EntitiesMerged,
            actor: identity.user_id,
            machine: identity.machine_name,
            occurred_at: timestamp,
            summary: "Merged example.com hostname into domain".into(),
            details: Fields::new(),
            target_ids: vec![DOMAIN, HOSTNAME],
        },
    ]
}

/// Canonical domain entity id in the demo snapshot (E-1).
pub fn demo_domain_id() -> Uuid {
    DOMAIN
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SearchAroundQuery, Workspace, search_around};

    #[test]
    fn demo_graph_has_cited_findings_resolution_and_playbook() {
        let data = demo_case_data();
        assert_eq!(data.investigation.name, "Example Corp");
        assert_eq!(data.investigation.marking, Marking::TlpClear);
        assert!(
            data.findings
                .iter()
                .all(|finding| !finding.source_ids.is_empty())
        );
        assert!(
            data.findings
                .iter()
                .all(|finding| finding.provenance == Provenance::Collected)
        );
        assert_eq!(data.resolutions.len(), 1);
        assert!(data.resolutions[0].is_active());
        assert!(data.is_absorbed(HOSTNAME));
        assert!(!data.is_absorbed(DOMAIN));
        assert!(
            data.timeline
                .iter()
                .any(|event| event.kind == TimelineKind::PlaybookRun)
        );
        let workspace = Workspace::new(data.clone());
        assert_eq!(data.entities.len(), 21);
        assert_eq!(workspace.visible_entities().len(), 20);
        let around = search_around(&data, &SearchAroundQuery::new(vec![DOMAIN], 2));
        assert!(around.hops_used >= 2);
        assert!(
            around
                .nodes
                .iter()
                .any(|node| node.id == ASN && node.hop == 2)
        );
        assert!(around.nodes.iter().any(|node| node.id == IP_V4));
        assert!(!around.nodes.iter().any(|node| node.id == HOSTNAME));
    }

    #[test]
    #[ignore = "rewrite examples/demo-case/investigation.osint.json from the builder"]
    fn write_checked_in_demo_archive() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/demo-case/investigation.osint.json");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        let json = serde_json::to_string_pretty(&demo_case_manifest()).expect("json");
        std::fs::write(&path, format!("{json}\n")).expect("write");
    }
}
