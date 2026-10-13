//! HTTP metadata for a domain or URL: status, redirect chain, headers, HTML metadata, links,
//! hostnames, visible emails, robots.txt, and sitemap discovery.

use std::net::IpAddr;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use url::Url;

use super::diagnose;
use super::html::{HtmlSummary, extract};
use super::net::{FetchOptions, FetchedResponse, fetch};
use super::rdap::parse_date;
use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::files::StoredBlob;
use crate::model::{
    EntityKey, EntityKind, EvidenceBatch, Fields, NewSource, SourceKind, Target, TargetKind,
    TimelineKind, relation,
};
use crate::sanitize::{sanitize_bytes, sanitize_line, truncate_with_marker};

const HTML_ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
const ROBOTS_MAX_BYTES: usize = 256 * 1024;
const SITEMAP_MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_SITEMAP_URLS: usize = 300;
const MAX_LINKED_HOSTNAME_ENTITIES: usize = 50;
const MAX_CONTENT_LINKS: usize = 60;
const MAX_ROBOTS_CONTENT: usize = 32 * 1024;

pub struct HttpCollector;

impl Collector for HttpCollector {
    fn id(&self) -> &'static str {
        "http"
    }

    fn label(&self) -> &'static str {
        "HTTP"
    }

    fn description(&self) -> &'static str {
        "Status, redirects, headers, page metadata, links, header diagnostics, and security.txt"
    }

    fn accepts(&self, kind: TargetKind) -> bool {
        matches!(kind, TargetKind::Domain | TargetKind::Url)
    }

    fn collect<'a>(
        &'a self,
        target: &'a Target,
        context: &'a CollectorContext,
    ) -> BoxFuture<'a, CollectResult> {
        Box::pin(collect(target, context))
    }
}

async fn collect(target: &Target, context: &CollectorContext) -> CollectResult {
    let mut warnings = Vec::new();
    let response = fetch_target(target, context, &mut warnings).await?;
    let extension = snapshot_extension(&response);
    let snapshot = context
        .files
        .store_snapshot(&response.body, extension)
        .map_err(|error| format!("could not store HTTP snapshot: {error}"))?;
    let html = parse_html(&response).await;
    let mut batch = build_batch(
        target,
        &response,
        html.as_ref(),
        &snapshot,
        context.net.settings.proxy.is_none(),
    );
    if context.settings.fetch_robots {
        collect_robots(&response.final_url, context, &mut batch, &mut warnings).await;
    }
    collect_security_txt(&response.final_url, context, &mut batch, &mut warnings).await;
    for warning in &warnings {
        context.job.log(format!("warning: {warning}"));
    }
    Ok(CollectionOutput { batch, warnings })
}

async fn fetch_target(
    target: &Target,
    context: &CollectorContext,
    warnings: &mut Vec<String>,
) -> Result<FetchedResponse, String> {
    let options = FetchOptions {
        max_body_bytes: context.net.settings.max_body_bytes,
        accept: Some(HTML_ACCEPT),
    };
    let primary = match target {
        Target::Domain(domain) => {
            Url::parse(&format!("https://{domain}/")).map_err(|error| error.to_string())?
        }
        Target::Url(url) => url.clone(),
        _ => {
            return Err(format!(
                "the HTTP collector does not accept {}",
                target.kind()
            ));
        }
    };
    context.job.progress(format!("Fetching {primary}"));
    context.job.command("GET", &[primary.to_string()]);
    match fetch(&context.net, &primary, &options, &context.job.cancel).await {
        Ok(response) => Ok(response),
        Err(error) if matches!(target, Target::Domain(_)) && error != "cancelled" => {
            let fallback = Url::parse(&primary.as_str().replacen("https://", "http://", 1))
                .map_err(|parse| parse.to_string())?;
            warnings.push(format!("HTTPS failed ({error}); retried over plain HTTP"));
            context.job.command("GET", &[fallback.to_string()]);
            fetch(&context.net, &fallback, &options, &context.job.cancel)
                .await
                .map_err(|fallback_error| {
                    format!("HTTP collection failed: {error}; plain HTTP: {fallback_error}")
                })
        }
        Err(error) => Err(format!("HTTP collection failed: {error}")),
    }
}

