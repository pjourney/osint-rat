//! Configurable agent command profiles. Built-in adapters for Claude Code and Codex; anything
//! else is a generic subprocess.

use std::path::PathBuf;
use std::time::Duration;

use crate::config::{AgentProfileConfig, AgentSettings, Config};

/// How the agent CLI reports output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentAdapter {
    /// Claude Code `--output-format stream-json`.
    Claude,
    /// Codex `exec --json`.
    Codex,
    /// Plain text on stdout.
    Text,
}

impl AgentAdapter {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" | "stream-json" => AgentAdapter::Claude,
            "codex" | "openai" => AgentAdapter::Codex,
            _ => AgentAdapter::Text,
        }
    }
}

/// How the prompt is delivered to the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentInput {
    Stdin,
    Argument,
    File,
}

impl AgentInput {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "argument" | "args" => AgentInput::Argument,
            "file" => AgentInput::File,
            _ => AgentInput::Stdin,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentWorkingDirectory {
    Investigation,
    Current,
}

/// A fully resolved agent command.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub adapter: AgentAdapter,
    pub input: AgentInput,
    pub working_directory: AgentWorkingDirectory,
    pub timeout: Duration,
    pub env: Vec<(String, String)>,
}

impl AgentProfile {
    pub fn claude() -> Self {
        Self {
            id: "claude".into(),
            name: "Claude Code".into(),
            program: "claude".into(),
            args: vec![
                "-p".into(),
                "--output-format".into(),
                "stream-json".into(),
                "--verbose".into(),
                "--include-partial-messages".into(),
                "--tools".into(),
                String::new(),
                "--no-session-persistence".into(),
                "--system-prompt".into(),
                "{instructions}".into(),
            ],
            adapter: AgentAdapter::Claude,
            input: AgentInput::Stdin,
            working_directory: AgentWorkingDirectory::Investigation,
            timeout: Duration::from_secs(600),
            env: Vec::new(),
        }
    }

    pub fn codex() -> Self {
        Self {
            id: "codex".into(),
            name: "Codex".into(),
            program: "codex".into(),
            args: vec![
                "exec".into(),
                "--json".into(),
                "--skip-git-repo-check".into(),
                "--sandbox".into(),
                "read-only".into(),
                "--ephemeral".into(),
                "--color".into(),
                "never".into(),
                "-".into(),
            ],
            adapter: AgentAdapter::Codex,
            input: AgentInput::Stdin,
            working_directory: AgentWorkingDirectory::Investigation,
            timeout: Duration::from_secs(600),
            env: Vec::new(),
        }
    }

    pub fn generic(program: &str) -> Self {
        Self {
            id: program.to_string(),
            name: program.to_string(),
            program: program.to_string(),
            args: vec!["-p".into(), "{prompt}".into()],
            adapter: AgentAdapter::Text,
            input: AgentInput::Argument,
            working_directory: AgentWorkingDirectory::Investigation,
            timeout: Duration::from_secs(600),
            env: Vec::new(),
        }
    }

