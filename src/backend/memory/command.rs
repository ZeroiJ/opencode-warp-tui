//! The `/memory` command surface (decision D19): a message-prefix
//! convention the OpenCode adapter intercepts at `submit`. The TUI is
//! unchanged — it submits the typed line through the `Backend` trait.
//!
//! Rules: strict parsing (malformed commands are errors, never prompts),
//! in-band replies, and a `\/memory` escape hatch that forwards the text to
//! the agent literally. Flags are `--scope=user|project`, `--kind=fact|
//! preference`, `--pinned`, `--all`; a leading `key:` on the first content
//! word addresses the memory by key.

use super::api::{NewMemoryArgs, ScopeTarget, WriteResult};
use super::key;
use super::record::{Kind, MemoryRecord, Scope};
use super::store::{ForgetOutcome, Handle};
use super::triage;

/// What the adapter should do with a submitted message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classified {
    /// A real `/memory` (or `/mem`) command: run the engine, reply in-band.
    Command,
    /// `\/memory …`: forward the text (minus the backslash) as a literal
    /// prompt.
    Escaped,
    /// Ordinary message: forward unmodified.
    Normal,
}

fn is_memory_prefix(text: &str) -> bool {
    text == "/memory" || text == "/mem" || text.starts_with("/memory ") || text.starts_with("/mem ")
}

/// Classify `text` (the adapter trims before calling). The escape hatch is
/// a leading backslash: `\/memory …` is Escaped, everything else that
/// matches the prefixes is a Command, and anything else is Normal.
pub fn classify(text: &str) -> Classified {
    match text.as_bytes().first() {
        Some(b'\\') if is_memory_prefix(&text[1..]) => Classified::Escaped,
        _ if is_memory_prefix(text) => Classified::Command,
        _ => Classified::Normal,
    }
}

/// The text after the `/memory `/`/mem ` prefix ("" for the bare form).
pub fn prefix_remainder(text: &str) -> &str {
    text.strip_prefix("/memory ")
        .or_else(|| text.strip_prefix("/mem "))
        .unwrap_or("")
}

/// A parsed memory command.
#[derive(Clone, Debug)]
pub enum Command {
    Remember(NewMemoryArgs),
    Update(NewMemoryArgs),
    Forget {
        handle: Handle,
        scope: ScopeTarget,
    },
    Pin {
        handle: Handle,
        pinned: bool,
        scope: ScopeTarget,
    },
    List {
        scope: ScopeTarget,
        kind: Option<Kind>,
        pinned: Option<bool>,
    },
    Show {
        handle: Handle,
        all: bool,
        scope: ScopeTarget,
    },
    /// Phase 8: list quarantined proposals (deterministic order).
    Suggest,
    /// Phase 8: confirm a proposal by suggest-index or exact id, with
    /// optional scope/kind overrides.
    Confirm {
        target: triage::Target,
        scope: Option<Scope>,
        kind: Option<Kind>,
    },
    /// Phase 8: discard by index/id, or `all` (requires `--confirm`).
    /// `target: None` = all.
    Discard {
        target: Option<triage::Target>,
        confirm_all: bool,
    },
    Help,
}

/// Parse the text after the prefix. Errors are usage errors rendered
/// in-band as `Block::Error` — never forwarded as prompts.
pub fn parse(text: &str) -> Result<Command, String> {
    let text = text.trim();
    if text.is_empty() || matches!(text, "help" | "-h" | "--help") {
        return Ok(Command::Help);
    }
    let mut scanner = TokenScanner::new(text);
    let verb = scanner.next_token().ok_or("missing command verb")?;
    match verb {
        "remember" => parse_write(&mut scanner, true),
        "update" => parse_write(&mut scanner, false),
        "forget" => parse_handle_command(&mut scanner, None),
        "pin" => parse_handle_command(&mut scanner, Some(true)),
        "unpin" => parse_handle_command(&mut scanner, Some(false)),
        "list" => parse_list(&mut scanner),
        "show" => parse_show(&mut scanner),
        "suggest" => parse_suggest(&mut scanner),
        "confirm" => parse_confirm(&mut scanner),
        "discard" => parse_discard(&mut scanner),
        other => Err(format!(
            "unknown memory command {other:?} — try `/memory help`"
        )),
    }
}

/// A parsed flag token. Unknown `--…` tokens are *not* flags: content
/// commands let them start the content (so content may begin with "--"),
/// non-content commands reject them.
#[derive(Clone, Copy, Debug)]
enum Flag {
    Scope(ScopeTarget),
    Kind(Kind),
    Pinned,
    All,
}