fn snapshot_extension(response: &FetchedResponse) -> &'static str {
    let content_type = response.content_type().unwrap_or_default();
    if response.is_html() {
        "html"
    } else if content_type.contains("json") {
        "json"
    } else if content_type.contains("xml") {
        "xml"
    } else if content_type.starts_with("text/") {
        "txt"
    } else {
        "bin"
    }
}

async fn parse_html(response: &FetchedResponse) -> Option<HtmlSummary> {
    if !response.is_html() || response.body.is_empty() {
        return None;
    }
    let body = String::from_utf8_lossy(&response.body).into_owned();
    let base = response.final_url.clone();
    tokio::task::spawn_blocking(move || extract(&body, &base))
        .await
        .ok()
}

/// CDN/edge networks recognizable from response headers.
pub fn detect_edge(headers: &[(String, String)]) -> Vec<&'static str> {
    let has = |name: &str| headers.iter().any(|(candidate, _)| candidate == name);
    let value = |name: &str| {
        headers
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.to_ascii_lowercase())
            .unwrap_or_default()
    };
    let server = value("server");
    let via = value("via");
    let mut found = Vec::new();
    let checks: [(&str, bool); 9] = [
        ("Cloudflare", has("cf-ray") || server.contains("cloudflare")),
        (
            "Amazon CloudFront",
            has("x-amz-cf-id") || via.contains("cloudfront"),
        ),
        (
            "Fastly",
            has("x-fastly-request-id")
                || (has("x-served-by") && value("x-served-by").contains("cache-")),
        ),
        (
            "Akamai",
            server.contains("akamai")
                || headers.iter().any(|(name, _)| name.starts_with("x-akamai")),
        ),
        ("Vercel", has("x-vercel-id") || server.contains("vercel")),
        (
            "Netlify",
            has("x-nf-request-id") || server.contains("netlify"),
        ),
        (
            "GitHub Pages",
            has("x-github-request-id") && server.contains("github"),
        ),
        ("Azure Front Door", has("x-azure-ref")),
        (
            "Google Frontend",
            server.contains("gws") || server.contains("google frontend") || server == "esf",
        ),
    ];
    for (name, matched) in checks {
        if matched {
            found.push(name);
        }
    }
    found
}

fn security_headers(headers: &[(String, String)]) -> String {
    let checks = [
        ("HSTS", "strict-transport-security"),
        ("CSP", "content-security-policy"),
        ("X-Frame-Options", "x-frame-options"),
        ("X-Content-Type-Options", "x-content-type-options"),
        ("Referrer-Policy", "referrer-policy"),
    ];
    checks
        .iter()
        .map(|(label, name)| {
            let present = headers.iter().any(|(candidate, _)| candidate == name);
            format!("{label} {}", if present { "✓" } else { "✗" })
        })
        .collect::<Vec<_>>()
        .join("  ")
}

