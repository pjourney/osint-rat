//! First-class object types, link verbs, and Action kinds.
//!
//! Collectors still write [`crate::model::Fields`] bags and snake_case relationship strings.
//! This module is the registry those values are interpreted against: required/optional
//! properties, inverse verbs, allowed endpoints, and the Action catalog. Unknown verbs
//! still store; Object View groups them under "Other".

use std::collections::{BTreeMap, HashSet};

use super::agent::AgentAction;
use super::common::Fields;
use super::entity::{Entity, EntityKind};
use super::finding::Finding;
use super::page::{Page, PageKind};
use super::relationship::{Relationship, kinds, normalize_relationship_kind};
use super::resolution::Resolution;
use super::source::Source;
use super::workspace::Workspace;

/// How a documented property is typed. Values stay strings on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyType {
    String,
    Hostname,
    Ip,
    Url,
    Email,
    Integer,
    Timestamp,
    Asn,
    Fingerprint,
    Coordinate,
}

impl PropertyType {
    pub fn as_str(self) -> &'static str {
        match self {
            PropertyType::String => "string",
            PropertyType::Hostname => "hostname",
            PropertyType::Ip => "ip",
            PropertyType::Url => "url",
            PropertyType::Email => "email",
            PropertyType::Integer => "integer",
            PropertyType::Timestamp => "timestamp",
            PropertyType::Asn => "asn",
            PropertyType::Fingerprint => "fingerprint",
            PropertyType::Coordinate => "coordinate",
        }
    }
}

/// One documented field on an object type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertySpec {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: PropertyType,
    pub required: bool,
    pub description: &'static str,
}

/// A named group of properties shown together in Object View.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyGroup {
    pub id: &'static str,
    pub label: &'static str,
    pub properties: &'static [PropertySpec],
}

/// A registered page/entity kind and its property schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectType {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub entity_kind: Option<&'static str>,
    pub page_kind: Option<&'static str>,
    pub groups: &'static [PropertyGroup],
}

impl ObjectType {
    pub fn entity_kind(&self) -> Option<EntityKind> {
        self.entity_kind.map(EntityKind::parse)
    }

    pub fn page_kind(&self) -> Option<PageKind> {
        self.page_kind.map(PageKind::parse)
    }

    pub fn property(&self, key: &str) -> Option<&'static PropertySpec> {
        self.groups
            .iter()
            .flat_map(|group| group.properties)
            .find(|property| property.key == key)
    }
}

const IDENTITY: PropertyGroup = PropertyGroup {
    id: "identity",
    label: "Identity",
    properties: &[
        PropertySpec {
            key: "kind",
            label: "Kind",
            kind: PropertyType::String,
            required: true,
            description: "Registered object type key.",
        },
        PropertySpec {
            key: "value",
            label: "Value",
            kind: PropertyType::String,
            required: true,
            description: "Canonical identifier for this object.",
        },
        PropertySpec {
            key: "label",
            label: "Label",
            kind: PropertyType::String,
            required: false,
            description: "Optional display name.",
        },
        PropertySpec {
            key: "provenance",
            label: "Provenance",
            kind: PropertyType::String,
            required: true,
            description: "collected, external, analyst, or agent.",
        },
    ],
};

const DOMAIN_REGISTRATION: PropertyGroup = PropertyGroup {
    id: "registration",
    label: "Registration",
    properties: &[
        PropertySpec {
            key: "registrar",
            label: "Registrar",
            kind: PropertyType::String,
            required: false,
            description: "Registrar named by RDAP.",
        },
        PropertySpec {
            key: "created",
            label: "Created",
            kind: PropertyType::Timestamp,
            required: false,
            description: "Registration or allocation time.",
        },
        PropertySpec {
            key: "updated",
            label: "Updated",
            kind: PropertyType::Timestamp,
            required: false,
            description: "Last registry update.",
        },
        PropertySpec {
            key: "expiration",
            label: "Expiration",
            kind: PropertyType::Timestamp,
            required: false,
            description: "Registry expiry.",
        },
        PropertySpec {
            key: "status",
            label: "Status",
            kind: PropertyType::String,
            required: false,
            description: "Registry or DNS status.",
        },
    ],
};

const DNS_POSTURE: PropertyGroup = PropertyGroup {
    id: "dns",
    label: "DNS and diagnostics",
    properties: &[
        PropertySpec {
            key: "diagnostics",
            label: "Diagnostics",
            kind: PropertyType::String,
            required: false,
            description: "Collected posture summary.",
        },
        PropertySpec {
            key: "spf",
            label: "SPF",
            kind: PropertyType::String,
            required: false,
            description: "Published SPF record.",
        },
        PropertySpec {
            key: "dmarc_policy",
            label: "DMARC policy",
            kind: PropertyType::String,
            required: false,
            description: "Published DMARC policy.",
        },
    ],
};

const NETWORK: PropertyGroup = PropertyGroup {
    id: "network",
    label: "Network",
    properties: &[
        PropertySpec {
            key: "asn",
            label: "ASN",
            kind: PropertyType::Asn,
            required: false,
            description: "Origin autonomous system.",
        },
        PropertySpec {
            key: "prefix",
            label: "Prefix",
            kind: PropertyType::String,
            required: false,
            description: "Announced prefix.",
        },
        PropertySpec {
            key: "country",
            label: "Country",
            kind: PropertyType::String,
            required: false,
            description: "Registry country.",
        },
        PropertySpec {
            key: "registry",
            label: "Registry",
            kind: PropertyType::String,
            required: false,
            description: "RIR or registry name.",
        },
        PropertySpec {
            key: "allocated",
            label: "Allocated",
            kind: PropertyType::Timestamp,
            required: false,
            description: "Allocation date.",
        },
        PropertySpec {
            key: "as_name",
            label: "AS name",
            kind: PropertyType::String,
            required: false,
            description: "Operator name for the ASN.",
        },
        PropertySpec {
            key: "ptr",
            label: "PTR",
            kind: PropertyType::Hostname,
            required: false,
            description: "Reverse DNS names.",
        },
        PropertySpec {
            key: "latitude",
            label: "Latitude",
            kind: PropertyType::Coordinate,
            required: false,
            description: "WGS-84 latitude when a local geo database or registry provided it.",
        },
        PropertySpec {
            key: "longitude",
            label: "Longitude",
            kind: PropertyType::Coordinate,
            required: false,
            description: "WGS-84 longitude when a local geo database or registry provided it.",
        },
        PropertySpec {
            key: "geo_source",
            label: "Geo source",
            kind: PropertyType::String,
            required: false,
            description: "How coordinates were derived: collected, geo_db, or country_centroid.",
        },
    ],
};

