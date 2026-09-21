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
pub mod memory_inject;

pub use config::OpenCodeConfig;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::stream::apply_event;
use super::{
    memory::{api::ListFilter, command, extract, triage, MemoryApi},
    AgentStatus, Backend, Block, Blocker, FileDiff, Question, SessionSummary, SlashCommand,
    StatusInfo, StreamEvent,
};
use client::{Client, ClientError};
use config::{Endpoint, EndpointSource, MemoryInjection, PrivateServer};
use events::{RawEvent, SseWorker};
use mapper::DraftState;
use memory_inject::Injector;

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
    /// One answerable question gate per question field, in push order
    /// (the mapper renders one `Block::Question` per field). `answer_question`
    /// targets the trailing gate and pops the matching entry.
    pending_questions: HashMap<String, Vec<PendingQuestion>>,
    commands: Vec<SlashCommand>,
    /// SSE gap flag: set while the live stream is down so reconnect
    /// notices don't spam the transcript (one "restored" per drop).
    connection_gap: bool,
    /// Phase 7C: lazily-loaded discovery caches (None = not fetched yet).
    /// Validation runs against these so bogus model/agent ids never reach
    /// the server (which would 204-accept them into a dead turn).
    models: Option<Vec<DiscoveredModel>>,
    agents: Option<Vec<DiscoveredAgent>>,
    /// Phase 7C: staged revert boundary awaiting explicit confirmation
    /// (rendered as a Question gate; digits drive commit/abandon).
    pending_revert: Option<PendingRevert>,
    /// Phase 7C: file-writing slash command awaiting explicit confirmation.
    pending_command: Option<PendingCommand>,
}

/// Live question gate. The TUI renders labels (`Block::Question.options`);
/// the wire reply needs the form id, the field key and the option **values**
/// (which may differ from labels). A gate without a live form (restored from
/// history, or a `question.v2.asked` on a server with no form route)
/// resolves locally with a visible notice.
#[derive(Clone, Debug)]
struct PendingQuestion {
    form_id: String,
    field_key: String,
    option_values: Vec<String>,
}

/// Phase 7C: one discovered model (`GET /api/model` entry). Only identity
/// + usability signals are kept; validation matches `id`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoveredModel {
    id: String,
    provider_id: String,
}

/// Phase 7C: one discovered agent (`GET /api/agent` entry).
#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoveredAgent {
    id: String,
}

/// Phase 7C: staged revert boundary awaiting confirmation. `files` is the
/// actual staged `files[]` (never fabricated); commit deletes post-boundary
/// messages irreversibly, abandon clears via `DELETE /revert`.
#[derive(Clone, Debug)]
struct PendingRevert {
    message_id: String,
    files: Vec<RevertFile>,
}

/// One staged revert file (reverse patch + counts, as returned by stage).
#[derive(Clone, Debug)]
struct RevertFile {
    file: String,
    patch: String,
    additions: u32,
    deletions: u32,
    status: String,
}

/// Phase 7C: file-writing slash command awaiting confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingCommand {
    name: String,
    text: String,
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
    /// Phase 6B: session-start injection gate + capability cache. `off`
    /// restores byte-equivalent Phase 5 session behavior.
    injection: MemoryInjection,
    injector: Injector,
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
            injection: config.injection,
            injector: Injector::default(),
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
                state.blocks.insert(id.clone(), Vec::new());
                drop(state);
                // Phase 6B hook #1: brand-new auto-created session. Inline
                // PUT before the id escapes — no first prompt can precede it.
                // Resume paths (`set_active`, `hydrate_active`, SSE echoes)
                // never call this. Failure only warns; creation stands.
                if self.injection == MemoryInjection::Auto {
                    self.injector
                        .inject_new_session(&self.client, &self.memory, &id);
                }
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
        // Cursor-based pagination, oldest-first: 2.0.8 has no `offset` and
        // defaults to `desc` (newest first), which would render the
        // transcript inverted. Fetch `order=asc` and follow `cursor.next`
        // until exhausted, capped to bound the work.
        let mut blocks: Vec<Block> = Vec::new();
        let mut cursor: Option<String> = None;
        const MAX_HISTORY_PAGES: usize = 50;
        for _ in 0..MAX_HISTORY_PAGES {
            let page = match self
                .client
                .list_messages_paged(&id, cursor.as_deref(), Some(200))
            {
                Ok(page) => page,
                Err(error) => {
                    log::warn!("history page failed: {error}");
                    break;
                }
            };
            let messages = page
                .get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let next = page
                .get("cursor")
                .and_then(|cursor| cursor.get("next"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let mapped: Vec<Block> = messages.iter().flat_map(mapper::map_message).collect();
            if mapped.is_empty() && next.as_deref().unwrap_or_default().is_empty() {
                break;
            }
            blocks.extend(mapped);
            match next {
                Some(next) if !next.is_empty() => cursor = Some(next),
                _ => break,
            }
        }
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
            let Some(sid) = apply_raw(&mut state, &raw) else {
                continue;
            };
            let draft = state.drafts.entry(sid.clone()).or_default();
            let events = mapper::map_event(&raw.typ, &raw.data, draft);
            let blocks = state.blocks.entry(sid).or_default();
            for event in &events {
                apply_event(blocks, event);
            }
        }
    }
}

