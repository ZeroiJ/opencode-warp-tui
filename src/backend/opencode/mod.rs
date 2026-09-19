//! OpenCode adapter: `OpenCodeBackend` implements the generic [`Backend`]
//! trait against a live OpenCode server. The TUI never sees OpenCode types.
//!
//! Threading: one SSE subscriber thread forwards raw envelopes over a
//! channel; `poll_stream` (called every frame by the transcript pump)
//! translates them via [`mapper`] and applies them with the shared
//! [`stream`](super::stream) applier — the same generic system `MockBackend`
//! uses. REST calls are blocking and quick (single roundtrips with
//! timeouts); every snapshot method serves memory so `render` never blocks.

pub mod client;
pub mod config;
pub mod events;
pub mod mapper;

pub use config::OpenCodeConfig;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::stream::apply_event;
use super::{
    memory::{api::ListFilter, command, MemoryApi},
    AgentStatus, Backend, Block, Blocker, SessionSummary, SlashCommand, StatusInfo, StreamEvent,
};
use client::{Client, ClientError};
use config::{Endpoint, EndpointSource, PrivateServer};
use events::{RawEvent, SseWorker};
use mapper::DraftState;

/// How the backend is reaching OpenCode (status/debugging only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionKind {
    Explicit,
    Discovered,
    Spawned,
}

impl From<EndpointSource> for ConnectionKind {
    fn from(source: EndpointSource) -> Self {
        match source {
            EndpointSource::Explicit => ConnectionKind::Explicit,
            EndpointSource::Discovered => ConnectionKind::Discovered,
            EndpointSource::Spawned => ConnectionKind::Spawned,
        }
    }
}

/// Live state behind the snapshot methods. Only `poll_stream` and action
/// handlers mutate; snapshot readers clone small data out.
#[derive(Debug, Default)]
struct State {
    summaries: Vec<SessionSummary>,
    infos: HashMap<String, Value>,
    blocks: HashMap<String, Vec<Block>>,
    active: usize,
    drafts: HashMap<String, DraftState>,
    busy: HashSet<String>,
    pending_permissions: HashMap<String, String>,
    _pending_questions: HashMap<String, String>,
    commands: Vec<SlashCommand>,
}

impl State {
    fn active_id(&self) -> Option<String> {
        self.summaries
            .get(self.active)
            .map(|summary| summary.id.clone())
    }
}

pub struct OpenCodeBackend {
    state: Arc<Mutex<State>>,
    client: Client,
    receiver: std::sync::mpsc::Receiver<RawEvent>,
    _worker: SseWorker,
    _server: Option<PrivateServer>,
    _connection: ConnectionKind,
    /// Memory engine (Phase 5): `/memory` commands are handled entirely
    /// locally; the session never sees them. Infallible to construct — a
    /// store failure disables memory, never the connection.
    memory: MemoryApi,
}