const GEO: PropertyGroup = PropertyGroup {
    id: "geo",
    label: "Location",
    properties: &[
        PropertySpec {
            key: "country",
            label: "Country",
            kind: PropertyType::String,
            required: false,
            description: "ISO country from registry, RDAP, or a local geo database.",
        },
        PropertySpec {
            key: "latitude",
            label: "Latitude",
            kind: PropertyType::Coordinate,
            required: false,
            description: "WGS-84 latitude.",
        },
        PropertySpec {
            key: "longitude",
            label: "Longitude",
            kind: PropertyType::Coordinate,
            required: false,
            description: "WGS-84 longitude.",
        },
        PropertySpec {
            key: "geo_source",
            label: "Geo source",
            kind: PropertyType::String,
            required: false,
            description: "How coordinates were derived.",
        },
    ],
};

const CERTIFICATE: PropertyGroup = PropertyGroup {
    id: "certificate",
    label: "Certificate",
    properties: &[
        PropertySpec {
            key: "subject",
            label: "Subject",
            kind: PropertyType::String,
            required: false,
            description: "Certificate subject.",
        },
        PropertySpec {
            key: "issuer",
            label: "Issuer",
            kind: PropertyType::String,
            required: false,
            description: "Issuing authority.",
        },
        PropertySpec {
            key: "not_before",
            label: "Not before",
            kind: PropertyType::Timestamp,
            required: false,
            description: "Validity start.",
        },
        PropertySpec {
            key: "not_after",
            label: "Not after",
            kind: PropertyType::Timestamp,
            required: false,
            description: "Validity end.",
        },
        PropertySpec {
            key: "sha256",
            label: "SHA-256",
            kind: PropertyType::Fingerprint,
            required: false,
            description: "Certificate fingerprint.",
        },
        PropertySpec {
            key: "san_count",
            label: "SAN count",
            kind: PropertyType::Integer,
            required: false,
            description: "Number of subject alternative names.",
        },
        PropertySpec {
            key: "validation",
            label: "Validation",
            kind: PropertyType::String,
            required: false,
            description: "Handshake validation result.",
        },
        PropertySpec {
            key: "diagnostics",
            label: "Diagnostics",
            kind: PropertyType::String,
            required: false,
            description: "Collected certificate posture.",
        },
    ],
};

const HTTP: PropertyGroup = PropertyGroup {
    id: "http",
    label: "HTTP",
    properties: &[
        PropertySpec {
            key: "status",
            label: "Status",
            kind: PropertyType::Integer,
            required: false,
            description: "HTTP status code.",
        },
        PropertySpec {
            key: "final_url",
            label: "Final URL",
            kind: PropertyType::Url,
            required: false,
            description: "URL after redirects.",
        },
        PropertySpec {
            key: "server",
            label: "Server",
            kind: PropertyType::String,
            required: false,
            description: "Server header.",
        },
        PropertySpec {
            key: "content-type",
            label: "Content type",
            kind: PropertyType::String,
            required: false,
            description: "Response content type.",
        },
        PropertySpec {
            key: "title",
            label: "Title",
            kind: PropertyType::String,
            required: false,
            description: "HTML title.",
        },
        PropertySpec {
            key: "diagnostics",
            label: "Diagnostics",
            kind: PropertyType::String,
            required: false,
            description: "Header and policy posture.",
        },
    ],
};

const REPOSITORY: PropertyGroup = PropertyGroup {
    id: "repository",
    label: "Repository",
    properties: &[
        PropertySpec {
            key: "url",
            label: "URL",
            kind: PropertyType::Url,
            required: false,
            description: "Canonical remote URL.",
        },
        PropertySpec {
            key: "owner",
            label: "Owner",
            kind: PropertyType::String,
            required: false,
            description: "Publishing organization or user.",
        },
        PropertySpec {
            key: "name",
            label: "Name",
            kind: PropertyType::String,
            required: false,
            description: "Repository name.",
        },
        PropertySpec {
            key: "head",
            label: "Head",
            kind: PropertyType::String,
            required: false,
            description: "Default branch.",
        },
        PropertySpec {
            key: "contributors",
            label: "Contributors",
            kind: PropertyType::Integer,
            required: false,
            description: "Count of observed contributor emails.",
        },
    ],
};

const FILE: PropertyGroup = PropertyGroup {
    id: "file",
    label: "File",
    properties: &[
        PropertySpec {
            key: "file_name",
            label: "File name",
            kind: PropertyType::String,
            required: false,
            description: "Original file name.",
        },
        PropertySpec {
            key: "size_bytes",
            label: "Size",
            kind: PropertyType::Integer,
            required: false,
            description: "Size in bytes.",
        },
        PropertySpec {
            key: "sha256",
            label: "SHA-256",
            kind: PropertyType::Fingerprint,
            required: false,
            description: "Content hash.",
        },
        PropertySpec {
            key: "mime",
            label: "MIME",
            kind: PropertyType::String,
            required: false,
            description: "Detected MIME type.",
        },
        PropertySpec {
            key: "created",
            label: "Created",
            kind: PropertyType::Timestamp,
            required: false,
            description: "Filesystem created time.",
        },
        PropertySpec {
            key: "modified",
            label: "Modified",
            kind: PropertyType::Timestamp,
            required: false,
            description: "Filesystem modified time.",
        },
    ],
};

