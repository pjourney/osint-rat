//! The external tool registry: built-in definitions for common OSINT utilities, user-defined
//! tools from configuration, and detection of what is installed on `PATH`.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::template::validate_template;
use super::which::find_on_path;
use crate::config::{ToolDefinitionConfig, ToolsConfig, duration_from_secs};
use crate::model::TargetKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    /// Ordinary lookups and fetches of public information.
    Passive,
    /// Probe traffic. os-int refuses to run tools marked active.
    Active,
}

impl Activity {
    pub fn parse(value: &str) -> Activity {
        if value.trim().eq_ignore_ascii_case("active") {
            Activity::Active
        } else {
            Activity::Passive
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Activity::Passive => "passive",
            Activity::Active => "active",
        }
    }
}

/// How tool output is interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputParser {
    Text,
    Json,
    Lines,
    Hostnames,
    Ips,
    Urls,
}

impl OutputParser {
    pub fn parse(value: &str) -> OutputParser {
        match value.trim().to_ascii_lowercase().as_str() {
            "json" => OutputParser::Json,
            "lines" => OutputParser::Lines,
            "hostnames" | "domains" | "subdomains" => OutputParser::Hostnames,
            "ips" | "addresses" => OutputParser::Ips,
            "urls" => OutputParser::Urls,
            _ => OutputParser::Text,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub accepts: Vec<TargetKind>,
    pub description: String,
    pub timeout: Duration,
    pub parser: OutputParser,
    pub activity: Activity,
    pub builtin: bool,
}

impl ToolDefinition {
    pub fn accepts(&self, kind: TargetKind) -> bool {
        self.accepts.contains(&kind)
    }
}

/// A tool definition plus where (if anywhere) its program was found.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolEntry {
    pub definition: ToolDefinition,
    pub path: Option<PathBuf>,
    /// Configuration problems that prevent the tool from running.
    pub problem: Option<String>,
}

impl ToolEntry {
    pub fn is_available(&self) -> bool {
        self.path.is_some() && self.problem.is_none()
    }
}

/// A program that os-int can use internally (git, gh, exiftool) or that is merely useful to know
/// about (jq, rg).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedProgram {
    pub name: &'static str,
    pub purpose: &'static str,
    pub path: Option<PathBuf>,
}

const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

/// Programs shown in the Tools screen and `os-int doctor` even when no tool definition uses them.
const UTILITIES: [(&str, &str); 15] = [
    ("whois", "registration lookups"),
    ("dig", "DNS queries"),
    ("host", "DNS lookups"),
    ("nslookup", "DNS lookups"),
    ("curl", "HTTP requests"),
    ("wget", "HTTP requests"),
    ("jq", "JSON processing"),
    ("rg", "text search"),
    ("fd", "file search"),
    ("git", "repository collector"),
    ("gh", "GitHub metadata for repositories"),
    ("exiftool", "file metadata"),
    ("openssl", "TLS inspection"),
    ("subfinder", "passive subdomain discovery"),
    ("amass", "passive subdomain enumeration"),
];

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn builtin_with_parser(
    name: &str,
    program: &str,
    args: &[&str],
    accepts: &[TargetKind],
    description: &str,
    activity: Activity,
    parser: OutputParser,
) -> ToolDefinition {
    let mut tool = builtin(name, program, args, accepts, description, activity);
    tool.parser = parser;
    tool
}

fn builtin(
    name: &str,
    program: &str,
    args: &[&str],
    accepts: &[TargetKind],
    description: &str,
    activity: Activity,
) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        program: program.to_string(),
        args: strings(args),
        accepts: accepts.to_vec(),
        description: description.to_string(),
        timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
        parser: OutputParser::Text,
        activity,
        builtin: true,
    }
}