fn last_modified(response: &FetchedResponse) -> Option<DateTime<Utc>> {
    let value = response.header("last-modified")?;
    DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

fn describe(
    response: &FetchedResponse,
    html: Option<&HtmlSummary>,
    snapshot: &StoredBlob,
) -> String {
    let mut lines = vec![
        format!("Requested: {}", response.requested_url),
        format!("Final URL: {}", response.final_url),
        format!("Status: {}", response.status),
    ];
    if !response.redirects.is_empty() {
        lines.push("Redirect chain:".into());
        for hop in &response.redirects {
            lines.push(format!("  {} {} -> {}", hop.status, hop.url, hop.location));
        }
    }
    if let Some(reason) = &response.blocked_redirect {
        lines.push(format!("Redirect not followed: {reason}"));
    }
    if let Some(address) = response.remote_addr {
        lines.push(format!("Remote address: {address}"));
    }
    lines.push(format!("Elapsed: {} ms", response.elapsed.as_millis()));
    lines.push(format!(
        "Body: {} bytes{} ({}), sha256 {}",
        response.body.len(),
        if response.body_truncated {
            " (truncated at network.max_body_bytes)"
        } else {
            ""
        },
        response.content_type().unwrap_or("unknown type"),
        snapshot.sha256
    ));
    if let Some(html) = html {
        describe_html(html, &mut lines);
    }
    lines.push(String::new());
    lines.push("Headers:".into());
    lines.extend(
        response
            .headers
            .iter()
            .map(|(name, value)| format!("  {name}: {value}")),
    );
    lines.push(String::new());
    lines.push(format!(
        "Security headers: {}",
        security_headers(&response.headers)
    ));
    diagnose::append_section(
        &mut lines,
        &diagnose::assess_http(response.final_url.scheme() == "https", &response.headers),
    );
    let edges = detect_edge(&response.headers);
    if !edges.is_empty() {
        lines.push(format!("Edge/CDN indicators: {}", edges.join(", ")));
    }
    if let Some(html) = html {
        describe_html_links(html, &mut lines);
    } else if !response.body.is_empty() {
        lines.push(String::new());
        lines.push("--- Body preview ---".into());
        let preview = sanitize_bytes(&response.body[..response.body.len().min(4096)]);
        lines.push(preview);
    }
    lines.join("\n")
}

fn describe_html(html: &HtmlSummary, lines: &mut Vec<String>) {
    let mut add = |label: &str, value: &Option<String>| {
        if let Some(value) = value {
            lines.push(format!("{label}: {value}"));
        }
    };
    add("Title", &html.title);
    add("Description", &html.description);
    add("Canonical", &html.canonical);
    add("Generator", &html.generator);
    add("Author", &html.author);
    add("Published", &html.published);
    if !html.meta.is_empty() {
        lines.push("Meta:".into());
        lines.extend(
            html.meta
                .iter()
                .map(|(name, value)| format!("  {name} = {value}")),
        );
    }
}

fn describe_html_links(html: &HtmlSummary, lines: &mut Vec<String>) {
    if !html.emails.is_empty() {
        lines.push(format!(
            "Email addresses visible in page: {}",
            html.emails.join(", ")
        ));
    }
    lines.push(String::new());
    lines.push(format!("Referenced hostnames ({}):", html.hostnames.len()));
    lines.extend(html.hostnames.iter().map(|host| format!("  {host}")));
    lines.push(String::new());
    lines.push(format!(
        "Links ({}, first {MAX_CONTENT_LINKS} shown):",
        html.links.len()
    ));
    lines.extend(
        html.links
            .iter()
            .take(MAX_CONTENT_LINKS)
            .map(|link| format!("  {link}")),
    );
    lines.push(String::new());
    lines.push("--- Visible text ---".into());
    lines.push(truncate_with_marker(&html.text, 16 * 1024).into_owned());
}

fn source_fields(
    response: &FetchedResponse,
    html: Option<&HtmlSummary>,
    snapshot: &StoredBlob,
) -> Fields {
    let mut fields = Fields::new();
    fields.push("status", response.status.to_string());
    fields.push("final_url", response.final_url.to_string());
    fields.push("redirects", response.redirects.len().to_string());
    fields.push_opt("server", response.header("server"));
    fields.push_opt("x-powered-by", response.header("x-powered-by"));
    fields.push_opt("content-type", response.content_type());
    fields.push("body_bytes", response.body.len().to_string());
    fields.push("body_sha256", snapshot.sha256.clone());
    if let Some(address) = response.remote_addr {
        fields.push("remote_address", address.to_string());
    }
    if let Some(html) = html {
        fields.push_opt("title", html.title.clone());
        fields.push_opt("generator", html.generator.clone());
        fields.push_opt("canonical", html.canonical.clone());
    }
    let edges = detect_edge(&response.headers);
    if !edges.is_empty() {
        fields.push("edge", edges.join(", "));
    }
    let diagnostics =
        diagnose::assess_http(response.final_url.scheme() == "https", &response.headers);
    let posture = diagnose::posture(&diagnostics);
    if !posture.is_empty() {
        fields.push("diagnostics", posture);
    }
    fields
}

/// Builds the evidence batch for one HTTP fetch. Pure and unit-tested.
pub fn build_batch(
    target: &Target,
    response: &FetchedResponse,
    html: Option<&HtmlSummary>,
    snapshot: &StoredBlob,
    record_remote_address: bool,
) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let title = format!("HTTP {} {}", response.status, response.final_url);
    let mut source = NewSource::collected(SourceKind::Http, "http", title);
    source.url = Some(response.final_url.to_string());
    source.content_hash = Some(snapshot.sha256.clone());
    source.snapshot_path = Some(snapshot.relative_path.clone());
    source.content = describe(response, html, snapshot);
    source.fields = source_fields(response, html, snapshot);
    source.author = html.and_then(|html| html.author.clone());
    source.published_at = html
        .and_then(|html| html.published.as_deref())
        .and_then(parse_date);
    let key = batch.add_source(source);
    let subject = batch.entity(target.entity_kind(), &target.value(), Some(key));
    relate_redirects(&mut batch, response, key);
    let final_url = EntityKey::new(EntityKind::Url, response.final_url.as_str());
    if subject != final_url {
        batch.relate(&subject, relation::SERVES, &final_url, Some(key));
    }
    if let Some(edge) = detect_edge(&response.headers).first() {
        batch.annotate(&final_url, "edge", *edge);
    }
    if let Some(server) = response.header("server") {
        batch.annotate(&final_url, "server", server);
    }
    let posture = diagnose::posture(&diagnose::assess_http(
        response.final_url.scheme() == "https",
        &response.headers,
    ));
    if !posture.is_empty() {
        batch.annotate(&subject, "diagnostics", posture);
    }
    let host = response.final_url.host_str().unwrap_or("url").to_string();
    diagnose::record_findings(
        &mut batch,
        &host,
        &diagnose::assess_http(response.final_url.scheme() == "https", &response.headers),
        Some(key),
    );
    let final_host = response.final_url.host_str().map(str::to_ascii_lowercase);
    if let Some(host) = final_host.as_deref() {
        relate_final_host(
            &mut batch,
            target,
            host,
            response,
            record_remote_address,
            key,
        );
    }
    if let Some(html) = html {
        relate_html(
            &mut batch,
            &subject,
            &final_url,
            final_host.as_deref(),
            target,
            html,
            key,
        );
    }
    if let Some(modified) = last_modified(response) {
        batch.event(
            modified,
            TimelineKind::Published,
            format!("Last-Modified of {}", response.final_url),
            Some(key),
            Some(final_url.clone()),
        );
    }
    if let Some(published) = html
        .and_then(|html| html.published.as_deref())
        .and_then(parse_date)
    {
        batch.event(
            published,
            TimelineKind::Published,
            format!("Page published: {}", response.final_url),
            Some(key),
            Some(final_url),
        );
    }
    batch.summary = format!(
        "HTTP {} from {}{}",
        response.status,
        response.final_url,
        html.and_then(|html| html.title.as_ref())
            .map(|title| format!(" — {title}"))
            .unwrap_or_default()
    );
    batch
}

