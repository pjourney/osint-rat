//! Public Git repository inspection.
//!
//! Uses `git` (and optionally `gh`) already on PATH. Clones are shallow, blob-filtered, and
//! stored inside the investigation directory. Private repositories are not accessed through
//! credential bypasses: git is invoked with an empty credential helper and no extra remotes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use url::Url;

use super::html::find_emails;
use super::{CollectResult, CollectionOutput, Collector, CollectorContext};
use crate::exec::{CommandSpec, ExecError, run};
use crate::files::sha256_hex;
use crate::model::{
    ArtifactKind, EntityKey, EntityKind, EvidenceBatch, Fields, NewArtifact, NewSource, SourceKind,
    Target, TargetKind, TimelineKind, relation,
};
use crate::paths::sanitize_filename;
use crate::sanitize::{sanitize_line, sanitize_text};

const GIT_TIMEOUT: Duration = Duration::from_secs(120);
const GIT_LOG_TIMEOUT: Duration = Duration::from_secs(30);
const GH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT: usize = 512 * 1024;
const MAX_COMMITS_IN_SOURCE: usize = 80;
const MAX_CONTRIBUTORS: usize = 40;
const MAX_REFERENCES: usize = 40;

pub struct RepositoryCollector;

impl Collector for RepositoryCollector {
    fn id(&self) -> &'static str {
        "repository"
    }

    fn label(&self) -> &'static str {
        "Repository"
    }

    fn description(&self) -> &'static str {
        "Public repository remotes, commits, contributors, and referenced hosts via git/gh"
    }

    fn accepts(&self, kind: TargetKind) -> bool {
        matches!(kind, TargetKind::Repository)
    }

    fn requires(&self) -> &'static [&'static str] {
        &["git"]
    }

    fn collect<'a>(
        &'a self,
        target: &'a Target,
        context: &'a CollectorContext,
    ) -> BoxFuture<'a, CollectResult> {
        Box::pin(collect(target, context))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoCommit {
    pub hash: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: Option<DateTime<Utc>>,
    pub subject: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoObservation {
    pub url: String,
    pub owner: Option<String>,
    pub name: Option<String>,
    pub default_branch: Option<String>,
    pub remotes: Vec<String>,
    pub commits: Vec<RepoCommit>,
    pub gh_summary: Option<String>,
    pub clone_path: Option<String>,
}

async fn collect(target: &Target, context: &CollectorContext) -> CollectResult {
    let Target::Repository(url) = target else {
        return Err(format!(
            "the repository collector does not accept {}",
            target.kind()
        ));
    };
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("only public https repository URLs can be collected".into());
    }
    let git = context
        .tools
        .path_of("git")
        .unwrap_or_else(|| PathBuf::from("git"));
    let mut observation = RepoObservation {
        url: url.clone(),
        ..RepoObservation::default()
    };
    if let Some((owner, name)) = owner_and_name(url) {
        observation.owner = Some(owner);
        observation.name = Some(name);
    }
    context.job.progress(format!("Listing remotes of {url}"));
    observation.remotes = ls_remote(&git, url, context).await?;
    observation.default_branch = observation
        .remotes
        .iter()
        .find(|line| line.contains("HEAD"))
        .cloned()
        .and_then(|line| line.split_whitespace().nth(1).map(str::to_string));
    let clone_dir = clone_destination(&context.files.repos_dir(), url);
    context
        .job
        .progress("Shallow clone (blob-filtered, public HTTPS)");
    clone_repo(
        &git,
        url,
        &clone_dir,
        context.settings.repository_history_depth,
        context,
    )
    .await?;
    let relative = relative_to(context.files.root(), &clone_dir);
    observation.clone_path = relative.clone();
    context.job.progress("Reading commit history");
    observation.commits = git_log(
        &git,
        &clone_dir,
        context.settings.repository_history_depth,
        context,
    )
    .await?;
    if context.tools.is_installed("gh") {
        context.job.progress("GitHub metadata via gh");
        match gh_view(url, context).await {
            Ok(text) => observation.gh_summary = Some(text),
            Err(message) => context.job.log(format!("warning: {message}")),
        }
    }
    Ok(CollectionOutput::new(build_batch(&observation)))
}

