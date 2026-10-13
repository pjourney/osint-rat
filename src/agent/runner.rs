//! Runs an agent CLI as a subprocess, streaming output back through the job context.

use crate::exec::{CommandSpec, ExecError, Stream, run};
use crate::files::InvestigationFiles;
use crate::jobs::{DeltaBuffer, JobContext};
use crate::model::AgentAction;
use crate::sanitize::sanitize_text;

use super::context::{AgentContext, agent_instructions, random_nonce, serialize_context};
use super::parse::{
    ParsedAgentOutput, StreamDelta, append_text_line, parse_actions, parse_claude_event,
    parse_codex_event,
};
use super::profile::{AgentAdapter, AgentInput, AgentProfile};

const PROMPT_FILE_NAME: &str = "prompt.md";

pub struct AgentRequest {
    pub profile: AgentProfile,
    pub context: AgentContext,
    pub files: InvestigationFiles,
    pub max_context_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct AgentOutcome {
    pub response: String,
    pub actions: Vec<AgentAction>,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
    pub error: Option<String>,
    pub context_path: String,
    pub program: String,
    pub args: Vec<String>,
}

/// Builds the prompt, invokes the agent, and returns the parsed response.
pub async fn run_agent(request: AgentRequest, job: &JobContext) -> Result<AgentOutcome, String> {
    request
        .files
        .ensure()
        .map_err(|error| format!("could not create investigation directory: {error}"))?;
    let nonce = random_nonce();
    let instructions = agent_instructions();
    let mut pack = serialize_context(&request.context, &nonce);
    if pack.len() > request.max_context_bytes {
        pack = crate::sanitize::truncate_with_marker(&pack, request.max_context_bytes).into_owned();
        job.log("context pack truncated to agent.max_context_bytes");
    }
    let blob = request
        .files
        .write_text(
            "agent",
            &format!("{}-{PROMPT_FILE_NAME}", job.job_id.simple()),
            &pack,
        )
        .map_err(|error| format!("could not write agent context pack: {error}"))?;
    let prompt_path = request
        .files
        .resolve(&blob.relative_path)
        .ok_or_else(|| "unsafe context pack path".to_string())?;
    let stdin_body = match request.profile.input {
        AgentInput::Stdin => Some(pack.as_bytes().to_vec()),
        AgentInput::File | AgentInput::Argument => None,
    };
    let workdir = request.profile.working_path(request.files.root());
    std::fs::create_dir_all(&workdir)
        .map_err(|error| format!("could not create agent working directory: {error}"))?;
    let rendered = request.profile.render_args(
        &pack,
        &instructions,
        &request.context.question,
        &prompt_path.display().to_string(),
        &workdir.display().to_string(),
    );
    let display_args = request.profile.display_args(&rendered);
    job.command(&request.profile.program, &display_args);
    let mut spec = CommandSpec::new(
        request.profile.program.clone(),
        rendered,
        request.profile.timeout,
    );
    spec.cwd = Some(workdir);
    spec.stdin = stdin_body;
    spec.env = request.profile.env.clone();
    spec.max_output_bytes = 2 * 1024 * 1024;
    let mut parsed = ParsedAgentOutput::default();
    let mut deltas = DeltaBuffer::default();
    let adapter = request.profile.adapter;
    let result = run(&spec, &job.cancel, |line| {
        handle_line(
            adapter,
            line.stream,
            &line.text,
            &mut parsed,
            job,
            &mut deltas,
        );
    })
    .await;
    deltas.flush(job);
    if parsed.actions.is_empty() {
        parsed.actions = parse_actions(&parsed.text);
    }
    parsed.text = sanitize_text(&parsed.text).into_owned();
    match result {
        Err(ExecError::Cancelled) => Err("cancelled".into()),
        Err(error) => Err(error.to_string()),
        Ok(outcome) => {
            if parsed.text.trim().is_empty() {
                parsed.text = sanitize_text(&outcome.stdout).into_owned();
                parsed.actions = parse_actions(&parsed.text);
            }
            if parsed.error.is_none() && !outcome.success {
                parsed.error = Some(format!(
                    "{} exited {}",
                    request.profile.program,
                    outcome.exit_code.unwrap_or(-1)
                ));
            }
            if parsed.error.is_some() && parsed.text.trim().is_empty() {
                parsed.text = parsed.error.clone().unwrap_or_default();
            }
            Ok(AgentOutcome {
                response: parsed.text,
                actions: parsed.actions,
                model: parsed.model,
                cost_usd: parsed.cost_usd,
                error: parsed.error,
                context_path: blob.relative_path,
                program: spec.program,
                args: display_args,
            })
        }
    }
}

fn handle_line(
    adapter: AgentAdapter,
    stream: Stream,
    line: &str,
    parsed: &mut ParsedAgentOutput,
    job: &JobContext,
    deltas: &mut DeltaBuffer,
) {
    if stream == Stream::Stderr {
        let text = sanitize_text(line).into_owned();
        if !text.trim().is_empty() {
            job.error_line(&text);
        }
        return;
    }
    let delta = match adapter {
        AgentAdapter::Claude => parse_claude_event(line, parsed),
        AgentAdapter::Codex => parse_codex_event(line, parsed),
        AgentAdapter::Text => Some(append_text_line(parsed, line)),
    };
    match delta {
        Some(StreamDelta::Text(text)) => {
            let text = sanitize_text(&text);
            if !text.is_empty() {
                deltas.push(job, &text);
            }
        }
        Some(StreamDelta::Status(status)) => {
            let _ = job.try_send(crate::jobs::JobEvent::AgentStatus {
                job_id: job.job_id,
                status: sanitize_text(&status).into_owned(),
            });
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::agent::profile::AgentProfile;

    #[test]
    fn claude_args_include_disabled_tools_and_system_prompt_placeholder() {
        let profile = AgentProfile::claude();
        assert!(
            profile
                .args
                .windows(2)
                .any(|pair| pair[0] == "--tools" && pair[1].is_empty())
        );
        assert!(profile.args.contains(&"{instructions}".to_string()));
    }

    #[tokio::test]
    async fn missing_agent_becomes_a_job_error() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut profile = AgentProfile::generic("os-int-no-such-agent");
        profile.timeout = Duration::from_secs(5);
        profile.args = vec!["{question}".into()];
        let request = AgentRequest {
            profile,
            context: AgentContext::from_target(
                &crate::model::Target::Domain("example.com".into()),
                "What stands out?",
            ),
            files: crate::files::InvestigationFiles::at(directory.path().to_path_buf()),
            max_context_bytes: 16 * 1024,
        };
        let actual = run_agent(request, &JobContext::detached(false)).await;
        assert!(actual.is_err(), "{actual:?}");
        assert!(actual.unwrap_err().contains("not found"));
    }
}