fn known_flag(token: &str) -> Option<Result<Flag, String>> {
    match token {
        "--pinned" => Some(Ok(Flag::Pinned)),
        "--all" => Some(Ok(Flag::All)),
        _ => {
            if let Some(value) = token.strip_prefix("--scope=") {
                return Some(match value {
                    "user" => Ok(Flag::Scope(ScopeTarget::User)),
                    "project" => Ok(Flag::Scope(ScopeTarget::Project)),
                    other => Err(format!("--scope={other:?} (want user or project)")),
                });
            }
            if let Some(value) = token.strip_prefix("--kind=") {
                return Some(match value {
                    "fact" => Ok(Flag::Kind(Kind::Fact)),
                    "preference" => Ok(Flag::Kind(Kind::Preference)),
                    other => Err(format!("--kind={other:?} (want fact or preference)")),
                });
            }
            None
        }
    }
}

fn parse_write(scanner: &mut TokenScanner<'_>, remember: bool) -> Result<Command, String> {
    let mut kind = Kind::Fact;
    let mut scope = ScopeTarget::User;
    let mut pinned = false;
    loop {
        let Some(token) = scanner.peek() else {
            break;
        };
        match known_flag(token) {
            Some(Ok(Flag::Scope(s))) => {
                scope = s;
                scanner.advance();
            }
            Some(Ok(Flag::Kind(k))) => {
                kind = k;
                scanner.advance();
            }
            Some(Ok(Flag::Pinned)) => {
                pinned = true;
                scanner.advance();
            }
            Some(Ok(Flag::All)) => return Err("--all is not valid here".into()),
            Some(Err(message)) => return Err(message),
            None => break, // first non-flag token starts the content
        }
    }
    let verb = if remember { "remember" } else { "update" };
    let raw = scanner.raw_rest();
    if raw.is_empty() {
        return Err(format!(
            "{verb} needs content — try `/memory {verb} <key:> <content>`"
        ));
    }
    // `key:` prefix on the first whitespace-delimited word.
    let (key, content) = match first_word(raw) {
        Some((word, rest)) if word.ends_with(':') => {
            let key = &word[..word.len() - 1];
            let key = key::validate_key(key).map_err(|e| format!("bad key: {e}"))?;
            let content = rest.trim();
            if content.is_empty() {
                return Err(format!("{verb} needs content after the key {key:?}"));
            }
            (Some(key), content.to_owned())
        }
        _ => (None, raw.to_owned()),
    };
    if !remember && key.is_none() {
        return Err("update needs a key — /memory update <key>: <content>".into());
    }
    let args = NewMemoryArgs {
        key,
        kind,
        content,
        pinned,
        scope,
        method: None,
        quote: None,
        session_id: None,
    };
    if remember {
        Ok(Command::Remember(args))
    } else {
        Ok(Command::Update(args))
    }
}

fn first_word(text: &str) -> Option<(&str, &str)> {
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    if end == 0 {
        None
    } else {
        Some((&text[..end], &text[end..]))
    }
}

fn parse_handle_command(
    scanner: &mut TokenScanner<'_>,
    pin: Option<bool>,
) -> Result<Command, String> {
    let mut scope = ScopeTarget::User;
    let mut handle: Option<Handle> = None;
    loop {
        let Some(token) = scanner.next_token() else {
            break;
        };
        match known_flag(token) {
            Some(Ok(Flag::Scope(s))) => scope = s,
            Some(Ok(Flag::Pinned | Flag::All | Flag::Kind(_))) => {
                return Err(format!("unexpected flag {token:?} here"));
            }
            Some(Err(message)) => return Err(message),
            None if token.starts_with("--") => return Err(format!("unknown flag {token:?}")),
            None => {
                if handle.is_some() {
                    return Err(format!("unexpected argument {token:?}"));
                }
                handle = Some(parse_handle(token)?);
            }
        }
    }
    let Some(handle) = handle else {
        return Err("missing <key|id>".into());
    };
    Ok(match pin {
        None => Command::Forget { handle, scope },
        Some(pinned) => Command::Pin {
            handle,
            pinned,
            scope,
        },
    })
}

