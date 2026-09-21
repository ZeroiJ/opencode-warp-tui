//! Temporary mock backend: scripted Warp-like sessions for TUI validation.
//!
//! Exists ONLY to render the UI in Phase 3. It will be replaced by the
//! OpenCode backend adapter (same [`Backend`](super::Backend) trait).
//!
//! Streaming is a per-session FIFO of [`StreamEvent`]s drained by
//! [`Backend::poll_stream`] — deterministic: tests advance it by hand, the
//! TUI advances it on a timer.

use std::collections::{HashMap, VecDeque};

use super::{
    memory::{command, triage, MemoryApi},
    AgentStatus, Backend, Block, Blocker, FileDiff, PermissionRequest, Question, Session,
    SessionSummary, ShellRun, SlashCommand, StatusInfo, StreamEvent, ToolCall, ToolState,
};

/// Demo scenarios exercisable via `/demo <letter>` in the prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    /// Normal assistant conversation.
    Conversation,
    /// Progressively streamed assistant response.
    Streaming,
    /// Tool execution running to done.
    ToolRun,
    /// Permission request gate.
    Permission,
    /// Question request gate.
    Question,
    /// Shell command with progressive output.
    Shell,
    /// File edit with diff.
    FileEdit,
    /// Plan with working indicator.
    Plan,
    /// Failure followed by recovery.
    ErrorRecovery,
    /// Multiple sessions.
    MultiSession,
}

impl Scenario {
    fn from_letter(letter: &str) -> Option<Self> {
        match letter {
            "a" => Some(Scenario::Conversation),
            "b" => Some(Scenario::Streaming),
            "c" => Some(Scenario::ToolRun),
            "d" => Some(Scenario::Permission),
            "e" => Some(Scenario::Question),
            "f" => Some(Scenario::Shell),
            "g" => Some(Scenario::FileEdit),
            "h" => Some(Scenario::Plan),
            "i" => Some(Scenario::ErrorRecovery),
            "j" => Some(Scenario::MultiSession),
            _ => None,
        }
    }
}

pub struct MockBackend {
    sessions: Vec<Session>,
    active: usize,
    next_id: usize,
    turn: usize,
    streams: HashMap<String, VecDeque<StreamEvent>>,
    /// Phase 7C parity: staged revert boundary / writer-command confirmation.
    pending_revert: Option<String>,
    pending_command: Option<(String, String)>,
    /// Phase 8 parity: a real memory engine + proposal queues in temp
    /// dirs, so `/memory suggest|confirm|discard` run the same triage
    /// code as the OpenCode adapter (dev/test only — nothing persists
    /// beyond the process).
    memory: MemoryApi,
}

