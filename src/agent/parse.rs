//! Parsers for agent stdout: Claude Code stream-json, Codex exec --json, and `os-int-actions` blocks.

use serde::Deserialize;
use serde_json::Value;

use crate::model::AgentAction;
use crate::sanitize::sanitize_text;

/// Incremental parse of one agent session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedAgentOutput {
    pub text: String,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
    pub error: Option<String>,
    pub session_id: Option<String>,
    pub actions: Vec<AgentAction>,
}

/// Something the runner should show immediately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamDelta {
    Text(String),
    Status(String),
}

/// Parses one Claude Code stream-json object. Unknown event types are ignored.
pub fn parse_claude_event(line: &str, output: &mut ParsedAgentOutput) -> Option<StreamDelta> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "system" => {
            if let Some(model) = value.get("model").and_then(Value::as_str) {
                output.model = Some(model.to_string());
            }
            if let Some(session) = value.get("session_id").and_then(Value::as_str) {
                output.session_id = Some(session.to_string());
            }
            let subtype = value
                .get("subtype")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let status = value.get("status").and_then(Value::as_str);
            if subtype == "status" {
                return Some(StreamDelta::Status(
                    status.unwrap_or("thinking").to_string(),
                ));
            }
            if subtype == "init" {
                return Some(StreamDelta::Status("starting".into()));
            }
            None
        }
        "assistant" | "stream_event" | "content_block_delta" => {
            let delta = extract_text(&value);
            if delta.is_empty() {
                return None;
            }
            output.text.push_str(&delta);
            Some(StreamDelta::Text(delta))
        }
        "result" => {
            if let Some(result) = value.get("result").and_then(Value::as_str)
                && output.text.trim().is_empty()
            {
                output.text = result.to_string();
            }
            if value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                output.error = Some(
                    value
                        .get("result")
                        .and_then(Value::as_str)
                        .or_else(|| value.get("error").and_then(Value::as_str))
                        .unwrap_or("the agent reported an error")
                        .to_string(),
                );
            }
            output.cost_usd = value.get("total_cost_usd").and_then(Value::as_f64);
            output.model = value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(output.model.take());
            output.actions = parse_actions(&output.text);
            None
        }
        "error" => {
            let message = value
                .get("error")
                .and_then(|error| {
                    error.as_str().map(str::to_string).or_else(|| {
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                })
                .or_else(|| {
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "agent error".into());
            output.error = Some(message.clone());
            Some(StreamDelta::Status(message))
        }
        _ => {
            if let Some(delta) = value
                .pointer("/delta/text")
                .and_then(Value::as_str)
                .map(str::to_string)
            {
                output.text.push_str(&delta);
                return Some(StreamDelta::Text(delta));
            }
            None
        }
    }
}

fn extract_text(value: &Value) -> String {
    if let Some(text) = value.pointer("/message/content") {
        return content_blocks(text);
    }
    if let Some(text) = value.get("content") {
        return content_blocks(text);
    }
    if let Some(text) = value.pointer("/delta/text").and_then(Value::as_str) {
        return text.to_string();
    }
    if let Some(text) = value.pointer("/event/delta/text").and_then(Value::as_str) {
        return text.to_string();
    }
    String::new()
}

fn content_blocks(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    block
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Parses one Codex `exec --json` event.
pub fn parse_codex_event(line: &str, output: &mut ParsedAgentOutput) -> Option<StreamDelta> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "thread.started" => {
            output.session_id = value
                .get("thread_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(StreamDelta::Status("starting".into()))
        }
        "turn.started" => Some(StreamDelta::Status("thinking".into())),
        "error" => {
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("agent error")
                .to_string();
            output.error = Some(message.clone());
            Some(StreamDelta::Status(message))
        }
        "turn.failed" => {
            let message = value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("turn failed")
                .to_string();
            output.error = Some(message.clone());
            Some(StreamDelta::Status(message))
        }
        "item.completed" | "agent.message" | "message.delta" | "agent.message.delta" => {
            let delta = value
                .pointer("/item/text")
                .or_else(|| value.pointer("/item/message"))
                .or_else(|| value.get("text"))
                .or_else(|| value.pointer("/delta/text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if delta.is_empty() {
                return None;
            }
            if event_type.ends_with("completed") && output.text.contains(&delta) {
                return None;
            }
            output.text.push_str(&delta);
            Some(StreamDelta::Text(delta))
        }
        "turn.completed" => {
            output.actions = parse_actions(&output.text);
            Some(StreamDelta::Status("done".into()))
        }
        _ => None,
    }
}

/// Appends a raw text line (generic adapter).
pub fn append_text_line(output: &mut ParsedAgentOutput, line: &str) -> StreamDelta {
    let mut text = sanitize_text(line).into_owned();
    text.push('\n');
    output.text.push_str(&text);
    StreamDelta::Text(text)
}

/// Extracts `os-int-actions` JSON from an agent response. Invalid JSON is ignored (the
/// surrounding text is still shown); individual actions that fail to deserialize are skipped.
pub fn parse_actions(text: &str) -> Vec<AgentAction> {
    let mut actions = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```os-int-actions") {
        let after = &rest[start + "```os-int-actions".len()..];
        let after = after.trim_start_matches('\r').trim_start_matches('\n');
        let Some(end) = after.find("```") else {
            break;
        };
        let block = after[..end].trim();
        rest = &after[end + 3..];
        actions.extend(decode_actions(block));
    }
    actions
}

fn decode_actions(block: &str) -> Vec<AgentAction> {
    if let Ok(actions) = serde_json::from_str::<Vec<AgentAction>>(block) {
        return actions;
    }
    if let Ok(action) = serde_json::from_str::<AgentAction>(block) {
        return vec![action];
    }
    #[derive(Deserialize)]
    struct Envelope {
        actions: Vec<AgentAction>,
    }
    if let Ok(envelope) = serde_json::from_str::<Envelope>(block) {
        return envelope.actions;
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_assistant_text_is_accumulated() {
        let input = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello "},{"type":"text","text":"world"}]}}"#;
        let mut output = ParsedAgentOutput::default();
        let delta = parse_claude_event(input, &mut output);
        assert_eq!(delta, Some(StreamDelta::Text("Hello world".into())));
        assert_eq!(output.text, "Hello world");
    }

    #[test]
    fn claude_error_result_is_recorded() {
        let input = r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login","total_cost_usd":0}"#;
        let mut output = ParsedAgentOutput::default();
        parse_claude_event(input, &mut output);
        assert_eq!(
            output.error.as_deref(),
            Some("Not logged in · Please run /login")
        );
        assert_eq!(output.cost_usd, Some(0.0));
    }

    #[test]
    fn codex_error_events_are_recorded() {
        let input =
            r#"{"type":"turn.failed","error":{"message":"unexpected status 401 Unauthorized"}}"#;
        let mut output = ParsedAgentOutput::default();
        let delta = parse_codex_event(input, &mut output);
        assert!(matches!(delta, Some(StreamDelta::Status(message)) if message.contains("401")));
        assert!(output.error.as_deref().unwrap_or_default().contains("401"));
    }

    #[test]
    fn actions_are_parsed_from_fenced_json() {
        let input = r#"Here is a finding.

```os-int-actions
[
  {"action":"ADD_FINDING","title":"Cloudflare fronts the site","sources":["S-31","S-32"],"confidence":"high"},
  {"action":"RUN_COLLECTOR","collector":"rdap","target":"example.com"}
]
```
"#;
        let actual = parse_actions(input);
        assert_eq!(actual.len(), 2);
        assert_eq!(actual[0].name(), "ADD_FINDING");
        assert_eq!(actual[1].name(), "RUN_COLLECTOR");
        assert_eq!(
            actual[0].cited_sources(),
            &["S-31".to_string(), "S-32".to_string()]
        );
    }

    #[test]
    fn invalid_action_blocks_are_ignored() {
        assert!(parse_actions("```os-int-actions\nnot json\n```").is_empty());
    }
}