const PAGE: PropertyGroup = PropertyGroup {
    id: "page",
    label: "Page",
    properties: &[
        PropertySpec {
            key: "title",
            label: "Title",
            kind: PropertyType::String,
            required: true,
            description: "Page title.",
        },
        PropertySpec {
            key: "target",
            label: "Target",
            kind: PropertyType::String,
            required: false,
            description: "Collector target when the page has one.",
        },
        PropertySpec {
            key: "description",
            label: "Description",
            kind: PropertyType::String,
            required: false,
            description: "Analyst description.",
        },
        PropertySpec {
            key: "notes",
            label: "Notes",
            kind: PropertyType::String,
            required: false,
            description: "Page notes written by the analyst.",
        },
    ],
};

macro_rules! object_type {
    ($key:literal, $label:literal, $description:literal, $entity:expr, $page:expr, $groups:expr) => {
        ObjectType {
            key: $key,
            label: $label,
            description: $description,
            entity_kind: $entity,
            page_kind: $page,
            groups: $groups,
        }
    };
}

/// Built-in object types. Keys match [`EntityKind`] / [`PageKind`] storage keys.
pub const OBJECT_TYPES: &[ObjectType] = &[
    object_type!(
        "domain",
        "Domain",
        "A registered DNS name.",
        Some("domain"),
        Some("domain"),
        &[IDENTITY, DOMAIN_REGISTRATION, DNS_POSTURE]
    ),
    object_type!(
        "hostname",
        "Hostname",
        "A host or nameserver that may not be a registrable domain.",
        Some("hostname"),
        None,
        &[IDENTITY, DNS_POSTURE]
    ),
    object_type!(
        "ip",
        "IP address",
        "An IPv4 or IPv6 address.",
        Some("ip"),
        Some("ip"),
        &[IDENTITY, NETWORK]
    ),
    object_type!(
        "asn",
        "ASN",
        "An autonomous system.",
        Some("asn"),
        Some("asn"),
        &[IDENTITY, NETWORK]
    ),
    object_type!(
        "url",
        "URL",
        "A retrieved web address.",
        Some("url"),
        Some("url"),
        &[IDENTITY, HTTP]
    ),
    object_type!(
        "email",
        "Email",
        "An email address.",
        Some("email"),
        Some("email"),
        &[IDENTITY]
    ),
    object_type!(
        "username",
        "Username",
        "A handle or account name.",
        Some("username"),
        Some("username"),
        &[IDENTITY]
    ),
    object_type!(
        "repository",
        "Repository",
        "A public source repository.",
        Some("repository"),
        Some("repository"),
        &[IDENTITY, REPOSITORY]
    ),
    object_type!(
        "file",
        "File",
        "A local file the analyst pointed at.",
        Some("file"),
        Some("file"),
        &[IDENTITY, FILE]
    ),
    object_type!(
        "certificate",
        "Certificate",
        "A TLS or CT-observed certificate.",
        Some("certificate"),
        None,
        &[IDENTITY, CERTIFICATE]
    ),
    object_type!(
        "person",
        "Person",
        "A person under investigation.",
        Some("person"),
        Some("person"),
        &[IDENTITY, PAGE]
    ),
    object_type!(
        "organization",
        "Organization",
        "An organization, registrar, or operator.",
        Some("organization"),
        Some("organization"),
        &[IDENTITY, PAGE, GEO]
    ),
    object_type!(
        "location",
        "Location",
        "A place name.",
        Some("location"),
        None,
        &[IDENTITY, GEO]
    ),
    object_type!(
        "phone",
        "Phone",
        "A telephone number.",
        Some("phone"),
        None,
        &[IDENTITY]
    ),
    object_type!(
        "unknown",
        "Unknown",
        "An identifier whose kind is not registered.",
        Some("unknown"),
        None,
        &[IDENTITY]
    ),
    object_type!(
        "generic",
        "Generic page",
        "A notes page without a collector target.",
        None,
        Some("generic"),
        &[IDENTITY, PAGE]
    ),
    object_type!(
        "event",
        "Event page",
        "A subject page for an incident or occurrence. First-class Event objects arrive later.",
        None,
        Some("event"),
        &[IDENTITY, PAGE]
    ),
];

/// Looks up a registered object type by entity or page storage key.
pub fn object_type(key: &str) -> Option<&'static ObjectType> {
    let normalized = key.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    OBJECT_TYPES.iter().find(|item| {
        item.key == normalized
            || item.entity_kind == Some(normalized.as_str())
            || item.page_kind == Some(normalized.as_str())
    })
}

pub fn object_type_for_entity(kind: &EntityKind) -> &'static ObjectType {
    object_type(kind.as_str()).unwrap_or_else(|| object_type("unknown").expect("unknown type"))
}

pub fn object_type_for_page(kind: &PageKind) -> &'static ObjectType {
    object_type(kind.as_str()).unwrap_or_else(|| object_type("generic").expect("generic type"))
}

/// A built-in or first-seen relationship verb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkType {
    pub kind: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub inverse: Option<&'static str>,
    pub from_kinds: &'static [&'static str],
    pub to_kinds: &'static [&'static str],
}

impl LinkType {
    pub fn allows_from(&self, kind: &EntityKind) -> bool {
        allows_kind(self.from_kinds, kind)
    }

    pub fn allows_to(&self, kind: &EntityKind) -> bool {
        allows_kind(self.to_kinds, kind)
    }
}

fn allows_kind(allowed: &[&str], kind: &EntityKind) -> bool {
    allowed.is_empty() || allowed.iter().any(|item| *item == kind.as_str())
}

const ANY: &[&str] = &[];

