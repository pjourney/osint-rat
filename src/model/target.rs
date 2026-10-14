//! Collection targets and identifier normalization.
//!
//! Every identifier that enters os-int (from the analyst, a collector, an agent proposal, or an
//! import) is normalized here so that `Example.COM.` and `example.com` are the same entity.

use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use url::Url;

use super::entity::EntityKind;

const MAX_HOSTNAME_LENGTH: usize = 253;
const MAX_LABEL_LENGTH: usize = 63;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Domain,
    Ip,
    Url,
    Email,
    Repository,
    File,
    Username,
    Asn,
}

impl TargetKind {
    pub const ALL: [TargetKind; 8] = [
        TargetKind::Domain,
        TargetKind::Ip,
        TargetKind::Url,
        TargetKind::Email,
        TargetKind::Repository,
        TargetKind::File,
        TargetKind::Username,
        TargetKind::Asn,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TargetKind::Domain => "domain",
            TargetKind::Ip => "ip",
            TargetKind::Url => "url",
            TargetKind::Email => "email",
            TargetKind::Repository => "repository",
            TargetKind::File => "file",
            TargetKind::Username => "username",
            TargetKind::Asn => "asn",
        }
    }

    pub fn parse(value: &str) -> Option<TargetKind> {
        match value.trim().to_ascii_lowercase().as_str() {
            "domain" | "hostname" | "host" => Some(TargetKind::Domain),
            "ip" | "ip_address" | "ipv4" | "ipv6" | "address" => Some(TargetKind::Ip),
            "url" | "uri" | "link" => Some(TargetKind::Url),
            "email" | "mail" => Some(TargetKind::Email),
            "repository" | "repo" | "git" => Some(TargetKind::Repository),
            "file" | "path" => Some(TargetKind::File),
            "username" | "user" | "handle" => Some(TargetKind::Username),
            "asn" | "as" => Some(TargetKind::Asn),
            _ => None,
        }
    }
}

impl fmt::Display for TargetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A validated, normalized subject for collectors and tools.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Domain(String),
    Ip(IpAddr),
    Url(Url),
    Email(String),
    Repository(String),
    File(PathBuf),
    Username(String),
    Asn(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetError(pub String);

impl fmt::Display for TargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TargetError {}

impl Target {
    pub fn kind(&self) -> TargetKind {
        match self {
            Target::Domain(_) => TargetKind::Domain,
            Target::Ip(_) => TargetKind::Ip,
            Target::Url(_) => TargetKind::Url,
            Target::Email(_) => TargetKind::Email,
            Target::Repository(_) => TargetKind::Repository,
            Target::File(_) => TargetKind::File,
            Target::Username(_) => TargetKind::Username,
            Target::Asn(_) => TargetKind::Asn,
        }
    }

    /// Parses `raw` as a target of the given kind.
    pub fn parse(kind: TargetKind, raw: &str) -> Result<Target, TargetError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(TargetError("the target is empty".into()));
        }
        match kind {
            TargetKind::Domain => normalize_hostname(trimmed).map(Target::Domain),
            TargetKind::Ip => trimmed
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<IpAddr>()
                .map(Target::Ip)
                .map_err(|_| TargetError(format!("{trimmed:?} is not an IPv4 or IPv6 address"))),
            TargetKind::Url => normalize_url(trimmed).map(Target::Url),
            TargetKind::Email => normalize_email(trimmed).map(Target::Email),
            TargetKind::Repository => normalize_repository(trimmed).map(Target::Repository),
            TargetKind::File => Ok(Target::File(PathBuf::from(trimmed))),
            TargetKind::Username => normalize_username(trimmed).map(Target::Username),
            TargetKind::Asn => parse_asn(trimmed).map(Target::Asn),
        }
    }

    /// Guesses the most likely kind for free-form input.
    pub fn guess(raw: &str) -> Result<Target, TargetError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(TargetError("the target is empty".into()));
        }
        if let Ok(ip) = trimmed
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
        {
            return Ok(Target::Ip(ip));
        }
        if looks_like_repository(trimmed) {
            return Target::parse(TargetKind::Repository, trimmed);
        }
        if trimmed.contains("://") {
            return Target::parse(TargetKind::Url, trimmed);
        }
        if trimmed.contains('@') && !trimmed.starts_with('@') {
            return Target::parse(TargetKind::Email, trimmed);
        }
        if trimmed.starts_with('@') {
            return Target::parse(TargetKind::Username, trimmed);
        }
        if trimmed.to_ascii_uppercase().starts_with("AS") && parse_asn(trimmed).is_ok() {
            return Target::parse(TargetKind::Asn, trimmed);
        }
        if trimmed.contains('/') {
            let path = PathBuf::from(trimmed);
            if path.exists() {
                return Ok(Target::File(path));
            }
            return Target::parse(TargetKind::Url, trimmed);
        }
        Target::parse(TargetKind::Domain, trimmed)
    }

    /// The canonical string form, also used as the entity value.
    pub fn value(&self) -> String {
        match self {
            Target::Domain(domain) => domain.clone(),
            Target::Ip(ip) => ip.to_string(),
            Target::Url(url) => url.to_string(),
            Target::Email(email) => email.clone(),
            Target::Repository(repo) => repo.clone(),
            Target::File(path) => path.display().to_string(),
            Target::Username(name) => name.clone(),
            Target::Asn(number) => format!("AS{number}"),
        }
    }

    pub fn entity_kind(&self) -> EntityKind {
        match self {
            Target::Domain(_) => EntityKind::Domain,
            Target::Ip(_) => EntityKind::Ip,
            Target::Url(_) => EntityKind::Url,
            Target::Email(_) => EntityKind::Email,
            Target::Repository(_) => EntityKind::Repository,
            Target::File(_) => EntityKind::File,
            Target::Username(_) => EntityKind::Username,
            Target::Asn(_) => EntityKind::Asn,
        }
    }

    /// The hostname a network collector should contact, if any.
    pub fn hostname(&self) -> Option<String> {
        match self {
            Target::Domain(domain) => Some(domain.clone()),
            Target::Url(url) => url
                .host_str()
                .map(|host| host.trim_matches(['[', ']']).to_string()),
            Target::Email(email) => email.rsplit_once('@').map(|(_, domain)| domain.to_string()),
            _ => None,
        }
    }

    /// Builds a target from an entity, when that entity kind can be collected on.
    pub fn from_entity(kind: &EntityKind, value: &str) -> Option<Target> {
        let target_kind = match kind {
            EntityKind::Domain | EntityKind::Hostname => TargetKind::Domain,
            EntityKind::Ip => TargetKind::Ip,
            EntityKind::Url => TargetKind::Url,
            EntityKind::Email => TargetKind::Email,
            EntityKind::Repository => TargetKind::Repository,
            EntityKind::File => TargetKind::File,
            EntityKind::Username => TargetKind::Username,
            EntityKind::Asn => TargetKind::Asn,
            _ => return None,
        };
        Target::parse(target_kind, value).ok()
    }
}