fn parse_list(scanner: &mut TokenScanner<'_>) -> Result<Command, String> {
    let mut scope = ScopeTarget::Default;
    let mut kind = None;
    let mut pinned = None;
    loop {
        let Some(token) = scanner.next_token() else {
            break;
        };
        match known_flag(token) {
            Some(Ok(Flag::Scope(s))) => scope = s,
            Some(Ok(Flag::Kind(k))) => kind = Some(k),
            Some(Ok(Flag::Pinned)) => pinned = Some(true),
            Some(Ok(Flag::All)) => return Err("--all is not valid for list".into()),
            Some(Err(message)) => return Err(message),
            None if token.starts_with("--") => return Err(format!("unknown flag {token:?}")),
            None => return Err(format!("unexpected argument {token:?} for list")),
        }
    }
    Ok(Command::List {
        scope,
        kind,
        pinned,
    })
}

fn parse_show(scanner: &mut TokenScanner<'_>) -> Result<Command, String> {
    let mut scope = ScopeTarget::Default;
    let mut all = false;
    let mut handle = None;
    loop {
        let Some(token) = scanner.next_token() else {
            break;
        };
        match known_flag(token) {
            Some(Ok(Flag::Scope(s))) => scope = s,
            Some(Ok(Flag::All)) => all = true,
            Some(Ok(Flag::Pinned | Flag::Kind(_))) => {
                return Err(format!("unexpected flag {token:?}"));
            }
            Some(Err(message)) => return Err(message),
            None if token.starts_with("--") => return Err(format!("unknown flag {token:?}")),
            None => {
                if handle.is_some() {
                    return Err(format!("unexpected argument {token:?}"));
                }
                handle = Some(parse_handle(token)?);
            }
        }
    }
    let Some(handle) = handle else {
        return Err("missing <key|id> — try `/memory show <key|id>`".into());
    };
    Ok(Command::Show { handle, all, scope })
}

/// A confirm/discard target token: `1`-based suggest index or exact id.
fn parse_target(token: &str) -> Result<triage::Target, String> {
    if let Ok(n) = token.parse::<usize>() {
        if n >= 1 {
            return Ok(triage::Target::Index(n));
        }
        return Err(format!("proposal index {token:?} starts at 1"));
    }
    if key::is_id_shape(token) {
        return Ok(triage::Target::Id(token.to_owned()));
    }
    Err(format!(
        "invalid proposal target {token:?} (want a suggest index or id)"
    ))
}

fn parse_suggest(scanner: &mut TokenScanner<'_>) -> Result<Command, String> {
    if let Some(token) = scanner.next_token() {
        return Err(format!("unexpected argument {token:?} for suggest"));
    }
    Ok(Command::Suggest)
}

fn parse_confirm(scanner: &mut TokenScanner<'_>) -> Result<Command, String> {
    let mut scope: Option<Scope> = None;
    let mut kind: Option<Kind> = None;
    let mut target = None;
    loop {
        let Some(token) = scanner.next_token() else {
            break;
        };
        if let Some(value) = token.strip_prefix("--scope=") {
            match value {
                "user" => scope = Some(Scope::User),
                "project" => scope = Some(Scope::Project),
                other => return Err(format!("--scope={other:?} (want user or project)")),
            }
        } else if let Some(value) = token.strip_prefix("--as=") {
            match value {
                "fact" => kind = Some(Kind::Fact),
                "preference" => kind = Some(Kind::Preference),
                other => return Err(format!("--as={other:?} (want fact or preference)")),
            }
        } else if token.starts_with("--") {
            return Err(format!("unknown flag {token:?}"));
        } else {
            if target.is_some() {
                return Err(format!("unexpected argument {token:?}"));
            }
            target = Some(parse_target(token)?);
        }
    }
    let Some(target) = target else {
        return Err("missing <n|id> — try `/memory confirm 1`".into());
    };
    Ok(Command::Confirm {
        target,
        scope,
        kind,
    })
}

fn parse_discard(scanner: &mut TokenScanner<'_>) -> Result<Command, String> {
    let mut target = None;
    let mut all = false;
    let mut confirm_all = false;
    loop {
        let Some(token) = scanner.next_token() else {
            break;
        };
        if token == "--confirm" {
            confirm_all = true;
        } else if token.starts_with("--") {
            return Err(format!("unknown flag {token:?}"));
        } else if token == "all" {
            if target.is_some() || all {
                return Err(format!("unexpected argument {token:?}"));
            }
            all = true;
        } else {
            if target.is_some() || all {
                return Err(format!("unexpected argument {token:?}"));
            }
            target = Some(parse_target(token)?);
        }
    }
    if all {
        return Ok(Command::Discard {
            target: None,
            confirm_all,
        });
    }
    let Some(target) = target else {
        return Err("missing <n|id|all> — try `/memory discard 1`".into());
    };
    if confirm_all {
        return Err("--confirm is only valid with `discard all`".into());
    }
    Ok(Command::Discard {
        target: Some(target),
        confirm_all: false,
    })
}