#[derive(Debug)]
pub enum ConnectError {
    Config(config::ConfigError),
    Client(ClientError),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::Config(e) => write!(f, "{e}"),
            ConnectError::Client(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConnectError {}

impl OpenCodeBackend {
    /// Connect: resolve endpoint → verify → load sessions → subscribe.
    pub fn connect(config: &OpenCodeConfig) -> Result<Self, ConnectError> {
        let (endpoint, kind, server) = Self::resolve_endpoint(config)?;
        let client = Client::new(&endpoint);
        client.health().map_err(ConnectError::Client)?;
        let (sender, receiver) = std::sync::mpsc::channel::<RawEvent>();
        let worker = Self::subscribe(&endpoint, sender);
        // Phase 5: build the local memory engine. Infallible — failure is
        // logged and remembered (memory commands then report errors, but the
        // connection proceeds).
        let memory = MemoryApi::open(
            super::memory::store::user_store_dir(),
            config
                .project_dir
                .clone()
                .or_else(|| std::env::current_dir().ok()),
        );
        let mut backend = Self {
            state: Arc::new(Mutex::new(State::default())),
            client,
            receiver,
            _worker: worker,
            _server: server,
            _connection: kind,
            memory,
        };
        backend.initial_load()?;
        Ok(backend)
    }

    fn resolve_endpoint(
        config: &OpenCodeConfig,
    ) -> Result<(Endpoint, ConnectionKind, Option<PrivateServer>), ConnectError> {
        if let Some(url) = config.server_url.clone() {
            return Ok((
                Endpoint {
                    url,
                    password: config.password.clone(),
                },
                EndpointSource::Explicit.into(),
                None,
            ));
        }
        if let Some(endpoint) = config::read_service_registration() {
            let probe = Client::new(&endpoint);
            if probe.health().is_ok() {
                return Ok((endpoint, EndpointSource::Discovered.into(), None));
            }
            log::warn!("registered OpenCode service unhealthy; spawning private server");
        }
        let server = PrivateServer::spawn(config).map_err(ConnectError::Config)?;
        let endpoint = server.endpoint.clone();
        Client::new(&endpoint)
            .health()
            .map_err(ConnectError::Client)?;
        Ok((endpoint, EndpointSource::Spawned.into(), Some(server)))
    }

    fn subscribe(endpoint: &Endpoint, sender: std::sync::mpsc::Sender<RawEvent>) -> SseWorker {
        let url = format!("{}/api/event", endpoint.url.trim_end_matches('/'));
        let password = endpoint.password.clone();
        SseWorker::spawn(move || open_sse(&url, password.as_deref()), sender)
    }

    /// Initial load: session list + history for the active session + command
    /// list. Other sessions hydrate lazily on selection (histories can be
    /// large; one roundtrip per `set_active` at most).
    fn initial_load(&mut self) -> Result<(), ConnectError> {
        let list = self.client.list_sessions().map_err(ConnectError::Client)?;
        {
            let mut state = self.lock_state();
            state.summaries = list
                .get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|item| {
                    let session = mapper::map_session(item);
                    (session.id, session.title, item.clone())
                })
                .filter(|(id, _, _)| !id.is_empty())
                .map(|(id, title, item)| {
                    state.infos.insert(id.clone(), item);
                    SessionSummary { id, title }
                })
                .collect();
        }
        if self.with_state(|state| state.summaries.is_empty()) {
            let created = self
                .client
                .create_session("New session")
                .map_err(ConnectError::Client)?;
            let (id, title) = created
                .get("data")
                .map(|data| {
                    (
                        data.get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                        data.get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("New session")
                            .to_owned(),
                    )
                })
                .unwrap_or_default();
            if !id.is_empty() {
                let mut state = self.lock_state();
                state.summaries.push(SessionSummary {
                    id: id.clone(),
                    title,
                });
                state.blocks.insert(id, Vec::new());
            }
        }
        self.hydrate_active();
        let commands = self
            .client
            .list_commands()
            .ok()
            .map(|value| mapper::map_commands(&value))
            .unwrap_or_default();
        self.lock_state().commands = commands;
        Ok(())
    }

    /// Load message history into the active session's blocks (once), and
    /// refresh its recorded info (title/model can change server-side).
    fn hydrate_active(&self) {
        let id = match self.with_state(|state| state.active_id()) {
            Some(id) => id,
            None => return,
        };
        if let Ok(info) = self.client.get_session(&id) {
            if let Some(info) = info.get("data") {
                if let Ok(mut state) = self.state.lock() {
                    state.infos.insert(id.clone(), info.clone());
                    if let Some(title) = info.get("title").and_then(Value::as_str) {
                        if let Some(summary) =
                            state.summaries.iter_mut().find(|summary| summary.id == id)
                        {
                            summary.title = title.to_owned();
                        }
                    }
                }
            }
        }
        let known = self.with_state(|state| {
            state
                .blocks
                .get(&id)
                .map(|blocks| !blocks.is_empty())
                .unwrap_or(false)
        });
        if known {
            return;
        }
        let blocks = self
            .client
            .list_messages(&id)
            .ok()
            .and_then(|messages| messages.get("data").cloned())
            .and_then(|data| data.as_array().cloned())
            .map(|messages| {
                messages
                    .iter()
                    .flat_map(mapper::map_message)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Ok(mut state) = self.state.lock() {
            state.blocks.insert(id, blocks);
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("backend state poisoned")
    }

    fn with_state<T>(&self, f: impl FnOnce(&State) -> T) -> T {
        f(&self.lock_state())
    }

    /// Drain raw worker events: translate (per-session drafts) and apply to
    /// blocks via the shared applier; track busy/gate side state.
    fn ingest(&self) {
        let raws: Vec<RawEvent> = {
            let mut raws = Vec::new();
            while let Ok(event) = self.receiver.try_recv() {
                raws.push(event);
            }
            raws
        };
        if raws.is_empty() {
            return;
        }
        if std::env::var("OWT_DEBUG_SSE").is_ok() {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/owt_sse.log")
            {
                for raw in &raws {
                    let _ = writeln!(f, "ingest {}", raw.typ);
                }
            }
        }
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        for raw in raws {
            let sid = session_of(&raw);
            let Some(sid) = sid.or_else(|| state.active_id()) else {
                continue;
            };
            // Busy tracking drives the statusline working dot.
            match raw.typ.as_str() {
                "session.next.step.started"
                | "session.step.started"
                | "session.execution.started" => {
                    state.busy.insert(sid.clone());
                }
                "session.next.step.ended"
                | "session.next.step.failed"
                | "session.step.ended"
                | "session.step.failed"
                | "session.execution.succeeded"
                | "session.execution.failed"
                | "session.idle" => {
                    state.busy.remove(&sid);
                }
                "permission.v2.asked" => {
                    if let Some(req) = raw.data.get("id").and_then(Value::as_str) {
                        state
                            .pending_permissions
                            .insert(sid.clone(), req.to_owned());
                    }
                }
                "permission.v2.replied" => {
                    state.pending_permissions.remove(&sid);
                }
                "question.v2.asked" => {
                    if let Some(req) = raw.data.get("id").and_then(Value::as_str) {
                        state._pending_questions.insert(sid.clone(), req.to_owned());
                    }
                }
                "question.v2.replied" | "question.v2.rejected" => {
                    state._pending_questions.remove(&sid);
                }
                "session.created" => {
                    // New remote session: add its shell (blocks hydrate on
                    // selection); ignore duplicates.
                    if let Some(id) = raw.data.get("sessionID").and_then(Value::as_str) {
                        if !state.summaries.iter().any(|summary| summary.id == id) {
                            let title = raw
                                .data
                                .get("info")
                                .and_then(|info| info.get("title"))
                                .and_then(Value::as_str)
                                .unwrap_or("Untitled")
                                .to_owned();
                            state.summaries.push(SessionSummary {
                                id: id.to_owned(),
                                title,
                            });
                        }
                    }
                    continue;
                }
                _ => {}
            }
            let draft = state.drafts.entry(sid.clone()).or_default();
            let events = mapper::map_event(&raw.typ, &raw.data, draft);
            let blocks = state.blocks.entry(sid).or_default();
            for event in &events {
                apply_event(blocks, event);
            }
        }
    }
}

/// Best-effort session attribution for a raw event: explicit `sessionID`
/// field, else the active session (most session-scoped events carry one).
fn session_of(raw: &RawEvent) -> Option<String> {
    raw.data
        .get("sessionID")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn open_sse(url: &str, password: Option<&str>) -> Option<Box<dyn std::io::BufRead + Send>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut request = agent.get(url);
    if let Some(password) = password {
        request = request.header(
            "Authorization",
            &format!(
                "Basic {}",
                crate::backend::opencode::client::basic_auth("opencode", password)
            ),
        );
    }
    let response = request.call().ok()?;
    if !(200..300).contains(&response.status().as_u16()) {
        return None;
    }
    Some(Box::new(std::io::BufReader::new(
        response.into_body().into_reader(),
    )))
}

impl Backend for OpenCodeBackend {
    fn session_summaries(&self) -> Vec<SessionSummary> {
        self.with_state(|state| state.summaries.clone())
    }

    fn session_blocks(&self, id: &str) -> Vec<Block> {
        self.with_state(|state| state.blocks.get(id).cloned().unwrap_or_default())
    }

    fn active(&self) -> usize {
        self.with_state(|state| state.active)
    }

    fn set_active(&mut self, index: usize) {
        let known = self.with_state(|state| index < state.summaries.len());
        if known {
            if let Ok(mut state) = self.state.lock() {
                state.active = index;
            }
            self.hydrate_active();
        }
    }

    fn blocker(&self) -> Option<Blocker> {
        use super::Blocker;
        self.with_state(|state| {
            let id = state.active_id()?;
            match state.blocks.get(&id)?.last()? {
                Block::Permission { .. } => Some(Blocker::Permission),
                Block::Question { .. } => Some(Blocker::Question),
                _ => None,
            }
        })
    }

    fn commands(&self) -> Vec<SlashCommand> {
        let commands = self.with_state(|state| state.commands.clone());
        if commands.is_empty() {
            vec![SlashCommand {
                name: "help".into(),
                description: "List available commands".into(),
            }]
        } else {
            commands
        }
    }

    fn status(&self) -> StatusInfo {
        self.with_state(|state| {
            let id = state.active_id().unwrap_or_default();
            let info = state.infos.get(&id).cloned().unwrap_or(Value::Null);
            let mut status = mapper::map_status(&info);
            if state.busy.contains(&id) {
                status.status = AgentStatus::Working;
            }
            status
        })
    }

    fn submit(&mut self, text: String) {
        let trimmed = text.trim().to_owned();
        if trimmed.is_empty() {
            return;
        }
        match command::classify(&trimmed) {
            // `\/memory …` → forward literally (backslash consumed); the
            // agent sees ordinary text, never a memory command.
            command::Classified::Escaped => self.submit_prompt(&trimmed[1..]),
            // `/memory …` / `/mem …` → handled locally, answered in-band.
            command::Classified::Command => self.run_memory_command(&trimmed),
            command::Classified::Normal => self.submit_prompt(&trimmed),
        }
    }

    fn resolve_permission(&mut self, accept: bool) {
        let found = self.with_state(|state| {
            let id = state.active_id()?;
            let blocks = state.blocks.get(&id)?;
            match blocks.last()? {
                Block::Permission { .. } => {
                    Some((id.clone(), state.pending_permissions.get(&id).cloned()))
                }
                _ => None,
            }
        });
        let Some((id, request)) = found else {
            return;
        };
        match request {
            Some(request) => match self.client.reply_permission(&id, &request, accept) {
                Ok(_) => {
                    if let Ok(mut state) = self.state.lock() {
                        state.pending_permissions.remove(&id);
                        if let Some(blocks) = state.blocks.get_mut(&id) {
                            blocks.pop();
                        }
                    }
                }
                Err(error) => {
                    log::warn!("permission reply rejected: {error}");
                    if let Ok(mut state) = self.state.lock() {
                        if let Some(blocks) = state.blocks.get_mut(&id) {
                            blocks.push(Block::Error {
                                text: format!("Permission reply rejected: {error}"),
                            });
                        }
                    }
                }
            },
            // Gate rendered without a tracked request id (e.g. restored from
            // history): resolve locally like the mock.
            None => {
                if let Ok(mut state) = self.state.lock() {
                    if let Some(blocks) = state.blocks.get_mut(&id) {
                        blocks.pop();
                        blocks.push(Block::Assistant {
                            text: if accept {
                                "Approved — applying the edit.".into()
                            } else {
                                "Denied — skipped the edit. Tell me how to proceed.".into()
                            },
                        });
                    }
                }
            }
        }
    }

    fn answer_question(&mut self, option: usize) {
        let found = self.with_state(|state| {
            let id = state.active_id()?;
            let blocks = state.blocks.get(&id)?;
            match blocks.last()? {
                Block::Question { q } => Some((id, q.options.get(option).cloned())),
                _ => None,
            }
        });
        let Some((id, answer)) = found else {
            return;
        };
        // No question-reply route exists on 1.18.31 (documented gap): record
        // the choice visibly instead of pretending to deliver it.
        if let Some(answer) = answer {
            log::warn!("question answer has no delivery route yet (Phase 5)");
            if let Ok(mut state) = self.state.lock() {
                if let Some(blocks) = state.blocks.get_mut(&id) {
                    blocks.push(Block::Notice {
                        text: format!("Chose “{answer}” — answer delivery lands in Phase 5."),
                        link: None,
                    });
                }
            }
        }
    }

    fn simulate_activity(&mut self) {}

    fn new_session(&mut self, title: String) -> String {
        let id = self
            .client
            .create_session(&title)
            .ok()
            .and_then(|created| {
                created
                    .get("data")
                    .and_then(|data| data.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("local-{}", self.with_state(|state| state.summaries.len())));
        if let Ok(mut state) = self.state.lock() {
            if !state.summaries.iter().any(|summary| summary.id == id) {
                state.summaries.push(SessionSummary {
                    id: id.clone(),
                    title,
                });
            }
            state.blocks.entry(id.clone()).or_default();
        }
        id
    }

    fn cancel(&mut self) {
        if let Some(id) = self.with_state(|state| state.active_id()) {
            if let Err(error) = self.client.interrupt(&id) {
                log::warn!("interrupt rejected: {error}");
            }
            if let Ok(mut state) = self.state.lock() {
                state.busy.remove(&id);
            }
        }
    }

    fn agent_status(&self) -> AgentStatus {
        self.with_state(|state| {
            let id = state.active_id().unwrap_or_default();
            if state.busy.contains(&id) {
                AgentStatus::Working
            } else {
                AgentStatus::Idle
            }
        })
    }

    fn push_stream(&mut self, session: String, events: Vec<StreamEvent>) {
        // Synthetic injection (tests/diagnostics): same path as worker
        // events, applied on the next pump.
        if events.is_empty() {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            let blocks = state.blocks.entry(session).or_default();
            for event in &events {
                apply_event(blocks, event);
            }
        }
    }

    fn poll_stream(&mut self) -> bool {
        self.ingest();
        // Contract parity with the mock: true while anything remains. The
        // live pump keys off `stream_active`, but tests reuse this signal.
        self.stream_active()
    }

    fn stream_active(&self) -> bool {
        // Worker events arrive continuously while a turn runs; the pump
        // repaints while the active session is busy.
        self.with_state(|state| state.active_id().is_some_and(|id| state.busy.contains(&id)))
    }
}

impl OpenCodeBackend {
    /// The ordinary prompt path: echo the User block, then send.
    fn submit_prompt(&mut self, text: &str) {
        let Some(id) = self.with_state(|state| state.active_id()) else {
            return;
        };
        if let Ok(mut state) = self.state.lock() {
            state
                .blocks
                .entry(id.clone())
                .or_default()
                .push(Block::User {
                    text: text.to_owned(),
                });
        }
        match self.client.send_prompt(&id, text) {
            Ok(_) => {
                if let Ok(mut state) = self.state.lock() {
                    state.busy.insert(id);
                }
            }
            Err(error) => {
                log::warn!("prompt rejected: {error}");
                if let Ok(mut state) = self.state.lock() {
                    if let Some(blocks) = state.blocks.get_mut(&id) {
                        blocks.push(Block::Error {
                            text: format!("Prompt rejected: {error}"),
                        });
                    }
                }
            }
        }
    }

    /// `/memory` handling: echo the typed line, run the engine, reply
    /// in-band (Assistant on success, Error on failure). The prompt never
    /// reaches OpenCode; a memory failure never takes the session down.
    fn run_memory_command(&mut self, text: &str) {
        let Some(id) = self.with_state(|state| state.active_id()) else {
            return;
        };
        if let Ok(mut state) = self.state.lock() {
            state
                .blocks
                .entry(id.clone())
                .or_default()
                .push(Block::User {
                    text: text.to_owned(),
                });
        }
        let reply = self.memory_reply(&id, text);
        if let Ok(mut state) = self.state.lock() {
            if let Some(blocks) = state.blocks.get_mut(&id) {
                blocks.push(reply);
            }
        }
    }

    /// Build the in-band reply block for a classified `/memory` command.
    fn memory_reply(&mut self, session_id: &str, text: &str) -> Block {
        match command::parse(command::prefix_remainder(text)) {
            Err(message) => Block::Error {
                text: format!("memory: {message}"),
            },
            Ok(command::Command::Help) => Block::Assistant {
                text: command::help_text().to_owned(),
            },
            Ok(command::Command::Remember(args)) => match self.memory.remember(session_id, &args) {
                Ok(result) => Block::Assistant {
                    text: command::written_reply("stored", &result),
                },
                Err(error) => Block::Error {
                    text: format!("memory: {error}"),
                },
            },
            Ok(command::Command::Update(args)) => match self.memory.update(session_id, &args) {
                Ok(result) => Block::Assistant {
                    text: command::written_reply("updated", &result),
                },
                Err(error) => Block::Error {
                    text: format!("memory: {error}"),
                },
            },
            Ok(command::Command::Forget { handle, scope }) => {
                match self.memory.forget(&handle, scope) {
                    Ok(outcome) => Block::Assistant {
                        text: command::forget_reply(&outcome),
                    },
                    Err(error) => Block::Error {
                        text: format!("memory: {error}"),
                    },
                }
            }
            Ok(command::Command::Pin {
                handle,
                pinned,
                scope,
            }) => match self.memory.set_pinned(&handle, pinned, scope) {
                Ok(record) => Block::Assistant {
                    text: command::pin_reply(&record, pinned),
                },
                Err(error) => Block::Error {
                    text: format!("memory: {error}"),
                },
            },
            Ok(command::Command::List {
                scope,
                kind,
                pinned,
            }) => {
                let filter = ListFilter {
                    scope,
                    kind,
                    pinned,
                };
                match self.memory.list(&filter) {
                    Ok(records) => Block::Assistant {
                        text: command::list_reply(&records),
                    },
                    Err(error) => Block::Error {
                        text: format!("memory: {error}"),
                    },
                }
            }
            Ok(command::Command::Show { handle, all, scope }) => {
                match self.memory.show(&handle, all, scope) {
                    Ok(records) => Block::Assistant {
                        text: command::show_reply(&records, &handle),
                    },
                    Err(error) => Block::Error {
                        text: format!("memory: {error}"),
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenCodeBackend, OpenCodeConfig};
    use crate::backend::Backend;

    /// Live read-only integration test. Requires a running server:
    /// `OWT_LIVE_SERVER_URL=http://127.0.0.1:PORT cargo test -- --ignored`.
    /// Makes no mutations (no sessions created, nothing submitted).
    #[test]
    #[ignore]
    fn live_server_read_paths() {
        let url = std::env::var("OWT_LIVE_SERVER_URL").expect("set OWT_LIVE_SERVER_URL");
        let backend = OpenCodeBackend::connect(&OpenCodeConfig {
            server_url: Some(url),
            ..Default::default()
        })
        .expect("connect");
        // Sessions load; summaries carry stable ids and titles.
        assert!(!backend.session_summaries().is_empty());
        for summary in backend.session_summaries() {
            assert!(summary.id.starts_with("ses_"));
        }
        // Commands degrade to the static fallback when the route is absent.
        let _ = backend.commands();
        let status = backend.status();
        assert!(!status.model.is_empty());
    }
}