impl fmt::Display for Target {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.kind(), self.value())
    }
}

/// Lowercases, strips a trailing dot, converts IDNs to punycode, and validates DNS syntax.
pub fn normalize_hostname(raw: &str) -> Result<String, TargetError> {
    let trimmed = raw.trim().trim_end_matches('.');
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let host_part = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('.');
    if host_part.is_empty() {
        return Err(TargetError("the hostname is empty".into()));
    }
    if host_part.starts_with('-') {
        return Err(TargetError(format!(
            "{host_part:?} is not a valid hostname"
        )));
    }
    if host_part.parse::<IpAddr>().is_ok() {
        return Err(TargetError(format!(
            "{host_part:?} is an IP address, not a hostname"
        )));
    }
    let ascii = match url::Host::parse(host_part) {
        Ok(url::Host::Domain(domain)) => domain,
        Ok(_) => {
            return Err(TargetError(format!(
                "{host_part:?} is an IP address, not a hostname"
            )));
        }
        Err(_) => {
            return Err(TargetError(format!(
                "{host_part:?} is not a valid hostname"
            )));
        }
    };
    let ascii = ascii.to_ascii_lowercase();
    validate_hostname_labels(&ascii)?;
    Ok(ascii)
}

fn validate_hostname_labels(hostname: &str) -> Result<(), TargetError> {
    if hostname.len() > MAX_HOSTNAME_LENGTH {
        return Err(TargetError(
            "the hostname is longer than 253 characters".into(),
        ));
    }
    if !hostname.contains('.') {
        return Err(TargetError(format!(
            "{hostname:?} has no dot; expected a domain such as example.com"
        )));
    }
    for label in hostname.split('.') {
        let is_valid_length = !label.is_empty() && label.len() <= MAX_LABEL_LENGTH;
        let has_valid_chars = label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        let has_valid_edges = !label.starts_with('-') && !label.ends_with('-');
        if !(is_valid_length && has_valid_chars && has_valid_edges) {
            return Err(TargetError(format!(
                "{hostname:?} is not a valid hostname (bad label {label:?})"
            )));
        }
    }
    Ok(())
}