/// A handle token: `owt_<millis>_<pid>_<seq>` is an id, anything else is an
/// exact key.
fn parse_handle(token: &str) -> Result<Handle, String> {
    if key::is_id_shape(token) {
        Ok(Handle::Id(token.to_owned()))
    } else {
        let key = key::validate_key(token).map_err(|_| format!("invalid key {token:?}"))?;
        Ok(Handle::Key(key))
    }
}

/// Whitespace tokenizer that remembers where it stopped, so command
/// *content* keeps its original internal spacing.
struct TokenScanner<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> TokenScanner<'a> {
    fn new(text: &'a str) -> Self {
        TokenScanner { text, pos: 0 }
    }

    fn peek(&self) -> Option<&'a str> {
        let bytes = self.text.as_bytes();
        let mut pos = self.pos;
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            return None;
        }
        let start = pos;
        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        Some(&self.text[start..pos])
    }

    fn advance(&mut self) {
        // Consume leading whitespace plus one full token, so the position
        // always lands on the next token boundary (naive `pos += token.len()`
        // from the pre-whitespace position cuts into the token).
        let bytes = self.text.as_bytes();
        let mut pos = self.pos;
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        self.pos = pos;
    }

    fn next_token(&mut self) -> Option<&'a str> {
        let token = self.peek()?;
        // Land past the token: skip its leading whitespace first (same
        // offset rule as `advance`, so the next token starts fresh).
        let bytes = self.text.as_bytes();
        let mut pos = self.pos;
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        self.pos = pos + token.len();
        Some(token)
    }

    /// Raw remainder from the current position to the end (original
    /// whitespace preserved), trimmed.
    fn raw_rest(&self) -> &'a str {
        self.text[self.pos..].trim()
    }
}

// ---------------------------------------------------------------------------
// In-band reply text (no memory content in logs; replies show ids/keys).
// ---------------------------------------------------------------------------

pub fn help_text() -> &'static str {
    "memory commands — `/memory` (or `/mem`) with in-band replies:
  remember [<key>:] <content>           [--scope=user|project] [--kind=fact|preference] [--pinned]
  update <key>: <content>               [--scope=…] [--kind=…]
  forget <key|id>                       [--scope=…]  (exact key or id; ambiguous → candidates listed)
  pin | unpin <key|id>                  [--scope=…]
  list                                  [--scope=…] [--kind=…] [--pinned]
  show <key|id>                         [--all] [--scope=…]
  suggest                               (quarantined rule proposals — not memories until confirmed)
  confirm <n|id>                        [--scope=user|project] [--as=fact|preference]
  discard <n|id|all>                    (`all` needs `--confirm`)
default scope: user ($XDG_DATA_HOME/owt); project store: <root>/.owt (--scope=project).
escape: start a message with \\/memory to send it to the agent literally."
}

/// Success reply for remember/update.
pub fn written_reply(verb: &str, result: &WriteResult) -> String {
    let created = &result.outcome.created;
    let mut text = format!(
        "{verb} ({}) id {} kind={}",
        created.scope.as_str(),
        created.id,
        created.kind.as_str()
    );
    if let Some(key) = &created.key {
        text.push_str(&format!(" key={key}"));
    }
    if created.pinned {
        text.push_str(" pinned");
    }
    if let Some(superseded) = &result.outcome.superseded {
        text.push_str(&format!(" · superseded id {}", superseded.id));
    }
    if let Some(warning) = &result.warning {
        text.push_str("\nnote: ");
        text.push_str(warning);
    }
    text
}

/// Success reply for forget.
pub fn forget_reply(outcome: &ForgetOutcome) -> String {
    let mut text = format!("forgot id {}", outcome.id);
    if let Some(key) = &outcome.key {
        text.push_str(&format!(" key={key}"));
    }
    text.push_str(" (tombstoned)");
    text
}

/// Success reply for pin/unpin.
pub fn pin_reply(record: &MemoryRecord, pinned: bool) -> String {
    format!(
        "{} id {}{}",
        if pinned { "pinned" } else { "unpinned" },
        record.id,
        record
            .key
            .as_deref()
            .map(|k| format!(" key={k}"))
            .unwrap_or_default()
    )
}

