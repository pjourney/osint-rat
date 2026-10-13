//! Diagnostic readings of public records.
//!
//! These checks describe what a name publishes (mail authentication, HTTP response headers,
//! certificate validity). They do not probe for vulnerabilities or suggest how to attack a host.

use chrono::{DateTime, TimeDelta, Utc};

use crate::model::{Confidence, EvidenceBatch, SourceKey};

/// How a single diagnostic reading should be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticState {
    /// The published control is present and restrictive.
    Present,
    /// An expected public record is absent.
    Missing,
    /// The record is present but weak, inconsistent, or near a limit.
    Review,
}

impl DiagnosticState {
    pub fn marker(self) -> &'static str {
        match self {
            DiagnosticState::Present => "present",
            DiagnosticState::Missing => "missing",
            DiagnosticState::Review => "review",
        }
    }
}

/// One observational check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub check: &'static str,
    pub state: DiagnosticState,
    pub summary: String,
}

impl Diagnostic {
    pub fn new(check: &'static str, state: DiagnosticState, summary: impl Into<String>) -> Self {
        Self {
            check,
            state,
            summary: summary.into(),
        }
    }
}

/// A DNS record reduced to the fields diagnostics need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DnsFact {
    pub name: String,
    pub record_type: String,
    pub value: String,
    pub points_to: Option<String>,
}

/// Mail-authentication and related DNS diagnostics for one domain.
pub fn assess_dns(domain: &str, records: &[DnsFact]) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    diagnostics.push(assess_mx(records));
    diagnostics.push(assess_spf(domain, records));
    diagnostics.push(assess_dmarc(domain, records));
    if let Some(diagnostic) = assess_tagged_txt(
        records,
        &format!("_mta-sts.{domain}"),
        "v=stsv1",
        "mta-sts",
        "MTA-STS policy is published",
    ) {
        diagnostics.push(diagnostic);
    }
    if let Some(diagnostic) = assess_tagged_txt(
        records,
        &format!("_smtp._tls.{domain}"),
        "v=tlsrptv1",
        "tls-rpt",
        "TLS-RPT reporting is published",
    ) {
        diagnostics.push(diagnostic);
    }
    if let Some(diagnostic) = assess_tagged_txt(
        records,
        &format!("default._bimi.{domain}"),
        "v=bimi1",
        "bimi",
        "BIMI record is published",
    ) {
        diagnostics.push(diagnostic);
    }
    if let Some(diagnostic) = assess_caa(records) {
        diagnostics.push(diagnostic);
    }
    diagnostics.push(assess_dnssec(domain, records));
    diagnostics
}

/// Stores gap diagnostics as collected findings and clears gaps that are now in a good state.
pub fn record_findings(
    batch: &mut EvidenceBatch,
    subject: &str,
    diagnostics: &[Diagnostic],
    source: Option<SourceKey>,
) {
    for diagnostic in diagnostics {
        let title = format!("{subject} {}", diagnostic.check);
        if diagnostic.state == DiagnosticState::Present {
            batch.resolve_finding(title);
            continue;
        }
        let confidence = if diagnostic.state == DiagnosticState::Missing {
            Confidence::High
        } else {
            Confidence::Medium
        };
        batch.add_finding(title, diagnostic.summary.clone(), confidence, source);
    }
}

/// Header diagnostics for one fetched response.
pub fn assess_http(is_https: bool, headers: &[(String, String)]) -> Vec<Diagnostic> {
    let has = |name: &str| headers.iter().any(|(candidate, _)| candidate == name);
    let header_value = |name: &str| {
        headers
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.to_ascii_lowercase())
    };
    let mut diagnostics = Vec::new();
    if is_https {
        diagnostics.push(Diagnostic::new(
            "https",
            DiagnosticState::Present,
            "Final response was served over HTTPS",
        ));
        diagnostics.push(if has("strict-transport-security") {
            Diagnostic::new(
                "hsts",
                DiagnosticState::Present,
                "Strict-Transport-Security is set",
            )
        } else {
            Diagnostic::new(
                "hsts",
                DiagnosticState::Review,
                "No Strict-Transport-Security header was returned",
            )
        });
    } else {
        diagnostics.push(Diagnostic::new(
            "https",
            DiagnosticState::Review,
            "Final response was not served over HTTPS",
        ));
    }
    diagnostics.push(if has("content-security-policy") {
        Diagnostic::new(
            "csp",
            DiagnosticState::Present,
            "Content-Security-Policy is set",
        )
    } else {
        Diagnostic::new(
            "csp",
            DiagnosticState::Review,
            "No Content-Security-Policy header was returned",
        )
    });
    diagnostics.push(if has("x-content-type-options") {
        Diagnostic::new(
            "content-type-options",
            DiagnosticState::Present,
            "X-Content-Type-Options is set",
        )
    } else {
        Diagnostic::new(
            "content-type-options",
            DiagnosticState::Review,
            "No X-Content-Type-Options header was returned",
        )
    });
    let framing = has("x-frame-options")
        || header_value("content-security-policy")
            .is_some_and(|value| value.contains("frame-ancestors"));
    diagnostics.push(if framing {
        Diagnostic::new(
            "framing",
            DiagnosticState::Present,
            "A framing control is set (X-Frame-Options or CSP frame-ancestors)",
        )
    } else {
        Diagnostic::new(
            "framing",
            DiagnosticState::Review,
            "No framing control was returned (X-Frame-Options or CSP frame-ancestors)",
        )
    });
    diagnostics
}