/// Normalizes a URL for fetching: adds `https://` when no scheme is present, accepts only HTTP(S),
/// strips credentials and fragments, and lowercases/punycodes the host.
pub fn normalize_url(raw: &str) -> Result<Url, TargetError> {
    let trimmed = raw.trim();
    let candidate = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let mut url = Url::parse(&candidate)
        .map_err(|error| TargetError(format!("{trimmed:?} is not a valid URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(TargetError(format!(
            "only http and https URLs can be fetched (got {}://)",
            url.scheme()
        )));
    }
    let host = url
        .host_str()
        .ok_or_else(|| TargetError(format!("{trimmed:?} has no host")))?
        .to_string();
    if host.ends_with('.') {
        let stripped = host.trim_end_matches('.').to_string();
        url.set_host(Some(&stripped))
            .map_err(|error| TargetError(format!("invalid host: {error}")))?;
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    Ok(url)
}

pub fn normalize_email(raw: &str) -> Result<String, TargetError> {
    let trimmed = raw.trim().trim_start_matches("mailto:");
    let (local, domain) = trimmed
        .rsplit_once('@')
        .ok_or_else(|| TargetError(format!("{trimmed:?} is not an email address")))?;
    let is_valid_local = !local.is_empty()
        && local.len() <= 64
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._%+-'".contains(c));
    if !is_valid_local {
        return Err(TargetError(format!("{trimmed:?} is not an email address")));
    }
    let domain = normalize_hostname(domain)?;
    Ok(format!("{}@{domain}", local.to_ascii_lowercase()))
}

pub fn normalize_username(raw: &str) -> Result<String, TargetError> {
    let name = raw.trim().trim_start_matches('@');
    let is_valid = !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || "._-".contains(c));
    if !is_valid {
        return Err(TargetError(format!("{raw:?} is not a plausible username")));
    }
    Ok(name.to_string())
}

pub fn parse_asn(raw: &str) -> Result<u32, TargetError> {
    let trimmed = raw.trim();
    let digits = trimmed
        .strip_prefix("AS")
        .or_else(|| trimmed.strip_prefix("as"))
        .or_else(|| trimmed.strip_prefix("As"))
        .unwrap_or(trimmed);
    digits
        .parse::<u32>()
        .map_err(|_| TargetError(format!("{raw:?} is not an AS number")))
}