/// Success reply for list (D24 order, one line per record).
pub fn list_reply(records: &[MemoryRecord]) -> String {
    if records.is_empty() {
        return "no memories stored".to_owned();
    }
    let mut lines = Vec::with_capacity(records.len());
    for record in records {
        let pinned = if record.pinned { "pinned " } else { "" };
        lines.push(format!(
            "[{}] {} · {} · {pinned}{} · {}",
            record.scope.as_str(),
            record.key.as_deref().unwrap_or("(no key)"),
            record.kind.as_str(),
            record.updated_at,
            truncate(&record.content, 96),
        ));
    }
    lines.join("\n")
}

/// Success reply for show (full content + status; `--all` shows the key's
/// SUPERSEDED history too).
pub fn show_reply(records: &[MemoryRecord], handle: &Handle) -> String {
    if records.is_empty() {
        return format!("no memory matching {handle}");
    }
    let mut blocks = Vec::with_capacity(records.len());
    for record in records {
        blocks.push(format!(
            "id {} · {} · scope {} · {}{} · {}\nstatus {}\n{}",
            record.id,
            record.kind.as_str(),
            record.scope.as_str(),
            record.key.as_deref().unwrap_or("(no key)"),
            if record.pinned { " · pinned" } else { "" },
            record.updated_at,
            record.status.as_str(),
            record.content
        ));
    }
    blocks.join("\n\n")
}

