//! User configuration (`config.toml`).
//!
//! A commented default file is generated on first launch. Every section and field is optional;
//! missing values fall back to the defaults defined here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, WrapErr};
use serde::{Deserialize, Serialize};

pub const DEFAULT_USER_AGENT: &str = concat!(
    "os-int/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/pjourney/osint-rat)"
);
const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 20;
const DEFAULT_CONNECT_TIMEOUT_SECONDS: u64 = 10;
const DEFAULT_MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
const DEFAULT_MAX_REDIRECTS: usize = 10;
const DEFAULT_AGENT_TIMEOUT_SECONDS: u64 = 600;
const DEFAULT_MAX_CONTEXT_BYTES: usize = 200 * 1024;
const DEFAULT_HISTORY_TURNS: usize = 4;
const DEFAULT_TICK_RATE_MS: u64 = 250;
const DEFAULT_MAX_CONCURRENT_JOBS: usize = 4;
const DEFAULT_TOOL_TIMEOUT_SECONDS: u64 = 60;
const DEFAULT_MAX_FILE_IMPORT_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_REPOSITORY_HISTORY_DEPTH: u32 = 200;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Theme, redraw interval, and the external editor.
    pub ui: UiConfig,
    /// Which agent profile is active, plus timeouts and context limits.
    pub agent: AgentSettings,
    /// Named agent profiles. Built-in profiles (`claude`, `codex`) can be overridden here.
    pub agents: BTreeMap<String, AgentProfileConfig>,
    /// HTTP client settings. DNS and TLS collectors do not use `proxy`.
    pub network: NetworkConfig,
    /// Data directory override and the largest evidence file that will be copied in.
    pub storage: StorageConfig,
    /// What the built-in collectors fetch, and how many jobs may run at once.
    pub collectors: CollectorConfig,
    /// Built-in tools to hide, and passive programs you add yourself.
    pub tools: ToolsConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// `default`, `high-contrast`, or `monochrome`.
    pub theme: String,
    /// Redraw interval while background jobs are running.
    pub tick_rate_ms: u64,
    /// Command used for long-form editing (falls back to `$VISUAL`, `$EDITOR`, then `vi`).
    pub editor: Option<String>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "default".into(),
            tick_rate_ms: DEFAULT_TICK_RATE_MS,
            editor: None,
        }
    }
}

/// Agent selection plus an optional inline profile (`program`, `args`, ...).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSettings {
    /// The profile id to use (`claude`, `codex`, or a key under `[agents]`).
    pub active: String,
    /// Inline profile: when `program` is set it takes precedence over `active`.
    pub name: Option<String>,
    pub program: Option<String>,
    pub args: Option<Vec<String>>,
    pub adapter: Option<String>,
    pub input: Option<String>,
    pub working_directory: Option<String>,
    pub timeout_seconds: u64,
    /// Upper bound on the context pack size sent to the agent.
    pub max_context_bytes: usize,
    /// Number of earlier turns in the same thread to include for follow-up questions.
    pub history_turns: usize,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            active: "claude".into(),
            name: None,
            program: None,
            args: None,
            adapter: None,
            input: None,
            working_directory: None,
            timeout_seconds: DEFAULT_AGENT_TIMEOUT_SECONDS,
            max_context_bytes: DEFAULT_MAX_CONTEXT_BYTES,
            history_turns: DEFAULT_HISTORY_TURNS,
        }
    }
}

/// A user-defined or overridden agent profile.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentProfileConfig {
    pub name: Option<String>,
    pub program: Option<String>,
    pub args: Option<Vec<String>>,
    /// Output parser: `claude` (stream-json), `codex` (exec --json), or `text`.
    pub adapter: Option<String>,
    /// How the prompt is delivered: `stdin`, `argument`, or `file`.
    pub input: Option<String>,
    /// `investigation` (default), `current`, or an absolute path.
    pub working_directory: Option<String>,
    pub timeout_seconds: Option<u64>,
    /// Extra environment variables for the agent process.
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    pub user_agent: String,
    pub request_timeout_seconds: u64,
    pub connect_timeout_seconds: u64,
    /// Maximum bytes retained from any single HTTP response body.
    pub max_body_bytes: usize,
    pub max_redirects: usize,
    /// Optional proxy for HTTP(S) requests, e.g. `socks5h://127.0.0.1:9050`.
    /// DNS and TLS collectors connect directly and do not use this proxy.
    pub proxy: Option<String>,
    /// Allow redirects from a public site to private/loopback addresses (off by default).
    pub allow_private_redirects: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            user_agent: DEFAULT_USER_AGENT.into(),
            request_timeout_seconds: DEFAULT_REQUEST_TIMEOUT_SECONDS,
            connect_timeout_seconds: DEFAULT_CONNECT_TIMEOUT_SECONDS,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            max_redirects: DEFAULT_MAX_REDIRECTS,
            proxy: None,
            allow_private_redirects: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Overrides the platform data directory.
    pub data_dir: Option<PathBuf>,
    /// Largest evidence file that will be copied into an investigation.
    pub max_file_import_bytes: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: None,
            max_file_import_bytes: DEFAULT_MAX_FILE_IMPORT_BYTES,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectorConfig {
    /// Fetch `/robots.txt` and discover sitemaps during HTTP collection.
    pub fetch_robots: bool,
    /// Also resolve `www.<domain>` during DNS collection.
    pub check_www: bool,
    pub max_concurrent_jobs: usize,
    /// Number of commits inspected when analysing a repository.
    pub repository_history_depth: u32,
}