impl MockBackend {
    /// Three scripted sessions covering conversation, shell, and gates.
    pub fn demo() -> Self {
        // Unique temp dir per instance: parallel tests must never share
        // quarantine queue files (the queue has no cross-process lock —
        // production use is single-threaded adapter code).
        static DEMO_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = DEMO_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("owt-mock-{}-{seq}", std::process::id()));
        let mut backend = Self {
            sessions: Vec::new(),
            active: 0,
            next_id: 0,
            turn: 0,
            streams: HashMap::new(),
            pending_revert: None,
            pending_command: None,
            memory: MemoryApi::open(dir, None),
        };
        let id0 = backend.fresh_id();
        let id1 = backend.fresh_id();
        let id2 = backend.fresh_id();
        backend.sessions.push(agent_session(id0));
        backend.sessions.push(shell_session(id1));
        backend.sessions.push(gate_session(id2));
        backend
    }

    fn fresh_id(&mut self) -> String {
        let id = format!("mock-{}", self.next_id);
        self.next_id += 1;
        id
    }

    /// Deterministic Phase 7C catalog (mirrors the opencode validation
    /// contract: unknown ids error visibly and are never "sent").
    fn mock_models() -> Vec<&'static str> {
        vec!["mock-sonnet"]
    }

    fn mock_agents() -> Vec<&'static str> {
        vec!["build", "general"]
    }

    /// Phase 8 parity: generate rule proposals from this session's user
    /// blocks into the temp quarantine queues. Silent, best-effort.
    fn refresh_proposals(&mut self, session_id: &str) {
        let texts: Vec<String> = self
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map(|session| {
                session
                    .blocks
                    .iter()
                    .filter_map(|block| match block {
                        Block::User { text } => {
                            let trimmed = text.trim();
                            if trimmed.is_empty()
                                || trimmed == "/memory"
                                || trimmed == "/mem"
                                || trimmed.starts_with("/memory ")
                                || trimmed.starts_with("/mem ")
                            {
                                None
                            } else {
                                Some(trimmed.to_owned())
                            }
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if texts.is_empty() {
            return;
        }
        if triage::refresh_session_proposals(&self.memory, session_id, &texts, false).is_err() {
            // Best-effort only; the session is never affected.
        }
    }

    /// Phase 8 parity: answer a classified `/memory` triage verb in-band.
    fn memory_triage(&mut self, text: &str) -> Option<Block> {
        let parsed = match command::parse(command::prefix_remainder(text)) {
            Ok(command::Command::Suggest) => {
                self.refresh_proposals(&self.active_id());
                triage::suggest_text(&mut self.memory).map(|text| Block::Assistant { text })
            }
            Ok(command::Command::Confirm {
                target,
                scope,
                kind,
            }) => triage::confirm_proposal(&mut self.memory, &target, scope, kind)
                .map(|text| Block::Assistant { text }),
            Ok(command::Command::Discard {
                target,
                confirm_all,
            }) => triage::discard_proposal(&mut self.memory, target.as_ref(), confirm_all)
                .map(|text| Block::Assistant { text }),
            _ => return None,
        };
        Some(match parsed {
            Ok(block) => block,
            Err(error) => Block::Error {
                text: format!("memory: {error}"),
            },
        })
    }

    fn session_mut(&mut self, id: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    fn active_id(&self) -> String {
        self.sessions[self.active].id.clone()
    }

    fn active_mut(&mut self) -> &mut Session {
        let active = self.active;
        &mut self.sessions[active]
    }

    /// Split assistant text into deterministic word chunks (spaces preserved).
    fn chunks(text: &str) -> Vec<StreamEvent> {
        text.split_inclusive(' ')
            .map(|word| StreamEvent::Chunk(word.to_owned()))
            .collect()
    }

    fn push_turn(&mut self, blocks: Vec<Block>) {
        self.drop_working();
        self.active_mut().blocks.extend(blocks);
    }

    fn drop_working(&mut self) {
        if matches!(self.active_mut().blocks.last(), Some(Block::Working { .. })) {
            self.active_mut().blocks.pop();
        }
    }

    /// Enqueue a scripted scenario for the active session.
    pub fn run_scenario(&mut self, scenario: Scenario) {
        let sid = self.active_id();
        let events = match scenario {
            Scenario::Conversation => canned_turn(0).into_iter().map(StreamEvent::Push).collect(),
            Scenario::Streaming => {
                let mut events = vec![StreamEvent::Push(Block::Thinking {
                    text: "Streaming this answer word by word.".into(),
                })];
                events.extend(Self::chunks(
                    "Large responses arrive **progressively**: each timer tick appends the next word, so the transcript grows live — *no* blocking wait for the full text.",
                ));
                events
            }
            Scenario::ToolRun => vec![
                StreamEvent::Push(Block::Tool {
                    call: ToolCall {
                        name: "Shell".into(),
                        detail: "cargo build --workspace".into(),
                        state: ToolState::Running,
                        output: Vec::new(),
                        elapsed: "0s".into(),
                    },
                }),
                StreamEvent::WorkLabel("Compiling workspace…".into()),
                StreamEvent::UpdateTool {
                    state: ToolState::Done,
                    elapsed: "14.2s".into(),
                    output: vec!["Build succeeded in 14.2s".into()],
                },
            ],
            Scenario::Permission => vec![StreamEvent::Push(Block::Permission {
                req: PermissionRequest {
                    tool: "Edit src/sync/client.rs".into(),
                    summary: "Modify 1 file · +34 −6".into(),
                },
            })],
            Scenario::Question => vec![StreamEvent::Push(Block::Question {
                q: Question {
                    prompt: "Which backoff strategy should the helper use?".into(),
                    options: vec![
                        "Equal jitter (recommended)".into(),
                        "Full jitter".into(),
                        "No backoff".into(),
                    ],
                },
            })],
            Scenario::Shell => vec![
                StreamEvent::Push(Block::Shell {
                    run: ShellRun {
                        command: "cargo test sync::client".into(),
                        output: Vec::new(),
                        state: ToolState::Running,
                    },
                }),
                StreamEvent::ShellLine("running 6 tests".into()),
                StreamEvent::ShellLine("test sync::client::retry_ok ... ok".into()),
                StreamEvent::ShellLine("test result: ok. 6 passed; 0 failed".into()),
                StreamEvent::UpdateTool {
                    state: ToolState::Done,
                    elapsed: "3.1s".into(),
                    output: Vec::new(),
                },
            ],
            Scenario::FileEdit => canned_turn(1).into_iter().map(StreamEvent::Push).collect(),
            Scenario::Plan => vec![
                StreamEvent::Push(Block::Working {
                    label: "Planning rollout…".into(),
                }),
                StreamEvent::Push(Block::Plan {
                    title: "Rollout plan".into(),
                    body: "Land the helper behind the existing flag, then enable it per endpoint."
                        .into(),
                }),
            ],
            Scenario::ErrorRecovery => vec![
                StreamEvent::Push(Block::Tool {
                    call: ToolCall {
                        name: "Lint".into(),
                        detail: "cargo clippy --workspace".into(),
                        state: ToolState::Running,
                        output: Vec::new(),
                        elapsed: "0s".into(),
                    },
                }),
                StreamEvent::UpdateTool {
                    state: ToolState::Failed,
                    elapsed: "3.1s".into(),
                    output: vec!["warning: unused import `Instant`".into()],
                },
                StreamEvent::Push(Block::Error {
                    text: "warning: unused import `Instant` in src/sync/retry.rs".into(),
                }),
                StreamEvent::Push(Block::Assistant {
                    text: "Fixed — removed the unused import and re-ran clippy clean.".into(),
                }),
            ],
            Scenario::MultiSession => {
                let id = self.new_session("review".into());
                let review = self.session_mut(&id).expect("just created");
                review.blocks.push(Block::User {
                    text: "review the retry helper diff".into(),
                });
                review.blocks.push(Block::Assistant {
                    text: "Reviewed in a **separate session** — switch back with `ctrl-p`.".into(),
                });
                self.set_active(0);
                return;
            }
        };
        self.push_stream(sid, events);
    }
}

impl Backend for MockBackend {
    fn session_summaries(&self) -> Vec<SessionSummary> {
        self.sessions
            .iter()
            .map(|session| SessionSummary {
                id: session.id.clone(),
                title: session.title.clone(),
            })
            .collect()
    }

    fn session_blocks(&self, id: &str) -> Vec<Block> {
        self.sessions
            .iter()
            .find(|session| session.id == id)
            .map(|session| session.blocks.clone())
            .unwrap_or_default()
    }

    fn active(&self) -> usize {
        self.active
    }

    fn set_active(&mut self, index: usize) {
        if index < self.sessions.len() {
            let leaving = self.sessions[self.active].id.clone();
            self.active = index;
            // Phase 8 parity: leaving a session ends it for extraction.
            if self.sessions[self.active].id != leaving {
                self.refresh_proposals(&leaving);
            }
        }
    }

    fn blocker(&self) -> Option<Blocker> {
        match self.sessions[self.active].blocks.last() {
            Some(Block::Permission { .. }) => Some(Blocker::Permission),
            Some(Block::Question { .. }) => Some(Blocker::Question),
            _ => None,
        }
    }

    fn commands(&self) -> Vec<SlashCommand> {
        vec![
            SlashCommand {
                name: "build".into(),
                description: "Build the workspace".into(),
            },
            SlashCommand {
                name: "test".into(),
                description: "Run the test suite".into(),
            },
            SlashCommand {
                name: "review".into(),
                description: "Review the working tree diff".into(),
            },
            SlashCommand {
                name: "init".into(),
                description: "Guided AGENTS.md setup (writes files)".into(),
            },
            SlashCommand {
                name: "plan".into(),
                description: "Draft an implementation plan".into(),
            },
            SlashCommand {
                name: "status".into(),
                description: "Show session status".into(),
            },
            SlashCommand {
                name: "usage".into(),
                description: "Show context usage".into(),
            },
            SlashCommand {
                name: "model".into(),
                description: "Switch the agent model".into(),
            },
            SlashCommand {
                name: "demo".into(),
                description: "Run a UI demo scenario (a–j)".into(),
            },
            SlashCommand {
                name: "new".into(),
                description: "Open a new session".into(),
            },
            SlashCommand {
                name: "help".into(),
                description: "List available commands".into(),
            },
        ]
    }

    fn status(&self) -> StatusInfo {
        let working = self.agent_status() == AgentStatus::Working;
        StatusInfo {
            model: "mock-sonnet".into(),
            context_pct: 34,
            cwd: "~/opencode-warp-tui".into(),
            branch: "main".into(),
            status: if working {
                AgentStatus::Working
            } else {
                AgentStatus::Idle
            },
        }
    }

    fn submit(&mut self, text: String) {
        let trimmed = text.trim().to_owned();
        if trimmed.is_empty() {
            return;
        }
        let sid = self.active_id();
        // Local slash routes (a real backend would route these).
        if trimmed == "/status" {
            self.push_turn(vec![
                Block::User { text: trimmed },
                Block::Assistant {
                    text: "Session `agent/session` · model **mock-sonnet** · context **34%** used."
                        .into(),
                },
            ]);
            return;
        }
        if trimmed == "/usage" {
            self.push_turn(vec![
                Block::User { text: trimmed },
                Block::Assistant {
                    text: "Context window **34%** used · 66% remaining · mock billing disabled."
                        .into(),
                },
            ]);
            return;
        }
        if trimmed == "/new" {
            let n = self.sessions.len() + 1;
            let id = self.new_session(format!("session {n}"));
            let session = self.session_mut(&id).expect("just created");
            session.blocks.push(Block::User { text: trimmed });
            session.blocks.push(Block::Assistant {
                text: "Fresh session — ask anything.".into(),
            });
            return;
        }
        // Phase 8 parity: triage verbs run the shared engine; every other
        // /memory verb keeps the historical canned-turn behavior.
        if matches!(command::classify(&trimmed), command::Classified::Command) {
            if let Some(reply) = self.memory_triage(&trimmed) {
                self.push_turn(vec![Block::User { text: trimmed }, reply]);
                return;
            }
        }
        // Phase 7C slash verbs (mirrors the opencode text routing).
        if let Some(rest) = trimmed
            .strip_prefix('/')
            .filter(|rest| !rest.starts_with('/'))
        {
            let mut parts = rest.splitn(2, char::is_whitespace);
            let verb = parts.next().unwrap_or("");
            let args = parts.next().unwrap_or("").trim();
            let handled = match verb {
                "diff" => {
                    self.diff();
                    true
                }
                "compact" => {
                    self.compact();
                    true
                }
                "fork" => {
                    self.fork(if args.is_empty() {
                        None
                    } else {
                        Some(args.to_owned())
                    });
                    true
                }
                "revert" => {
                    self.stage_revert(args.to_owned());
                    true
                }
                "model" => {
                    self.switch_model(args.to_owned());
                    true
                }
                "agent" => {
                    self.switch_agent(args.to_owned());
                    true
                }
                _ => {
                    if self.commands().iter().any(|command| command.name == verb) {
                        self.execute_command(verb.to_owned(), args.to_owned());
                        true
                    } else {
                        false
                    }
                }
            };
            if handled {
                return;
            }
        }
        if let Some(letter) = trimmed.strip_prefix("/demo") {
            let scenario = Scenario::from_letter(letter.trim());
            self.push_turn(vec![Block::User { text: trimmed }]);
            match scenario {
                Some(scenario) => self.run_scenario(scenario),
                None => self.push_turn(vec![Block::Assistant {
                    text: "Usage: `/demo <letter>` with a–j (see `?` shortcuts).".into(),
                }]),
            }
            return;
        }
        // Default: user block now, scripted answer streamed progressively.
        self.push_turn(vec![Block::User { text: trimmed }]);
        let turn = self.turn;
        self.turn += 1;
        let mut events = vec![StreamEvent::Push(Block::Working {
            label: "Warp is working…".into(),
        })];
        for block in canned_turn(turn % 2) {
            match block {
                Block::Assistant { text } => {
                    events.push(StreamEvent::WorkLabel("Composing answer…".into()));
                    events.extend(Self::chunks(&text));
                }
                other => events.push(StreamEvent::Push(other)),
            }
        }
        // A fresh answer supersedes the spinner: drop it at stream end.
        events.push(StreamEvent::WorkLabel(String::new()));
        self.push_stream(sid, events);
    }

    fn resolve_permission(&mut self, accept: bool) {
        if !matches!(
            self.active_mut().blocks.last(),
            Some(Block::Permission { .. })
        ) {
            return;
        }
        self.active_mut().blocks.pop();
        self.push_turn(vec![Block::Assistant {
            text: if accept {
                "Approved — applying the edit.".into()
            } else {
                "Denied — skipped the edit. Tell me how to proceed.".into()
            },
        }]);
    }

    fn answer_question(&mut self, option: usize) {
        // Phase 7C confirm gates take precedence over the generic gate.
        if self.pending_revert.is_some() {
            let commit = option == 0;
            self.pending_revert = None;
            self.active_mut().blocks.pop();
            self.push_turn(vec![Block::Notice {
                text: if commit {
                    "Reverted to the selected message.".into()
                } else {
                    "Revert abandoned — messages kept.".into()
                },
                link: None,
            }]);
            return;
        }
        if let Some((name, _)) = self.pending_command.clone() {
            self.pending_command = None;
            self.active_mut().blocks.pop();
            if option == 0 {
                self.push_turn(vec![
                    Block::User {
                        text: format!("/{name}"),
                    },
                    Block::Assistant {
                        text: format!("Ran {name}."),
                    },
                ]);
            } else {
                self.push_turn(vec![Block::Notice {
                    text: "Command cancelled.".into(),
                    link: None,
                }]);
            }
            return;
        }
        let answer = match self.active_mut().blocks.last() {
            Some(Block::Question { q }) => q.options.get(option).cloned(),
            _ => None,
        };
        let Some(answer) = answer else {
            return;
        };
        self.active_mut().blocks.pop();
        self.push_turn(vec![Block::Assistant {
            text: format!("Using **{answer}** for the rollout."),
        }]);
    }

    fn compact(&mut self) {
        self.push_turn(vec![Block::Notice {
            text: "Compacted session.".into(),
            link: None,
        }]);
    }

    fn diff(&mut self) {
        self.push_turn(vec![Block::Edits {
            files: vec![FileDiff {
                path: "notes.txt".into(),
                added: 1,
                removed: 0,
                lines: vec![(false, "line one".into()), (true, "probe-change-1".into())],
            }],
        }]);
    }

    fn stage_revert(&mut self, message_id: String) {
        if message_id.trim().is_empty() {
            self.push_turn(vec![Block::Error {
                text: "revert: usage `/revert <message-id>`.".into(),
            }]);
            return;
        }
        self.pending_revert = Some(message_id.clone());
        self.push_turn(vec![
            Block::Edits {
                files: vec![FileDiff {
                    path: "notes.txt".into(),
                    added: 0,
                    removed: 1,
                    lines: vec![(false, "probe-change-1".into())],
                }],
            },
            Block::Question {
                q: Question {
                    prompt: format!(
                        "Files restored (1). Committing deletes messages after {message_id} — irreversible. Commit?"
                    ),
                    options: vec!["Commit".into(), "Keep messages".into()],
                },
            },
        ]);
    }

    fn commit_revert(&mut self) {
        if self.pending_revert.is_none() {
            self.push_turn(vec![Block::Error {
                text: "revert: no staged boundary.".into(),
            }]);
            return;
        }
        self.pending_revert = None;
        if matches!(
            self.active_mut().blocks.last(),
            Some(Block::Question { .. })
        ) {
            self.active_mut().blocks.pop();
        }
        self.push_turn(vec![Block::Notice {
            text: "Reverted to the selected message.".into(),
            link: None,
        }]);
    }

    fn abandon_revert(&mut self) {
        if self.pending_revert.is_none() {
            self.push_turn(vec![Block::Error {
                text: "revert: no staged boundary.".into(),
            }]);
            return;
        }
        self.pending_revert = None;
        if matches!(
            self.active_mut().blocks.last(),
            Some(Block::Question { .. })
        ) {
            self.active_mut().blocks.pop();
        }
        self.push_turn(vec![Block::Notice {
            text: "Revert abandoned — messages kept.".into(),
            link: None,
        }]);
    }

    fn fork(&mut self, before: Option<String>) {
        if let Some(before) = &before {
            if before != "msg_1" {
                self.push_turn(vec![Block::Error {
                    text: format!("fork: unknown message {before}."),
                }]);
                return;
            }
        }
        let parent = self.active_id();
        let id = self.fresh_id();
        let blocks = self
            .session_mut(&parent)
            .map(|session| session.blocks.clone())
            .unwrap_or_default();
        self.sessions.push(Session {
            id: id.clone(),
            title: format!("{} (fork #1)", self.sessions[self.active].title),
            blocks,
        });
        // Parent preserved, active unchanged; the child hydrates lazily.
        self.push_turn(vec![Block::Notice {
            text: format!("Forked to {id}."),
            link: None,
        }]);
    }

    fn switch_model(&mut self, id: String) {
        if !Self::mock_models().contains(&id.as_str()) {
            self.push_turn(vec![Block::Error {
                text: format!("Unknown model: {id}. Available: mock-sonnet"),
            }]);
            return;
        }
        self.push_turn(vec![Block::Notice {
            text: format!("Model switched to {id}."),
            link: None,
        }]);
    }

    fn switch_agent(&mut self, id: String) {
        if !Self::mock_agents().contains(&id.as_str()) {
            self.push_turn(vec![Block::Error {
                text: format!("Unknown agent: {id}. Available: build, general"),
            }]);
            return;
        }
        self.push_turn(vec![Block::Notice {
            text: format!("Agent switched to {id}."),
            link: None,
        }]);
    }

    fn execute_command(&mut self, name: String, text: String) {
        let known = self.commands().iter().any(|command| command.name == name);
        if !known {
            self.push_turn(vec![Block::Error {
                text: format!("Unknown command: {name}."),
            }]);
            return;
        }
        if name == "init" && self.pending_command.is_none() {
            self.pending_command = Some((name.clone(), text));
            self.push_turn(vec![Block::Question {
                q: Question {
                    prompt: "The `init` command writes workspace files. Run it?".into(),
                    options: vec!["Run".into(), "Cancel".into()],
                },
            }]);
            return;
        }
        self.pending_command = None;
        let line = format!(
            "/{name}{}",
            if text.is_empty() {
                String::new()
            } else {
                format!(" {text}")
            }
        );
        self.push_turn(vec![
            Block::User { text: line },
            Block::Assistant {
                text: format!("Ran {name}."),
            },
        ]);
    }

    fn simulate_activity(&mut self) {
        let turn = self.turn;
        self.turn += 1;
        match turn % 4 {
            0 => self.run_scenario(Scenario::Plan),
            1 => self.run_scenario(Scenario::ToolRun),
            2 => self.run_scenario(Scenario::Permission),
            _ => self.run_scenario(Scenario::Question),
        }
    }

    fn new_session(&mut self, title: String) -> String {
        let id = self.fresh_id();
        self.sessions.push(Session {
            id: id.clone(),
            title,
            blocks: Vec::new(),
        });
        id
    }

    fn cancel(&mut self) {
        self.streams.clear();
        self.drop_working();
    }

    fn agent_status(&self) -> AgentStatus {
        if self.stream_active() {
            AgentStatus::Working
        } else {
            AgentStatus::Idle
        }
    }

    fn push_stream(&mut self, session: String, events: Vec<StreamEvent>) {
        if events.is_empty() {
            return;
        }
        self.streams.entry(session).or_default().extend(events);
    }

    fn poll_stream(&mut self) -> bool {
        // Drain one event per non-empty session queue so background sessions
        // progress while the user reads another tab.
        let ids: Vec<String> = self
            .streams
            .iter()
            .filter_map(|(id, queue)| (!queue.is_empty()).then_some(id.clone()))
            .collect();
        for id in ids {
            let event = self
                .streams
                .get_mut(&id)
                .and_then(|queue| queue.pop_front());
            if let Some(event) = event {
                if let Some(session) = self.session_mut(&id) {
                    super::stream::apply_event(&mut session.blocks, &event);
                }
            }
        }
        self.streams.retain(|_, queue| !queue.is_empty());
        self.stream_active()
    }

    fn stream_active(&self) -> bool {
        self.streams.values().any(|queue| !queue.is_empty())
    }
}

/// Scripted answer turns shared by submit and the conversation scenario.
fn canned_turn(turn: usize) -> Vec<Block> {
    match turn % 2 {
        0 => vec![
            Block::Thinking {
                text: "I'll reproduce the flow first, then patch the helper and add a regression test.".into(),
            },
            Block::Tool {
                call: ToolCall {
                    name: "Read".into(),
                    detail: "src/sync/client.rs".into(),
                    state: ToolState::Done,
                    output: Vec::new(),
                    elapsed: "0.4s".into(),
                },
            },
            Block::Assistant {
                text: "Found it — the helper drops the error. Added `retry_with_backoff` with equal jitter; see the [backoff docs](https://docs.rs/backoff) for the strategy table.\n- retries **5** attempts\n- base delay `100ms`, capped at `5s`\n- honors `Retry-After` when present".into(),
            },
        ],
        _ => vec![
            Block::Shell {
                run: ShellRun {
                    command: "cargo test sync::client".into(),
                    output: vec![
                        "running 6 tests".into(),
                        "test sync::client::retry_ok ... ok".into(),
                        "test result: ok. 6 passed; 0 failed".into(),
                    ],
                    state: ToolState::Done,
                },
            },
            Block::Edits {
                files: vec![FileDiff {
                    path: "src/sync/retry.rs".into(),
                    added: 34,
                    removed: 6,
                    lines: vec![
                        (false, "fn retry(op: Op) -> Result<Out> {".into()),
                        (true, "fn retry_with_backoff(op: Op, budget: Budget) -> Result<Out> {".into()),
                        (true, "    let mut delay = budget.base;".into()),
                        (false, "    loop { return op.run(); }".into()),
                        (true, "    loop { match op.run() {".into()),
                        (true, "        Err(e) if budget.allows() => sleep(jitter(delay)),".into()),
                    ],
                }],
            },
        ],
    }
}

fn agent_session(id: String) -> Session {
    Session {
        id,
        title: "agent/session".into(),
        blocks: vec![
            Block::Notice {
                text: "Connected to the mock backend · type / for commands · ? for shortcuts ·".into(),
                link: Some((
                    "mock backend docs".into(),
                    "https://example.invalid/mock-docs".into(),
                )),
            },
            Block::User {
                text: "add a retry helper with exponential backoff to the sync client".into(),
            },
            Block::Thinking {
                text: "I'll look at the sync client structure first, then add the helper with tests.".into(),
            },
            Block::Tool {
                call: ToolCall {
                    name: "Read".into(),
                    detail: "src/sync/client.rs".into(),
                    state: ToolState::Done,
                    output: Vec::new(),
                    elapsed: "0.4s".into(),
                },
            },
            Block::Assistant {
                text: "Done — added `retry_with_backoff` with equal jitter.\n- retries **5** attempts\n- base delay `100ms`, capped at `5s`".into(),
            },
            Block::Shell {
                run: ShellRun {
                    command: "cargo test sync::client".into(),
                    output: vec![
                        "running 6 tests".into(),
                        "test sync::client::retry_ok ... ok".into(),
                        "test result: ok. 6 passed; 0 failed".into(),
                    ],
                    state: ToolState::Done,
                },
            },
            Block::Edits {
                files: vec![FileDiff {
                    path: "src/sync/retry.rs".into(),
                    added: 34,
                    removed: 6,
                    lines: vec![
                        (false, "fn retry(op: Op) -> Result<Out> {".into()),
                        (true, "fn retry_with_backoff(op: Op, budget: Budget) -> Result<Out> {".into()),
                        (true, "    let mut delay = budget.base;".into()),
                    ],
                }],
            },
            Block::Plan {
                title: "Rollout plan".into(),
                body: "Land the helper behind the existing flag, then enable it per endpoint.".into(),
            },
            Block::Tool {
                call: ToolCall {
                    name: "Lint".into(),
                    detail: "cargo clippy --workspace".into(),
                    state: ToolState::Failed,
                    output: Vec::new(),
                    elapsed: "3.1s".into(),
                },
            },
            Block::Error {
                text: "warning: unused import `Instant` in src/sync/retry.rs".into(),
            },
            Block::Tool {
                call: ToolCall {
                    name: "Fetch".into(),
                    detail: "mcp docs server".into(),
                    state: ToolState::Waiting,
                    output: Vec::new(),
                    elapsed: "—".into(),
                },
            },
        ],
    }
}

fn shell_session(id: String) -> Session {
    Session {
        id,
        title: "dotfiles".into(),
        blocks: vec![
            Block::User {
                text: "check git status in ~/dotfiles".into(),
            },
            Block::Shell {
                run: ShellRun {
                    command: "git status --short".into(),
                    output: vec![
                        " M nvim/init.lua".into(),
                        "?? scripts/new-machine.sh".into(),
                    ],
                    state: ToolState::Done,
                },
            },
            Block::Assistant {
                text: "Two entries: one modified file and one untracked script.".into(),
            },
        ],
    }
}

fn gate_session(id: String) -> Session {
    Session {
        id,
        title: "gates".into(),
        blocks: vec![Block::Permission {
            req: PermissionRequest {
                tool: "Edit src/sync/client.rs".into(),
                summary: "Modify 1 file · +34 −6".into(),
            },
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::super::{AgentStatus, Backend, Block, Blocker, StreamEvent, ToolState};
    use super::{MockBackend, Scenario};
    fn active_id(backend: &MockBackend) -> String {
        let summaries = backend.session_summaries();
        summaries[backend.active()].id.clone()
    }

    fn active_blocks(backend: &MockBackend) -> Vec<Block> {
        let id = active_id(backend);
        backend.session_blocks(&id)
    }

    fn drain(backend: &mut MockBackend) {
        while backend.poll_stream() {}
    }

    #[test]
    fn stream_chunks_grow_one_assistant_block() {
        let mut backend = MockBackend::demo();
        let sid = active_id(&backend);
        backend.push_stream(
            sid,
            vec![
                StreamEvent::Chunk("hello ".into()),
                StreamEvent::Chunk("world".into()),
            ],
        );
        assert!(backend.stream_active());
        assert!(backend.poll_stream());
        assert!(!backend.poll_stream());
        assert!(!backend.stream_active());
        let blocks = active_blocks(&backend);
        let last = blocks.last().unwrap();
        match last {
            Block::Assistant { text } => assert_eq!(text, "hello world"),
            other => panic!("expected assistant block, got {other:?}"),
        }
    }

    #[test]
    fn submit_streams_answer_to_completion() {
        let mut backend = MockBackend::demo();
        let before = active_blocks(&backend).len();
        backend.submit("hi".into());
        assert!(backend.stream_active());
        assert_eq!(backend.agent_status(), AgentStatus::Working);
        drain(&mut backend);
        assert_eq!(backend.agent_status(), AgentStatus::Idle);
        assert!(active_blocks(&backend).len() > before + 1);
    }

    #[test]
    fn tool_state_updates_in_place() {
        let mut backend = MockBackend::demo();
        let before = active_blocks(&backend).len();
        backend.run_scenario(Scenario::ToolRun);
        drain(&mut backend);
        let blocks = active_blocks(&backend);
        let new_blocks = &blocks[before..];
        let tools: Vec<_> = new_blocks
            .iter()
            .filter_map(|block| match block {
                Block::Tool { call } => Some(call),
                _ => None,
            })
            .collect();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].state, ToolState::Done);
    }

    #[test]
    fn permission_gate_resolves() {
        let mut backend = MockBackend::demo();
        backend.run_scenario(Scenario::Permission);
        drain(&mut backend);
        assert_eq!(backend.blocker(), Some(Blocker::Permission));
        backend.resolve_permission(false);
        assert_eq!(backend.blocker(), None);
    }

    #[test]
    fn question_gate_answers() {
        let mut backend = MockBackend::demo();
        backend.run_scenario(Scenario::Question);
        drain(&mut backend);
        assert_eq!(backend.blocker(), Some(Blocker::Question));
        backend.answer_question(0);
        assert_eq!(backend.blocker(), None);
    }

    #[test]
    fn cancel_drops_pending_events() {
        let mut backend = MockBackend::demo();
        backend.run_scenario(Scenario::Streaming);
        assert!(backend.stream_active());
        backend.cancel();
        assert!(!backend.stream_active());
        assert_eq!(backend.agent_status(), AgentStatus::Idle);
    }

    #[test]
    fn sessions_switch_and_create() {
        let mut backend = MockBackend::demo();
        assert_eq!(backend.session_summaries().len(), 3);
        backend.set_active(2);
        assert_eq!(backend.blocker(), Some(Blocker::Permission));
        let id = backend.new_session("extra".into());
        backend.set_active(3);
        assert_eq!(active_id(&backend), id);
    }

    #[test]
    fn shell_scenario_streams_output_lines() {
        let mut backend = MockBackend::demo();
        let before = active_blocks(&backend).len();
        backend.run_scenario(Scenario::Shell);
        drain(&mut backend);
        let blocks = active_blocks(&backend);
        let new_blocks = &blocks[before..];
        let runs: Vec<_> = new_blocks
            .iter()
            .filter_map(|block| match block {
                Block::Shell { run } => Some(run),
                _ => None,
            })
            .collect();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].output.len(), 3);
        assert_eq!(runs[0].state, ToolState::Done);
    }

    #[test]
    fn compact_and_diff_push_blocks() {
        let mut backend = MockBackend::demo();
        backend.compact();
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Notice { .. })
        ));
        backend.diff();
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Edits { .. })
        ));
    }

    #[test]
    fn revert_stage_confirm_commit() {
        let mut backend = MockBackend::demo();
        backend.stage_revert("msg_1".into());
        assert_eq!(backend.blocker(), Some(Blocker::Question));
        backend.answer_question(0);
        assert_eq!(backend.blocker(), None);
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Notice { text, .. }) if text.contains("Reverted")
        ));
    }

    #[test]
    fn revert_stage_reject_abandons() {
        let mut backend = MockBackend::demo();
        backend.stage_revert("msg_1".into());
        backend.answer_question(1);
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Notice { text, .. }) if text.contains("abandoned")
        ));
        // No boundary left: committing now errors visibly.
        backend.commit_revert();
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Error { .. })
        ));
    }

    #[test]
    fn fork_registers_child_and_keeps_parent() {
        let mut backend = MockBackend::demo();
        let before = backend.session_summaries().len();
        let parent = active_id(&backend);
        backend.fork(None);
        assert_eq!(backend.session_summaries().len(), before + 1);
        assert_eq!(active_id(&backend), parent);
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Notice { text, .. }) if text.contains("Forked")
        ));
        backend.fork(Some("msg_bogus".into()));
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Error { .. })
        ));
    }

    #[test]
    fn switch_validation_rejects_unknown() {
        let mut backend = MockBackend::demo();
        backend.switch_model("bogus".into());
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Error { text }) if text.contains("bogus")
        ));
        backend.switch_model("mock-sonnet".into());
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Notice { text, .. }) if text.contains("mock-sonnet")
        ));
        backend.switch_agent("bogus".into());
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Error { .. })
        ));
        backend.switch_agent("general".into());
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Notice { text, .. }) if text.contains("general")
        ));
    }

    #[test]
    fn commands_unknown_gates_writer() {
        let mut backend = MockBackend::demo();
        backend.execute_command("nope".into(), String::new());
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Error { text }) if text.contains("Unknown command")
        ));
        // Writer command gates; cancel leaves no POST behind.
        backend.execute_command("init".into(), String::new());
        assert_eq!(backend.blocker(), Some(Blocker::Question));
        backend.answer_question(1);
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Notice { text, .. }) if text.contains("cancelled")
        ));
        // Read-only command runs immediately.
        backend.execute_command("review".into(), String::new());
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Assistant { text }) if text.contains("Ran review")
        ));
    }

    #[test]
    fn slash_verbs_route_to_ops() {
        let mut backend = MockBackend::demo();
        backend.submit("/diff".into());
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Edits { .. })
        ));
        backend.submit("/model bogus".into());
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Error { .. })
        ));
        // Unknown /words keep the historical prompt-turn behavior.
        let before = active_blocks(&backend).len();
        backend.submit("/tmp path".into());
        drain(&mut backend);
        assert!(active_blocks(&backend).len() > before);
    }

    #[test]
    fn memory_triage_suggest_confirm_flow() {
        let mut backend = MockBackend::demo();
        // A user statement enters through the normal submit path, then
        // suggest refreshes from this session's user blocks.
        backend.submit("Always file triage paperwork first.".into());
        drain(&mut backend);
        backend.submit("/memory suggest".into());
        let last = active_blocks(&backend).last().cloned();
        match last {
            Some(Block::Assistant { text }) => {
                assert!(text.contains("quarantined"), "got: {text}");
                assert!(text.contains("Always file triage paperwork first."));
            }
            other => panic!("expected suggest reply, got {other:?}"),
        }
        backend.submit("/memory confirm 1".into());
        match active_blocks(&backend).last() {
            Some(Block::Assistant { text }) => assert!(text.contains("confirmed"), "got: {text}"),
            other => panic!("expected confirm reply, got {other:?}"),
        }
        // Queue drained; a second suggest finds nothing pending.
        backend.submit("/memory suggest".into());
        match active_blocks(&backend).last() {
            Some(Block::Assistant { text }) => {
                assert!(text.contains("no pending proposals"), "got: {text}")
            }
            other => panic!("expected empty suggest, got {other:?}"),
        }
    }

    #[test]
    fn memory_triage_discard_flow() {
        let mut backend = MockBackend::demo();
        backend.submit("Never skip the changelog entry.".into());
        drain(&mut backend);
        backend.submit("/memory suggest".into());
        assert!(matches!(
            active_blocks(&backend).last(),
            Some(Block::Assistant { .. })
        ));
        backend.submit("/memory discard 1".into());
        match active_blocks(&backend).last() {
            Some(Block::Assistant { text }) => assert!(text.contains("discarded"), "got: {text}"),
            other => panic!("expected discard reply, got {other:?}"),
        }
    }
}