/// Built-in link verbs. Collectors emit these strings; unknown snake_case still stores.
pub const LINK_TYPES: &[LinkType] = &[
    LinkType {
        kind: kinds::RESOLVES_TO,
        label: "Resolves to",
        description: "A name resolved to an address.",
        inverse: Some(kinds::REVERSE_DNS),
        from_kinds: &["domain", "hostname"],
        to_kinds: &["ip"],
    },
    LinkType {
        kind: kinds::REVERSE_DNS,
        label: "Reverse DNS",
        description: "The hostname in a pointer record for an address.",
        inverse: Some(kinds::RESOLVES_TO),
        from_kinds: &["ip"],
        to_kinds: &["hostname", "domain"],
    },
    LinkType {
        kind: kinds::ALIAS_OF,
        label: "Alias of",
        description: "A CNAME or another name for the same subject.",
        inverse: Some(kinds::ALIAS_OF),
        from_kinds: &["domain", "hostname"],
        to_kinds: &["domain", "hostname"],
    },
    LinkType {
        kind: kinds::USES_NAMESERVER,
        label: "Uses nameserver",
        description: "A domain's nameserver.",
        inverse: None,
        from_kinds: &["domain"],
        to_kinds: &["hostname", "domain"],
    },
    LinkType {
        kind: kinds::MAIL_HANDLED_BY,
        label: "Mail handled by",
        description: "The target of an MX record.",
        inverse: None,
        from_kinds: &["domain", "email"],
        to_kinds: &["hostname", "domain"],
    },
    LinkType {
        kind: kinds::SPF_INCLUDES,
        label: "SPF includes",
        description: "A domain named by an SPF include or redirect.",
        inverse: None,
        from_kinds: &["domain"],
        to_kinds: &["domain", "hostname"],
    },
    LinkType {
        kind: kinds::ANNOUNCED_BY,
        label: "Announced by",
        description: "The origin ASN for an address.",
        inverse: None,
        from_kinds: &["ip"],
        to_kinds: &["asn"],
    },
    LinkType {
        kind: kinds::OPERATED_BY,
        label: "Operated by",
        description: "A network operator, or the organization that publishes a repository.",
        inverse: None,
        from_kinds: ANY,
        to_kinds: &["organization", "person"],
    },
    LinkType {
        kind: kinds::REGISTERED_WITH,
        label: "Registered with",
        description: "The registrar named by RDAP.",
        inverse: None,
        from_kinds: &["domain"],
        to_kinds: &["organization"],
    },
    LinkType {
        kind: kinds::REGISTERED_TO,
        label: "Registered to",
        description: "The registrant named by RDAP.",
        inverse: None,
        from_kinds: &["domain"],
        to_kinds: &["organization", "person", "domain"],
    },
    LinkType {
        kind: kinds::REDIRECTS_TO,
        label: "Redirects to",
        description: "One HTTP hop to the next URL.",
        inverse: None,
        from_kinds: &["url", "domain", "hostname"],
        to_kinds: &["url", "domain", "hostname"],
    },
    LinkType {
        kind: kinds::SERVES,
        label: "Serves",
        description: "The final URL a collected page was served from.",
        inverse: None,
        from_kinds: &["domain", "hostname", "url"],
        to_kinds: &["url"],
    },
    LinkType {
        kind: kinds::LINKS_TO,
        label: "Links to",
        description: "A hostname linked from a collected page.",
        inverse: None,
        from_kinds: &["url", "domain", "hostname"],
        to_kinds: &["hostname", "domain", "url"],
    },
    LinkType {
        kind: kinds::MENTIONS,
        label: "Mentions",
        description: "An identifier mentioned in collected text.",
        inverse: None,
        from_kinds: ANY,
        to_kinds: ANY,
    },
    LinkType {
        kind: kinds::PRESENTS_CERTIFICATE,
        label: "Presents certificate",
        description: "The certificate a TLS handshake presented.",
        inverse: None,
        from_kinds: &["domain", "hostname", "url"],
        to_kinds: &["certificate"],
    },
    LinkType {
        kind: kinds::ISSUED_BY,
        label: "Issued by",
        description: "The issuer of a certificate.",
        inverse: None,
        from_kinds: &["certificate"],
        to_kinds: &["organization"],
    },
    LinkType {
        kind: kinds::COVERS,
        label: "Covers",
        description: "A name listed on a certificate.",
        inverse: None,
        from_kinds: &["certificate"],
        to_kinds: &["domain", "hostname"],
    },
    LinkType {
        kind: kinds::SUBDOMAIN_OF,
        label: "Subdomain of",
        description: "A discovered hostname beneath the name that was collected.",
        inverse: Some(kinds::CONTAINS),
        from_kinds: &["domain", "hostname"],
        to_kinds: &["domain", "hostname"],
    },
    LinkType {
        kind: kinds::CONTAINS,
        label: "Contains",
        description: "Inverse of subdomain_of. Built-in collectors do not write it.",
        inverse: Some(kinds::SUBDOMAIN_OF),
        from_kinds: &["domain", "hostname"],
        to_kinds: &["domain", "hostname"],
    },
    LinkType {
        kind: kinds::REFERENCES,
        label: "References",
        description: "A URL or host a page, policy, or repository points at.",
        inverse: None,
        from_kinds: ANY,
        to_kinds: ANY,
    },
    LinkType {
        kind: kinds::CONTRIBUTED_TO,
        label: "Contributed to",
        description: "An email address that committed to a repository.",
        inverse: None,
        from_kinds: &["email", "person"],
        to_kinds: &["repository"],
    },
    LinkType {
        kind: kinds::HOSTED_ON,
        label: "Hosted on",
        description: "An email address at a domain.",
        inverse: None,
        from_kinds: &["email"],
        to_kinds: &["domain"],
    },
    LinkType {
        kind: kinds::ASSOCIATED_WITH,
        label: "Associated with",
        description: "Fallback when a relationship name normalizes to an empty string.",
        inverse: Some(kinds::ASSOCIATED_WITH),
        from_kinds: ANY,
        to_kinds: ANY,
    },
];

/// Group label used in Object View for verbs that are not in [`LINK_TYPES`].
pub const OTHER_LINK_GROUP: &str = "Other";