fn relate_redirects(batch: &mut EvidenceBatch, response: &FetchedResponse, key: usize) {
    let mut hops: Vec<Url> = response
        .redirects
        .iter()
        .map(|hop| hop.url.clone())
        .collect();
    hops.push(response.final_url.clone());
    for pair in hops.windows(2) {
        let from = EntityKey::new(EntityKind::Url, pair[0].as_str());
        let to = EntityKey::new(EntityKind::Url, pair[1].as_str());
        batch.relate(&from, relation::REDIRECTS_TO, &to, Some(key));
    }
}

fn relate_final_host(
    batch: &mut EvidenceBatch,
    target: &Target,
    host: &str,
    response: &FetchedResponse,
    record_remote_address: bool,
    key: usize,
) {
    if host.parse::<IpAddr>().is_ok() {
        return;
    }
    let domain = target.hostname().unwrap_or_default();
    let host_key = if host == domain {
        EntityKey::new(target.entity_kind(), &target.value())
    } else {
        let host_key = EntityKey::new(EntityKind::Hostname, host);
        if host.ends_with(&format!(".{domain}")) && matches!(target, Target::Domain(_)) {
            let domain_key = EntityKey::new(EntityKind::Domain, &domain);
            batch.relate(&host_key, relation::SUBDOMAIN_OF, &domain_key, Some(key));
        }
        host_key
    };
    if host_key.kind == EntityKind::Url {
        return;
    }
    if record_remote_address && let Some(address) = response.remote_addr {
        let ip = EntityKey::new(EntityKind::Ip, &address.ip().to_string());
        batch.relate(&host_key, relation::RESOLVES_TO, &ip, Some(key));
    }
}