/// Certificate validity diagnostics. `now` is injected so tests stay stable.
pub fn assess_tls(
    trusted: bool,
    validation_error: Option<&str>,
    not_after: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if trusted {
        diagnostics.push(Diagnostic::new(
            "certificate",
            DiagnosticState::Present,
            "Certificate validates for this hostname against the Mozilla root store",
        ));
    } else {
        let detail = validation_error.unwrap_or("validation failed");
        diagnostics.push(Diagnostic::new(
            "certificate",
            DiagnosticState::Review,
            format!("Certificate is not trusted: {detail}"),
        ));
    }
    if let Some(not_after) = not_after {
        if not_after <= now {
            diagnostics.push(Diagnostic::new(
                "expiry",
                DiagnosticState::Review,
                "Certificate is past its not-after date",
            ));
        } else if not_after <= now + TimeDelta::days(14) {
            diagnostics.push(Diagnostic::new(
                "expiry",
                DiagnosticState::Review,
                "Certificate expires within 14 days",
            ));
        } else {
            diagnostics.push(Diagnostic::new(
                "expiry",
                DiagnosticState::Present,
                "Certificate is not within 14 days of expiry",
            ));
        }
    }
    diagnostics
}

/// Compact `check state` list for entity fields and job summaries.
pub fn posture(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| format!("{} {}", diagnostic.check, diagnostic.state.marker()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders diagnostics into the text body of a source.
pub fn append_section(lines: &mut Vec<String>, diagnostics: &[Diagnostic]) {
    if diagnostics.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push("Diagnostics:".into());
    for diagnostic in diagnostics {
        lines.push(format!(
            "  [{}] {}",
            diagnostic.state.marker(),
            diagnostic.summary
        ));
    }
}

fn assess_mx(records: &[DnsFact]) -> Diagnostic {
    let exchanges: Vec<&DnsFact> = records
        .iter()
        .filter(|record| record.record_type == "MX")
        .collect();
    if exchanges.is_empty() {
        return Diagnostic::new("mx", DiagnosticState::Missing, "No MX records published");
    }
    let null_mx = exchanges.len() == 1 && is_null_mx(exchanges[0]);
    if null_mx {
        return Diagnostic::new(
            "mx",
            DiagnosticState::Present,
            "Null MX is published (the domain does not accept mail)",
        );
    }
    Diagnostic::new(
        "mx",
        DiagnosticState::Present,
        format!("{} mail exchanger(s) published", exchanges.len()),
    )
}

fn is_null_mx(record: &DnsFact) -> bool {
    let exchange = record.points_to.as_deref().unwrap_or("").trim_matches('.');
    let preference_zero = record.value.trim().starts_with('0');
    preference_zero && exchange.is_empty()
}

fn assess_spf(domain: &str, records: &[DnsFact]) -> Diagnostic {
    let policies: Vec<String> = txt_on(records, domain).filter_map(spf_body).collect();
    match policies.len() {
        0 => Diagnostic::new("spf", DiagnosticState::Missing, "No SPF record published"),
        1 => spf_qualifier(&policies[0]),
        _ => Diagnostic::new(
            "spf",
            DiagnosticState::Review,
            "Multiple SPF records are published",
        ),
    }
}

fn spf_body(value: &str) -> Option<String> {
    let text = unquote(value);
    let mut terms = text.split_whitespace();
    if !terms.next()?.eq_ignore_ascii_case("v=spf1") {
        return None;
    }
    Some(text)
}

fn spf_qualifier(policy: &str) -> Diagnostic {
    let all = policy.split_whitespace().find_map(|term| {
        let lower = term.to_ascii_lowercase();
        let bare = lower.trim_start_matches(['+', '-', '~', '?']);
        if bare == "all" {
            Some(term.to_string())
        } else {
            None
        }
    });
    match all.as_deref() {
        Some("-all") => Diagnostic::new(
            "spf",
            DiagnosticState::Present,
            "SPF hard-fails unauthorized senders (-all)",
        ),
        Some("~all") => Diagnostic::new(
            "spf",
            DiagnosticState::Present,
            "SPF soft-fails unauthorized senders (~all)",
        ),
        Some("+all") => Diagnostic::new(
            "spf",
            DiagnosticState::Review,
            "SPF +all authorizes any sender",
        ),
        Some("?all") => Diagnostic::new(
            "spf",
            DiagnosticState::Review,
            "SPF ?all does not authorize or reject senders",
        ),
        Some(other) => Diagnostic::new(
            "spf",
            DiagnosticState::Review,
            format!("SPF all mechanism is {other}"),
        ),
        None => Diagnostic::new(
            "spf",
            DiagnosticState::Review,
            "SPF record has no all mechanism",
        ),
    }
}

fn assess_dmarc(domain: &str, records: &[DnsFact]) -> Diagnostic {
    let name = format!("_dmarc.{domain}");
    let policies: Vec<String> = txt_on(records, &name).filter_map(dmarc_policy).collect();
    match policies.as_slice() {
        [] => Diagnostic::new(
            "dmarc",
            DiagnosticState::Missing,
            format!("No DMARC record at {name}"),
        ),
        [policy]
            if policy.eq_ignore_ascii_case("reject")
                || policy.eq_ignore_ascii_case("quarantine") =>
        {
            Diagnostic::new(
                "dmarc",
                DiagnosticState::Present,
                format!("DMARC policy is p={policy}"),
            )
        }
        [policy] if policy.eq_ignore_ascii_case("none") => Diagnostic::new(
            "dmarc",
            DiagnosticState::Review,
            "DMARC policy is p=none (monitoring only)",
        ),
        [policy] => Diagnostic::new(
            "dmarc",
            DiagnosticState::Review,
            format!("DMARC policy is p={policy}"),
        ),
        _ => Diagnostic::new(
            "dmarc",
            DiagnosticState::Review,
            format!("Multiple DMARC records at {name}"),
        ),
    }
}

fn dmarc_policy(value: &str) -> Option<String> {
    let text = unquote(value);
    if !text.to_ascii_lowercase().starts_with("v=dmarc1") {
        return None;
    }
    text.split(';').map(str::trim).find_map(|tag| {
        tag.split_once('=')
            .filter(|(name, _)| name.eq_ignore_ascii_case("p"))
            .map(|(_, policy)| policy.trim().to_string())
    })
}

fn assess_tagged_txt(
    records: &[DnsFact],
    name: &str,
    prefix: &str,
    check: &'static str,
    summary: &str,
) -> Option<Diagnostic> {
    let found =
        txt_on(records, name).any(|value| unquote(value).to_ascii_lowercase().starts_with(prefix));
    if found {
        Some(Diagnostic::new(check, DiagnosticState::Present, summary))
    } else {
        None
    }
}

fn assess_dnssec(domain: &str, records: &[DnsFact]) -> Diagnostic {
    let count = records
        .iter()
        .filter(|record| record.record_type == "DS" && record.name.eq_ignore_ascii_case(domain))
        .count();
    if count == 0 {
        return Diagnostic::new(
            "dnssec",
            DiagnosticState::Review,
            "No DS record; DNSSEC is not signed at the delegation",
        );
    }
    Diagnostic::new(
        "dnssec",
        DiagnosticState::Present,
        format!("{count} DS record(s) published"),
    )
}

fn assess_caa(records: &[DnsFact]) -> Option<Diagnostic> {
    let values: Vec<&str> = records
        .iter()
        .filter(|record| record.record_type == "CAA")
        .map(|record| record.value.as_str())
        .collect();
    if values.is_empty() {
        return None;
    }
    let shown = values.join("; ");
    let shown = if shown.chars().count() > 180 {
        format!("{}…", shown.chars().take(180).collect::<String>())
    } else {
        shown
    };
    Some(Diagnostic::new(
        "caa",
        DiagnosticState::Present,
        format!("CAA is published: {shown}"),
    ))
}

fn txt_on<'a>(records: &'a [DnsFact], name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    records.iter().filter_map(move |record| {
        if record.record_type == "TXT" && record.name.eq_ignore_ascii_case(name) {
            Some(record.value.as_str())
        } else {
            None
        }
    })
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches('"').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txt(name: &str, value: &str) -> DnsFact {
        DnsFact {
            name: name.into(),
            record_type: "TXT".into(),
            value: value.into(),
            points_to: None,
        }
    }

    fn mx(value: &str, points_to: &str) -> DnsFact {
        DnsFact {
            name: "example.com".into(),
            record_type: "MX".into(),
            value: value.into(),
            points_to: Some(points_to.into()),
        }
    }

    fn state<'a>(diagnostics: &'a [Diagnostic], check: &str) -> &'a Diagnostic {
        diagnostics
            .iter()
            .find(|diagnostic| diagnostic.check == check)
            .unwrap_or_else(|| panic!("missing {check}"))
    }

    #[test]
    fn restrictive_mail_policy_is_present() {
        let input = [
            mx("10 mx.example.net.", "mx.example.net"),
            txt("example.com", "\"v=spf1 include:_spf.example.net -all\""),
            txt("_dmarc.example.com", "\"v=DMARC1; p=reject\""),
            txt("_mta-sts.example.com", "\"v=STSv1; id=20240101\""),
        ];
        let actual = assess_dns("example.com", &input);
        assert_eq!(state(&actual, "spf").state, DiagnosticState::Present);
        assert_eq!(state(&actual, "dmarc").state, DiagnosticState::Present);
        assert_eq!(state(&actual, "mx").state, DiagnosticState::Present);
        assert_eq!(state(&actual, "mta-sts").state, DiagnosticState::Present);
        assert!(posture(&actual).contains("spf present"));
        assert_eq!(state(&actual, "dnssec").state, DiagnosticState::Review);
    }

    #[test]
    fn ds_records_mark_dnssec_present() {
        let input = [DnsFact {
            name: "example.com".into(),
            record_type: "DS".into(),
            value: "12345 13 2 abcd".into(),
            points_to: None,
        }];
        let actual = assess_dns("example.com", &input);
        assert_eq!(state(&actual, "dnssec").state, DiagnosticState::Present);
    }

    #[test]
    fn gaps_become_findings_and_good_states_resolve() {
        let diagnostics = assess_dns("example.com", &[txt("example.com", "\"v=spf1 -all\"")]);
        let mut batch = EvidenceBatch::new();
        record_findings(&mut batch, "example.com", &diagnostics, Some(0));
        assert!(
            batch
                .findings
                .iter()
                .any(|finding| finding.title == "example.com dmarc")
        );
        assert!(
            batch
                .resolved_findings
                .iter()
                .any(|title| title == "example.com spf")
        );
        assert!(
            !batch
                .findings
                .iter()
                .any(|finding| finding.title == "example.com spf")
        );
    }

    #[test]
    fn missing_and_permissive_mail_policy_is_flagged() {
        let input = [
            txt("example.com", "\"v=spf1 +all\""),
            txt("example.com", "\"v=spf1 -all\""),
            txt("_dmarc.example.com", "\"v=DMARC1; p=none\""),
        ];
        let actual = assess_dns("example.com", &input);
        assert_eq!(state(&actual, "mx").state, DiagnosticState::Missing);
        assert_eq!(state(&actual, "spf").state, DiagnosticState::Review);
        assert_eq!(state(&actual, "dmarc").state, DiagnosticState::Review);
        assert!(state(&actual, "spf").summary.contains("Multiple SPF"));
    }

    #[test]
    fn null_mx_is_an_explicit_policy() {
        let input = [mx("0 .", "")];
        let actual = assess_dns("example.com", &input);
        assert_eq!(state(&actual, "mx").state, DiagnosticState::Present);
        assert!(state(&actual, "mx").summary.contains("Null MX"));
    }

    #[test]
    fn http_headers_are_read_as_observations() {
        let headers = vec![
            ("strict-transport-security".into(), "max-age=1".into()),
            (
                "content-security-policy".into(),
                "frame-ancestors 'none'".into(),
            ),
        ];
        let actual = assess_http(true, &headers);
        assert_eq!(state(&actual, "https").state, DiagnosticState::Present);
        assert_eq!(state(&actual, "hsts").state, DiagnosticState::Present);
        assert_eq!(state(&actual, "csp").state, DiagnosticState::Present);
        assert_eq!(state(&actual, "framing").state, DiagnosticState::Present);
        assert_eq!(
            state(&actual, "content-type-options").state,
            DiagnosticState::Review
        );
    }

    #[test]
    fn expired_certificate_is_review() {
        let now = DateTime::from_timestamp(1_800_000_000, 0).expect("timestamp");
        let expired = DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp");
        let actual = assess_tls(false, Some("expired"), Some(expired), now);
        assert_eq!(state(&actual, "certificate").state, DiagnosticState::Review);
        assert_eq!(state(&actual, "expiry").state, DiagnosticState::Review);
    }
}
