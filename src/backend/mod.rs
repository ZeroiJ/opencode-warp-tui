//! Backend interface — the seam between presentation and data.
//!
//! The TUI must never depend on Warp internals or (later) on OpenCode
//! directly. Views render from [`Backend`] snapshots; user gestures call back
//! into it. Phase 2 ships only the [`MockBackend`](mock::MockBackend); the
//! OpenCode adapter implements this same trait in a later phase.

pub mod memory;
pub mod mock;
pub mod opencode;
pub mod stream;

use std::fmt;

/// Execution state of a tool call or shell command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolState {
    Running,
    Done,
    Failed,
    Waiting,
}

/// One agent tool invocation (read, edit, shell, MCP, …).
#[derive(Clone, Debug)]
pub struct ToolCall {
    pub name: String,
    pub detail: String,
    pub state: ToolState,
    /// Captured output lines (completed/failed tools; empty while running).
    pub output: Vec<String>,
    pub elapsed: String,
}

/// A shell command plus captured output.
#[derive(Clone, Debug)]
pub struct ShellRun {
    pub command: String,
    pub output: Vec<String>,
    pub state: ToolState,
}

/// One edited file's unified-diff-style hunks (already split per line).
#[derive(Clone, Debug)]
pub struct FileDiff {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    /// `(is_addition, line)` pairs.
    pub lines: Vec<(bool, String)>,
}

/// A permission gate blocking the transcript.
#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub tool: String,
    pub summary: String,
}

/// An agent question with numbered options.
#[derive(Clone, Debug)]
pub struct Question {
    pub prompt: String,
    pub options: Vec<String>,
}

/// One transcript block, mirroring Warp's block-list vocabulary
/// (user input, thinking, assistant sections, tool calls, shell output,
/// diffs, plans, blockers, errors).
#[derive(Clone, Debug)]
pub enum Block {
    User {
        text: String,
    },
    Thinking {
        text: String,
    },
    Assistant {
        text: String,
    },
    Tool {
        call: ToolCall,
    },
    Shell {
        run: ShellRun,
    },
    Edits {
        files: Vec<FileDiff>,
    },
    Plan {
        title: String,
        body: String,
    },
    Permission {
        req: PermissionRequest,
    },
    Question {
        q: Question,
    },
    Working {
        label: String,
    },
    Error {
        text: String,
    },
    Notice {
        text: String,
        link: Option<(String, String)>,
    },
}

/// One agent session (a tab in the tab strip).
#[derive(Clone, Debug)]
pub struct Session {
    /// Stable backend identity for tab keys and stream routing. Opaque to
    /// the TUI (OpenCode uses `ses_*` strings; the mock uses `mock-N`).
    pub id: String,
    pub title: String,
    pub blocks: Vec<Block>,
}

/// A slash command offered by the backend.
#[derive(Clone, Debug)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
}

/// One session's tab-strip identity (cheap snapshot; blocks come separately).
#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
}

/// Statusline snapshot.
#[derive(Clone, Debug)]
pub struct StatusInfo {
    pub model: String,
    pub context_pct: u8,
    pub cwd: String,
    pub branch: String,
    pub status: AgentStatus,
}

/// Agent activity state for the statusline and tab indicators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    Idle,
    Working,
}

/// One incremental backend event. The frontend applies events in order via
/// [`Backend::poll_stream`], re-rendering after each — this is the entire
/// streaming architecture: no threads or callbacks in the TUI, just a pump.
#[derive(Clone, Debug)]
pub enum StreamEvent {
    /// Append text to the trailing assistant block (creating it if needed).
    Chunk(String),
    /// Append text to the trailing thinking block (creating it if needed).
    /// Carries streamed reasoning (OpenCode `reasoning.delta`); kept separate
    /// from `Chunk` so thinking keeps its muted presentation.
    ThinkChunk(String),
    /// Append a fully-formed block.
    Push(Block),
    /// Update the latest tool/shell block's state (start → done), even when
    /// a working indicator was pushed after it. Carries captured output.
    UpdateTool {
        state: ToolState,
        elapsed: String,
        output: Vec<String>,
    },
    /// Append an output line to the latest shell block.
    ShellLine(String),
    /// Replace the trailing working block's label (phase changes).
    WorkLabel(String),
}