    /// Applies a config overlay onto this profile.
    pub fn overlay(&mut self, overlay: &AgentProfileConfig) {
        if let Some(name) = &overlay.name {
            self.name = name.clone();
        }
        if let Some(program) = &overlay.program {
            self.program = program.clone();
        }
        if let Some(args) = &overlay.args {
            self.args = args.clone();
        }
        if let Some(adapter) = &overlay.adapter {
            self.adapter = AgentAdapter::parse(adapter);
        }
        if let Some(input) = &overlay.input {
            self.input = AgentInput::parse(input);
        }
        if let Some(working_directory) = &overlay.working_directory {
            self.working_directory = match working_directory.as_str() {
                "current" => AgentWorkingDirectory::Current,
                _ => AgentWorkingDirectory::Investigation,
            };
        }
        if let Some(timeout_seconds) = overlay.timeout_seconds {
            self.timeout = Duration::from_secs(timeout_seconds.max(1));
        }
        self.env.extend(
            overlay
                .env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }

    /// Substitutes placeholders in each argument. Arguments are never re-split.
    pub fn render_args(
        &self,
        prompt: &str,
        instructions: &str,
        question: &str,
        prompt_file: &str,
        workdir: &str,
    ) -> Vec<String> {
        self.args
            .iter()
            .map(|arg| {
                arg.replace("{prompt}", prompt)
                    .replace("{instructions}", instructions)
                    .replace("{question}", question)
                    .replace("{prompt_file}", prompt_file)
                    .replace("{workdir}", workdir)
            })
            .collect()
    }

    /// Arguments shown in the job view: secrets redacted, large prompt text elided.
    pub fn display_args(&self, rendered: &[String]) -> Vec<String> {
        crate::exec::redact::display_args(rendered)
    }

    pub fn working_path(&self, investigation_dir: &std::path::Path) -> PathBuf {
        match self.working_directory {
            AgentWorkingDirectory::Investigation => investigation_dir.to_path_buf(),
            AgentWorkingDirectory::Current => {
                std::env::current_dir().unwrap_or_else(|_| investigation_dir.to_path_buf())
            }
        }
    }
}

/// Resolves the active agent from configuration.
///
/// Precedence: an inline `[agent] program = ...` profile, then `[agents.<active>]` overlaying a
/// built-in, then the built-in named by `agent.active` (default `claude`).
pub fn resolve_profile(config: &Config) -> AgentProfile {
    let settings = &config.agent;
    if settings
        .program
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return inline_profile(settings);
    }
    let mut profile = builtin(&settings.active);
    if let Some(overlay) = config.agents.get(&settings.active) {
        profile.overlay(overlay);
    }
    profile.timeout = Duration::from_secs(settings.timeout_seconds.max(1));
    profile
}

fn builtin(id: &str) -> AgentProfile {
    match id.trim().to_ascii_lowercase().as_str() {
        "claude" | "claude-code" => AgentProfile::claude(),
        "codex" => AgentProfile::codex(),
        other => AgentProfile::generic(other),
    }
}

fn inline_profile(settings: &AgentSettings) -> AgentProfile {
    let program = settings.program.clone().unwrap_or_else(|| "claude".into());
    let mut profile = match program.as_str() {
        "claude" => AgentProfile::claude(),
        "codex" => AgentProfile::codex(),
        _ => AgentProfile::generic(&program),
    };
    profile.program = program;
    if let Some(name) = &settings.name {
        profile.name = name.clone();
        profile.id = "custom".into();
    }
    if let Some(args) = &settings.args {
        profile.args = args.clone();
    }
    if let Some(adapter) = &settings.adapter {
        profile.adapter = AgentAdapter::parse(adapter);
    }
    if let Some(input) = &settings.input {
        profile.input = AgentInput::parse(input);
    }
    if let Some(working_directory) = &settings.working_directory {
        profile.working_directory = match working_directory.as_str() {
            "current" => AgentWorkingDirectory::Current,
            _ => AgentWorkingDirectory::Investigation,
        };
    }
    profile.timeout = Duration::from_secs(settings.timeout_seconds.max(1));
    profile
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_selects_claude_stream_json() {
        let actual = resolve_profile(&Config::default());
        assert_eq!(actual.program, "claude");
        assert_eq!(actual.adapter, AgentAdapter::Claude);
        assert!(actual.args.contains(&"stream-json".to_string()));
        assert!(actual.args.contains(&"--tools".to_string()));
    }

    #[test]
    fn inline_program_overrides_active() {
        let mut input = Config::default();
        input.agent.active = "claude".into();
        input.agent.program = Some("codex".into());
        let actual = resolve_profile(&input);
        assert_eq!(actual.program, "codex");
        assert_eq!(actual.adapter, AgentAdapter::Codex);
    }

    #[test]
    fn named_overlay_replaces_args() {
        let mut input = Config::default();
        input.agent.active = "claude".into();
        input.agents.insert(
            "claude".into(),
            AgentProfileConfig {
                args: Some(vec!["-p".into(), "{prompt}".into()]),
                ..AgentProfileConfig::default()
            },
        );
        let actual = resolve_profile(&input);
        assert_eq!(actual.args, vec!["-p".to_string(), "{prompt}".to_string()]);
        assert_eq!(actual.adapter, AgentAdapter::Claude);
    }

    #[test]
    fn placeholders_are_substituted_without_resplitting() {
        let profile = AgentProfile::generic("demo");
        let actual = profile.render_args("hello world; rm -rf /", "inst", "q", "/tmp/p", "/work");
        assert_eq!(
            actual,
            vec!["-p".to_string(), "hello world; rm -rf /".to_string()]
        );
    }
}
