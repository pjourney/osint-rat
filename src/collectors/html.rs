//! Extraction of metadata, links, hostnames, emails, and visible text from HTML.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use scraper::{Html, Selector};
use url::Url;

use crate::model::target::normalize_email;
use crate::sanitize::{sanitize_line, sanitize_text};

const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_LINKS: usize = 300;
const MAX_HOSTNAMES: usize = 150;
const MAX_EMAILS: usize = 50;
const MAX_META: usize = 60;
const HIDDEN_ELEMENTS: [&str; 7] = [
    "script", "style", "noscript", "template", "svg", "head", "iframe",
];
const NOT_EMAIL_TLDS: [&str; 9] = [
    "png", "jpg", "jpeg", "gif", "svg", "webp", "css", "js", "ico",
];

static EMAIL_PATTERN: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[a-z0-9._%+'-]+@[a-z0-9-]+(?:\.[a-z0-9-]+)*\.[a-z]{2,24}\b").ok()
});

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HtmlSummary {
    pub title: Option<String>,
    pub description: Option<String>,
    pub generator: Option<String>,
    pub canonical: Option<String>,
    pub published: Option<String>,
    pub author: Option<String>,
    pub meta: Vec<(String, String)>,
    /// Absolute HTTP(S) URLs of `<a href>` links, in document order.
    pub links: Vec<String>,
    /// Hostnames referenced by links and embedded resources.
    pub hostnames: Vec<String>,
    /// Email addresses visible in text or `mailto:` links.
    pub emails: Vec<String>,
    pub text: String,
}

fn selector(css: &str) -> Option<Selector> {
    Selector::parse(css).ok()
}

fn clean(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    sanitize_line(&collapsed).into_owned()
}

/// Parses an HTML document. CPU-bound: call from a blocking task for large documents.
pub fn extract(html: &str, base: &Url) -> HtmlSummary {
    let document = Html::parse_document(html);
    let mut summary = HtmlSummary {
        title: first_text(&document, "title"),
        canonical: first_attr(&document, "link[rel~=canonical]", "href")
            .and_then(|href| base.join(&href).ok())
            .map(|url| url.to_string()),
        ..HtmlSummary::default()
    };
    extract_meta(&document, &mut summary);
    let (links, hostnames, mailto) = extract_links(&document, base);
    summary.links = links;
    summary.hostnames = hostnames;
    summary.text = visible_text(&document);
    summary.emails = find_emails(&summary.text, mailto);
    summary
}

fn first_text(document: &Html, css: &str) -> Option<String> {
    let selector = selector(css)?;
    document
        .select(&selector)
        .next()
        .map(|element| clean(&element.text().collect::<String>()))
        .filter(|text| !text.is_empty())
}

fn first_attr(document: &Html, css: &str, attribute: &str) -> Option<String> {
    let selector = selector(css)?;
    document
        .select(&selector)
        .find_map(|element| element.value().attr(attribute).map(str::to_string))
}

fn extract_meta(document: &Html, summary: &mut HtmlSummary) {
    let Some(selector) = selector("meta[content]") else {
        return;
    };
    for element in document.select(&selector) {
        let attributes = element.value();
        let Some(name) = attributes
            .attr("name")
            .or_else(|| attributes.attr("property"))
        else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let content = clean(attributes.attr("content").unwrap_or_default());
        if content.is_empty() {
            continue;
        }
        match name.as_str() {
            "description" | "og:description" if summary.description.is_none() => {
                summary.description = Some(content.clone())
            }
            "generator" => summary.generator = Some(content.clone()),
            "author" | "article:author" if summary.author.is_none() => {
                summary.author = Some(content.clone())
            }
            "article:published_time" | "date" | "dc.date" if summary.published.is_none() => {
                summary.published = Some(content.clone())
            }
            _ => {}
        }
        let is_interesting = name.starts_with("og:")
            || name.starts_with("twitter:")
            || name.starts_with("article:")
            || matches!(
                name.as_str(),
                "description" | "generator" | "author" | "application-name" | "keywords" | "robots"
            );
        if is_interesting && summary.meta.len() < MAX_META {
            summary.meta.push((name, content));
        }
    }
}

type ExtractedLinks = (Vec<String>, Vec<String>, Vec<String>);

