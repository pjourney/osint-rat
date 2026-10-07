//! Running a registered external tool against a target and turning its output into evidence.

use std::path::PathBuf;
use std::sync::LazyLock;

use regex::Regex;

use crate::exec::{CommandSpec, ExecError, run};
use crate::files::sha256_hex;
use crate::jobs::JobContext;
use crate::model::{EntityKey, EntityKind, EvidenceBatch, NewSource, SourceKind, Target, relation};
use crate::sanitize::{sanitize_line, sanitize_text};
use crate::tools::template::render_args;
use crate::tools::{OutputParser, ToolDefinition};

use super::CollectionOutput;

static HOSTNAME_PATTERN: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,24}\b").ok()
});
static IPV4_PATTERN: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"\b(?:(?:25[0-5]|2[0-4]\d|[01]?\d\d?)\.){3}(?:25[0-5]|2[0-4]\d|[01]?\d\d?)\b").ok()
});
static URL_PATTERN: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s"'<>]+"#).ok());

/// Executes `definition` against `target` and returns a collection output.
pub async fn run_tool(
    definition: &ToolDefinition,
    program: PathBuf,
    target: &Target,
    job: &JobContext,
) -> Result<CollectionOutput, String> {
    let args = render_args(&definition.args, target).map_err(|error| error.to_string())?;
    let mut spec = CommandSpec::new(program.display().to_string(), args, definition.timeout);
    spec.max_output_bytes = crate::exec::DEFAULT_MAX_OUTPUT_BYTES;
    job.command(&spec.program, &spec.args);
    let outcome = match run(&spec, &job.cancel, |line| {
        job.log(sanitize_line(&line.text).into_owned());
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(ExecError::Cancelled) => return Err("cancelled".into()),
        Err(error) => return Err(error.to_string()),
    };
    if !outcome.success && outcome.stdout.trim().is_empty() {
        return Err(format!(
            "{} exited {}: {}",
            definition.name,
            outcome.exit_code.unwrap_or(-1),
            sanitize_line(outcome.stderr.lines().next().unwrap_or("no output"))
        ));
    }
    let stdout = sanitize_text(&outcome.stdout).into_owned();
    let stderr = sanitize_text(&outcome.stderr).into_owned();
    Ok(CollectionOutput::new(build_batch(
        definition,
        target,
        &spec.program,
        &spec.args,
        &stdout,
        &stderr,
        outcome.exit_code,
    )))
}

/// Builds the evidence batch from tool output. Pure and unit-tested.
pub fn build_batch(
    definition: &ToolDefinition,
    target: &Target,
    program: &str,
    args: &[String],
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
) -> EvidenceBatch {
    let mut batch = EvidenceBatch::new();
    let mut source = NewSource::collected(
        SourceKind::Command,
        &format!("tool:{}", definition.name),
        format!("{} on {}", definition.name, target.value()),
    );
    let mut content = format!(
        "Program: {program}\nArguments: {}\nExit: {}\n\n--- stdout ---\n{stdout}",
        args.join(" "),
        exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "?".into())
    );
    if !stderr.trim().is_empty() {
        content.push_str("\n\n--- stderr ---\n");
        content.push_str(stderr);
    }
    source.content_hash = Some(sha256_hex(content.as_bytes()));
    source.content = content;
    source.fields.push("program", program);
    source.fields.push("tool", definition.name.clone());
    source.fields.push("activity", definition.activity.as_str());
    source.fields.push(
        "parser",
        format!("{:?}", definition.parser).to_ascii_lowercase(),
    );
    let key = batch.add_source(source);
    let subject = batch.entity(target.entity_kind(), &target.value(), Some(key));
    let extracted = extract_values(definition.parser, stdout);
    for (kind, value) in extracted {
        let discovered = EntityKey::new(kind, &value);
        if discovered == subject {
            continue;
        }
        batch.relate(&subject, relation::MENTIONS, &discovered, Some(key));
    }
    batch.summary = format!(
        "{} produced {} bytes of output",
        definition.name,
        stdout.len()
    );
    batch
}

/// Extracts identifiers from tool output according to the configured parser.
pub fn extract_values(parser: OutputParser, stdout: &str) -> Vec<(EntityKind, String)> {
    match parser {
        OutputParser::Hostnames => find_all(&HOSTNAME_PATTERN, stdout, EntityKind::Hostname),
        OutputParser::Ips => find_all(&IPV4_PATTERN, stdout, EntityKind::Ip),
        OutputParser::Urls => find_all(&URL_PATTERN, stdout, EntityKind::Url),
        OutputParser::Lines => stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('-'))
            .filter_map(classify_line)
            .collect(),
        OutputParser::Json | OutputParser::Text => Vec::new(),
    }
}

fn find_all(
    pattern: &LazyLock<Option<Regex>>,
    text: &str,
    kind: EntityKind,
) -> Vec<(EntityKind, String)> {
    let Some(regex) = pattern.as_ref() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for matched in regex.find_iter(text) {
        let value = matched.as_str();
        if !found.iter().any(|(_, existing)| existing == value) {
            found.push((kind.clone(), value.to_string()));
        }
        if found.len() >= 80 {
            break;
        }
    }
    found
}

fn classify_line(line: &str) -> Option<(EntityKind, String)> {
    if line.parse::<std::net::IpAddr>().is_ok() {
        return Some((EntityKind::Ip, line.to_string()));
    }
    if line.contains("://") {
        return Some((EntityKind::Url, line.to_string()));
    }
    if line.contains('.') && !line.contains(' ') {
        return Some((EntityKind::Hostname, line.to_string()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TargetKind;
    use crate::tools::Activity;

    fn whois_tool() -> ToolDefinition {
        ToolDefinition {
            name: "whois".into(),
            program: "whois".into(),
            args: vec!["{domain}".into()],
            accepts: vec![TargetKind::Domain],
            description: "WHOIS".into(),
            timeout: std::time::Duration::from_secs(30),
            parser: OutputParser::Hostnames,
            activity: Activity::Passive,
            builtin: true,
        }
    }

    #[test]
    fn extracts_hostnames_from_whois_style_output() {
        let input = "nserver: a.iana-servers.net.\nwhois: whois.iana.org\n";
        let actual = extract_values(OutputParser::Hostnames, input);
        assert!(
            actual
                .iter()
                .any(|(_, value)| value == "a.iana-servers.net")
        );
        assert!(actual.iter().any(|(_, value)| value == "whois.iana.org"));
    }

    #[test]
    fn lines_parser_classifies_each_line() {
        let input = "192.0.2.10\ncdn.example.net\nhttps://example.com/a\n";
        let actual = extract_values(OutputParser::Lines, input);
        assert_eq!(actual.len(), 3);
        assert_eq!(actual[0].0, EntityKind::Ip);
        assert_eq!(actual[1].0, EntityKind::Hostname);
        assert_eq!(actual[2].0, EntityKind::Url);
    }

    #[test]
    fn batch_records_command_and_mentions() {
        let target = Target::Domain("example.com".into());
        let tool = whois_tool();
        let actual = build_batch(
            &tool,
            &target,
            "whois",
            &["example.com".into()],
            "nserver: ns1.example.net\n",
            "",
            Some(0),
        );
        assert_eq!(actual.sources[0].kind, SourceKind::Command);
        assert!(actual.sources[0].content.contains("Program: whois"));
        assert!(actual.relationships.iter().any(|relationship| {
            relationship.kind == "mentions" && relationship.to.value == "ns1.example.net"
        }));
    }
}