fn owner_and_name(url: &str) -> Option<(String, String)> {
    let parsed = Url::parse(url).ok()?;
    let mut segments = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty());
    let owner = segments.next()?.to_string();
    let name = segments.next()?.trim_end_matches(".git").to_string();
    Some((owner, name))
}

fn clone_destination(repos_dir: &Path, url: &str) -> PathBuf {
    let slug = match (Url::parse(url).ok(), owner_and_name(url)) {
        (Some(parsed), Some((owner, name))) => {
            let host = parsed.host_str().unwrap_or("repo");
            sanitize_filename(&format!("{host}-{owner}-{name}"))
        }
        _ => sanitize_filename(url),
    };
    repos_dir.join(slug)
}

fn relative_to(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .filter(|relative| crate::paths::is_safe_relative_path(relative))
}

async fn git_command(
    git: &Path,
    args: Vec<String>,
    cwd: Option<&Path>,
    context: &CollectorContext,
    timeout: Duration,
) -> Result<String, String> {
    let mut spec = CommandSpec::new(git.display().to_string(), args, timeout);
    spec.cwd = cwd.map(Path::to_path_buf);
    spec.env = vec![
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_ASKPASS".into(), "echo".into()),
        ("GCM_INTERACTIVE".into(), "never".into()),
    ];
    spec.max_output_bytes = MAX_OUTPUT;
    context.job.command(&spec.program, &spec.args);
    match run(&spec, &context.job.cancel, |_| {}).await {
        Ok(outcome) if outcome.success => Ok(sanitize_text(&outcome.stdout).into_owned()),
        Ok(outcome) => Err(format!(
            "git failed ({}): {}",
            outcome.exit_code.unwrap_or(-1),
            sanitize_line(outcome.stderr.lines().next().unwrap_or("no output"))
        )),
        Err(ExecError::Cancelled) => Err("cancelled".into()),
        Err(error) => Err(error.to_string()),
    }
}

async fn ls_remote(
    git: &Path,
    url: &str,
    context: &CollectorContext,
) -> Result<Vec<String>, String> {
    let output = git_command(
        git,
        vec!["ls-remote".into(), "--symref".into(), url.to_string()],
        None,
        context,
        GIT_TIMEOUT,
    )
    .await?;
    Ok(output
        .lines()
        .map(str::to_string)
        .filter(|line| !line.is_empty())
        .collect())
}

async fn clone_repo(
    git: &Path,
    url: &str,
    destination: &Path,
    depth: u32,
    context: &CollectorContext,
) -> Result<(), String> {
    if destination.join(".git").exists() {
        git_command(
            git,
            vec![
                "fetch".into(),
                "--depth".into(),
                depth.max(1).to_string(),
                "origin".into(),
            ],
            Some(destination),
            context,
            GIT_TIMEOUT,
        )
        .await?;
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {parent:?}: {error}"))?;
    }
    git_command(
        git,
        vec![
            "clone".into(),
            "--depth".into(),
            depth.max(1).to_string(),
            "--filter=blob:none".into(),
            "--no-tags".into(),
            "--single-branch".into(),
            "-c".into(),
            "credential.helper=".into(),
            url.to_string(),
            destination.display().to_string(),
        ],
        None,
        context,
        GIT_TIMEOUT,
    )
    .await?;
    Ok(())
}

async fn git_log(
    git: &Path,
    repo: &Path,
    depth: u32,
    context: &CollectorContext,
) -> Result<Vec<RepoCommit>, String> {
    let output = git_command(
        git,
        vec![
            "log".into(),
            format!("-{depth}"),
            "--format=%H%x1f%an%x1f%ae%x1f%at%x1f%s".into(),
        ],
        Some(repo),
        context,
        GIT_LOG_TIMEOUT,
    )
    .await?;
    Ok(parse_git_log(&output))
}