fn extract_links(document: &Html, base: &Url) -> ExtractedLinks {
    let mut links = Vec::new();
    let mut seen_links = BTreeSet::new();
    let mut hostnames = BTreeSet::new();
    let mut mailto = Vec::new();
    let resources = [
        ("a[href]", "href"),
        ("link[href]", "href"),
        ("script[src]", "src"),
        ("img[src]", "src"),
        ("iframe[src]", "src"),
        ("source[src]", "src"),
        ("form[action]", "action"),
    ];
    for (css, attribute) in resources {
        let Some(selector) = selector(css) else {
            continue;
        };
        for element in document.select(&selector) {
            let Some(raw) = element.value().attr(attribute).map(str::trim) else {
                continue;
            };
            if let Some(address) = raw.strip_prefix("mailto:") {
                let address = address.split('?').next().unwrap_or_default();
                mailto.push(address.to_string());
                continue;
            }
            let Ok(url) = base.join(raw) else {
                continue;
            };
            if !matches!(url.scheme(), "http" | "https") {
                continue;
            }
            if let Some(host) = url.host_str()
                && hostnames.len() < MAX_HOSTNAMES
            {
                hostnames.insert(host.to_ascii_lowercase());
            }
            let is_anchor = css.starts_with("a[");
            if is_anchor && links.len() < MAX_LINKS && seen_links.insert(url.to_string()) {
                links.push(url.to_string());
            }
        }
    }
    (links, hostnames.into_iter().collect(), mailto)
}

fn visible_text(document: &Html) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut total = 0;
    for node in document.tree.nodes() {
        let Some(text) = node.value().as_text() else {
            continue;
        };
        let is_hidden = node.ancestors().any(|ancestor| {
            ancestor
                .value()
                .as_element()
                .map(|element| HIDDEN_ELEMENTS.contains(&element.name()))
                .unwrap_or(false)
        });
        if is_hidden {
            continue;
        }
        let line = clean(text);
        if line.is_empty() {
            continue;
        }
        total += line.len() + 1;
        if total > MAX_TEXT_BYTES {
            lines.push("[… text truncated]".into());
            break;
        }
        lines.push(line);
    }
    sanitize_text(&lines.join("\n")).into_owned()
}

/// Finds email addresses in visible text plus explicit `mailto:` addresses.
pub fn find_emails(text: &str, mailto: Vec<String>) -> Vec<String> {
    let mut found = BTreeSet::new();
    let candidates = EMAIL_PATTERN
        .as_ref()
        .map(|pattern| {
            pattern
                .find_iter(text)
                .map(|m| m.as_str().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for candidate in mailto.into_iter().chain(candidates) {
        let tld = candidate
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if NOT_EMAIL_TLDS.contains(&tld.as_str()) {
            continue;
        }
        if let Ok(email) = normalize_email(&candidate) {
            found.insert(email);
        }
        if found.len() >= MAX_EMAILS {
            break;
        }
    }
    found.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<!doctype html>
<html><head>
  <title> Example   Domain </title>
  <meta name="description" content="Illustrative example">
  <meta name="generator" content="WordPress 6.5">
  <meta property="og:site_name" content="Example">
  <link rel="canonical" href="/home">
  <script src="https://cdn.example.net/app.js"></script>
  <style>.hidden { color: red }</style>
</head><body>
  <h1>Welcome</h1>
  <p>Contact admin@example.com or <a href="mailto:press@example.com?subject=hi">press</a>.</p>
  <p>Logo: logo@2x.png</p>
  <a href="https://other.example.org/page">Other</a>
  <a href="/about">About</a>
  <a href="javascript:alert(1)">bad</a>
  <script>var secret = "ignore me";</script>
</body></html>"#;

    fn base() -> Url {
        Url::parse("https://example.com/").expect("base url")
    }

    #[test]
    fn extracts_metadata() {
        let actual = extract(PAGE, &base());
        assert_eq!(actual.title.as_deref(), Some("Example Domain"));
        assert_eq!(actual.description.as_deref(), Some("Illustrative example"));
        assert_eq!(actual.generator.as_deref(), Some("WordPress 6.5"));
        assert_eq!(
            actual.canonical.as_deref(),
            Some("https://example.com/home")
        );
        assert!(actual.meta.iter().any(|(name, _)| name == "og:site_name"));
    }

    #[test]
    fn extracts_links_and_hostnames() {
        let actual = extract(PAGE, &base());
        assert_eq!(
            actual.links,
            vec![
                "https://other.example.org/page",
                "https://example.com/about"
            ]
        );
        assert_eq!(
            actual.hostnames,
            vec!["cdn.example.net", "example.com", "other.example.org"]
        );
    }

    #[test]
    fn extracts_visible_emails_only() {
        let actual = extract(PAGE, &base());
        assert_eq!(
            actual.emails,
            vec!["admin@example.com", "press@example.com"]
        );
    }

    #[test]
    fn visible_text_excludes_scripts_and_styles() {
        let actual = extract(PAGE, &base());
        assert!(actual.text.contains("Welcome"));
        assert!(!actual.text.contains("ignore me"));
        assert!(!actual.text.contains("color: red"));
    }

    #[test]
    fn hostile_markup_is_sanitized() {
        let input = "<html><head><title>\u{1b}]0;owned\u{07}Title</title></head><body>\u{1b}[2Jtext</body></html>";
        let actual = extract(input, &base());
        assert_eq!(actual.title.as_deref(), Some("Title"));
        assert_eq!(actual.text, "text");
    }
}