fn relate_html(
    batch: &mut EvidenceBatch,
    subject: &EntityKey,
    final_url: &EntityKey,
    final_host: Option<&str>,
    target: &Target,
    html: &HtmlSummary,
    key: usize,
) {
    let own_host = target.hostname();
    let linked = html
        .hostnames
        .iter()
        .filter(|host| {
            Some(host.as_str()) != final_host && Some(host.as_str()) != own_host.as_deref()
        })
        .filter(|host| host.parse::<IpAddr>().is_err())
        .take(MAX_LINKED_HOSTNAME_ENTITIES);
    for host in linked {
        let host_key = EntityKey::new(EntityKind::Hostname, host);
        batch.relate(subject, relation::LINKS_TO, &host_key, Some(key));
    }
    for email in &html.emails {
        let email_key = EntityKey::new(EntityKind::Email, email);
        batch.relate(final_url, relation::MENTIONS, &email_key, Some(key));
    }
    if let Some(generator) = &html.generator {
        batch.annotate(final_url, "generator", generator.clone());
    }
    if let Some(title) = &html.title {
        batch.annotate(final_url, "title", title.clone());
    }
}

/// Robots.txt directives worth reporting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RobotsSummary {
    pub sitemaps: Vec<String>,
    pub disallowed: Vec<String>,
}

pub fn parse_robots(text: &str) -> RobotsSummary {
    let mut summary = RobotsSummary::default();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let Some((directive, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match directive.trim().to_ascii_lowercase().as_str() {
            "sitemap" if !value.is_empty() => summary.sitemaps.push(value.to_string()),
            "disallow"
                if !value.is_empty() && !summary.disallowed.iter().any(|path| path == value) =>
            {
                summary.disallowed.push(value.to_string())
            }
            _ => {}
        }
    }
    summary
}

/// Extracts `<loc>` URLs from a sitemap or sitemap index.
pub fn parse_sitemap(xml: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<loc>") {
        let after = &rest[start + "<loc>".len()..];
        let Some(end) = after.find("</loc>") else {
            break;
        };
        let value = after[..end].trim().replace("&amp;", "&");
        if value.starts_with("http") {
            urls.push(sanitize_line(&value).into_owned());
        }
        if urls.len() >= MAX_SITEMAP_URLS {
            break;
        }
        rest = &after[end..];
    }
    urls
}

async fn collect_robots(
    base: &Url,
    context: &CollectorContext,
    batch: &mut EvidenceBatch,
    warnings: &mut Vec<String>,
) {
    let Ok(robots_url) = base.join("/robots.txt") else {
        return;
    };
    context.job.progress(format!("Fetching {robots_url}"));
    context.job.command("GET", &[robots_url.to_string()]);
    let options = FetchOptions {
        max_body_bytes: ROBOTS_MAX_BYTES,
        accept: Some("text/plain,*/*;q=0.5"),
    };
    let mut sitemap_candidates = Vec::new();
    match fetch(&context.net, &robots_url, &options, &context.job.cancel).await {
        Ok(response) if response.is_success() && !response.is_html() => {
            let text = sanitize_bytes(&response.body);
            let robots = parse_robots(&text);
            sitemap_candidates.extend(robots.sitemaps.iter().cloned());
            add_robots_source(batch, &response, &text, &robots);
        }
        Ok(response) => warnings.push(format!("robots.txt: HTTP {}", response.status)),
        Err(error) => warnings.push(format!("robots.txt: {error}")),
    }
    if sitemap_candidates.is_empty()
        && let Ok(default) = base.join("/sitemap.xml")
    {
        sitemap_candidates.push(default.to_string());
    }
    if let Some(sitemap) = sitemap_candidates
        .first()
        .and_then(|candidate| Url::parse(candidate).ok())
    {
        collect_sitemap(&sitemap, context, batch, warnings).await;
    }
}

const SECURITY_TXT_MAX_BYTES: usize = 64 * 1024;

/// A published `security.txt` (RFC 9116).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SecurityTxt {
    pub contacts: Vec<String>,
    pub expires: Option<String>,
    pub policies: Vec<String>,
    pub canonicals: Vec<String>,
    pub acknowledgments: Vec<String>,
}