fn truncate(text: &str, max: usize) -> String {
    let count = text.chars().count();
    let mut out: String = text.chars().take(max).collect();
    if count > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_routes_prefixes() {
        assert_eq!(classify("/memory"), Classified::Command);
        assert_eq!(classify("/memory list"), Classified::Command);
        assert_eq!(classify("/mem list"), Classified::Command);
        assert_eq!(classify("/mem"), Classified::Command);
        // Escape hatch.
        assert_eq!(classify("\\/memory list"), Classified::Escaped);
        assert_eq!(classify("\\/mem x"), Classified::Escaped);
        // Escaped form that is not a memory prefix → Normal.
        assert_eq!(classify("\\/remember x"), Classified::Normal);
        // Ordinary messages.
        assert_eq!(classify("hello"), Classified::Normal);
        assert_eq!(classify("/memoryish list"), Classified::Normal);
        assert_eq!(classify(""), Classified::Normal);
    }

    #[test]
    fn prefix_remainder_slices() {
        assert_eq!(prefix_remainder("/memory"), "");
        assert_eq!(prefix_remainder("/mem"), "");
        assert_eq!(prefix_remainder("/memory  list  here"), " list  here");
        assert_eq!(prefix_remainder("/mem x"), "x");
    }

    #[test]
    fn parse_help_and_unknown_verb() {
        assert!(matches!(parse(""), Ok(Command::Help)));
        assert!(matches!(parse("help"), Ok(Command::Help)));
        assert!(parse("wat").is_err());
        assert!(parse("remember").is_err()); // missing content
    }

    #[test]
    fn parse_remember_key_and_content() {
        match parse("remember lang: Prefer Rust for new services.").unwrap() {
            Command::Remember(args) => {
                assert_eq!(args.key.as_deref(), Some("lang"));
                assert_eq!(args.content, "Prefer Rust for new services.");
                assert_eq!(args.scope, ScopeTarget::User);
                assert_eq!(args.kind, Kind::Fact);
                assert!(!args.pinned);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_remember_preserves_internal_spacing() {
        match parse("remember  keyless   spaced   content").unwrap() {
            Command::Remember(args) => {
                assert!(args.key.is_none());
                assert_eq!(args.content, "keyless   spaced   content");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_remember_flags() {
        match parse("remember --scope=project --kind=preference --pinned clue: always test")
            .unwrap()
        {
            Command::Remember(args) => {
                assert_eq!(args.scope, ScopeTarget::Project);
                assert_eq!(args.kind, Kind::Preference);
                assert!(args.pinned);
                assert_eq!(args.key.as_deref(), Some("clue"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(parse("remember --scope=bogus x").is_err());
        assert!(parse("remember --kind=bogus x").is_err());
    }

    #[test]
    fn parse_content_may_begin_with_dashes() {
        match parse("remember --verbose flag text").unwrap() {
            Command::Remember(args) => {
                assert!(args.key.is_none());
                assert_eq!(args.content, "--verbose flag text");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_update_needs_key() {
        assert!(parse("update no key here").is_err());
        match parse("update lang: Prefer Go.").unwrap() {
            Command::Update(args) => assert_eq!(args.key.as_deref(), Some("lang")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parse_handle_commands() {
        match parse("forget lang").unwrap() {
            Command::Forget { handle, scope } => {
                assert_eq!(handle, Handle::Key("lang".into()));
                assert_eq!(scope, ScopeTarget::User);
            }
            other => panic!("unexpected {other:?}"),
        }
        match parse("forget owt_1780000000000_1234_7").unwrap() {
            Command::Forget { handle, .. } => {
                assert_eq!(handle, Handle::Id("owt_1780000000000_1234_7".into()));
            }
            other => panic!("unexpected {other:?}"),
        }
        match parse("pin lang --scope=project").unwrap() {
            Command::Pin {
                handle,
                pinned,
                scope,
            } => {
                assert_eq!(handle, Handle::Key("lang".into()));
                assert!(pinned);
                assert_eq!(scope, ScopeTarget::Project);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(parse("forget").is_err());
        assert!(parse("forget a b").is_err());
        assert!(parse("forget --bogus lang").is_err());
    }

    #[test]
    fn parse_list_and_show_flags() {
        match parse("list --scope=project --kind=preference --pinned").unwrap() {
            Command::List {
                scope,
                kind,
                pinned,
            } => {
                assert_eq!(scope, ScopeTarget::Project);
                assert_eq!(kind, Some(Kind::Preference));
                assert_eq!(pinned, Some(true));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(parse("list extra").is_err());
        match parse("show lang --all").unwrap() {
            Command::Show { handle, all, scope } => {
                assert_eq!(handle, Handle::Key("lang".into()));
                assert!(all);
                assert_eq!(scope, ScopeTarget::Default);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(parse("show").is_err());
    }

    #[test]
    fn parse_triage_verbs() {
        assert!(matches!(parse("suggest"), Ok(Command::Suggest)));
        assert!(parse("suggest extra").is_err());
        match parse("confirm 2").unwrap() {
            Command::Confirm {
                target,
                scope,
                kind,
            } => {
                assert!(matches!(target, triage::Target::Index(2)));
                assert!(scope.is_none() && kind.is_none());
            }
            other => panic!("unexpected {other:?}"),
        }
        match parse("confirm owt_1_2_3 --scope=project --as=preference").unwrap() {
            Command::Confirm {
                target,
                scope,
                kind,
            } => {
                assert!(matches!(target, triage::Target::Id(_)));
                assert_eq!(scope, Some(Scope::Project));
                assert_eq!(kind, Some(Kind::Preference));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(parse("confirm").is_err());
        assert!(parse("confirm 0").is_err());
        assert!(parse("confirm abc").is_err());
        assert!(parse("confirm 1 --as=bogus").is_err());
        match parse("discard all").unwrap() {
            Command::Discard {
                target,
                confirm_all,
            } => {
                assert!(target.is_none() && !confirm_all);
            }
            other => panic!("unexpected {other:?}"),
        }
        match parse("discard all --confirm").unwrap() {
            Command::Discard {
                target,
                confirm_all,
            } => {
                assert!(target.is_none() && confirm_all);
            }
            other => panic!("unexpected {other:?}"),
        }
        match parse("discard 1").unwrap() {
            Command::Discard {
                target,
                confirm_all,
            } => {
                assert!(matches!(target, Some(triage::Target::Index(1))));
                assert!(!confirm_all);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(parse("discard").is_err());
        assert!(parse("discard 1 --confirm").is_err());
        assert!(parse("discard 1 2").is_err());
    }

    #[test]
    fn replies_reference_ids_never_content_in_error_path() {
        // list_reply truncates long content.
        let record = |content: &str| MemoryRecord {
            id: "owt_1_2_3".into(),
            key: Some("k".into()),
            kind: Kind::Fact,
            scope: crate::backend::memory::record::Scope::User,
            content: content.into(),
            source: crate::backend::memory::record::Source::User,
            status: crate::backend::memory::record::Status::Active,
            pinned: false,
            created_at: "2026-09-18T10:00:00Z".into(),
            updated_at: "2026-09-18T10:00:00Z".into(),
            session_id: "ses".into(),
            source_ref: None,
            quote: None,
            method: None,
        };
        let reply = list_reply(&[record(&"x".repeat(200))]);
        assert!(reply.contains("…"));
        assert!(reply.contains("[user] k"));
    }
}