impl Default for CollectorConfig {
    fn default() -> Self {
        Self {
            fetch_robots: true,
            check_www: true,
            max_concurrent_jobs: DEFAULT_MAX_CONCURRENT_JOBS,
            repository_history_depth: DEFAULT_REPOSITORY_HISTORY_DEPTH,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    /// Built-in tool names to hide (for example `["dig"]`).
    pub disabled: Vec<String>,
    pub custom: Vec<ToolDefinitionConfig>,
}

/// A user-defined external tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolDefinitionConfig {
    pub name: String,
    pub program: String,
    /// Argument template. Placeholders: `{target}`, `{domain}`, `{host}`, `{ip}`, `{url}`,
    /// `{email}`, `{username}`, `{repo}`, `{file}`, `{asn}`.
    pub args: Vec<String>,
    /// Target kinds the tool accepts (`domain`, `ip`, `url`, `email`, `repository`, `file`, ...).
    pub accepts: Vec<String>,
    pub description: String,
    pub timeout_seconds: u64,
    /// `text`, `json`, `lines`, `hostnames`, `ips`, or `urls`.
    pub parser: String,
    /// Must be `passive`. Tools marked `active` are refused.
    pub activity: String,
}

impl Default for ToolDefinitionConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            program: String::new(),
            args: Vec::new(),
            accepts: Vec::new(),
            description: String::new(),
            timeout_seconds: DEFAULT_TOOL_TIMEOUT_SECONDS,
            parser: "text".into(),
            activity: "passive".into(),
        }
    }
}

/// The commented configuration file written on first launch.
pub const DEFAULT_CONFIG_TOML: &str = r#"# os-int configuration
# Every setting is optional; delete a line to fall back to the built-in default.
# Full reference: https://github.com/pjourney/osint-rat/blob/main/docs/guide.md#configuration

[ui]
# "default", "high-contrast", or "monochrome"
theme = "default"
# Redraw interval (ms) while background jobs are running. The UI is otherwise event-driven.
tick_rate_ms = 250
# Editor for long-form notes (falls back to $VISUAL, $EDITOR, then vi).
# editor = "nvim"

[agent]
# Active agent profile. Built-in: "claude" (Claude Code) and "codex" (OpenAI Codex CLI).
# Define more under [agents.<id>] below.
active = "claude"
timeout_seconds = 600
# Maximum size of the investigation context sent with each question.
max_context_bytes = 204800
# Earlier turns of the same conversation included with follow-up questions.
history_turns = 4

# Inline profile (overrides `active` when `program` is set). Placeholders are substituted
# inside individual arguments; nothing is ever passed through a shell.
#   {prompt}       full prompt text          {prompt_file}  path to a file with the prompt
#   {instructions} os-int system instructions {question}    the analyst's question only
#   {workdir}      agent working directory
# name = "Claude Code"
# program = "claude"
# args = ["-p", "{prompt}"]
# working_directory = "investigation"

# Example: a generic agent that takes the prompt as an argument.
# [agents.opencode]
# name = "opencode"
# program = "opencode"
# args = ["run", "{prompt}"]
# adapter = "text"
# input = "argument"

[network]
# Defaults to "os-int/<version> (+https://github.com/pjourney/osint-rat)".
# user_agent = "os-int"
request_timeout_seconds = 20
connect_timeout_seconds = 10
# Maximum bytes kept from a single HTTP response body.
max_body_bytes = 5242880
max_redirects = 10
# Proxy for HTTP(S) requests (DNS and TLS collectors connect directly).
# proxy = "socks5h://127.0.0.1:9050"
allow_private_redirects = false

[storage]
# Platform default unless overridden:
# data_dir = "~/osint-data"
max_file_import_bytes = 536870912

[collectors]
fetch_robots = true
check_www = true
max_concurrent_jobs = 4
repository_history_depth = 200

[tools]
# Built-in tools to hide:
# disabled = ["dig"]

# Custom external tools. Arguments are passed directly to the program (no shell).
# activity must be "passive". Active probing tools are refused.
# [[tools.custom]]
# name = "subfinder"
# program = "subfinder"
# args = ["-silent", "-d", "{domain}"]
# accepts = ["domain"]
# description = "Passive subdomain discovery"
# timeout_seconds = 120
# parser = "hostnames"
# activity = "passive"
"#;

/// The result of loading configuration.
#[derive(Debug)]
pub struct LoadedConfig {
    pub config: Config,
    pub path: PathBuf,
    /// `true` when the file did not exist and defaults were written.
    pub created: bool,
}