impl SecurityTxt {
    pub fn has_contact(&self) -> bool {
        !self.contacts.is_empty()
    }
}

/// Parses contact, expiry, policy, and canonical fields. Other lines are kept out of entities.
pub fn parse_security_txt(text: &str) -> SecurityTxt {
    let mut parsed = SecurityTxt::default();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        let value = sanitize_line(value).trim().to_string();
        if value.is_empty() {
            continue;
        }
        match field.trim().to_ascii_lowercase().as_str() {
            "contact" if parsed.contacts.len() < 20 => parsed.contacts.push(value),
            "expires" if parsed.expires.is_none() => parsed.expires = Some(value),
            "policy" if parsed.policies.len() < 10 => parsed.policies.push(value),
            "canonical" if parsed.canonicals.len() < 5 => parsed.canonicals.push(value),
            "acknowledgments" | "acknowledgements" if parsed.acknowledgments.len() < 10 => {
                parsed.acknowledgments.push(value);
            }
            _ => {}
        }
    }
    parsed
}

async fn collect_security_txt(
    base: &Url,
    context: &CollectorContext,
    batch: &mut EvidenceBatch,
    warnings: &mut Vec<String>,
) {
    if base.scheme() != "https" {
        return;
    }
    let host = base.host_str().unwrap_or("url").to_string();
    let mut candidates = Vec::new();
    if let Ok(url) = base.join("/.well-known/security.txt") {
        candidates.push(url);
    }
    if let Ok(url) = base.join("/security.txt") {
        candidates.push(url);
    }
    let options = FetchOptions {
        max_body_bytes: SECURITY_TXT_MAX_BYTES,
        accept: Some("text/plain,*/*;q=0.5"),
    };
    let mut saw_absent = false;
    for url in &candidates {
        context.job.progress(format!("Fetching {url}"));
        context.job.command("GET", &[url.to_string()]);
        match fetch(&context.net, url, &options, &context.job.cancel).await {
            Ok(response) if response.is_success() && !response.is_html() => {
                let text = sanitize_bytes(&response.body);
                add_security_txt(batch, &host, &response, &text);
                return;
            }
            Ok(response) if response.status == 404 || response.status == 410 => {
                saw_absent = true;
            }
            Ok(response) => {
                warnings.push(format!("security.txt {url}: HTTP {}", response.status));
                return;
            }
            Err(error) if error == "cancelled" => return,
            Err(error) => {
                warnings.push(format!("security.txt {url}: {error}"));
                return;
            }
        }
    }
    if saw_absent {
        let diagnostic = diagnose::Diagnostic::new(
            "security.txt",
            diagnose::DiagnosticState::Missing,
            "No security.txt was published",
        );
        let source = if batch.sources.is_empty() {
            None
        } else {
            Some(0)
        };
        diagnose::record_findings(batch, &host, &[diagnostic], source);
    }
}