/// Looks up a built-in link type after snake_case normalization.
pub fn link_type(kind: &str) -> Option<&'static LinkType> {
    let normalized = normalize_relationship_kind(kind);
    LINK_TYPES.iter().find(|item| item.kind == normalized)
}

/// Inverse verb registered for `kind`, if the catalog names one.
pub fn inverse_of(kind: &str) -> Option<&'static str> {
    link_type(kind).and_then(|item| item.inverse)
}

/// Catalog grouping for a stored verb. Unknown verbs are `None` and belong under "Other".
pub fn link_group_label(kind: &str) -> &'static str {
    link_type(kind)
        .map(|item| item.label)
        .unwrap_or(OTHER_LINK_GROUP)
}

/// Whether this verb is a built-in catalog entry.
pub fn is_catalogued_link(kind: &str) -> bool {
    link_type(kind).is_some()
}

/// A recorded custom (or first-seen) link verb for one investigation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CataloguedLink {
    pub kind: String,
    pub description: String,
    pub inverse: Option<String>,
}

impl CataloguedLink {
    pub fn from_first_use(kind: &str) -> Self {
        let normalized = normalize_relationship_kind(kind);
        match link_type(&normalized) {
            Some(known) => Self {
                kind: known.kind.to_string(),
                description: known.description.to_string(),
                inverse: known.inverse.map(str::to_string),
            },
            None => Self {
                kind: normalized,
                description: String::new(),
                inverse: None,
            },
        }
    }

    pub fn is_other(&self) -> bool {
        !is_catalogued_link(&self.kind)
    }
}

/// An Action kind the workbench can store. Reserved kinds are catalogued but refused on accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionType {
    pub kind: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    /// `false` for stubs that parse/store but must not apply yet.
    pub implemented: bool,
}

pub const ACTION_TYPES: &[ActionType] = &[
    ActionType {
        kind: "ADD_FINDING",
        label: "Add finding",
        description: "Record a claim. Accepted AI findings keep agent provenance.",
        implemented: true,
    },
    ActionType {
        kind: "ADD_NOTE",
        label: "Add note",
        description: "Write a standalone note.",
        implemented: true,
    },
    ActionType {
        kind: "CREATE_PAGE",
        label: "Create page",
        description: "Add a subject page.",
        implemented: true,
    },
    ActionType {
        kind: "LINK_ENTITY",
        label: "Link entity",
        description: "Create a directed relationship between two entities.",
        implemented: true,
    },
    ActionType {
        kind: "RUN_COLLECTOR",
        label: "Run collector",
        description: "Queue a passive collector against a target.",
        implemented: true,
    },
    ActionType {
        kind: "GENERATE_REPORT",
        label: "Generate report",
        description: "Write a Markdown or JSON report.",
        implemented: true,
    },
    ActionType {
        kind: "PROPOSE_RESOLVE",
        label: "Propose resolve",
        description: "Suggest merging entities that are the same real-world thing. Accept is required.",
        implemented: true,
    },
    ActionType {
        kind: "PROPOSE_UNRESOLVE",
        label: "Propose unresolve",
        description: "Suggest splitting a previously resolved entity. Accept is required.",
        implemented: true,
    },
    ActionType {
        kind: "RUN_PLAYBOOK",
        label: "Run playbook",
        description: "Run a bundled playbook. Accept is required. Nested playbooks and active tools are refused. Automations propose; they do not silently write.",
        implemented: true,
    },
    ActionType {
        kind: "ASSERT_FINDING_TEMPLATE",
        label: "Assert finding template",
        description: "Propose a templated finding from a playbook. Accept writes the finding; playbooks never write it silently.",
        implemented: true,
    },
];

pub fn action_type(kind: &str) -> Option<&'static ActionType> {
    let normalized = kind.trim().to_ascii_uppercase().replace('-', "_");
    ACTION_TYPES.iter().find(|item| item.kind == normalized)
}

pub fn action_type_for(action: &AgentAction) -> &'static ActionType {
    action_type(action.name()).expect("every AgentAction variant is catalogued")
}

/// Local actor recorded on accept/reject. Uses [`super::Identity::from_env`].
pub fn local_actor() -> String {
    super::Identity::from_env().user_id
}

/// One property shown in Object View.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyValue {
    pub key: String,
    pub label: String,
    pub value: String,
    pub required: bool,
}

/// A property group with the values present on this object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyGroupView {
    pub id: String,
    pub label: String,
    pub properties: Vec<PropertyValue>,
}

/// A linked object grouped under a catalog verb (or "Other").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedObject {
    pub direction: LinkDirection,
    pub relationship_kind: String,
    pub entity_ref: String,
    pub entity_label: String,
    pub entity_kind: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinkDirection {
    Outgoing,
    Incoming,
}

/// Linked objects that share a catalog group label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkGroupView {
    pub label: String,
    pub catalogued: bool,
    pub links: Vec<LinkedObject>,
}

/// Supporting source row for Object View.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectSource {
    pub reference: String,
    pub title: String,
    pub provenance: String,
}

/// Finding row for Object View.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectFinding {
    pub reference: String,
    pub title: String,
    pub provenance: String,
}

/// A constituent absorbed into the canonical object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectConstituent {
    pub reference: String,
    pub kind: String,
    pub value: String,
    pub label: String,
}

/// Read model for the Object View screen. Built from a workspace snapshot; no I/O.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectView {
    pub object_type: String,
    pub object_label: String,
    pub title: String,
    pub short_ref: String,
    pub groups: Vec<PropertyGroupView>,
    pub links: Vec<LinkGroupView>,
    pub sources: Vec<ObjectSource>,
    pub findings: Vec<ObjectFinding>,
    pub constituents: Vec<ObjectConstituent>,
    pub resolution_ref: Option<String>,
}