impl Config {
    /// Loads the config file, creating a commented default file when it does not exist.
    pub fn load_or_create(path: &Path) -> Result<LoadedConfig> {
        if !path.exists() {
            write_default_config(path)?;
            return Ok(LoadedConfig {
                config: Config::default(),
                path: path.to_path_buf(),
                created: true,
            });
        }
        let config = Config::load(path)?;
        Ok(LoadedConfig {
            config,
            path: path.to_path_buf(),
            created: false,
        })
    }

    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("could not read config file {}", path.display()))?;
        Config::parse(&text).wrap_err_with(|| format!("invalid config file {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Config> {
        let config: Config = toml::from_str(text)?;
        Ok(config)
    }

    /// Writes the configuration. Comments in an existing file are not preserved.
    pub fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self).wrap_err("could not serialize configuration")?;
        let text = format!(
            "# os-int configuration (saved from the Settings screen)\n# See docs/guide.md#configuration for all options.\n\n{body}"
        );
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .wrap_err_with(|| format!("could not create {}", parent.display()))?;
        }
        std::fs::write(path, text).wrap_err_with(|| format!("could not write {}", path.display()))
    }

    /// Non-fatal problems worth showing to the user.
    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if !matches!(
            self.ui.theme.as_str(),
            "default" | "high-contrast" | "monochrome"
        ) {
            warnings.push(format!(
                "unknown theme {:?}; using \"default\"",
                self.ui.theme
            ));
        }
        if self.network.request_timeout_seconds == 0 {
            warnings
                .push("network.request_timeout_seconds is 0; requests would never time out".into());
        }
        if self.agent.timeout_seconds == 0 {
            warnings.push("agent.timeout_seconds is 0; agent calls would never time out".into());
        }
        for tool in &self.tools.custom {
            if tool.name.trim().is_empty() || tool.program.trim().is_empty() {
                warnings.push("a [[tools.custom]] entry is missing `name` or `program`".into());
            }
        }
        warnings
    }
}

fn write_default_config(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("could not create config directory {}", parent.display()))?;
    }
    std::fs::write(path, DEFAULT_CONFIG_TOML)
        .wrap_err_with(|| format!("could not write default config to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_file_matches_built_in_defaults() {
        let actual = Config::parse(DEFAULT_CONFIG_TOML).expect("default config parses");
        let expected = Config::default();
        assert_eq!(actual, expected);
    }

    #[test]
    fn partial_config_uses_defaults() {
        let input = "[agent]\nprogram = \"claude\"\n\n[network]\nrequest_timeout_seconds = 5\n";
        let actual = Config::parse(input).expect("parses");
        assert_eq!(actual.agent.program.as_deref(), Some("claude"));
        assert_eq!(actual.network.request_timeout_seconds, 5);
        assert_eq!(actual.network.max_redirects, DEFAULT_MAX_REDIRECTS);
        assert_eq!(actual.ui.theme, "default");
    }

    #[test]
    fn prompt_style_inline_agent_parses() {
        let input = r#"
[agent]
name = "Claude Code"
program = "claude"
args = ["-p", "{prompt}"]
working_directory = "investigation"
"#;
        let actual = Config::parse(input).expect("parses");
        assert_eq!(actual.agent.name.as_deref(), Some("Claude Code"));
        assert_eq!(
            actual.agent.args,
            Some(vec!["-p".to_string(), "{prompt}".to_string()])
        );
    }

    #[test]
    fn custom_tools_parse() {
        let input = r#"
[[tools.custom]]
name = "subfinder"
program = "subfinder"
args = ["-silent", "-d", "{domain}"]
accepts = ["domain"]
parser = "hostnames"
"#;
        let actual = Config::parse(input).expect("parses");
        let tool = &actual.tools.custom[0];
        assert_eq!(tool.name, "subfinder");
        assert_eq!(tool.timeout_seconds, DEFAULT_TOOL_TIMEOUT_SECONDS);
        assert_eq!(tool.activity, "passive");
    }

    #[test]
    fn invalid_config_reports_error() {
        let input = "[network]\nrequest_timeout_seconds = \"soon\"\n";
        assert!(Config::parse(input).is_err());
    }

    #[test]
    fn save_and_load_round_trip() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("nested/config.toml");
        let mut input = Config::default();
        input.agent.active = "codex".into();
        input.save(&path).expect("saves");
        let actual = Config::load(&path).expect("loads");
        assert_eq!(actual, input);
    }

    #[test]
    fn load_or_create_writes_defaults() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("config.toml");
        let loaded = Config::load_or_create(&path).expect("creates");
        assert!(loaded.created);
        assert!(path.exists());
        assert_eq!(loaded.config, Config::default());
    }

    #[test]
    fn warnings_flag_bad_values() {
        let mut input = Config::default();
        input.ui.theme = "neon".into();
        input.network.request_timeout_seconds = 0;
        assert_eq!(input.warnings().len(), 2);
    }
}