fn add_security_txt(batch: &mut EvidenceBatch, host: &str, response: &FetchedResponse, text: &str) {
    let parsed = parse_security_txt(text);
    let mut source =
        NewSource::collected(SourceKind::Http, "http", format!("security.txt for {host}"));
    source.url = Some(response.final_url.to_string());
    source.content_hash = Some(crate::files::sha256_hex(&response.body));
    source.content = truncate_with_marker(text, SECURITY_TXT_MAX_BYTES).into_owned();
    source
        .fields
        .push("contacts", parsed.contacts.len().to_string());
    source.fields.push_opt("expires", parsed.expires.clone());
    let key = batch.add_source(source);
    let page = EntityKey::new(EntityKind::Url, response.final_url.as_str());
    for contact in &parsed.contacts {
        relate_security_contact(batch, &page, contact, key);
    }
    for policy in &parsed.policies {
        if let Ok(url) = Url::parse(policy) {
            let policy_key = EntityKey::new(EntityKind::Url, url.as_str());
            batch.relate(&page, relation::REFERENCES, &policy_key, Some(key));
        }
    }
    let mut diagnostics = vec![diagnose::Diagnostic::new(
        "security.txt",
        diagnose::DiagnosticState::Present,
        "security.txt is published",
    )];
    match parsed.expires.as_deref().and_then(parse_date) {
        Some(expires) if expires <= Utc::now() => diagnostics.push(diagnose::Diagnostic::new(
            "security.txt-expires",
            diagnose::DiagnosticState::Review,
            "security.txt Expires is in the past",
        )),
        Some(expires) => {
            batch.event(
                expires,
                TimelineKind::Expiration,
                format!("security.txt for {host} expires"),
                Some(key),
                Some(page.clone()),
            );
            diagnostics.push(diagnose::Diagnostic::new(
                "security.txt-expires",
                diagnose::DiagnosticState::Present,
                "security.txt Expires is in the future",
            ));
        }
        None => diagnostics.push(diagnose::Diagnostic::new(
            "security.txt-expires",
            diagnose::DiagnosticState::Review,
            "security.txt has no Expires field",
        )),
    }
    if !parsed.has_contact() {
        diagnostics.push(diagnose::Diagnostic::new(
            "security.txt-contact",
            diagnose::DiagnosticState::Review,
            "security.txt has no Contact field",
        ));
    } else {
        diagnostics.push(diagnose::Diagnostic::new(
            "security.txt-contact",
            diagnose::DiagnosticState::Present,
            "security.txt publishes a Contact",
        ));
    }
    diagnose::record_findings(batch, host, &diagnostics, Some(key));
}

fn relate_security_contact(batch: &mut EvidenceBatch, page: &EntityKey, contact: &str, key: usize) {
    let value = contact.strip_prefix("mailto:").unwrap_or(contact).trim();
    if value.contains('@') && !value.contains('/') {
        let email = EntityKey::new(EntityKind::Email, value);
        batch.relate(page, relation::MENTIONS, &email, Some(key));
        return;
    }
    if let Ok(url) = Url::parse(contact) {
        let url_key = EntityKey::new(EntityKind::Url, url.as_str());
        batch.relate(page, relation::REFERENCES, &url_key, Some(key));
    }
}

fn add_robots_source(
    batch: &mut EvidenceBatch,
    response: &FetchedResponse,
    text: &str,
    robots: &RobotsSummary,
) {
    let mut source = NewSource::collected(
        SourceKind::Http,
        "http",
        format!(
            "robots.txt for {}",
            response.final_url.host_str().unwrap_or_default()
        ),
    );
    source.url = Some(response.final_url.to_string());
    source.content_hash = Some(crate::files::sha256_hex(&response.body));
    source.content = truncate_with_marker(text, MAX_ROBOTS_CONTENT).into_owned();
    source
        .fields
        .push("disallow_rules", robots.disallowed.len().to_string());
    source.fields.push("sitemaps", robots.sitemaps.join(", "));
    batch.add_source(source);
}