/// Built-in tool definitions. They are only offered when their program is installed.
pub fn builtin_tools() -> Vec<ToolDefinition> {
    use TargetKind::{Asn, Domain, File, Ip, Repository, Url};
    let mut tools = vec![
        builtin(
            "whois",
            "whois",
            &["{target}"],
            &[Domain, Ip, Asn],
            "WHOIS registration lookup",
            Activity::Passive,
        ),
        builtin(
            "dig",
            "dig",
            &[
                "+noall", "+answer", "{domain}", "A", "{domain}", "AAAA", "{domain}", "MX",
                "{domain}", "NS", "{domain}", "TXT", "{domain}", "CAA",
            ],
            &[Domain],
            "Common DNS records via dig",
            Activity::Passive,
        ),
        builtin(
            "dig-ptr",
            "dig",
            &["+noall", "+answer", "-x", "{ip}"],
            &[Ip],
            "Reverse DNS via dig",
            Activity::Passive,
        ),
        builtin(
            "host",
            "host",
            &["{target}"],
            &[Domain, Ip],
            "DNS lookup via host",
            Activity::Passive,
        ),
        builtin(
            "nslookup",
            "nslookup",
            &["{target}"],
            &[Domain, Ip],
            "DNS lookup via nslookup",
            Activity::Passive,
        ),
        builtin(
            "curl-headers",
            "curl",
            &[
                "-sS",
                "-I",
                "-L",
                "--max-redirs",
                "5",
                "--max-time",
                "20",
                "{url}",
            ],
            &[Domain, Url],
            "HTTP response headers via curl",
            Activity::Passive,
        ),
        builtin(
            "openssl-cert",
            "openssl",
            &[
                "s_client",
                "-connect",
                "{host}:443",
                "-servername",
                "{host}",
                "-showcerts",
            ],
            &[Domain, Url],
            "TLS handshake and certificate chain via openssl",
            Activity::Passive,
        ),
        builtin(
            "exiftool",
            "exiftool",
            &["-json", "{file}"],
            &[File],
            "File metadata via ExifTool",
            Activity::Passive,
        ),
        builtin(
            "git-ls-remote",
            "git",
            &["ls-remote", "--heads", "--tags", "{repo}"],
            &[Repository],
            "Branches and tags of a public repository",
            Activity::Passive,
        ),
        builtin(
            "gh-repo",
            "gh",
            &["repo", "view", "{repo}"],
            &[Repository],
            "GitHub repository summary via gh",
            Activity::Passive,
        ),
        builtin_with_parser(
            "subfinder",
            "subfinder",
            &["-silent", "-d", "{domain}"],
            &[Domain],
            "Passive subdomain discovery via subfinder",
            Activity::Passive,
            OutputParser::Hostnames,
        ),
        builtin_with_parser(
            "amass",
            "amass",
            &["enum", "-passive", "-silent", "-d", "{domain}"],
            &[Domain],
            "Passive subdomain enumeration via amass",
            Activity::Passive,
            OutputParser::Hostnames,
        ),
    ];
    for tool in &mut tools {
        if tool.name == "exiftool" {
            tool.parser = OutputParser::Json;
        }
    }
    tools
}

/// Converts a `[[tools.custom]]` entry into a definition, reporting configuration problems.
pub fn custom_tool(config: &ToolDefinitionConfig) -> (ToolDefinition, Option<String>) {
    let accepts: Vec<TargetKind> = config
        .accepts
        .iter()
        .filter_map(|kind| TargetKind::parse(kind))
        .collect();
    let mut problem = None;
    if config.name.trim().is_empty() || config.program.trim().is_empty() {
        problem = Some("missing `name` or `program`".to_string());
    } else if let Err(error) = validate_template(&config.args) {
        problem = Some(error.to_string());
    } else if accepts.is_empty() {
        problem = Some("`accepts` lists no known target kinds".to_string());
    } else if config.activity.trim().eq_ignore_ascii_case("active") {
        problem = Some(
            "active tools are refused; os-int only runs passive observation and diagnostic programs"
                .to_string(),
        );
    }
    let definition = ToolDefinition {
        name: config.name.trim().to_string(),
        program: config.program.trim().to_string(),
        args: config.args.clone(),
        accepts,
        description: config.description.clone(),
        timeout: duration_from_secs(config.timeout_seconds.max(1)),
        parser: OutputParser::parse(&config.parser),
        activity: Activity::parse(&config.activity),
        builtin: false,
    };
    (definition, problem)
}

#[derive(Clone, Debug, Default)]
pub struct ToolRegistry {
    tools: Vec<ToolEntry>,
    utilities: Vec<DetectedProgram>,
}

