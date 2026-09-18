//! Pure OpenCode → generic-UI translation. No I/O, no threads.
//!
//! Every function takes `serde_json::Value` (tolerant reader: unknown or
//! missing fields degrade to fallbacks instead of failing) and returns generic
//! [`Session`](super::Session)/[`Block`](super::Block)/[`StreamEvent`](super::StreamEvent)
//! values. Shapes verified against OpenCode 1.18.31 live responses,
//! `opencode export` output, `packages/schema`, and `/v2/openapi.json` — see
//! `research/opencode-architecture.md` for exact sources.

use serde_json::Value;

use crate::backend::{
    AgentStatus, Block, PermissionRequest, Question, Session, ShellRun, SlashCommand, StatusInfo,
    StreamEvent, ToolCall, ToolState,
};

fn str_at(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn opt_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// A draft assistant turn assembled from `session.next.*` events and message
/// history. The worker owns one per session; tests drive it directly.
#[derive(Clone, Debug, Default)]
pub struct DraftState {
    text_open: bool,
    thinking_open: bool,
    tools: Vec<ToolDraft>,
    shells: Vec<ShellDraft>,
}

#[derive(Clone, Debug, Default)]
struct ToolDraft {
    call_id: String,
    name: String,
    done: bool,
}

#[derive(Clone, Debug, Default)]
struct ShellDraft {
    call_id: String,
    done: bool,
}

/// Summarize a tool input object into one Warp-style detail line, preferring
/// the most identifying field per known tool.
pub fn summarize_tool_input(tool: &str, input: &Value) -> String {
    let pick = |keys: &[&str]| {
        keys.iter()
            .filter_map(|key| input.get(key).and_then(Value::as_str))
            .next()
            .map(str::to_owned)
    };
    match tool {
        "bash" | "Bash" => pick(&["command", "cmd", "script"]).unwrap_or_default(),
        "read" | "Read" | "edit" | "Edit" | "write" | "Write" | "patch" | "Patch" => {
            pick(&["file", "path", "filePath", "filename"]).unwrap_or_default()
        }
        "glob" | "Glob" => pick(&["pattern", "glob", "path"]).unwrap_or_default(),
        "grep" | "Grep" | "search" => {
            pick(&["pattern", "query", "text", "path"]).unwrap_or_default()
        }
        "webfetch" | "WebFetch" => pick(&["url"]).unwrap_or_default(),
        "websearch" | "WebSearch" => pick(&["query"]).unwrap_or_default(),
        "task" | "Task" => pick(&["description", "subagent_type", "prompt"])
            .map(|text| text.chars().take(80).collect())
            .unwrap_or_default(),
        "question" | "ask" | "AskUserQuestion" => pick(&["question", "prompt"]).unwrap_or_default(),
        _ => pick(&["command", "file", "path", "pattern", "query", "url", "text"])
            .map(|text| text.chars().take(80).collect())
            .unwrap_or_default(),
    }
}

/// Map tool-result content items to plain output lines.
pub fn tool_output_lines(content: &Value) -> Vec<String> {
    match content {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item.get("type").and_then(Value::as_str) {
                Some("text") => item.get("text").and_then(Value::as_str).map(str::to_owned),
                Some("file") => item
                    .get("path")
                    .or_else(|| item.get("filename"))
                    .and_then(Value::as_str)
                    .map(|path| format!("<file {path}>")),
                _ => item.get("text").and_then(Value::as_str).map(str::to_owned),
            })
            .collect(),
        Value::String(text) => vec![text.clone()],
        _ => Vec::new(),
    }
}

/// Map one OpenCode session object to a generic session shell (blocks come
/// from history hydration + live events, not from here).
pub fn map_session(value: &Value) -> Session {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("ses_unknown")
        .to_owned();
    let title = opt_str(value, "title")
        .filter(|title| !title.is_empty())
        .or_else(|| opt_str(value, "slug"))
        .unwrap_or_else(|| "Untitled".to_owned());
    Session {
        id,
        title,
        blocks: Vec::new(),
    }
}