/// The backend contract. Snapshot-based and object-safe so views can hold
/// `Rc<RefCell<dyn Backend>>`:
/// - `session_summaries` / `session_blocks` return owned snapshots (cloned).
///   Backends keep them cheap; the transcript's per-frame clone is small
///   strings and documented as such (row-level caching is Phase-5 work).
/// - Streaming is a polled event queue: `poll_stream` advances every
///   non-empty session and reports whether anything remains. Any backend
///   (mock script or live adapter worker) drives the same frontend pump.
pub trait Backend {
    fn session_summaries(&self) -> Vec<SessionSummary>;
    /// Full transcript snapshot for one session id (empty when unknown).
    fn session_blocks(&self, id: &str) -> Vec<Block>;
    fn active(&self) -> usize;
    fn set_active(&mut self, index: usize);
    /// Latest-block blocker, if the transcript ends in a gate the user must
    /// answer before typing continues.
    fn blocker(&self) -> Option<Blocker>;
    fn commands(&self) -> Vec<SlashCommand>;
    fn status(&self) -> StatusInfo;
    /// Submit prompt text; the backend appends user + response blocks (or
    /// enqueues a stream — see below).
    fn submit(&mut self, text: String);
    fn resolve_permission(&mut self, accept: bool);
    fn answer_question(&mut self, option: usize);
    /// Phase 7C (all additive; every method has MockBackend parity):
    /// advanced agent operations. Destructive-adjacent ops confirm through
    /// the existing Question-gate digits (no new TUI seams); failures are
    /// in-band Errors that never kill the session.
    /// Admit an async compaction (lifecycle via compaction events/markers).
    fn compact(&mut self);
    /// Render the session diff read-only (empty → explicit empty state).
    fn diff(&mut self);
    /// Stage a revert boundary (restores files immediately, shows affected
    /// files, requires digit confirmation before commit).
    fn stage_revert(&mut self, message_id: String);
    /// Commit the staged boundary (deletes post-boundary messages,
    /// irreversible). Errors visibly with no boundary staged.
    fn commit_revert(&mut self);
    /// Abandon the staged boundary (messages kept).
    fn abandon_revert(&mut self);
    /// Fork the session (full history, or history-before a message).
    /// Disabled while a turn is running (conservative).
    fn fork(&mut self, before: Option<String>);
    /// Switch model/agent after client-side discovery validation; bogus
    /// ids are rejected locally and never sent (the server 204-accepts
    /// anything into a dead turn).
    fn switch_model(&mut self, id: String);
    fn switch_agent(&mut self, id: String);
    /// Execute a discovered server slash command (writer commands confirm
    /// via Question-gate digits; unknown names error visibly, never sent).
    fn execute_command(&mut self, name: String, text: String);
    /// Append one scripted agent turn (demo control).
    fn simulate_activity(&mut self);
    /// Create a session and return its id.
    fn new_session(&mut self, title: String) -> String;
    /// Drop pending stream events and clear working indicators.
    fn cancel(&mut self);
    /// Overall agent state for the active session.
    fn agent_status(&self) -> AgentStatus;
    /// Enqueue incremental events for a session's stream.
    fn push_stream(&mut self, session: String, events: Vec<StreamEvent>);
    /// Apply the next pending event in every non-empty session queue.
    /// Returns true while any events remain (pump again).
    fn poll_stream(&mut self) -> bool;
    /// Whether any session has pending stream events.
    fn stream_active(&self) -> bool;
}

/// Which gate blocks the transcript, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocker {
    Permission,
    Question,
}

impl fmt::Display for ToolState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolState::Running => write!(f, "running"),
            ToolState::Done => write!(f, "done"),
            ToolState::Failed => write!(f, "failed"),
            ToolState::Waiting => write!(f, "waiting"),
        }
    }
}