/// Parses `git log --format=%H%x1f%an%x1f%ae%x1f%at%x1f%s`.
pub fn parse_git_log(output: &str) -> Vec<RepoCommit> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\u{1f}').collect();
            if parts.len() < 5 {
                return None;
            }
            Some(RepoCommit {
                hash: parts[0].chars().take(40).collect(),
                author_name: sanitize_line(parts[1]).into_owned(),
                author_email: sanitize_line(parts[2]).into_owned(),
                timestamp: parts[3]
                    .parse::<i64>()
                    .ok()
                    .and_then(|seconds| DateTime::from_timestamp(seconds, 0)),
                subject: sanitize_line(parts[4]).into_owned(),
            })
        })
        .collect()
}

async fn gh_view(url: &str, context: &CollectorContext) -> Result<String, String> {
    let program = context
        .tools
        .path_of("gh")
        .unwrap_or_else(|| PathBuf::from("gh"));
    let spec = CommandSpec::new(
        program.display().to_string(),
        vec!["repo".into(), "view".into(), url.to_string()],
        GH_TIMEOUT,
    );
    context.job.command(&spec.program, &spec.args);
    match run(&spec, &context.job.cancel, |_| {}).await {
        Ok(outcome) if outcome.success => Ok(sanitize_text(&outcome.stdout).into_owned()),
        Ok(outcome) => Err(format!(
            "gh repo view failed: {}",
            sanitize_line(&outcome.stderr)
        )),
        Err(error) => Err(error.to_string()),
    }
}

/// Hostnames and URLs mentioned in commit subjects (public history only).
pub fn referenced_hosts(commits: &[RepoCommit]) -> Vec<String> {
    let mut found = BTreeSet::new();
    for commit in commits {
        for token in commit.subject.split_whitespace() {
            let candidate = token.trim_matches(|c: char| {
                !c.is_ascii_alphanumeric() && c != '.' && c != '/' && c != ':'
            });
            if let Ok(url) = Url::parse(candidate)
                && let Some(host) = url.host_str()
            {
                found.insert(host.to_ascii_lowercase());
            }
        }
        if found.len() >= MAX_REFERENCES {
            break;
        }
    }
    found.into_iter().collect()
}

/// Builds the evidence batch. Pure and unit-tested.
pub fn build_batch(observation: &RepoObservation) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let (content, fields) = describe(observation);
    let mut source = NewSource::collected(
        SourceKind::Repository,
        "repository",
        format!("Repository {}", observation.url),
    );
    source.url = Some(observation.url.clone());
    source.content_hash = Some(sha256_hex(content.as_bytes()));
    source.content = content;
    source.fields = fields;
    let key = batch.add_source(source);
    let repo = batch.entity(EntityKind::Repository, &observation.url, Some(key));
    if let Some(owner) = &observation.owner {
        let organization = EntityKey::new(EntityKind::Organization, owner);
        batch.relate(&repo, relation::OPERATED_BY, &organization, Some(key));
    }
    let mut seen_authors = BTreeSet::new();
    for commit in observation.commits.iter().take(MAX_CONTRIBUTORS * 4) {
        if seen_authors.len() >= MAX_CONTRIBUTORS {
            break;
        }
        if !commit.author_email.contains('@') || !seen_authors.insert(commit.author_email.clone()) {
            continue;
        }
        let email = EntityKey::new(EntityKind::Email, &commit.author_email);
        batch.label(&email, commit.author_name.clone());
        batch.relate(&email, relation::CONTRIBUTED_TO, &repo, Some(key));
    }
    for host in referenced_hosts(&observation.commits) {
        let host_key = EntityKey::new(EntityKind::Hostname, &host);
        batch.relate(&repo, relation::REFERENCES, &host_key, Some(key));
    }
    for commit in observation
        .commits
        .iter()
        .filter(|commit| commit.timestamp.is_some())
        .take(8)
    {
        if let Some(timestamp) = commit.timestamp {
            batch.event(
                timestamp,
                TimelineKind::Commit,
                format!(
                    "{}: {}",
                    &commit.hash[..commit.hash.len().min(7)],
                    commit.subject
                ),
                Some(key),
                Some(repo.clone()),
            );
        }
    }
    if let Some(path) = &observation.clone_path {
        batch.artifacts.push(NewArtifact {
            kind: ArtifactKind::Repository,
            name: observation
                .name
                .clone()
                .unwrap_or_else(|| observation.url.clone()),
            path: path.clone(),
            mime: None,
            size_bytes: 0,
            sha256: None,
            fields: Fields::new(),
            source: Some(key),
        });
    }
    batch.summary = format!(
        "{} commits inspected{}",
        observation.commits.len(),
        observation
            .default_branch
            .as_deref()
            .map(|branch| format!(" from {branch}"))
            .unwrap_or_default()
    );
    batch
}