impl ObjectView {
    pub fn for_entity(workspace: &Workspace, entity: &Entity) -> Self {
        let object_type = object_type_for_entity(&entity.kind);
        let mut identity = Fields::new();
        identity.push("kind", entity.kind.as_str());
        identity.push("value", entity.value.clone());
        identity.push("label", entity.label.clone());
        identity.push("provenance", entity.provenance.as_str());
        identity.push("marking", entity.marking().as_str());
        let groups = property_groups(object_type, &identity, &entity.fields);
        let members = object_member_ids(workspace, entity.id);
        let mut source_ids = entity.source_ids.clone();
        for member_id in &members {
            if let Some(member) = workspace.entity(*member_id) {
                for source_id in &member.source_ids {
                    if !source_ids.contains(source_id) {
                        source_ids.push(*source_id);
                    }
                }
            }
        }
        Self {
            object_type: object_type.key.to_string(),
            object_label: object_type.label.to_string(),
            title: entity.display_name().to_string(),
            short_ref: entity.short_ref().to_string(),
            groups,
            links: link_groups(workspace, &members),
            sources: sources_for(workspace, &source_ids),
            findings: findings_for_members(workspace, &members),
            constituents: constituents_for(workspace, entity.id),
            resolution_ref: workspace
                .latest_active_resolution(entity.id)
                .map(Resolution::short_ref)
                .map(|reference| reference.to_string()),
        }
    }

    pub fn for_page(workspace: &Workspace, page: &Page) -> Self {
        if let Some(entity) = page.entity_id.and_then(|id| workspace.entity(id)) {
            let mut view = Self::for_entity(workspace, entity);
            let mut page_fields = Fields::new();
            page_fields.push("title", page.title.clone());
            page_fields.push_opt("target", page.target.clone());
            page_fields.push("description", page.description.clone());
            page_fields.push("notes", page.notes.clone());
            if let Some(page_group) = property_groups(
                object_type_for_page(&page.kind),
                &Fields::new(),
                &page_fields,
            )
            .into_iter()
            .find(|group| group.id == "page")
                && !page_group.properties.is_empty()
            {
                view.groups.push(page_group);
            }
            return view;
        }
        let object_type = object_type_for_page(&page.kind);
        let mut identity = Fields::new();
        identity.push("kind", page.kind.as_str());
        identity.push(
            "value",
            page.target.clone().unwrap_or_else(|| page.title.clone()),
        );
        identity.push("label", page.title.clone());
        identity.push("provenance", "analyst");
        let mut page_fields = Fields::new();
        page_fields.push("title", page.title.clone());
        page_fields.push_opt("target", page.target.clone());
        page_fields.push("description", page.description.clone());
        page_fields.push("notes", page.notes.clone());
        Self {
            object_type: object_type.key.to_string(),
            object_label: object_type.label.to_string(),
            title: page.title.clone(),
            short_ref: page.short_ref().to_string(),
            groups: property_groups(object_type, &identity, &page_fields),
            links: Vec::new(),
            sources: Vec::new(),
            findings: findings_for_page(workspace, page.id),
            constituents: Vec::new(),
            resolution_ref: None,
        }
    }
}

fn property_groups(
    object_type: &ObjectType,
    identity: &Fields,
    fields: &Fields,
) -> Vec<PropertyGroupView> {
    let mut used = Vec::new();
    let mut groups = Vec::new();
    for group in object_type.groups {
        let source = if group.id == "identity" {
            identity
        } else {
            fields
        };
        let properties: Vec<PropertyValue> = group
            .properties
            .iter()
            .filter_map(|spec| {
                let value = source.get(spec.key)?;
                if value.trim().is_empty() && !spec.required {
                    return None;
                }
                used.push(spec.key);
                Some(PropertyValue {
                    key: spec.key.to_string(),
                    label: spec.label.to_string(),
                    value: value.to_string(),
                    required: spec.required,
                })
            })
            .collect();
        if !properties.is_empty() {
            groups.push(PropertyGroupView {
                id: group.id.to_string(),
                label: group.label.to_string(),
                properties,
            });
        }
    }
    let other: Vec<PropertyValue> = fields
        .iter()
        .filter(|(key, _)| !used.iter().any(|known| known == key))
        .map(|(key, value)| PropertyValue {
            key: key.to_string(),
            label: key.to_string(),
            value: value.to_string(),
            required: false,
        })
        .collect();
    if !other.is_empty() {
        groups.push(PropertyGroupView {
            id: "other".into(),
            label: "Other".into(),
            properties: other,
        });
    }
    groups
}

fn object_member_ids(workspace: &Workspace, entity_id: uuid::Uuid) -> Vec<uuid::Uuid> {
    let canonical = workspace.canonical_id(entity_id);
    let mut ids = vec![canonical];
    ids.extend(workspace.data.constituent_ids_of(canonical));
    ids
}

fn constituents_for(workspace: &Workspace, entity_id: uuid::Uuid) -> Vec<ObjectConstituent> {
    workspace
        .constituents_of(workspace.canonical_id(entity_id))
        .into_iter()
        .map(|entity| ObjectConstituent {
            reference: entity.short_ref().to_string(),
            kind: entity.kind.as_str().to_string(),
            value: entity.value.clone(),
            label: entity.display_name().to_string(),
        })
        .collect()
}