/// Apply one raw event's state transitions (busy flags, permission and
/// question gates, connection-gap reconciliation). Returns the session the
/// event should render into, or `None` when it must be skipped: synthetic
/// worker signals, session bookkeeping, or events with no session at all.
fn apply_raw(state: &mut State, raw: &RawEvent) -> Option<String> {
    let sid = session_of(raw);
    let sid = sid.or_else(|| state.active_id())?;
    // Busy tracking drives the statusline working dot.
    match raw.typ.as_str() {
        "session.next.step.started" | "session.step.started" | "session.execution.started" => {
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
        // Live 2.0.8 question gates: `form.created` with
        // `metadata.kind == "question"` arrives instead of
        // `question.v2.asked`. Record routing data per field so
        // `answer_question` can POST a form reply.
        "form.created" => {
            let pendings = pending_from_form(&raw.data);
            let list = state.pending_questions.entry(sid.clone()).or_default();
            list.extend(pendings);
        }
        "form.replied" | "form.rejected" => {
            if let Some(form_id) = raw.data.get("id").and_then(Value::as_str) {
                if let Some(list) = state.pending_questions.get_mut(&sid) {
                    list.retain(|pending| pending.form_id != form_id);
                    if list.is_empty() {
                        state.pending_questions.remove(&sid);
                    }
                }
            }
        }
        "question.v2.asked" => {
            // Older/SDK envelope (no live 2.0.8): the mapper still
            // renders the gate, but without a form id there is no
            // verified reply route — answering resolves locally.
        }
        "question.v2.replied" | "question.v2.rejected" => {
            state.pending_questions.remove(&sid);
        }
        // Synthetic worker signals (SSE gap): reconcile the busy flag so
        // the statusline never spins forever on a dropped stream, and
        // surface the gap in-band.
        "connection.lost" => {
            if !state.busy.is_empty() {
                state.busy.clear();
                state.connection_gap = true;
                if let Some(id) = state.active_id() {
                    if let Some(blocks) = state.blocks.get_mut(&id) {
                        blocks.push(Block::Notice {
                            text: "Connection to OpenCode lost — reconnecting…".into(),
                            link: None,
                        });
                    }
                }
            }
            return None;
        }
        "connection.restored" => {
            if state.connection_gap {
                state.connection_gap = false;
                if let Some(id) = state.active_id() {
                    if let Some(blocks) = state.blocks.get_mut(&id) {
                        blocks.push(Block::Notice {
                            text: "Reconnected to OpenCode.".into(),
                            link: None,
                        });
                    }
                }
            }
            return None;
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
            return None;
        }
        _ => {}
    }
    Some(sid)
}

/// Best-effort session attribution for a raw event: explicit `sessionID`
/// field, else the active session (most session-scoped events carry one).
/// `form.*` envelopes nest the session id under `form.sessionID`.
fn session_of(raw: &RawEvent) -> Option<String> {
    raw.data
        .get("sessionID")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            raw.data
                .get("form")
                .and_then(|form| form.get("sessionID"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

/// Extract answerable question gates from a `form.created` envelope whose
/// `metadata.kind` is "question". Returns one entry per question field
/// (the mapper pushes one `Block::Question` per field). Other form kinds
/// (parameter requests, tool metadata) yield nothing.
fn pending_from_form(data: &Value) -> Vec<PendingQuestion> {
    let form = match data.get("form") {
        Some(form) => form,
        None => return Vec::new(),
    };
    let kind = form
        .get("metadata")
        .and_then(|metadata| metadata.get("kind"))
        .and_then(Value::as_str);
    if kind != Some("question") {
        return Vec::new();
    }
    let form_id = form.get("id").and_then(Value::as_str).unwrap_or_default();
    let fields = form.get("fields").and_then(Value::as_array);
    let Some(fields) = fields else {
        return Vec::new();
    };
    fields
        .iter()
        .map(|field| PendingQuestion {
            form_id: form_id.to_owned(),
            field_key: field
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            option_values: field
                .get("options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| option.get("value").and_then(Value::as_str))
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

/// Phase 7C: per-file patch display cap (binary/truncation server behavior
/// is unverified — fail safe with a visible marker).
const MAX_DIFF_LINES_PER_FILE: usize = 120;

/// An entry is unusable when the discovery payload says so (`enabled:
/// false`, or a disabled/excluded/unavailable status).
fn model_usable(entry: &Value) -> bool {
    if entry.get("enabled").and_then(Value::as_bool) == Some(false) {
        return false;
    }
    !matches!(
        entry
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("active"),
        "disabled" | "excluded" | "unavailable"
    )
}

/// Parse `GET /api/model` (`{data: [{id, providerID, …}]}`) into usable
/// models only.
fn parse_models(value: &Value) -> Vec<DiscoveredModel> {
    value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|entry| model_usable(entry))
        .filter_map(|entry| {
            Some(DiscoveredModel {
                id: entry.get("id").and_then(Value::as_str)?.to_owned(),
                provider_id: entry.get("providerID").and_then(Value::as_str)?.to_owned(),
            })
        })
        .collect()
}

/// Parse `GET /api/agent` (`{data: [{id, …}]}`).
fn parse_agents(value: &Value) -> Vec<DiscoveredAgent> {
    value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| {
            Some(DiscoveredAgent {
                id: entry.get("id").and_then(Value::as_str)?.to_owned(),
            })
        })
        .collect()
}

/// Validate a model id against discovery (7C-R3). Returns the verified
/// `(model_id, provider_id)`; invalid selections never reach the server
/// (which would 204-accept them into a dead turn).
fn validate_model(models: &[DiscoveredModel], id: &str) -> Result<(String, String), String> {
    models
        .iter()
        .find(|model| model.id == id)
        .map(|model| (model.id.clone(), model.provider_id.clone()))
        .ok_or_else(|| {
            let mut known: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
            known.sort_unstable();
            let shown = known
                .iter()
                .take(10)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            let more = known.len().saturating_sub(10);
            let tail = if more > 0 {
                format!(" …and {more} more")
            } else {
                String::new()
            };
            format!("Unknown model: {id}. Available: {shown}{tail}")
        })
}

/// Validate an agent id against discovery (7C-R3).
fn validate_agent(agents: &[DiscoveredAgent], id: &str) -> Result<String, String> {
    agents
        .iter()
        .find(|agent| agent.id == id)
        .map(|agent| agent.id.clone())
        .ok_or_else(|| {
            let mut known: Vec<&str> = agents.iter().map(|agent| agent.id.as_str()).collect();
            known.sort_unstable();
            format!("Unknown agent: {id}. Available: {}", known.join(", "))
        })
}

/// Parse staged revert `files[]` (`data: {messageID, files: [{file, patch,
/// additions, deletions, status}]}`).
fn parse_revert_files(data: &Value) -> Vec<RevertFile> {
    data.get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|file| RevertFile {
            file: file
                .get("file")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_owned(),
            patch: file
                .get("patch")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            additions: file.get("additions").and_then(Value::as_u64).unwrap_or(0) as u32,
            deletions: file.get("deletions").and_then(Value::as_u64).unwrap_or(0) as u32,
            status: file
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        })
        .collect()
}

/// Render a unified patch into `(is_addition, line)` pairs for `FileDiff`.
/// Skips file headers (`diff --git`/`index`/`---`/`+++`), keeps hunk
/// headers + context as plain lines. Returns the lines plus whether the
/// cap truncated the output.
fn patch_lines(patch: &str, cap: usize) -> (Vec<(bool, String)>, bool) {
    let mut lines = Vec::new();
    for line in patch.lines() {
        if lines.len() >= cap {
            lines.push((false, "… (truncated)".to_owned()));
            return (lines, true);
        }
        if let Some(rest) = line.strip_prefix('+') {
            if rest.starts_with("++") {
                continue;
            }
            lines.push((true, rest.to_owned()));
        } else if let Some(rest) = line.strip_prefix('-') {
            if rest.starts_with("--") {
                continue;
            }
            lines.push((false, rest.to_owned()));
        } else if line.starts_with("diff --git") || line.starts_with("index ") {
            continue;
        } else {
            let rest = line.strip_prefix(' ').unwrap_or(line);
            lines.push((false, rest.to_owned()));
        }
    }
    (lines, false)
}

/// Slash commands that write files always confirm before execution.
/// Command metadata does not flag writability, so this list is explicit
/// (starts with `init`; unknown non-readonly commands also confirm).
const WRITER_COMMANDS: &[&str] = &["init"];
/// Commands with verified read-only behavior (R5-3: `review`).
const READONLY_COMMANDS: &[&str] = &["review"];

fn needs_command_confirm(name: &str) -> bool {
    WRITER_COMMANDS.contains(&name) || !READONLY_COMMANDS.contains(&name)
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

impl OpenCodeBackend {
    // Phase 7C: advanced agent operations. All REST contracts verified
    // live on 2.0.8 (see research/memory/phase7c-r5-probes.md). Every
    // failure is an in-band Error/Notice; the session never dies.

    /// Push an Error block to the active session's transcript.
    fn push_error(&self, text: String) {
        if let Some(id) = self.with_state(|state| state.active_id()) {
            if let Ok(mut state) = self.state.lock() {
                if let Some(blocks) = state.blocks.get_mut(&id) {
                    blocks.push(Block::Error { text });
                }
            }
        }
    }

    /// Busy guard for ops with unverified during-generation behavior
    /// (fork, revert, command exec). Returns the active id when idle.
    fn when_idle(&self, op: &str) -> Option<String> {
        let (id, busy) = self.with_state(|state| {
            let id = state.active_id()?;
            let busy = state.busy.contains(&id);
            Some((id, busy))
        })?;
        if busy {
            self.push_error(format!("{op} is disabled while a turn is running."));
            return None;
        }
        Some(id)
    }

    /// Lazily-loaded model discovery (cached; a fetch failure surfaces and
    /// caches nothing).
    fn ensure_models(&self) -> Result<Vec<DiscoveredModel>, String> {
        if let Some(models) = self.with_state(|state| state.models.clone()) {
            return Ok(models);
        }
        match self.client.list_models() {
            Ok(value) => {
                let models = parse_models(&value);
                if let Ok(mut state) = self.state.lock() {
                    state.models = Some(models.clone());
                }
                Ok(models)
            }
            Err(error) => Err(format!("model discovery failed: {error}")),
        }
    }

    fn ensure_agents(&self) -> Result<Vec<DiscoveredAgent>, String> {
        if let Some(agents) = self.with_state(|state| state.agents.clone()) {
            return Ok(agents);
        }
        match self.client.list_agents() {
            Ok(value) => {
                let agents = parse_agents(&value);
                if let Ok(mut state) = self.state.lock() {
                    state.agents = Some(agents.clone());
                }
                Ok(agents)
            }
            Err(error) => Err(format!("agent discovery failed: {error}")),
        }
    }

    /// Confirm-gate options for the staged revert (matched in
    /// `answer_revert` so a live agent question can never be consumed as
    /// a revert confirmation).
    const REVERT_OPTIONS: [&'static str; 2] = ["Commit", "Keep messages"];
    /// Confirm-gate options for file-writing slash commands.
    const COMMAND_OPTIONS: [&'static str; 2] = ["Run", "Cancel"];

    /// Shared commit body (trait method + digit confirmation converge
    /// here). The caller pops the confirm gate first.
    fn do_commit_revert(&mut self) {
        let pending = self.with_state(|state| state.pending_revert.clone());
        let Some(pending) = pending else {
            self.push_error("revert: no staged boundary.".into());
            return;
        };
        let Some(id) = self.when_idle("Revert commit") else {
            return;
        };
        if let Err(error) = self.client.commit_revert(&id) {
            self.push_error(format!("Revert commit failed: {error}"));
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.pending_revert = None;
            // Commit deleted post-boundary messages server-side: drop the
            // stale transcript so the next hydrate replays server truth.
            state.blocks.remove(&id);
            state.blocks.insert(
                id.clone(),
                vec![Block::Notice {
                    text: format!(
                        "Reverted to {} ({} file(s) restored).",
                        pending.message_id,
                        pending.files.len()
                    ),
                    link: None,
                }],
            );
        }
        self.hydrate_active();
    }

    /// Shared abandon body (trait method + digit confirmation converge here).
    fn do_abandon_revert(&mut self) {
        if self.with_state(|state| state.pending_revert.is_none()) {
            self.push_error("revert: no staged boundary.".into());
            return;
        }
        let Some(id) = self.with_state(|state| state.active_id()) else {
            return;
        };
        if let Err(error) = self.client.delete_revert(&id) {
            self.push_error(format!("Revert abandon failed: {error}"));
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.pending_revert = None;
            if let Some(blocks) = state.blocks.get_mut(&id) {
                blocks.push(Block::Notice {
                    text: "Revert abandoned — messages kept.".into(),
                    link: None,
                });
            }
        }
    }

    /// Pop the trailing Question block (confirm-gate answer consumed).
    fn pop_trailing_question(&self) {
        if let Some(id) = self.with_state(|state| state.active_id()) {
            if let Ok(mut state) = self.state.lock() {
                if let Some(blocks) = state.blocks.get_mut(&id) {
                    if matches!(blocks.last(), Some(Block::Question { .. })) {
                        blocks.pop();
                    }
                }
            }
        }
    }

    /// Answer the staged-revert confirm gate (digits 1/2). The gate is
    /// only consumed when its options match — a live agent question asked
    /// while a boundary is staged is never eaten as a confirmation.
    fn answer_revert(&mut self, option: usize) {
        let matched = self.with_state(|state| {
            let id = state.active_id()?;
            let blocks = state.blocks.get(&id)?;
            match blocks.last()? {
                Block::Question { q }
                    if q.options
                        == Self::REVERT_OPTIONS
                            .iter()
                            .map(|option| (*option).to_owned())
                            .collect::<Vec<_>>() =>
                {
                    state.pending_revert.clone().map(|_| true)
                }
                _ => None,
            }
        });
        if matched.is_none() {
            return;
        }
        if option == 0 {
            self.commit_revert();
        } else {
            self.abandon_revert();
        }
    }

    /// POST a slash command (the confirmed path — gating lives in
    /// `execute_command`).
    fn run_command(&mut self, name: &str, text: &str) {
        let Some(id) = self.with_state(|state| state.active_id()) else {
            return;
        };
        match self.client.execute_command(&id, name, text) {
            Ok(_) => {
                if let Ok(mut state) = self.state.lock() {
                    state.busy.insert(id.clone());
                    if let Some(blocks) = state.blocks.get_mut(&id) {
                        let line = if text.is_empty() {
                            format!("/{name}")
                        } else {
                            format!("/{name} {text}")
                        };
                        blocks.push(Block::User { text: line });
                    }
                }
            }
            Err(error) => self.push_error(format!("Command /{name} failed: {error}")),
        }
    }

    /// Answer a writer-command confirm gate (digits 1/2).
    fn answer_command(&mut self, option: usize) {
        let pending = self.with_state(|state| {
            let id = state.active_id()?;
            let blocks = state.blocks.get(&id)?;
            match blocks.last()? {
                Block::Question { q }
                    if q.options
                        == Self::COMMAND_OPTIONS
                            .iter()
                            .map(|option| (*option).to_owned())
                            .collect::<Vec<_>>() =>
                {
                    state.pending_command.clone()
                }
                _ => None,
            }
        });
        let Some(pending) = pending else {
            return;
        };
        self.pop_trailing_question();
        if option == 0 {
            if let Ok(mut state) = self.state.lock() {
                state.pending_command = None;
            }
            self.run_command(&pending.name, &pending.text);
        } else if let Ok(mut state) = self.state.lock() {
            state.pending_command = None;
            if let Some(id) = state.active_id() {
                if let Some(blocks) = state.blocks.get_mut(&id) {
                    blocks.push(Block::Notice {
                        text: "Command cancelled.".into(),
                        link: None,
                    });
                }
            }
        }
    }

    /// Route `/verb …` lines: 7C operations and discovered server commands
    /// execute locally; all other text (including unknown `/words`) is
    /// sent as an ordinary prompt, exactly as before.
    fn submit_or_command(&mut self, trimmed: &str) {
        let Some(rest) = trimmed
            .strip_prefix('/')
            .filter(|rest| !rest.starts_with('/'))
        else {
            self.submit_prompt(trimmed);
            return;
        };
        let mut parts = rest.splitn(2, char::is_whitespace);
        let verb = parts.next().unwrap_or("");
        let args = parts.next().unwrap_or("").trim();
        match verb {
            "diff" => {
                if !args.is_empty() {
                    self.push_error("diff: usage `/diff`.".into());
                } else {
                    self.diff();
                }
            }
            "compact" => {
                if !args.is_empty() {
                    self.push_error("compact: usage `/compact`.".into());
                } else {
                    self.compact();
                }
            }
            "fork" => self.fork(if args.is_empty() {
                None
            } else {
                Some(args.to_owned())
            }),
            "revert" => self.stage_revert(args.to_owned()),
            "model" => self.switch_model(args.to_owned()),
            "agent" => self.switch_agent(args.to_owned()),
            _ => {
                let known = self
                    .with_state(|state| state.commands.iter().any(|command| command.name == verb));
                if known {
                    self.execute_command(verb.to_owned(), args.to_owned());
                } else {
                    self.submit_prompt(trimmed);
                }
            }
        }
    }
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
        let leaving = self.with_state(|state| state.active_id());
        let known = self.with_state(|state| index < state.summaries.len());
        if known {
            if let Ok(mut state) = self.state.lock() {
                state.active = index;
            }
            self.hydrate_active();
            // Phase 8 (8-R3): leaving a session ends it for extraction
            // purposes — generate proposals from its history. Silent and
            // best-effort; the session is never affected.
            if let Some(left) = leaving {
                let current = self.with_state(|state| state.active_id());
                if Some(left.clone()) != current {
                    self.refresh_proposals(&left);
                }
            }
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
            // Phase 7C slash ops + discovered server commands. Anything
            // else keeps the historical behavior (prompt text).
            command::Classified::Normal => self.submit_or_command(&trimmed),
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
        // Phase 7C confirm gates take precedence over form delivery. Each
        // answer_* only consumes the gate on an options match, so a live
        // agent question asked while a confirmation is pending falls
        // through to form delivery below.
        if self.with_state(|state| state.pending_revert.is_some()) {
            self.answer_revert(option);
            if self.with_state(|state| state.pending_revert.is_none()) {
                return;
            }
        }
        if self.with_state(|state| state.pending_command.is_some()) {
            self.answer_command(option);
            if self.with_state(|state| state.pending_command.is_none()) {
                return;
            }
        }
        let found = self.with_state(|state| {
            let id = state.active_id()?;
            let blocks = state.blocks.get(&id)?;
            match blocks.last()? {
                Block::Question { q } => {
                    let pending = state
                        .pending_questions
                        .get(&id)
                        .and_then(|list| list.last().cloned());
                    Some((id, q.options.get(option).cloned(), pending))
                }
                _ => None,
            }
        });
        let Some((id, label, pending)) = found else {
            return;
        };
        let Some(pending) = pending else {
            // Gate with no live form (restored from history, or a
            // `question.v2` gate on a server with no form route): resolve
            // locally like the permission fallback — visible, honest.
            if let Some(label) = label {
                if let Ok(mut state) = self.state.lock() {
                    if let Some(blocks) = state.blocks.get_mut(&id) {
                        blocks.pop();
                        blocks.push(Block::Assistant {
                            text: format!("Chose “{label}” — no live question form to deliver to."),
                        });
                    }
                }
            }
            return;
        };
        let Some(answer) = pending.option_values.get(option).cloned() else {
            // Out-of-range option: refuse politely, keep the gate.
            if let Ok(mut state) = self.state.lock() {
                if let Some(blocks) = state.blocks.get_mut(&id) {
                    blocks.push(Block::Error {
                        text: format!(
                            "Answer not delivered — option {} is out of range.",
                            option + 1
                        ),
                    });
                }
            }
            return;
        };
        // Live form gate: deliver over the verified form-reply route.
        match self
            .client
            .reply_form(&id, &pending.form_id, &pending.field_key, &answer)
        {
            Ok(_) => {
                if let Ok(mut state) = self.state.lock() {
                    // Pop the matching pending entry (trailing gate) so any
                    // earlier stacked gates keep their alignment; `form.replied`
                    // clears whichever form was delivered.
                    if let Some(list) = state.pending_questions.get_mut(&id) {
                        list.pop();
                        if list.is_empty() {
                            state.pending_questions.remove(&id);
                        }
                    }
                    if let Some(blocks) = state.blocks.get_mut(&id) {
                        blocks.pop();
                        blocks.push(Block::Assistant {
                            text: format!("Answered: {}", label.unwrap_or(answer)),
                        });
                    }
                }
            }
            Err(error) => {
                log::warn!("question answer rejected: {error}");
                if let Ok(mut state) = self.state.lock() {
                    if let Some(blocks) = state.blocks.get_mut(&id) {
                        // Visible not-delivered fallback; the gate stays so
                        // the user can retry.
                        blocks.push(Block::Error {
                            text: format!("Answer was NOT delivered: {error}"),
                        });
                    }
                }
            }
        }
    }

    fn simulate_activity(&mut self) {}

    fn compact(&mut self) {
        let Some(id) = self.with_state(|state| state.active_id()) else {
            return;
        };
        // No busy guard: compact steers at the next step boundary by
        // design; the compaction events/markers carry the outcome.
        match self.client.compact_session(&id) {
            Ok(_) => {
                if let Ok(mut state) = self.state.lock() {
                    state.busy.insert(id.clone());
                    if let Some(blocks) = state.blocks.get_mut(&id) {
                        blocks.push(Block::Notice {
                            text: "Compact request admitted — compacting…".into(),
                            link: None,
                        });
                    }
                }
            }
            Err(error) => self.push_error(format!("Compact rejected: {error}")),
        }
    }

    fn diff(&mut self) {
        let Some(id) = self.with_state(|state| state.active_id()) else {
            return;
        };
        // Read-only: no busy guard, no session mutation, no fake events.
        let value = match self.client.session_diff(&id) {
            Ok(value) => value,
            Err(error) => {
                self.push_error(format!("Diff failed: {error}"));
                return;
            }
        };
        let entries = value
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // Status labels ride a summary Notice (FileDiff carries path/counts/patch).
        let mut summary = Vec::new();
        let files = entries
            .iter()
            .map(|file| {
                let path = file.get("file").and_then(Value::as_str).unwrap_or("?");
                let added = file.get("additions").and_then(Value::as_u64).unwrap_or(0);
                let removed = file.get("deletions").and_then(Value::as_u64).unwrap_or(0);
                let status = file.get("status").and_then(Value::as_str).unwrap_or("");
                summary.push(format!("{path} ({status}, +{added} −{removed})"));
                let patch = file.get("patch").and_then(Value::as_str).unwrap_or("");
                let (lines, _) = patch_lines(patch, MAX_DIFF_LINES_PER_FILE);
                FileDiff {
                    path: path.to_owned(),
                    added: added as u32,
                    removed: removed as u32,
                    lines,
                }
            })
            .collect::<Vec<_>>();
        if let Ok(mut state) = self.state.lock() {
            if let Some(blocks) = state.blocks.get_mut(&id) {
                if files.is_empty() {
                    blocks.push(Block::Notice {
                        text: "No differences.".into(),
                        link: None,
                    });
                } else {
                    blocks.push(Block::Notice {
                        text: format!("Diff — {} file(s): {}", files.len(), summary.join(", ")),
                        link: None,
                    });
                    blocks.push(Block::Edits { files });
                }
            }
        }
    }
    fn stage_revert(&mut self, message_id: String) {
        let message_id = message_id.trim().to_owned();
        if message_id.is_empty() {
            self.push_error("revert: usage `/revert <message-id>`.".into());
            return;
        }
        let Some(id) = self.when_idle("Revert") else {
            return;
        };
        let staged = match self.client.stage_revert(&id, &message_id) {
            Ok(value) => value,
            Err(error) => {
                self.push_error(format!("Revert stage failed: {error}"));
                return;
            }
        };
        let files = staged
            .get("data")
            .map(parse_revert_files)
            .unwrap_or_default();
        if let Ok(mut state) = self.state.lock() {
            state.pending_revert = Some(PendingRevert {
                message_id: message_id.clone(),
                files: files.clone(),
            });
            if let Some(blocks) = state.blocks.get_mut(&id) {
                // Show the actual staged reverse patches (never fabricated).
                let file_blocks = files
                    .iter()
                    .map(|file| {
                        let (lines, _) = patch_lines(&file.patch, MAX_DIFF_LINES_PER_FILE);
                        FileDiff {
                            path: file.file.clone(),
                            added: file.additions,
                            removed: file.deletions,
                            lines,
                        }
                    })
                    .collect::<Vec<_>>();
                if !file_blocks.is_empty() {
                    blocks.push(Block::Edits { files: file_blocks });
                }
                let names = files
                    .iter()
                    .map(|file| {
                        if file.status.is_empty() {
                            file.file.clone()
                        } else {
                            format!("{} ({})", file.file, file.status)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let restored = if names.is_empty() {
                    "No files were changed by this boundary.".to_owned()
                } else {
                    format!("Affected files already restored: {names}.")
                };
                blocks.push(Block::Question {
                    q: Question {
                        prompt: format!(
                            "{restored} Committing deletes messages after {message_id} — irreversible through this API. Commit?"
                        ),
                        options: Self::REVERT_OPTIONS
                            .iter()
                            .map(|option| (*option).to_owned())
                            .collect(),
                    },
                });
            }
        }
    }

    fn commit_revert(&mut self) {
        let had_pending = self.with_state(|state| state.pending_revert.is_some());
        if had_pending {
            self.pop_trailing_question();
        }
        self.do_commit_revert();
    }

    fn abandon_revert(&mut self) {
        let had_pending = self.with_state(|state| state.pending_revert.is_some());
        if had_pending {
            self.pop_trailing_question();
        }
        self.do_abandon_revert();
    }

    fn fork(&mut self, before: Option<String>) {
        let Some(id) = self.when_idle("Fork") else {
            return;
        };
        let parent_title = self.with_state(|state| {
            state
                .summaries
                .iter()
                .find(|summary| summary.id == id)
                .map(|summary| summary.title.clone())
                .unwrap_or_default()
        });
        let child = match self.client.fork_session(&id, before.as_deref()) {
            Ok(value) => value,
            Err(error) => {
                self.push_error(format!("Fork failed: {error}"));
                return;
            }
        };
        let data = child.get("data").cloned().unwrap_or(Value::Null);
        let child_id = data
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if child_id.is_empty() {
            self.push_error("Fork failed: server returned no child id.".into());
            return;
        }
        let child_title = data
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{parent_title} (fork)"));
        if let Ok(mut state) = self.state.lock() {
            if !state.summaries.iter().any(|summary| summary.id == child_id) {
                state.summaries.push(SessionSummary {
                    id: child_id.clone(),
                    title: child_title.clone(),
                });
            }
            // Empty blocks: the child hydrates lazily on selection like
            // every other known session. Parent and focus are preserved.
            state.blocks.entry(child_id.clone()).or_default();
            if let Some(blocks) = state.blocks.get_mut(&id) {
                blocks.push(Block::Notice {
                    text: format!("Forked to {child_title} ({child_id})."),
                    link: None,
                });
            }
        }
        // 7C-R4, option (a): a fork child is a new session for memory
        // injection — it inherits history/agent/model but NOT instruction
        // entries, so it receives a fresh snapshot under the frozen
        // Phase 6 rules (no architecture change, no raw-entry copying).
        // 7C-R2: verify the no-inheritance assumption tolerantly — a 404
        // (empty) is the expected case; inherited entries only warn.
        match self
            .client
            .try_get_instruction_entry(&child_id, "owt.memory")
        {
            Ok(Some(_)) => log::warn!("fork child {child_id} inherited instruction entries"),
            Ok(None) => {}
            Err(error) => log::warn!("fork child entry check failed: {error}"),
        }
        if self.injection == MemoryInjection::Auto {
            self.injector
                .inject_new_session(&self.client, &self.memory, &child_id);
        }
    }

    fn switch_model(&mut self, id: String) {
        let id = id.trim().to_owned();
        if id.is_empty() {
            self.push_error("model: usage `/model <model-id>`.".into());
            return;
        }
        let models = match self.ensure_models() {
            Ok(models) => models,
            Err(error) => {
                self.push_error(error);
                return;
            }
        };
        let (model_id, provider_id) = match validate_model(&models, &id) {
            Ok(pair) => pair,
            Err(error) => {
                self.push_error(format!("model: {error}"));
                return;
            }
        };
        let Some(active) = self.with_state(|state| state.active_id()) else {
            return;
        };
        if let Err(error) = self.client.switch_model(&active, &model_id, &provider_id) {
            self.push_error(format!("Model switch failed: {error}"));
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            if let Some(blocks) = state.blocks.get_mut(&active) {
                blocks.push(Block::Notice {
                    text: format!("Model switched to {model_id}."),
                    link: None,
                });
            }
        }
        // Refresh recorded session info (model included); blocks are known
        // so history is untouched.
        self.hydrate_active();
    }

    fn switch_agent(&mut self, id: String) {
        let id = id.trim().to_owned();
        if id.is_empty() {
            self.push_error("agent: usage `/agent <agent-id>`.".into());
            return;
        }
        let agents = match self.ensure_agents() {
            Ok(agents) => agents,
            Err(error) => {
                self.push_error(error);
                return;
            }
        };
        let agent = match validate_agent(&agents, &id) {
            Ok(agent) => agent,
            Err(error) => {
                self.push_error(format!("agent: {error}"));
                return;
            }
        };
        let Some(active) = self.with_state(|state| state.active_id()) else {
            return;
        };
        if let Err(error) = self.client.switch_agent(&active, &agent) {
            self.push_error(format!("Agent switch failed: {error}"));
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            if let Some(blocks) = state.blocks.get_mut(&active) {
                blocks.push(Block::Notice {
                    text: format!("Agent switched to {agent}."),
                    link: None,
                });
            }
        }
        self.hydrate_active();
    }

    fn execute_command(&mut self, name: String, text: String) {
        let Some(_id) = self.when_idle("Command execution") else {
            return;
        };
        let known =
            self.with_state(|state| state.commands.iter().any(|command| command.name == name));
        if !known {
            self.push_error(format!("Unknown command: {name}."));
            return;
        }
        if needs_command_confirm(&name) {
            if self.with_state(|state| state.pending_command.is_none()) {
                if let Some(id) = self.with_state(|state| state.active_id()) {
                    if let Ok(mut state) = self.state.lock() {
                        state.pending_command = Some(PendingCommand {
                            name: name.clone(),
                            text: text.clone(),
                        });
                        if let Some(blocks) = state.blocks.get_mut(&id) {
                            let what = if WRITER_COMMANDS.contains(&name.as_str()) {
                                format!("The `/{name}` command writes workspace files.")
                            } else {
                                format!("`/{name}` may modify files.")
                            };
                            blocks.push(Block::Question {
                                q: Question {
                                    prompt: format!("{what} Run it?"),
                                    options: Self::COMMAND_OPTIONS
                                        .iter()
                                        .map(|option| (*option).to_owned())
                                        .collect(),
                                },
                            });
                        }
                    }
                }
                return;
            }
            self.push_error("Confirm or cancel the pending command first (1/2).".into());
            return;
        }
        self.run_command(&name, &text);
    }

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
        // Phase 6B hook #2: brand-new session. The PUT runs inline before
        // the id is returned, so the first `submit_prompt` for this id
        // cannot precede it (single-threaded blocking adapter). Resume
        // (`set_active`/`hydrate_active`) has no hook by design — the
        // snapshot stays frozen.
        if self.injection == MemoryInjection::Auto {
            self.injector
                .inject_new_session(&self.client, &self.memory, &id);
        }
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
            // Phase 8 proposal triage (8-R12): quarantined suggestions are
            // refreshed from this session's history first, then listed.
            Ok(command::Command::Suggest) => {
                self.refresh_proposals(session_id);
                match triage::suggest_text(&mut self.memory) {
                    Ok(text) => Block::Assistant { text },
                    Err(error) => Block::Error {
                        text: format!("memory: {error}"),
                    },
                }
            }
            Ok(command::Command::Confirm {
                target,
                scope,
                kind,
            }) => match triage::confirm_proposal(&mut self.memory, &target, scope, kind) {
                Ok(text) => Block::Assistant { text },
                Err(error) => Block::Error {
                    text: format!("memory: {error}"),
                },
            },
            Ok(command::Command::Discard {
                target,
                confirm_all,
            }) => match triage::discard_proposal(&mut self.memory, target.as_ref(), confirm_all) {
                Ok(text) => Block::Assistant { text },
                Err(error) => Block::Error {
                    text: format!("memory: {error}"),
                },
            },
        }
    }

    /// Best-effort session-end proposal refresh (Phase 8, 8-R1–R3): read
    /// this session's user-role history, generate rule proposals into the
    /// quarantine queues. Silent; any failure only logs — the session is
    /// never affected (memory-never-fatal invariant).
    fn refresh_proposals(&mut self, session_id: &str) {
        let texts = self.session_user_texts(session_id);
        if texts.is_empty() {
            return;
        }
        let project_rooted = self.memory.has_project();
        match triage::refresh_session_proposals(&self.memory, session_id, &texts, project_rooted) {
            Ok(stats) => {
                if stats.proposed > 0 {
                    log::info!(
                        "memory: session {session_id} proposed {} candidate(s)",
                        stats.proposed
                    );
                }
            }
            Err(error) => log::warn!("memory: proposal refresh skipped: {error}"),
        }
    }

    /// User-role message texts for one session, oldest-first (Phase 8
    /// input set — 8-R2 is structural: only `type == "user"` messages).
    fn session_user_texts(&self, session_id: &str) -> Vec<String> {
        const MAX_PAGES: usize = 50;
        let mut texts = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page =
                match self
                    .client
                    .list_messages_paged(session_id, cursor.as_deref(), Some(200))
                {
                    Ok(page) => page,
                    Err(_) => break,
                };
            let messages = page
                .get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            texts.extend(extract::user_texts(&messages));
            cursor = page
                .get("cursor")
                .and_then(|cursor| cursor.get("next"))
                .and_then(Value::as_str)
                .filter(|next| !next.is_empty())
                .map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        texts
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_raw, needs_command_confirm, parse_agents, parse_models, parse_revert_files,
        patch_lines, pending_from_form, session_of, validate_agent, validate_model,
        OpenCodeBackend, OpenCodeConfig, PendingQuestion, RawEvent, State,
    };
    use crate::backend::{Backend, Block};
    use serde_json::json;

    fn state_with_active() -> State {
        let mut state = State::default();
        state.summaries.push(crate::backend::SessionSummary {
            id: "ses_1".into(),
            title: "t".into(),
        });
        state
    }

    #[test]
    fn session_of_reads_form_nested_id() {
        let raw = RawEvent {
            typ: "form.created".into(),
            data: json!({"form": {"id": "frm_1", "sessionID": "ses_9"}}),
        };
        assert_eq!(session_of(&raw).as_deref(), Some("ses_9"));
        let raw = RawEvent {
            typ: "session.step.started".into(),
            data: json!({"sessionID": "ses_7"}),
        };
        assert_eq!(session_of(&raw).as_deref(), Some("ses_7"));
    }

    #[test]
    fn pending_from_form_parses_question_fields() {
        let data = json!({"form": {
            "id": "frm_1",
            "metadata": {"kind": "question"},
            "fields": [
                {"key": "q0", "options": [
                    {"value": "fast", "label": "Fast"},
                    {"value": "slow", "label": "Slow"},
                ]},
                {"key": "q1", "options": [
                    {"value": "yes", "label": "Yes"},
                ]},
            ]
        }});
        let pendings = pending_from_form(&data);
        assert_eq!(pendings.len(), 2);
        assert_eq!(pendings[0].form_id, "frm_1");
        assert_eq!(pendings[0].field_key, "q0");
        assert_eq!(pendings[0].option_values, vec!["fast", "slow"]);
        assert_eq!(pendings[1].field_key, "q1");
        // Non-question forms contribute nothing.
        let other = json!({"form": {"id": "frm_2", "metadata": {"kind": "task"}}});
        assert!(pending_from_form(&other).is_empty());
        // Envelope without a nested form: nothing, no panic.
        assert!(pending_from_form(&json!({"id": "x"})).is_empty());
    }

    #[test]
    fn apply_raw_records_and_clears_question_gates() {
        let mut state = state_with_active();
        let created = RawEvent {
            typ: "form.created".into(),
            data: json!({"form": {
                "id": "frm_1",
                "sessionID": "ses_1",
                "metadata": {"kind": "question"},
                "fields": [{"key": "q0", "options": [{"value": "fast", "label": "Fast"}]}]
            }}),
        };
        let sid = apply_raw(&mut state, &created).expect("maps into ses_1");
        assert_eq!(sid, "ses_1");
        let pendings = state
            .pending_questions
            .get("ses_1")
            .cloned()
            .unwrap_or_default();
        assert_eq!(pendings.len(), 1);
        assert_eq!(pendings[0].form_id, "frm_1");
        // Simulate the mapper pushing the gate block (as ingest does after
        // apply_raw maps the event).
        state
            .blocks
            .entry("ses_1".into())
            .or_default()
            .push(Block::Question {
                q: crate::backend::Question {
                    prompt: "Pick".into(),
                    options: vec!["Fast".into()],
                },
            });
        // form.replied clears this form's gates.
        let replied = RawEvent {
            typ: "form.replied".into(),
            data: json!({"id": "frm_1", "sessionID": "ses_1", "answer": {"q0": "fast"}}),
        };
        assert_eq!(apply_raw(&mut state, &replied).as_deref(), Some("ses_1"));
        assert!(!state.pending_questions.contains_key("ses_1"));
    }

    #[test]
    fn apply_raw_reconciles_connection_gap_busy() {
        let mut state = state_with_active();
        state.busy.insert("ses_1".into());
        // The active session always has rendered blocks in the real flow;
        // the drop notice is appended there.
        state
            .blocks
            .insert("ses_1".into(), vec![Block::Assistant { text: "hi".into() }]);
        let lost = RawEvent {
            typ: "connection.lost".into(),
            data: json!({}),
        };
        // Synthetic signal: skipped for mapping, busy cleared, gap latched.
        assert!(apply_raw(&mut state, &lost).is_none());
        assert!(state.busy.is_empty());
        assert!(state.connection_gap);
        let blocks = state.blocks.get("ses_1").cloned().unwrap_or_default();
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block, Block::Notice { .. })),
            "drop is surfaced in-band"
        );
        let restored = RawEvent {
            typ: "connection.restored".into(),
            data: json!({}),
        };
        assert!(apply_raw(&mut state, &restored).is_none());
        assert!(!state.connection_gap);
    }

    #[test]
    fn apply_raw_question_v2_clears_pending_on_reply() {
        let mut state = state_with_active();
        state.pending_questions.insert(
            "ses_1".into(),
            vec![PendingQuestion {
                form_id: String::new(),
                field_key: String::new(),
                option_values: vec![],
            }],
        );
        let replied = RawEvent {
            typ: "question.v2.replied".into(),
            data: json!({"id": "que_1", "sessionID": "ses_1"}),
        };
        assert_eq!(apply_raw(&mut state, &replied).as_deref(), Some("ses_1"));
        assert!(!state.pending_questions.contains_key("ses_1"));
    }

    #[test]
    fn discovery_parses_and_validates() {
        let models = parse_models(&json!({"data": [
            {"id": "m1", "providerID": "p1"},
            {"id": "dead", "providerID": "p1", "enabled": false},
            {"id": "gone", "providerID": "p1", "status": "disabled"},
            {"providerID": "p1"},
        ]}));
        assert_eq!(models.len(), 1);
        assert_eq!(
            validate_model(&models, "m1"),
            Ok(("m1".to_owned(), "p1".to_owned()))
        );
        let err = validate_model(&models, "bogus").expect_err("bogus rejected");
        assert!(err.contains("bogus") && err.contains("m1"), "got: {err}");

        let agents = parse_agents(&json!({"data": [{"id": "build"}, {"id": "general"}]}));
        assert_eq!(validate_agent(&agents, "general"), Ok("general".to_owned()));
        assert!(validate_agent(&agents, "bogus").is_err());
        // Empty discovery rejects everything (never send blind).
        assert!(validate_model(&[], "m1").is_err());
    }

    #[test]
    fn revert_files_and_patch_lines_parse() {
        let files = parse_revert_files(&json!({"messageID": "msg_1", "files": [
            {"file": "notes.txt", "patch": "@@ -1 +1 @@\n line\n+new\n-old\n", "additions": 1, "deletions": 1, "status": "modified"},
        ]}));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file, "notes.txt");
        let (lines, truncated) = patch_lines(&files[0].patch, 120);
        assert!(!truncated);
        assert!(lines.contains(&(true, "new".to_owned())));
        assert!(lines.contains(&(false, "old".to_owned())));
        // Headers skipped, hunk header kept as context.
        assert!(!lines.iter().any(|(_, line)| line.starts_with("diff --git")));
        let (_, truncated) = patch_lines("a\nb\nc\nd\n", 2);
        assert!(truncated);
    }

    #[test]
    fn writer_commands_need_confirm() {
        assert!(needs_command_confirm("init"));
        assert!(!needs_command_confirm("review"));
        // Unknown writability defaults to confirmation, not silent exec.
        assert!(needs_command_confirm("mystery"));
    }

    // Phase 7C backend tests: real `OpenCodeBackend` against a stub
    // server. The SSE worker never connects (`|| None` backs off; Drop
    // joins after one short sleep), so every test is deterministic.
    use super::client::Client;
    use super::config::{Endpoint, MemoryInjection};
    use super::events::SseWorker;
    use super::memory_inject::Injector;
    use super::ConnectionKind;
    use crate::backend::memory::api::MemoryApi;
    use crate::backend::SlashCommand;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    struct Stub {
        addr: std::net::SocketAddr,
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl Stub {
        fn spawn(reply: (u16, String)) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let seen_thread = seen.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request_line = String::new();
                    if reader.read_line(&mut request_line).is_err() {
                        continue;
                    }
                    let mut content_length = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).is_err() {
                            break;
                        }
                        let trimmed = line.trim_end().to_ascii_lowercase();
                        if trimmed.is_empty() {
                            break;
                        }
                        if let Some(value) = trimmed.strip_prefix("content-length:") {
                            content_length = value.trim().parse().unwrap_or(0);
                        }
                    }
                    let mut body = vec![0u8; content_length];
                    if content_length > 0 {
                        let _ = reader.read_exact(&mut body);
                    }
                    seen_thread.lock().unwrap().push(format!(
                        "{} | {}",
                        request_line.trim_end(),
                        String::from_utf8_lossy(&body)
                    ));
                    let (status, response_body) = reply.clone();
                    let reason = match status {
                        200 => "OK",
                        204 => "No Content",
                        404 => "Not Found",
                        _ => "Error",
                    };
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} {reason}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    );
                }
            });
            Stub { addr, seen }
        }

        fn client(&self) -> Client {
            Client::new(&Endpoint {
                url: format!("http://{}", self.addr),
                password: None,
            })
        }

        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    fn test_backend(client: Client) -> OpenCodeBackend {
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = SseWorker::spawn(|| None, sender);
        // Unique temp dir per backend: parallel tests must never share
        // store or quarantine files.
        static BACKEND_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = BACKEND_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("owt-7c-test-{}-{seq}", std::process::id()));
        let state = Arc::new(Mutex::new(state_with_active()));
        // Production invariant: every known session has a blocks entry
        // (created at session creation/hydrate); seed it here.
        state
            .lock()
            .unwrap()
            .blocks
            .insert("ses_1".into(), Vec::new());
        OpenCodeBackend {
            state,
            client,
            receiver,
            _worker: worker,
            _server: None,
            _connection: ConnectionKind::Discovered,
            memory: MemoryApi::open(dir, None),
            injection: MemoryInjection::Off,
            injector: Injector::default(),
        }
    }

    fn last_text(backend: &OpenCodeBackend) -> String {
        backend
            .lock_state()
            .blocks
            .get("ses_1")
            .and_then(|blocks| blocks.last())
            .map(|block| match block {
                Block::Assistant { text } => text.clone(),
                Block::Error { text } => text.clone(),
                Block::Notice { text, .. } => text.clone(),
                Block::Question { q } => q.prompt.clone(),
                _ => "<other>".to_owned(),
            })
            .unwrap_or_default()
    }

    fn set_commands(backend: &OpenCodeBackend, names: &[&str]) {
        backend.lock_state().commands = names
            .iter()
            .map(|name| SlashCommand {
                name: (*name).to_owned(),
                description: String::new(),
            })
            .collect();
    }

    const MODELS_JSON: &str = r#"{"data":[{"id":"m1","providerID":"p1"}]}"#;
    const AGENTS_JSON: &str = r#"{"data":[{"id":"general"}]}"#;

    #[test]
    fn switch_model_rejects_bogus_before_send() {
        let stub = Stub::spawn((200, MODELS_JSON.to_owned()));
        let mut backend = test_backend(stub.client());
        backend.switch_model("bogus".into());
        // Only the discovery GET went out; the switch POST never happened.
        assert_eq!(stub.seen().len(), 1);
        assert!(stub.seen()[0].starts_with("GET /api/model"));
        assert!(last_text(&backend).contains("bogus"));
    }

    #[test]
    fn switch_model_sends_valid() {
        let stub = Stub::spawn((200, MODELS_JSON.to_owned()));
        let mut backend = test_backend(stub.client());
        backend.switch_model("m1".into());
        let seen = stub.seen();
        assert!(seen.len() >= 2, "got: {seen:?}");
        assert!(
            seen[1].starts_with("POST /api/session/ses_1/model"),
            "got: {seen:?}"
        );
        assert!(last_text(&backend).contains("m1") || seen.len() > 2);
    }

    #[test]
    fn switch_agent_rejects_bogus_before_send() {
        let stub = Stub::spawn((200, AGENTS_JSON.to_owned()));
        let mut backend = test_backend(stub.client());
        backend.switch_agent("bogus".into());
        assert_eq!(stub.seen().len(), 1);
        assert!(last_text(&backend).contains("bogus"));
    }

    #[test]
    fn execute_unknown_command_posts_nothing() {
        let stub = Stub::spawn((204, String::new()));
        let mut backend = test_backend(stub.client());
        set_commands(&backend, &["review"]);
        backend.execute_command("nope".into(), String::new());
        assert!(stub.seen().is_empty());
        assert!(last_text(&backend).contains("Unknown command"));
    }

    #[test]
    fn writer_command_gates_then_posts_once() {
        let stub = Stub::spawn((204, String::new()));
        let mut backend = test_backend(stub.client());
        set_commands(&backend, &["init", "review"]);
        backend.execute_command("init".into(), String::new());
        assert!(stub.seen().is_empty(), "gate first, no POST");
        assert!(matches!(
            backend
                .lock_state()
                .blocks
                .get("ses_1")
                .and_then(|b| b.last()),
            Some(Block::Question { .. })
        ));
        backend.answer_question(0);
        let seen = stub.seen();
        assert_eq!(seen.len(), 1, "exactly one POST, got: {seen:?}");
        assert!(seen[0].starts_with("POST /api/session/ses_1/command"));
    }

    #[test]
    fn commit_abandon_without_boundary_error() {
        let stub = Stub::spawn((204, String::new()));
        let mut backend = test_backend(stub.client());
        backend.commit_revert();
        assert!(last_text(&backend).contains("no staged boundary"));
        backend.abandon_revert();
        assert!(last_text(&backend).contains("no staged boundary"));
        assert!(stub.seen().is_empty());
    }

    #[test]
    fn diff_empty_and_populated() {
        let stub = Stub::spawn((200, r#"{"data":[]}"#.to_owned()));
        let mut backend = test_backend(stub.client());
        backend.diff();
        assert!(last_text(&backend).contains("No differences"));

        let stub = Stub::spawn((
            200,
            r#"{"data":[{"file":"notes.txt","patch":"@@ -1 +1,2 @@\n line one\n+probe-change-1\n","additions":1,"deletions":0,"status":"modified"}]}"#.to_owned(),
        ));
        let mut backend = test_backend(stub.client());
        backend.diff();
        let blocks = backend
            .lock_state()
            .blocks
            .get("ses_1")
            .cloned()
            .unwrap_or_default();
        assert!(blocks
            .iter()
            .any(|block| matches!(block, Block::Edits { .. })));
        assert!(last_text(&backend).contains("notes.txt") || blocks.len() == 2);
    }

    #[test]
    fn fork_busy_guard_posts_nothing() {
        let stub = Stub::spawn((200, r#"{"data":{"id":"ses_2"}}"#.to_owned()));
        let mut backend = test_backend(stub.client());
        backend.lock_state().busy.insert("ses_1".into());
        backend.fork(None);
        assert!(stub.seen().is_empty());
        assert!(last_text(&backend).contains("disabled"));
    }

    #[test]
    fn revert_stage_commit_roundtrip() {
        let stub = Stub::spawn((
            200,
            r#"{"data":{"messageID":"msg_1","files":[{"file":"notes.txt","patch":"@@ -1,2 +1 @@\n line\n-probe\n","additions":0,"deletions":1,"status":"modified"}]}}"#.to_owned(),
        ));
        let mut backend = test_backend(stub.client());
        backend.stage_revert("msg_1".into());
        assert_eq!(stub.seen().len(), 1);
        assert!(stub.seen()[0].starts_with("POST /api/session/ses_1/revert/stage"));
        let blocks = backend
            .lock_state()
            .blocks
            .get("ses_1")
            .cloned()
            .unwrap_or_default();
        assert!(blocks
            .iter()
            .any(|block| matches!(block, Block::Edits { .. })));
        assert!(matches!(blocks.last(), Some(Block::Question { .. })));
        // Digit 1 commits: exactly one commit POST, boundary cleared.
        backend.answer_question(0);
        let seen = stub.seen();
        assert!(seen
            .iter()
            .any(|req| req.starts_with("POST /api/session/ses_1/revert/commit")));
        assert!(backend.lock_state().pending_revert.is_none());
        assert!(last_text(&backend).contains("Reverted to msg_1"));
    }

    #[test]
    fn revert_stage_reject_abandons() {
        let stub = Stub::spawn((
            200,
            r#"{"data":{"messageID":"msg_1","files":[]}}"#.to_owned(),
        ));
        let mut backend = test_backend(stub.client());
        backend.stage_revert("msg_1".into());
        backend.answer_question(1);
        let seen = stub.seen();
        assert!(seen
            .iter()
            .any(|req| req.starts_with("DELETE /api/session/ses_1/revert")));
        assert!(backend.lock_state().pending_revert.is_none());
        assert!(last_text(&backend).contains("abandoned"));
    }

    #[test]
    fn compact_admits_and_marks_busy() {
        let stub = Stub::spawn((200, r#"{"data":{"id":"ses_1"}}"#.to_owned()));
        let mut backend = test_backend(stub.client());
        backend.compact();
        assert_eq!(stub.seen().len(), 1);
        assert!(stub.seen()[0].starts_with("POST /api/session/ses_1/compact"));
        assert!(backend.lock_state().busy.contains("ses_1"));
        assert!(last_text(&backend).contains("Compact"));
    }

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

    /// Wipe the shared temp-dir quarantine queues (the 7C helper shares
    /// one temp dir per process; proposal state must not leak across
    /// tests).
    fn clear_queues() {
        let dir = std::env::temp_dir().join(format!("owt-7c-test-{}", std::process::id()));
        let _ = std::fs::remove_file(dir.join("proposals.jsonl"));
        let _ = std::fs::remove_file(dir.join("discarded.jsonl"));
    }

    #[test]
    fn suggest_refreshes_from_stub_history() {
        clear_queues();
        let history = r#"{"data":[{"type":"user","text":"Always write adapter tests first."},{"type":"assistant","text":"Never skip documentation."}],"cursor":{"previous":null,"next":null}}"#;
        let stub = Stub::spawn((200, history.to_owned()));
        let mut backend = test_backend(stub.client());
        backend.submit("/memory suggest".into());
        // The user message proposes; the identical assistant text cannot
        // (structural user-only input, 8-R2).
        let text = last_text(&backend);
        assert!(text.contains("quarantined"), "got: {text}");
        assert!(text.contains("Always write adapter tests first."));
        // Exactly one proposal: the assistant echo produced nothing.
        assert!(!text.contains("2. ["));
    }

    #[test]
    fn confirm_stores_and_discard_drops() {
        clear_queues();
        let history = r#"{"data":[{"type":"user","text":"Never land untested refactors."}],"cursor":{"previous":null,"next":null}}"#;
        let stub = Stub::spawn((200, history.to_owned()));
        let mut backend = test_backend(stub.client());
        backend.submit("/memory suggest".into());
        backend.submit("/memory confirm 1".into());
        let text = last_text(&backend);
        assert!(text.contains("confirmed"), "got: {text}");
        assert!(text.contains("method=rule:imperative-v1"), "got: {text}");
        // Second session, discard path.
        let history = r#"{"data":[{"type":"user","text":"I prefer tabs everywhere."}],"cursor":{"previous":null,"next":null}}"#;
        let stub = Stub::spawn((200, history.to_owned()));
        let mut backend = test_backend(stub.client());
        backend.submit("/memory suggest".into());
        backend.submit("/memory discard 1".into());
        assert!(last_text(&backend).contains("discarded"));
    }
}