fn describe(observation: &RepoObservation) -> (String, Fields) {
    let mut lines = vec![format!("Repository: {}", observation.url)];
    let mut fields = Fields::new();
    fields.push("url", observation.url.clone());
    fields.push_opt("owner", observation.owner.clone());
    fields.push_opt("name", observation.name.clone());
    if let Some(branch) = &observation.default_branch {
        lines.push(format!("HEAD: {branch}"));
        fields.push("head", branch.clone());
    }
    if !observation.remotes.is_empty() {
        lines.push(format!("Refs ({}):", observation.remotes.len()));
        lines.extend(
            observation
                .remotes
                .iter()
                .take(40)
                .map(|line| format!("  {line}")),
        );
    }
    let emails: Vec<String> = observation
        .commits
        .iter()
        .flat_map(|commit| find_emails(&commit.author_email, Vec::new()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !emails.is_empty() {
        lines.push(format!(
            "Contributor emails (from public history): {}",
            emails.join(", ")
        ));
        fields.push("contributors", emails.len().to_string());
    }
    lines.push(String::new());
    lines.push(format!("Recent commits ({}):", observation.commits.len()));
    for commit in observation.commits.iter().take(MAX_COMMITS_IN_SOURCE) {
        let date = commit
            .timestamp
            .map(|timestamp| timestamp.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "unknown".into());
        lines.push(format!(
            "  {} {} <{}> {}: {}",
            &commit.hash[..commit.hash.len().min(7)],
            commit.author_name,
            commit.author_email,
            date,
            commit.subject
        ));
    }
    if let Some(gh) = &observation.gh_summary {
        lines.push(String::new());
        lines.push("gh repo view:".into());
        lines.push(gh.clone());
    }
    (lines.join("\n"), fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_paths_include_the_host() {
        let root = Path::new("/tmp/repos");
        let github = clone_destination(root, "https://github.com/acme/project");
        let gitlab = clone_destination(root, "https://gitlab.com/acme/project.git");
        assert_ne!(github, gitlab);
        assert!(github.ends_with("github.com-acme-project"));
        assert!(gitlab.ends_with("gitlab.com-acme-project"));
    }

    #[test]
    fn parses_git_log_lines() {
        let input = "abcdef0123456789abcdef0123456789abcdef01\u{1f}Ada Lovelace\u{1f}ada@example.com\u{1f}1700000000\u{1f}Fix docs https://api.example.com/v1";
        let actual = parse_git_log(input);
        assert_eq!(actual[0].author_email, "ada@example.com");
        assert_eq!(referenced_hosts(&actual), vec!["api.example.com"]);
    }

    #[test]
    fn batch_links_contributors_and_owner() {
        let input = RepoObservation {
            url: "https://github.com/acme/project".into(),
            owner: Some("acme".into()),
            name: Some("project".into()),
            default_branch: Some("refs/heads/main".into()),
            remotes: vec!["abc HEAD".into()],
            commits: vec![RepoCommit {
                hash: "abcdef0".into(),
                author_name: "Ada".into(),
                author_email: "ada@example.com".into(),
                timestamp: DateTime::from_timestamp(1_700_000_000, 0),
                subject: "init".into(),
            }],
            gh_summary: None,
            clone_path: Some("repos/acme-project".into()),
        };
        let actual = build_batch(&input);
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "operated_by")
        );
        assert!(
            actual
                .relationships
                .iter()
                .any(|relationship| relationship.kind == "contributed_to")
        );
        assert_eq!(actual.artifacts[0].kind, ArtifactKind::Repository);
        assert_eq!(actual.timeline.len(), 1);
    }
}