/// Statusline model label: `agent · provider/model`, falling back gracefully.
pub fn status_model(session: &Value) -> String {
    let agent = opt_str(session, "agent").unwrap_or_default();
    let model = session
        .get("model")
        .map(|model| {
            let provider = model
                .get("providerID")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let id = model.get("id").and_then(Value::as_str).unwrap_or("?");
            format!("{provider}/{id}")
        })
        .unwrap_or_default();
    match (agent.is_empty(), model.is_empty()) {
        (true, true) => "opencode".to_owned(),
        (true, false) => model,
        (false, true) => agent,
        (false, false) => format!("{agent} · {model}"),
    }
}

/// Working directory for the statusline, if the session records one.
pub fn status_cwd(session: &Value) -> String {
    session
        .get("location")
        .and_then(|location| location.get("directory"))
        .and_then(Value::as_str)
        .or_else(|| session.get("directory").and_then(Value::as_str))
        .unwrap_or("~")
        .to_owned()
}

/// Translate one SSE data payload into generic stream events. Handles both
/// the live 2.0.1 `session.*` family (captured from a running server) and the
/// `session.next.*` family from `packages/schema` (forward-compatible).
/// Unknown types yield no events.
pub fn map_event(typ: &str, data: &Value, draft: &mut DraftState) -> Vec<StreamEvent> {
    // Both families share payload shapes; only the prefix differs.
    let short = typ
        .strip_prefix("session.next.")
        .or_else(|| typ.strip_prefix("session."))
        .unwrap_or(typ);
    match short {
        "text.started" => {
            draft.text_open = true;
            vec![]
        }
        "text.delta" => {
            let delta = data.get("delta").and_then(Value::as_str).unwrap_or("");
            if delta.is_empty() {
                vec![]
            } else {
                draft.text_open = true;
                vec![StreamEvent::Chunk(delta.to_owned())]
            }
        }
        "text.ended" => {
            draft.text_open = false;
            vec![]
        }
        "reasoning.started" => {
            draft.thinking_open = true;
            vec![]
        }
        "reasoning.delta" => {
            // Reasoning payloads use `text` in some versions, `delta` in others.
            let fragment = data
                .get("delta")
                .or_else(|| data.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if fragment.is_empty() {
                vec![]
            } else {
                draft.thinking_open = true;
                vec![StreamEvent::ThinkChunk(fragment.to_owned())]
            }
        }
        "reasoning.ended" => {
            draft.thinking_open = false;
            vec![]
        }
        "tool.input.started" => {
            // 2.0.1 names the tool here (`name`); `tool.called` may omit it.
            let call_id = non_empty(data, &["id", "callID"]);
            let name = non_empty(data, &["name", "tool"]);
            if !draft.tools.iter().any(|tool| tool.call_id == call_id) {
                draft.tools.push(ToolDraft {
                    call_id,
                    name,
                    done: false,
                });
            }
            vec![]
        }
        "tool.called" => {
            let call_id = non_empty(data, &["id", "callID"]);
            let mut name = non_empty(data, &["tool", "name"]);
            if name.is_empty() {
                name = draft
                    .tools
                    .iter()
                    .find(|tool| tool.call_id == call_id)
                    .map(|tool| tool.name.clone())
                    .unwrap_or_default();
            } else if let Some(tool) = draft.tools.iter_mut().find(|tool| tool.call_id == call_id) {
                tool.name = name.clone();
            }
            let input = data.get("input").cloned().unwrap_or(Value::Null);
            // The `shell` tool's execution is rendered from `shell.*` events
            // instead (command, live output, exit status); a parallel Tool
            // block would duplicate it.
            if name == "shell" {
                if !draft.tools.iter().any(|tool| tool.call_id == call_id) {
                    draft.tools.push(ToolDraft {
                        call_id,
                        name,
                        done: false,
                    });
                }
                return vec![];
            }
            draft.tools.push(ToolDraft {
                call_id,
                name: name.clone(),
                done: false,
            });
            vec![StreamEvent::Push(Block::Tool {
                call: ToolCall {
                    name: if name.is_empty() {
                        "tool".into()
                    } else {
                        name.clone()
                    },
                    detail: summarize_tool_input(&name, &input),
                    state: ToolState::Running,
                    output: Vec::new(),
                    elapsed: String::new(),
                },
            })]
        }
        "tool.success" => {
            let call_id = non_empty(data, &["id", "callID"]);
            mark_tool_done(draft, &call_id);
            let output = data
                .get("content")
                .map(tool_output_lines)
                .unwrap_or_default();
            vec![StreamEvent::UpdateTool {
                state: ToolState::Done,
                elapsed: String::new(),
                output,
            }]
        }
        "tool.failed" => {
            let call_id = non_empty(data, &["id", "callID"]);
            mark_tool_done(draft, &call_id);
            let output = data
                .get("content")
                .map(tool_output_lines)
                .unwrap_or_default();
            let message = data
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("tool failed")
                .to_owned();
            let mut events = vec![StreamEvent::UpdateTool {
                state: ToolState::Failed,
                elapsed: String::new(),
                output,
            }];
            events.push(StreamEvent::Push(Block::Error { text: message }));
            events
        }
        // Input streaming only refines the detail line; the block already
        // exists from `tool.called`. Deltas accumulate untracked.
        "tool.input.delta" | "tool.input.ended" | "tool.progress" => vec![],
        "shell.started" => {
            let call_id = str_at(data, "callID");
            let command = str_at(data, "command");
            draft.shells.push(ShellDraft {
                call_id,
                done: false,
            });
            vec![StreamEvent::Push(Block::Shell {
                run: ShellRun {
                    command,
                    output: Vec::new(),
                    state: ToolState::Running,
                },
            })]
        }
        // 2.0.1 `shell.created` carries `info{command,...}` instead.
        "shell.created" => {
            let info = data.get("info").cloned().unwrap_or(Value::Null);
            let command = info
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            draft.shells.push(ShellDraft {
                call_id: info
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                done: false,
            });
            vec![StreamEvent::Push(Block::Shell {
                run: ShellRun {
                    command,
                    output: Vec::new(),
                    state: ToolState::Running,
                },
            })]
        }
        "shell.ended" => {
            let call_id = str_at(data, "callID");
            for shell in draft.shells.iter_mut().rev() {
                if shell.call_id == call_id {
                    shell.done = true;
                    break;
                }
            }
            let output = data
                .get("output")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let mut events = Vec::new();
            for line in output {
                events.push(StreamEvent::ShellLine(line));
            }
            events.push(StreamEvent::UpdateTool {
                state: ToolState::Done,
                elapsed: String::new(),
                output: Vec::new(),
            });
            events
        }
        // 2.0.1 `shell.exited` carries `{id, status, exit}` (output arrives
        // via the sibling `tool.success` content, applied to the shell block
        // by `UpdateTool`).
        "shell.exited" => {
            let state = match (
                data.get("status").and_then(Value::as_str).unwrap_or(""),
                data.get("exit").and_then(Value::as_u64).unwrap_or(0),
            ) {
                ("exited", 0) => ToolState::Done,
                _ => ToolState::Failed,
            };
            vec![StreamEvent::UpdateTool {
                state,
                elapsed: String::new(),
                output: Vec::new(),
            }]
        }
        "step.started" | "execution.started" => vec![StreamEvent::Push(Block::Working {
            label: "Working…".into(),
        })],
        "step.ended" => {
            let finish = str_at(data, "finish");
            let mut events = vec![StreamEvent::WorkLabel(String::new())];
            if finish == "error" {
                events.push(StreamEvent::Push(Block::Error {
                    text: "Step failed.".into(),
                }));
            }
            events
        }
        "execution.succeeded" => vec![StreamEvent::WorkLabel(String::new())],
        "step.failed" | "execution.failed" => {
            let message = data
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Step failed.")
                .to_owned();
            vec![
                StreamEvent::WorkLabel(String::new()),
                StreamEvent::Push(Block::Error { text: message }),
            ]
        }
        "permission.v2.asked" => vec![StreamEvent::Push(Block::Permission {
            req: PermissionRequest {
                tool: str_at(data, "action"),
                summary: permission_summary(data),
            },
        })],
        "permission.v2.replied" => vec![],
        "question.v2.asked" => map_question(data),
        "question.v2.replied" | "question.v2.rejected" => vec![],
        // prompt/inbox/instructions/usage/context/compaction/revert/synthetic
        // and agent/model switches carry no transcript content in Phase 4.
        _ => vec![],
    }
}

/// First non-empty string among several candidate keys (version-tolerant
/// field lookup: 2.0.1 uses `id`, the schema uses `callID`, etc.).
fn non_empty(data: &Value, keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|key| data.get(key).and_then(Value::as_str))
        .find(|text| !text.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn mark_tool_done(draft: &mut DraftState, call_id: &str) {
    for tool in draft.tools.iter_mut().rev() {
        if tool.call_id == call_id {
            tool.done = true;
            return;
        }
    }
}

fn permission_summary(data: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(message) = data.get("message").and_then(Value::as_str) {
        if !message.is_empty() {
            parts.push(message.to_owned());
        }
    }
    if let Some(resources) = data.get("resources").and_then(Value::as_array) {
        let list: Vec<&str> = resources.iter().filter_map(Value::as_str).collect();
        if !list.is_empty() {
            parts.push(list.join(", "));
        }
    }
    if parts.is_empty() {
        parts.push("Approval requested.".into());
    }
    parts.join(" · ")
}

fn map_question(data: &Value) -> Vec<StreamEvent> {
    let questions = data.get("questions").and_then(Value::as_array);
    let Some(questions) = questions else {
        return vec![];
    };
    let mut events = Vec::new();
    for question in questions {
        let header = question.get("header").and_then(Value::as_str).unwrap_or("");
        let prompt = question
            .get("question")
            .and_then(Value::as_str)
            .unwrap_or("");
        let title = if header.is_empty() {
            prompt.to_owned()
        } else if prompt.is_empty() {
            header.to_owned()
        } else {
            format!("{header}: {prompt}")
        };
        let options = question
            .get("options")
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(|option| option.get("label").and_then(Value::as_str))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        // Answer routing needs the request id; the UI resolves by position
        // (latest gate), matching the existing Blocker contract.
        events.push(StreamEvent::Push(Block::Question {
            q: Question {
                prompt: title,
                options,
            },
        }));
    }
    events
}

/// Map one REST/history message object to transcript blocks. Handles both the
/// V2 `content[]` shape and V1-export `parts[]` shape.
pub fn map_message(message: &Value) -> Vec<Block> {
    let typ = message.get("type").and_then(Value::as_str).unwrap_or("");
    match typ {
        "user" => vec![Block::User {
            text: message
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        }],
        "assistant" => {
            let items = message
                .get("content")
                .or_else(|| message.get("parts"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            map_assistant_items(&items)
        }
        "shell" => {
            let command = str_at(message, "command");
            let status = message.get("status").and_then(Value::as_str).unwrap_or("");
            let state = match status {
                "running" => ToolState::Running,
                "exited" => ToolState::Done,
                "timeout" | "killed" => ToolState::Failed,
                _ => ToolState::Done,
            };
            let output = message
                .get("output")
                .and_then(|output| output.get("output"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect();
            vec![Block::Shell {
                run: ShellRun {
                    command,
                    output,
                    state,
                },
            }]
        }
        _ => vec![],
    }
}

fn map_assistant_items(items: &[Value]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut thinking = String::new();
    let flush_text = |text: &mut String, blocks: &mut Vec<Block>| {
        if !text.trim().is_empty() {
            blocks.push(Block::Assistant {
                text: std::mem::take(text),
            });
        } else {
            text.clear();
        }
    };
    let flush_thinking = |thinking: &mut String, blocks: &mut Vec<Block>| {
        if !thinking.trim().is_empty() {
            blocks.push(Block::Thinking {
                text: std::mem::take(thinking),
            });
        } else {
            thinking.clear();
        }
    };
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                flush_thinking(&mut thinking, &mut blocks);
                if let Some(fragment) = item.get("text").and_then(Value::as_str) {
                    text.push_str(fragment);
                }
            }
            Some("reasoning") => {
                flush_text(&mut text, &mut blocks);
                if let Some(fragment) = item.get("text").and_then(Value::as_str) {
                    thinking.push_str(fragment);
                }
            }
            Some("tool") => {
                flush_text(&mut text, &mut blocks);
                flush_thinking(&mut thinking, &mut blocks);
                blocks.push(map_history_tool(item));
            }
            // step-start/step-finish markers and unknown parts carry no content.
            _ => {}
        }
    }
    flush_text(&mut text, &mut blocks);
    flush_thinking(&mut thinking, &mut blocks);
    blocks
}

fn map_history_tool(item: &Value) -> Block {
    let name = item.get("tool").and_then(Value::as_str).unwrap_or("tool");
    let state = item.get("state");
    let status = state
        .and_then(|state| state.get("status"))
        .and_then(Value::as_str);
    let tool_state = match status {
        Some("completed") => ToolState::Done,
        Some("running") | Some("streaming") => ToolState::Running,
        Some("error") => ToolState::Failed,
        _ => ToolState::Done,
    };
    let input = state
        .and_then(|state| state.get("input"))
        .cloned()
        .unwrap_or(Value::Null);
    let output = state
        .and_then(|state| state.get("output"))
        .map(|output| match output {
            Value::String(text) => text.lines().map(str::to_owned).collect(),
            other => tool_output_lines(other),
        })
        .unwrap_or_default();
    let call = ToolCall {
        name: name.to_owned(),
        detail: summarize_tool_input(name, &input),
        state: tool_state,
        output,
        elapsed: String::new(),
    };
    let call = if call.detail.is_empty() {
        ToolCall {
            detail: str_at(item, "title"),
            ..call
        }
    } else {
        call
    };
    Block::Tool { call }
}

/// Slash-command entries from `GET /api/command` (shape-tolerant).
pub fn map_commands(value: &Value) -> Vec<SlashCommand> {
    let items = value
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| value.as_array());
    let Some(items) = items else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let name = item
                .get("name")
                .or_else(|| item.get("id"))
                .or_else(|| item.get("command"))
                .and_then(Value::as_str)?;
            Some(SlashCommand {
                name: name.trim_start_matches('/').to_owned(),
                description: item
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            })
        })
        .collect()
}