async fn collect_sitemap(
    url: &Url,
    context: &CollectorContext,
    batch: &mut EvidenceBatch,
    warnings: &mut Vec<String>,
) {
    context.job.progress(format!("Fetching {url}"));
    context.job.command("GET", &[url.to_string()]);
    let options = FetchOptions {
        max_body_bytes: SITEMAP_MAX_BYTES,
        accept: Some("application/xml,text/xml;q=0.9,*/*;q=0.5"),
    };
    let response = match fetch(&context.net, url, &options, &context.job.cancel).await {
        Ok(response) if response.is_success() && !response.is_html() => response,
        Ok(response) => {
            warnings.push(format!("sitemap {url}: HTTP {}", response.status));
            return;
        }
        Err(error) => {
            warnings.push(format!("sitemap {url}: {error}"));
            return;
        }
    };
    let text = String::from_utf8_lossy(&response.body);
    let urls = parse_sitemap(&text);
    if urls.is_empty() {
        return;
    }
    let mut source = NewSource::collected(
        SourceKind::Http,
        "http",
        format!("Sitemap {}", response.final_url),
    );
    source.url = Some(response.final_url.to_string());
    source.content_hash = Some(crate::files::sha256_hex(&response.body));
    source.content = format!(
        "{} URLs listed (first {MAX_SITEMAP_URLS} kept):\n{}",
        urls.len(),
        urls.join("\n")
    );
    source.fields.push("urls", urls.len().to_string());
    batch.add_source(source);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::collectors::net::RedirectHop;

    fn response() -> FetchedResponse {
        FetchedResponse {
            requested_url: Url::parse("https://example.com/").expect("url"),
            final_url: Url::parse("https://www.example.com/").expect("url"),
            redirects: vec![RedirectHop {
                url: Url::parse("https://example.com/").expect("url"),
                status: 301,
                location: "https://www.example.com/".into(),
            }],
            status: 200,
            headers: vec![
                ("server".into(), "cloudflare".into()),
                ("cf-ray".into(), "abc-AMS".into()),
                ("content-type".into(), "text/html; charset=utf-8".into()),
                (
                    "last-modified".into(),
                    "Wed, 21 Oct 2015 07:28:00 GMT".into(),
                ),
            ],
            body: b"<html></html>".to_vec(),
            body_truncated: false,
            remote_addr: Some("192.0.2.5:443".parse().expect("addr")),
            elapsed: Duration::from_millis(120),
            blocked_redirect: None,
        }
    }

    fn snapshot() -> StoredBlob {
        StoredBlob {
            relative_path: "snapshots/ab/abc.html".into(),
            sha256: "abc".into(),
            size_bytes: 13,
        }
    }

    #[test]
    fn security_txt_contacts_and_expiry_are_parsed() {
        let input = "\
Contact: mailto:security@example.com
Expires: 2020-01-01T00:00:00.000Z
Policy: https://example.com/policy
# not a field
Acknowledgments: https://example.com/thanks
";
        let actual = parse_security_txt(input);
        assert_eq!(actual.contacts, vec!["mailto:security@example.com"]);
        assert_eq!(actual.expires.as_deref(), Some("2020-01-01T00:00:00.000Z"));
        assert_eq!(actual.policies, vec!["https://example.com/policy"]);
        assert_eq!(actual.acknowledgments, vec!["https://example.com/thanks"]);
        assert!(actual.has_contact());
    }

    #[test]
    fn detects_cloudflare() {
        assert_eq!(detect_edge(&response().headers), vec!["Cloudflare"]);
    }

    #[test]
    fn batch_records_redirects_hosts_and_addresses() {
        let input_html = HtmlSummary {
            title: Some("Example".into()),
            hostnames: vec!["cdn.example.net".into(), "www.example.com".into()],
            emails: vec!["admin@example.com".into()],
            ..HtmlSummary::default()
        };
        let target = Target::Domain("example.com".into());
        let actual = build_batch(&target, &response(), Some(&input_html), &snapshot(), true);
        let has = |kind: &str, to: &str| {
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == kind && relationship.to.value == to)
        };
        assert!(has("redirects_to", "https://www.example.com/"));
        assert!(has("serves", "https://www.example.com/"));
        assert!(has("subdomain_of", "example.com"));
        assert!(has("resolves_to", "192.0.2.5"));
        assert!(has("links_to", "cdn.example.net"));
        assert!(has("mentions", "admin@example.com"));
        assert!(!has("links_to", "www.example.com"));
        assert_eq!(actual.timeline.len(), 1);
        assert!(
            actual.sources[0]
                .content
                .contains("Edge/CDN indicators: Cloudflare")
        );
    }

    #[test]
    fn proxied_fetches_do_not_record_remote_address() {
        let target = Target::Domain("example.com".into());
        let actual = build_batch(&target, &response(), None, &snapshot(), false);
        assert!(
            !actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "resolves_to")
        );
    }

    #[test]
    fn robots_directives_are_parsed() {
        let input = "User-agent: *\nDisallow: /admin # private\nDisallow: /admin\nSitemap: https://example.com/sitemap.xml\n";
        let actual = parse_robots(input);
        assert_eq!(actual.disallowed, vec!["/admin"]);
        assert_eq!(actual.sitemaps, vec!["https://example.com/sitemap.xml"]);
    }

    #[test]
    fn sitemap_locations_are_parsed() {
        let input = "<urlset><url><loc> https://example.com/a?x=1&amp;y=2 </loc></url><url><loc>https://example.com/b</loc></url></urlset>";
        assert_eq!(
            parse_sitemap(input),
            vec!["https://example.com/a?x=1&y=2", "https://example.com/b"]
        );
    }
}
