//! Passive username pack: public profile URL checks and graph edges.
//!
//! Each check is a single HTTP(S) GET of a well-known public profile URL. No login, no
//! brute force, no credential stuffing. 2xx responses become `associated_with` links.

use futures::future::BoxFuture;
use url::Url;

use super::net::{FetchOptions, fetch};
use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::files::sha256_hex;
use crate::model::{
    EntityKind, EvidenceBatch, NewSource, SourceKind, Target, TargetKind, relation,
};

/// Public profile URL templates. `{username}` is replaced with the normalized handle.
pub const PROFILE_TEMPLATES: &[(&str, &str)] = &[
    ("GitHub", "https://github.com/{username}"),
    ("GitLab", "https://gitlab.com/{username}"),
    ("Codeberg", "https://codeberg.org/{username}"),
    ("Reddit", "https://www.reddit.com/user/{username}"),
    (
        "Hacker News",
        "https://news.ycombinator.com/user?id={username}",
    ),
    ("Keybase", "https://keybase.io/{username}"),
];

pub struct UsernameCollector;

impl Collector for UsernameCollector {
    fn id(&self) -> &str {
        "username"
    }

    fn label(&self) -> &str {
        "Username"
    }

    fn description(&self) -> &str {
        "Public profile URLs for a username (GitHub, GitLab, Reddit, …)"
    }

    fn accepts(&self, kind: TargetKind) -> bool {
        kind == TargetKind::Username
    }

    fn collect<'a>(
        &'a self,
        target: &'a Target,
        context: &'a CollectorContext,
    ) -> BoxFuture<'a, CollectResult> {
        Box::pin(collect(target, context))
    }
}

/// One public profile observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileHit {
    pub site: String,
    pub url: String,
    pub status: u16,
    pub present: bool,
}

async fn collect(target: &Target, context: &CollectorContext) -> CollectResult {
    let Target::Username(username) = target else {
        return Err(format!(
            "the username collector does not accept {}",
            target.kind()
        ));
    };
    let cap = context.settings.max_username_profiles.max(1) as usize;
    let mut hits = Vec::new();
    let mut warnings = Vec::new();
    for (site, template) in PROFILE_TEMPLATES.iter().take(cap) {
        if context.job.cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        let url_text = template.replace("{username}", username);
        let Ok(url) = Url::parse(&url_text) else {
            continue;
        };
        context.job.progress(format!("Checking {site}"));
        context.job.command("GET", &[url.to_string()]);
        let options = FetchOptions {
            max_body_bytes: 8 * 1024,
            accept: Some("text/html,application/json;q=0.8,*/*;q=0.1"),
            extra_headers: Vec::new(),
        };
        match fetch(&context.net, &url, &options, &context.job.cancel).await {
            Ok(response) => {
                let present = profile_is_present(response.status);
                hits.push(ProfileHit {
                    site: (*site).to_string(),
                    url: response.final_url.to_string(),
                    status: response.status,
                    present,
                });
            }
            Err(error) => {
                warnings.push(format!("{site}: {error}"));
            }
        }
    }
    if hits.is_empty() && warnings.is_empty() {
        return Err("no public profile URLs could be checked".into());
    }
    let mut output = CollectionOutput::new(build_batch(target, &hits));
    output.warnings = warnings;
    Ok(output)
}

/// A public profile is present only on a 2xx response. 3xx redirects are not treated as a hit.
pub fn profile_is_present(status: u16) -> bool {
    (200..300).contains(&status)
}

/// Builds the evidence batch from profile hits. Pure and unit-tested.
pub fn build_batch(target: &Target, hits: &[ProfileHit]) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let username = target.value();
    let mut source = NewSource::collected(
        SourceKind::Http,
        "username",
        format!("Public profiles for {username}"),
    );
    let mut lines = vec![format!("username: {username}")];
    for hit in hits {
        lines.push(format!(
            "{} {} HTTP {} {}",
            hit.site,
            hit.url,
            hit.status,
            if hit.present { "present" } else { "absent" }
        ));
    }
    source.content = lines.join("\n");
    source.content_hash = Some(sha256_hex(source.content.as_bytes()));
    source.fields.push("username", username.clone());
    source.fields.push(
        "present",
        hits.iter()
            .filter(|hit| hit.present)
            .map(|hit| hit.site.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    );
    let key = batch.add_source(source);
    let subject = batch.entity(EntityKind::Username, &username, Some(key));
    for hit in hits.iter().filter(|hit| hit.present) {
        let url = batch.entity(EntityKind::Url, &hit.url, Some(key));
        batch.annotate(&url, "site", hit.site.clone());
        batch.relate(&subject, relation::ASSOCIATED_WITH, &url, Some(key));
        if let Some(host) = Url::parse(&hit.url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
        {
            let host_key = batch.entity(EntityKind::Hostname, &host, Some(key));
            batch.relate(&url, relation::LINKS_TO, &host_key, Some(key));
        }
    }
    let present = hits.iter().filter(|hit| hit.present).count();
    batch.summary = format!(
        "{present} public profile(s) for {username} ({} checked)",
        hits.len()
    );
    batch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_profiles_become_associated_urls() {
        let target = Target::Username("octocat".into());
        let hits = vec![
            ProfileHit {
                site: "GitHub".into(),
                url: "https://github.com/octocat".into(),
                status: 200,
                present: true,
            },
            ProfileHit {
                site: "GitLab".into(),
                url: "https://gitlab.com/octocat".into(),
                status: 404,
                present: false,
            },
        ];
        let actual = build_batch(&target, &hits);
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "associated_with"
                    && relationship.to.value.contains("github.com/octocat"))
        );
        assert!(
            !actual
                .relationships
                .iter()
                .any(|relationship| relationship.to.value.contains("gitlab.com/octocat"))
        );
        assert_eq!(actual.sources[0].collector.as_deref(), Some("username"));
    }

    #[test]
    fn templates_include_github() {
        assert!(
            PROFILE_TEMPLATES
                .iter()
                .any(|(site, url)| *site == "GitHub" && url.contains("{username}"))
        );
    }

    #[test]
    fn redirects_are_not_present_profiles() {
        assert!(profile_is_present(200));
        assert!(!profile_is_present(302));
        assert!(!profile_is_present(301));
        assert!(!profile_is_present(404));
        let target = Target::Username("octocat".into());
        let hits = vec![ProfileHit {
            site: "GitHub".into(),
            url: "https://github.com/octocat".into(),
            status: 302,
            present: profile_is_present(302),
        }];
        let actual = build_batch(&target, &hits);
        assert!(
            !actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "associated_with")
        );
    }
}