/// Status snapshot pieces from a session object.
pub fn map_status(session: &Value) -> StatusInfo {
    StatusInfo {
        model: status_model(session),
        context_pct: 0,
        cwd: status_cwd(session),
        branch: String::new(),
        status: AgentStatus::Idle,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        map_commands, map_event, map_message, map_session, status_cwd, status_model,
        summarize_tool_input, tool_output_lines, DraftState,
    };
    use crate::backend::{Block, StreamEvent, ToolState};

    #[test]
    fn text_deltas_become_chunks() {
        let mut draft = DraftState::default();
        let events = map_event(
            "session.next.text.delta",
            &json!({"assistantMessageID": "msg_1", "textID": "t", "delta": "hi "}),
            &mut draft,
        );
        assert!(matches!(events.as_slice(), [StreamEvent::Chunk(text)] if text == "hi "));
        let events = map_event("session.next.text.ended", &json!({}), &mut draft);
        assert!(events.is_empty());
        assert!(!draft.text_open);
    }

    #[test]
    fn reasoning_maps_to_think_chunks() {
        let mut draft = DraftState::default();
        let events = map_event(
            "session.next.reasoning.delta",
            &json!({"text": "because"}),
            &mut draft,
        );
        assert!(matches!(events.as_slice(), [StreamEvent::ThinkChunk(text)] if text == "because"));
    }

    #[test]
    fn tool_lifecycle_round_trip() {
        let mut draft = DraftState::default();
        let called = map_event(
            "session.next.tool.called",
            &json!({"callID": "c1", "tool": "bash", "input": {"command": "git status"}}),
            &mut draft,
        );
        match called.as_slice() {
            [StreamEvent::Push(Block::Tool { call })] => {
                assert_eq!(call.name, "bash");
                assert_eq!(call.detail, "git status");
                assert_eq!(call.state, ToolState::Running);
            }
            other => panic!("unexpected {other:?}"),
        }
        let done = map_event(
            "session.next.tool.success",
            &json!({"callID": "c1", "content": [{"type": "text", "text": "ok"}]}),
            &mut draft,
        );
        match done.as_slice() {
            [StreamEvent::UpdateTool { state, output, .. }] => {
                assert_eq!(*state, ToolState::Done);
                assert_eq!(output, &vec!["ok".to_owned()]);
            }
            other => panic!("unexpected {other:?}"),
        }
        let failed = map_event(
            "session.next.tool.failed",
            &json!({"callID": "c1", "error": {"message": "boom"}}),
            &mut draft,
        );
        assert!(matches!(
            failed.as_slice(),
            [
                StreamEvent::UpdateTool { .. },
                StreamEvent::Push(Block::Error { .. })
            ]
        ));
    }

    #[test]
    fn shell_events_compose_lines_then_done() {
        let mut draft = DraftState::default();
        let started = map_event(
            "session.next.shell.started",
            &json!({"callID": "s1", "command": "ls"}),
            &mut draft,
        );
        assert!(matches!(
            started.as_slice(),
            [StreamEvent::Push(Block::Shell { .. })]
        ));
        let ended = map_event(
            "session.next.shell.ended",
            &json!({"callID": "s1", "output": "a\nb"}),
            &mut draft,
        );
        assert!(matches!(
            ended.as_slice(),
            [
                StreamEvent::ShellLine(_),
                StreamEvent::ShellLine(_),
                StreamEvent::UpdateTool { .. }
            ]
        ));
    }

    #[test]
    fn permission_and_question_gates() {
        let mut draft = DraftState::default();
        let asked = map_event(
            "permission.v2.asked",
            &json!({"id": "per_1", "action": "edit", "resources": ["a.rs"], "message": "Apply?"}),
            &mut draft,
        );
        match asked.as_slice() {
            [StreamEvent::Push(Block::Permission { req })] => {
                assert_eq!(req.tool, "edit");
                assert!(req.summary.contains("a.rs"));
            }
            other => panic!("unexpected {other:?}"),
        }
        let asked = map_event(
            "question.v2.asked",
            &json!({"id": "que_1", "questions": [
                {"header": "Pick", "question": "Which?", "options": [
                    {"label": "A", "description": "first"},
                    {"label": "B", "description": "second"},
                ]}
            ]}),
            &mut draft,
        );
        match asked.as_slice() {
            [StreamEvent::Push(Block::Question { q })] => {
                assert_eq!(q.options, vec!["A".to_owned(), "B".to_owned()]);
                assert!(q.prompt.contains("Which?"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn live_v2_family_maps_end_to_end() {
        // Shapes captured from a running 2.0.1 server (see
        // research/opencode-architecture.md): no `session.next.` prefix,
        // tool named in `input.started`, shell via `shell.created/exited`.
        let mut draft = DraftState::default();
        let started = map_event(
            "session.tool.input.started",
            &json!({"id": "call_1", "name": "bash", "sessionID": "ses_1"}),
            &mut draft,
        );
        assert!(started.is_empty());
        let called = map_event(
            "session.tool.called",
            &json!({"id": "call_1", "input": {"command": "echo hi"}, "sessionID": "ses_1"}),
            &mut draft,
        );
        match called.as_slice() {
            [StreamEvent::Push(Block::Tool { call })] => {
                assert_eq!(call.name, "bash");
                assert_eq!(call.detail, "echo hi");
            }
            other => panic!("unexpected {other:?}"),
        }
        let created = map_event(
            "shell.created",
            &json!({"info": {"id": "sh_1", "command": "echo hi"}}),
            &mut draft,
        );
        assert!(matches!(
            created.as_slice(),
            [StreamEvent::Push(Block::Shell { .. })]
        ));
        let exited = map_event(
            "shell.exited",
            &json!({"id": "sh_1", "status": "exited", "exit": 0}),
            &mut draft,
        );
        assert!(matches!(
            exited.as_slice(),
            [StreamEvent::UpdateTool {
                state: ToolState::Done,
                ..
            }]
        ));
        let step = map_event("session.execution.started", &json!({}), &mut draft);
        assert!(matches!(
            step.as_slice(),
            [StreamEvent::Push(Block::Working { .. })]
        ));
        let done = map_event("session.execution.succeeded", &json!({}), &mut draft);
        assert!(matches!(done.as_slice(), [StreamEvent::WorkLabel(label)] if label.is_empty()));
    }

    #[test]
    fn shell_tool_call_defers_to_shell_events() {
        let mut draft = DraftState::default();
        let called = map_event(
            "session.tool.called",
            &json!({"id": "c9", "tool": "shell", "input": {"command": "ls"}}),
            &mut draft,
        );
        assert!(called.is_empty());
    }

    #[test]
    fn unknown_events_are_ignored() {
        let mut draft = DraftState::default();
        assert!(map_event("session.next.compaction.delta", &json!({}), &mut draft).is_empty());
        assert!(map_event("something.new", &json!({}), &mut draft).is_empty());
    }

    #[test]
    fn history_messages_map_in_order() {
        let message = json!({
            "type": "assistant",
            "content": [
                {"type": "reasoning", "text": "let me think "},
                {"type": "text", "text": "hello"},
                {"type": "tool", "tool": "read", "state": {
                    "status": "completed",
                    "input": {"file": "a.rs"},
                    "output": "line1\nline2",
                }},
                {"type": "text", "text": "done"},
            ],
        });
        let blocks = map_message(&message);
        assert!(matches!(blocks[0], Block::Thinking { .. }));
        assert!(matches!(blocks[1], Block::Assistant { .. }));
        assert!(matches!(blocks[2], Block::Tool { .. }));
        assert!(matches!(blocks[3], Block::Assistant { .. }));
        match &blocks[2] {
            Block::Tool { call } => {
                assert_eq!(call.detail, "a.rs");
                assert_eq!(call.output.len(), 2);
            }
            other => panic!("unexpected {other:?}"),
        }
        let user = map_message(&json!({"type": "user", "text": "hi"}));
        assert!(matches!(user.as_slice(), [Block::User { .. }]));
    }

    #[test]
    fn session_mapping_falls_back() {
        let full = map_session(&json!({"id": "ses_1", "title": "T"}));
        assert_eq!((full.id.as_str(), full.title.as_str()), ("ses_1", "T"));
        let bare = map_session(&json!({"id": "ses_2"}));
        assert_eq!(bare.title, "Untitled");
    }

    #[test]
    fn status_helpers_degrade() {
        let session = json!({
            "agent": "build",
            "model": {"providerID": "opencode", "id": "m1"},
            "location": {"directory": "/tmp/x"},
        });
        assert_eq!(status_model(&session), "build · opencode/m1");
        assert_eq!(status_cwd(&session), "/tmp/x");
        assert_eq!(status_model(&json!({})), "opencode");
        assert_eq!(status_cwd(&json!({})), "~");
    }

    #[test]
    fn tool_details_prefer_identifying_fields() {
        assert_eq!(
            summarize_tool_input("bash", &json!({"command": "ls"})),
            "ls"
        );
        assert_eq!(summarize_tool_input("wat", &json!({"a": 1})), "");
        assert_eq!(
            tool_output_lines(&json!([{"type": "text", "text": "x"}])),
            vec!["x".to_owned()]
        );
    }

    #[test]
    fn commands_tolerate_shapes() {
        let commands = map_commands(&json!({"data": [
            {"name": "plan", "description": "d"},
            {"name": "/test"},
        ]}));
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[1].name, "test");
    }
}