impl ToolRegistry {
    /// Builds the registry and probes `PATH` for each program.
    pub fn detect(config: &ToolsConfig) -> ToolRegistry {
        let mut tools: Vec<ToolEntry> = builtin_tools()
            .into_iter()
            .filter(|tool| !config.disabled.iter().any(|name| name == &tool.name))
            .map(|definition| ToolEntry {
                path: find_on_path(&definition.program),
                definition,
                problem: None,
            })
            .collect();
        for custom in &config.custom {
            let (definition, problem) = custom_tool(custom);
            tools.retain(|existing| existing.definition.name != definition.name);
            tools.push(ToolEntry {
                path: find_on_path(&definition.program),
                definition,
                problem,
            });
        }
        let utilities = UTILITIES
            .iter()
            .map(|(name, purpose)| DetectedProgram {
                name,
                purpose,
                path: find_on_path(name),
            })
            .collect();
        ToolRegistry { tools, utilities }
    }

    pub fn tools(&self) -> &[ToolEntry] {
        &self.tools
    }

    pub fn utilities(&self) -> &[DetectedProgram] {
        &self.utilities
    }

    pub fn find(&self, name: &str) -> Option<&ToolEntry> {
        self.tools
            .iter()
            .find(|entry| entry.definition.name.eq_ignore_ascii_case(name))
    }

    /// Installed tools that accept the given target kind.
    pub fn available_for(&self, kind: TargetKind) -> Vec<&ToolEntry> {
        self.tools
            .iter()
            .filter(|entry| entry.is_available() && entry.definition.accepts(kind))
            .collect()
    }

    pub fn is_installed(&self, program: &str) -> bool {
        self.utilities
            .iter()
            .find(|utility| utility.name == program)
            .map(|utility| utility.path.is_some())
            .unwrap_or_else(|| find_on_path(program).is_some())
    }

    pub fn path_of(&self, program: &str) -> Option<PathBuf> {
        self.utilities
            .iter()
            .find(|utility| utility.name == program)
            .and_then(|utility| utility.path.clone())
            .or_else(|| find_on_path(program))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_templates_are_valid() {
        for tool in builtin_tools() {
            assert!(validate_template(&tool.args).is_ok(), "tool {}", tool.name);
            assert!(!tool.accepts.is_empty(), "tool {}", tool.name);
        }
    }

    #[test]
    fn builtin_tools_are_passive() {
        assert!(
            builtin_tools()
                .iter()
                .all(|tool| tool.activity == Activity::Passive)
        );
        assert!(builtin_tools().iter().all(|tool| tool.name != "traceroute"));
    }

    #[test]
    fn active_custom_tools_are_refused() {
        let input = ToolDefinitionConfig {
            name: "probe".into(),
            program: "probe".into(),
            args: vec!["{domain}".into()],
            accepts: vec!["domain".into()],
            activity: "active".into(),
            ..ToolDefinitionConfig::default()
        };
        let (definition, problem) = custom_tool(&input);
        assert_eq!(definition.activity, Activity::Active);
        assert!(problem.unwrap_or_default().contains("refused"));
        let registry = ToolRegistry::detect(&ToolsConfig {
            custom: vec![input],
            ..ToolsConfig::default()
        });
        let entry = registry.find("probe").expect("present");
        assert!(!entry.is_available());
    }

    #[test]
    fn custom_tools_are_validated() {
        let input = ToolDefinitionConfig {
            name: "bad".into(),
            program: "bad".into(),
            args: vec!["{unknown}".into()],
            accepts: vec!["domain".into()],
            ..ToolDefinitionConfig::default()
        };
        let (_, problem) = custom_tool(&input);
        assert!(problem.is_some());
    }

    #[test]
    fn custom_tools_override_builtins_and_disabled_are_hidden() {
        let input = ToolsConfig {
            disabled: vec!["dig".into()],
            custom: vec![ToolDefinitionConfig {
                name: "whois".into(),
                program: "my-whois".into(),
                args: vec!["{domain}".into()],
                accepts: vec!["domain".into()],
                ..ToolDefinitionConfig::default()
            }],
        };
        let registry = ToolRegistry::detect(&input);
        let whois = registry.find("whois").expect("present");
        assert_eq!(whois.definition.program, "my-whois");
        assert!(!whois.definition.builtin);
        assert!(registry.find("dig").is_none());
    }
}
