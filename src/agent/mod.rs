//! External AI agent CLI integration.
//!
//! Agents are ordinary subprocesses (`claude`, `codex`, or a configured command). os-int never
//! talks to an LLM API itself. Context is serialized as Markdown with untrusted evidence fenced
//! off; proposed actions are previewed before they change investigation state.

mod context;
mod parse;
mod profile;
mod runner;

pub use context::{AgentContext, ContextFocus, serialize_context};
pub use parse::{ParsedAgentOutput, parse_actions, parse_claude_event, parse_codex_event};
pub use profile::{AgentAdapter, AgentInput, AgentProfile, AgentWorkingDirectory, resolve_profile};
pub use runner::{AgentOutcome, AgentRequest, run_agent};

use crate::config::Config;
use crate::tools::ToolRegistry;

/// The selected agent plus whether its program is currently on PATH.
#[derive(Clone, Debug)]
pub struct AgentStatus {
    pub profile: AgentProfile,
    pub available: bool,
    pub message: Option<String>,
}

impl AgentStatus {
    pub fn resolve(config: &Config, tools: &ToolRegistry) -> Self {
        let profile = resolve_profile(config);
        let available = tools.path_of(&profile.program).is_some()
            || crate::tools::find_on_path(&profile.program).is_some();
        let message = if available {
            None
        } else {
            Some(format!(
                "{} ({}) was not found on PATH.\n\nOSINT collection features remain available.\nConfigure another agent under Settings → Agent.",
                profile.name, profile.program
            ))
        };
        Self {
            profile,
            available,
            message,
        }
    }
}