fn link_groups(workspace: &Workspace, member_ids: &[uuid::Uuid]) -> Vec<LinkGroupView> {
    let mut grouped: BTreeMap<String, LinkGroupView> = BTreeMap::new();
    let mut seen: HashSet<(LinkDirection, String, uuid::Uuid)> = HashSet::new();
    let push = |grouped: &mut BTreeMap<String, LinkGroupView>,
                seen: &mut HashSet<(LinkDirection, String, uuid::Uuid)>,
                relationship: &Relationship,
                direction: LinkDirection,
                other: &Entity| {
        if !seen.insert((direction, relationship.kind.clone(), other.id)) {
            return;
        }
        let label = link_group_label(&relationship.kind);
        let catalogued = is_catalogued_link(&relationship.kind);
        let entry = grouped.entry(label.to_string()).or_insert(LinkGroupView {
            label: label.to_string(),
            catalogued,
            links: Vec::new(),
        });
        entry.links.push(LinkedObject {
            direction,
            relationship_kind: relationship.kind.clone(),
            entity_ref: other.short_ref().to_string(),
            entity_label: other.display_name().to_string(),
            entity_kind: other.kind.as_str().to_string(),
        });
    };
    let members: std::collections::HashSet<uuid::Uuid> = member_ids.iter().copied().collect();
    for member_id in member_ids {
        for relationship in workspace.outgoing(*member_id) {
            let other_id = workspace.canonical_id(relationship.to_id);
            if members.contains(&other_id) {
                continue;
            }
            if let Some(other) = workspace.entity(other_id) {
                push(
                    &mut grouped,
                    &mut seen,
                    relationship,
                    LinkDirection::Outgoing,
                    other,
                );
            }
        }
        for relationship in workspace.incoming(*member_id) {
            let other_id = workspace.canonical_id(relationship.from_id);
            if members.contains(&other_id) {
                continue;
            }
            if let Some(other) = workspace.entity(other_id) {
                push(
                    &mut grouped,
                    &mut seen,
                    relationship,
                    LinkDirection::Incoming,
                    other,
                );
            }
        }
    }
    let mut groups: Vec<LinkGroupView> = grouped.into_values().collect();
    groups.sort_by(|left, right| match (left.catalogued, right.catalogued) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => left.label.cmp(&right.label),
    });
    groups
}

fn sources_for(workspace: &Workspace, source_ids: &[uuid::Uuid]) -> Vec<ObjectSource> {
    source_ids
        .iter()
        .filter_map(|id| workspace.source(*id))
        .map(object_source)
        .collect()
}

fn object_source(source: &Source) -> ObjectSource {
    ObjectSource {
        reference: source.short_ref().to_string(),
        title: source.title.clone(),
        provenance: source.provenance.as_str().to_string(),
    }
}

fn findings_for_members(workspace: &Workspace, member_ids: &[uuid::Uuid]) -> Vec<ObjectFinding> {
    let page_ids: Vec<_> = member_ids
        .iter()
        .flat_map(|id| workspace.pages_for_entity(*id))
        .map(|page| page.id)
        .collect();
    let member_sources: Vec<uuid::Uuid> = member_ids
        .iter()
        .filter_map(|id| workspace.entity(*id))
        .flat_map(|entity| entity.source_ids.iter().copied())
        .collect();
    workspace
        .data
        .findings
        .iter()
        .filter(|finding| {
            finding
                .page_id
                .is_some_and(|page_id| page_ids.contains(&page_id))
                || finding
                    .source_ids
                    .iter()
                    .any(|source_id| member_sources.contains(source_id))
        })
        .map(object_finding)
        .collect()
}

fn findings_for_page(workspace: &Workspace, page_id: uuid::Uuid) -> Vec<ObjectFinding> {
    workspace
        .data
        .findings
        .iter()
        .filter(|finding| finding.page_id == Some(page_id))
        .map(object_finding)
        .collect()
}