/// Accepts `https://github.com/owner/repo(.git)`, `github.com/owner/repo`, `git@host:owner/repo`,
/// and local directory paths. Remote repositories are normalized to an `https://` URL without
/// a `.git` suffix.
pub fn normalize_repository(raw: &str) -> Result<String, TargetError> {
    let trimmed = raw.trim();
    if trimmed.starts_with('-') {
        return Err(TargetError(format!("{trimmed:?} is not a repository")));
    }
    let path = PathBuf::from(trimmed);
    if path.is_dir() {
        return Ok(path.display().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("git@") {
        let (host, repo_path) = rest
            .split_once(':')
            .ok_or_else(|| TargetError(format!("{trimmed:?} is not a repository URL")))?;
        let host = normalize_hostname(host)?;
        return Ok(format!("https://{host}/{}", strip_git_suffix(repo_path)));
    }
    let url = normalize_url(trimmed)?;
    let host = url.host_str().unwrap_or_default();
    let segments: Vec<&str> = url
        .path_segments()
        .map(|segments| segments.filter(|segment| !segment.is_empty()).collect())
        .unwrap_or_default();
    if segments.len() < 2 {
        return Err(TargetError(format!(
            "{trimmed:?} does not look like a repository (expected host/owner/name)"
        )));
    }
    let path = segments.join("/");
    Ok(format!("https://{host}/{}", strip_git_suffix(&path)))
}

fn strip_git_suffix(path: &str) -> String {
    path.trim_end_matches('/')
        .trim_end_matches(".git")
        .to_string()
}

fn looks_like_repository(raw: &str) -> bool {
    const FORGES: [&str; 4] = [
        "github.com/",
        "gitlab.com/",
        "codeberg.org/",
        "bitbucket.org/",
    ];
    let lower = raw.to_ascii_lowercase();
    let without_scheme = lower
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(&lower);
    let without_www = without_scheme.trim_start_matches("www.");
    if lower.starts_with("git@") || lower.ends_with(".git") {
        return true;
    }
    FORGES.iter().any(|forge| {
        without_www
            .strip_prefix(forge)
            .map(|rest| rest.trim_end_matches('/').split('/').count() == 2)
            .unwrap_or(false)
    })
}

/// Returns true when `hostname` is a strict subdomain of `apex` (not the apex itself).
pub fn is_subdomain_of(hostname: &str, apex: &str) -> bool {
    let host = hostname.trim().trim_end_matches('.').to_ascii_lowercase();
    let apex = apex.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || apex.is_empty() || host == apex {
        return false;
    }
    host.ends_with(&format!(".{apex}"))
}

/// Returns `true` for loopback, private, link-local, and other non-public addresses.
pub fn is_non_public_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            let is_shared_cgnat = octets[0] == 100 && (64..=127).contains(&octets[1]);
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || is_shared_cgnat
        }
        IpAddr::V6(v6) => {
            let first_segment = v6.segments()[0];
            let is_unique_local = (first_segment & 0xfe00) == 0xfc00;
            let is_link_local = (first_segment & 0xffc0) == 0xfe80;
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || is_unique_local
                || is_link_local
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostnames_are_lowercased_and_trailing_dot_removed() {
        assert_eq!(
            normalize_hostname("Example.COM."),
            Ok("example.com".to_string())
        );
        assert_eq!(
            normalize_hostname(" www.Example.com/path "),
            Ok("www.example.com".to_string())
        );
    }

    #[test]
    fn hostnames_are_punycoded() {
        assert_eq!(
            normalize_hostname("bücher.example"),
            Ok("xn--bcher-kva.example".to_string())
        );
    }

    #[test]
    fn invalid_hostnames_are_rejected() {
        for input in [
            "",
            "localhost",
            "-oProxyCommand=x",
            "exa mple.com",
            "192.0.2.1",
            "a..b",
        ] {
            assert!(normalize_hostname(input).is_err(), "input {input:?}");
        }
    }

    #[test]
    fn urls_get_scheme_and_lose_credentials_and_fragment() {
        let actual = normalize_url("User:Secret@Example.com:443/a?b=1#frag").expect("valid url");
        assert_eq!(actual.as_str(), "https://example.com/a?b=1");
    }

    #[test]
    fn non_http_urls_are_rejected() {
        assert!(normalize_url("file:///etc/passwd").is_err());
        assert!(normalize_url("ftp://example.com/").is_err());
    }

    #[test]
    fn emails_are_normalized() {
        assert_eq!(
            normalize_email("Admin@Example.COM"),
            Ok("admin@example.com".into())
        );
        assert!(normalize_email("not-an-email").is_err());
    }

    #[test]
    fn repositories_are_normalized() {
        let expected = "https://github.com/rust-lang/rust".to_string();
        assert_eq!(
            normalize_repository("github.com/rust-lang/rust.git"),
            Ok(expected.clone())
        );
        assert_eq!(
            normalize_repository("git@github.com:rust-lang/rust.git"),
            Ok(expected)
        );
        assert!(normalize_repository("https://github.com/").is_err());
    }

    #[test]
    fn guess_detects_kinds() {
        assert_eq!(
            Target::guess("192.0.2.5").map(|t| t.kind()),
            Ok(TargetKind::Ip)
        );
        assert_eq!(
            Target::guess("2001:db8::1").map(|t| t.kind()),
            Ok(TargetKind::Ip)
        );
        assert_eq!(
            Target::guess("https://example.com/x").map(|t| t.kind()),
            Ok(TargetKind::Url)
        );
        assert_eq!(
            Target::guess("alice@example.com").map(|t| t.kind()),
            Ok(TargetKind::Email)
        );
        assert_eq!(
            Target::guess("github.com/a/b").map(|t| t.kind()),
            Ok(TargetKind::Repository)
        );
        assert_eq!(
            Target::guess("AS13335").map(|t| t.kind()),
            Ok(TargetKind::Asn)
        );
        assert_eq!(
            Target::guess("example.com").map(|t| t.kind()),
            Ok(TargetKind::Domain)
        );
    }

    #[test]
    fn subdomain_of_requires_a_strict_suffix() {
        assert!(is_subdomain_of("www.example.com", "example.com"));
        assert!(is_subdomain_of("a.b.example.com.", "example.com"));
        assert!(!is_subdomain_of("example.com", "example.com"));
        assert!(!is_subdomain_of("example.com.evil.net", "example.com"));
        assert!(!is_subdomain_of("notexample.com", "example.com"));
    }

    #[test]
    fn non_public_addresses_are_detected() {
        for input in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
        ] {
            let ip: IpAddr = input.parse().expect("valid ip");
            assert!(is_non_public_ip(&ip), "input {input}");
        }
        let public: IpAddr = "1.1.1.1".parse().expect("valid ip");
        assert!(!is_non_public_ip(&public));
    }
}