fn object_finding(finding: &Finding) -> ObjectFinding {
    ObjectFinding {
        reference: finding.short_ref().to_string(),
        title: finding.title.clone(),
        provenance: finding.provenance.as_str().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Confidence, Investigation, InvestigationData, Provenance, Relationship, now,
    };
    use uuid::Uuid;

    fn sample_entity(kind: EntityKind, value: &str) -> Entity {
        Entity {
            id: Uuid::new_v4(),
            investigation_id: Uuid::new_v4(),
            seq: 1,
            kind,
            value: value.into(),
            label: String::new(),
            notes: String::new(),
            provenance: Provenance::Collected,
            fields: Fields::new(),
            source_ids: Vec::new(),
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn catalog_lookup_finds_known_object_and_link_types() {
        let domain = object_type("domain").expect("domain type");
        assert_eq!(domain.label, "Domain");
        assert!(domain.property("registrar").is_some());
        assert!(domain.property("value").is_some());
        assert!(object_type("not-a-type").is_none());
        let resolves = link_type("Resolves To").expect("resolves_to");
        assert_eq!(resolves.kind, kinds::RESOLVES_TO);
        assert_eq!(resolves.label, "Resolves to");
        assert!(resolves.allows_from(&EntityKind::Domain));
        assert!(resolves.allows_to(&EntityKind::Ip));
        assert!(!resolves.allows_to(&EntityKind::Email));
    }

    #[test]
    fn inverse_registration_is_symmetric_for_pairs() {
        for link in LINK_TYPES {
            let Some(inverse) = link.inverse else {
                continue;
            };
            let registered = link_type(inverse).expect("inverse is catalogued");
            assert_eq!(
                registered.inverse,
                Some(link.kind),
                "{} inverse {} must point back",
                link.kind,
                inverse
            );
        }
        assert_eq!(inverse_of("subdomain_of"), Some(kinds::CONTAINS));
        assert_eq!(inverse_of("contains"), Some(kinds::SUBDOMAIN_OF));
        assert_eq!(inverse_of("resolves_to"), Some(kinds::REVERSE_DNS));
        assert_eq!(inverse_of("alias_of"), Some(kinds::ALIAS_OF));
    }

    #[test]
    fn unknown_verbs_store_under_other() {
        assert!(link_type("mentions_in_passing").is_none());
        assert_eq!(link_group_label("mentions_in_passing"), OTHER_LINK_GROUP);
        let custom = CataloguedLink::from_first_use("Mentions In Passing");
        assert_eq!(custom.kind, "mentions_in_passing");
        assert!(custom.is_other());
        let known = CataloguedLink::from_first_use("resolves_to");
        assert!(!known.is_other());
        assert_eq!(known.inverse.as_deref(), Some(kinds::REVERSE_DNS));
    }

    #[test]
    fn action_catalog_covers_implemented_and_reserved_kinds() {
        assert!(action_type("ADD_FINDING").expect("finding").implemented);
        assert!(action_type("PROPOSE_RESOLVE").expect("resolve").implemented);
        assert!(action_type("RUN_PLAYBOOK").expect("playbook").implemented);
        assert!(
            action_type("ASSERT_FINDING_TEMPLATE")
                .expect("template")
                .implemented
        );
        assert_eq!(
            action_type_for(&AgentAction::AddNote { body: "x".into() }).kind,
            "ADD_NOTE"
        );
        assert!(
            !AgentAction::ProposeResolve {
                entities: vec!["E-1".into()],
                reason: String::new(),
            }
            .is_reserved()
        );
        assert!(
            !AgentAction::RunPlaybook {
                playbook: "domain-infra".into(),
            }
            .is_reserved()
        );
    }

    #[test]
    fn object_view_shows_identity_group_and_catalogued_links() {
        let investigation_id = Uuid::new_v4();
        let mut domain = sample_entity(EntityKind::Domain, "example.com");
        domain.investigation_id = investigation_id;
        domain.seq = 1;
        domain.fields.push("registrar", "Example Registrar");
        domain.fields.push("odd_key", "custom");
        let mut ip = sample_entity(EntityKind::Ip, "192.0.2.10");
        ip.investigation_id = investigation_id;
        ip.id = Uuid::new_v4();
        ip.seq = 2;
        let relationship = Relationship {
            id: Uuid::new_v4(),
            investigation_id,
            seq: 1,
            from_id: domain.id,
            to_id: ip.id,
            kind: kinds::RESOLVES_TO.to_string(),
            provenance: Provenance::Collected,
            confidence: Confidence::High,
            notes: String::new(),
            source_ids: Vec::new(),
            created_at: now(),
        };
        let custom = Relationship {
            id: Uuid::new_v4(),
            investigation_id,
            seq: 2,
            from_id: domain.id,
            to_id: ip.id,
            kind: "spotted_near".into(),
            provenance: Provenance::Analyst,
            confidence: Confidence::Low,
            notes: String::new(),
            source_ids: Vec::new(),
            created_at: now(),
        };
        let mut data = InvestigationData::empty(Investigation {
            id: investigation_id,
            name: "Case".into(),
            description: String::new(),
            tags: Vec::new(),
            marking: crate::model::Marking::Unmarked,
            created_at: now(),
            updated_at: now(),
        });
        data.entities = vec![domain.clone(), ip];
        data.relationships = vec![relationship, custom];
        let workspace = Workspace::new(data);
        let view = ObjectView::for_entity(&workspace, workspace.entity(domain.id).expect("domain"));
        assert!(
            view.groups
                .iter()
                .any(|group| group.id == "identity" && !group.properties.is_empty())
        );
        assert!(view.groups.iter().any(|group| group.id == "registration"
            && group.properties.iter().any(|item| item.key == "registrar")));
        assert!(view.groups.iter().any(|group| group.id == "other"
            && group.properties.iter().any(|item| item.key == "odd_key")));
        assert!(
            view.links
                .iter()
                .any(|group| group.label == "Resolves to" && group.catalogued)
        );
        assert!(
            view.links
                .iter()
                .any(|group| group.label == OTHER_LINK_GROUP && !group.catalogued)
        );
    }

    #[test]
    fn object_view_dedups_collapsed_parallel_links() {
        let investigation_id = Uuid::new_v4();
        let mut domain = sample_entity(EntityKind::Domain, "example.com");
        domain.investigation_id = investigation_id;
        domain.seq = 1;
        let mut host = sample_entity(EntityKind::Hostname, "example.com");
        host.investigation_id = investigation_id;
        host.seq = 2;
        let mut ip = sample_entity(EntityKind::Ip, "192.0.2.10");
        ip.investigation_id = investigation_id;
        ip.seq = 3;
        let from_domain = Relationship {
            id: Uuid::new_v4(),
            investigation_id,
            seq: 1,
            from_id: domain.id,
            to_id: ip.id,
            kind: kinds::RESOLVES_TO.to_string(),
            provenance: Provenance::Collected,
            confidence: Confidence::High,
            notes: String::new(),
            source_ids: Vec::new(),
            created_at: now(),
        };
        let from_host = Relationship {
            id: Uuid::new_v4(),
            investigation_id,
            seq: 2,
            from_id: host.id,
            to_id: ip.id,
            kind: kinds::RESOLVES_TO.to_string(),
            provenance: Provenance::Collected,
            confidence: Confidence::High,
            notes: String::new(),
            source_ids: Vec::new(),
            created_at: now(),
        };
        let mut data = InvestigationData::empty(Investigation {
            id: investigation_id,
            name: "Case".into(),
            description: String::new(),
            tags: Vec::new(),
            marking: crate::model::Marking::Unmarked,
            created_at: now(),
            updated_at: now(),
        });
        data.entities = vec![domain.clone(), host.clone(), ip.clone()];
        data.relationships = vec![from_domain, from_host];
        data.resolutions.push(Resolution {
            id: Uuid::new_v4(),
            investigation_id,
            seq: 1,
            canonical_id: domain.id,
            constituent_ids: vec![domain.id, host.id],
            status: crate::model::ResolutionStatus::Active,
            reason: "same host".into(),
            created_by: "local".into(),
            created_at: now(),
            unresolved_by: None,
            unresolved_at: None,
            property_writes: Vec::new(),
            source_writes: Vec::new(),
        });
        let workspace = Workspace::new(data);
        let view = ObjectView::for_entity(&workspace, workspace.entity(domain.id).expect("domain"));
        let resolves: Vec<&LinkedObject> = view
            .links
            .iter()
            .filter(|group| group.label == "Resolves to")
            .flat_map(|group| group.links.iter())
            .filter(|link| {
                link.direction == LinkDirection::Outgoing
                    && link.relationship_kind == kinds::RESOLVES_TO
                    && link.entity_kind == "ip"
            })
            .collect();
        assert_eq!(resolves.len(), 1, "{:?}", view.links);
    }
}
