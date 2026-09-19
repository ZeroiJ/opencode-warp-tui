//! The exact Phase 2B `v:1` memory record (decision D16), its validation,
//! RFC3339-UTC timestamp handling, and the D24 deterministic ordering.
//!
//! Files are loaded and validated through [`record_from_line`]; ingestion is
//! validated through [`NewMemory::validate`] (defense in depth — the API
//! layer validates first, the store re-validates). Unknown JSON fields are
//! tolerated on load (forward compatibility); wrong types and bad enum
//! values are not.

use serde_json::{Map, Value};

use super::key;
use super::MemoryError;

/// Schema version of every persisted record.
pub const SCHEMA_VERSION: i64 = 1;

/// Content character cap (D25).
pub const MAX_CONTENT_CHARS: usize = 4096;
/// Quote character cap (D25).
pub const MAX_QUOTE_CHARS: usize = 256;
/// Key length cap (D25: `^[a-z0-9][a-z0-9._-]{0,63}$`).
pub const MAX_KEY_CHARS: usize = 64;

/// Memory kinds (D16). Only these two exist in V1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Fact,
    Preference,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Fact => "fact",
            Kind::Preference => "preference",
        }
    }

    pub fn parse(value: &str) -> Option<Kind> {
        match value {
            "fact" => Some(Kind::Fact),
            "preference" => Some(Kind::Preference),
            _ => None,
        }
    }
}

/// Memory scopes (D16). User is machine-wide; project is per-project-root.
/// No session scope exists in V1 (D2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    User,
    Project,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Project => "project",
        }
    }

    pub fn parse(value: &str) -> Option<Scope> {
        match value {
            "user" => Some(Scope::User),
            "project" => Some(Scope::Project),
            _ => None,
        }
    }
}

/// Record source (D6/D16). V1 writers emit `user` only; `inferred` is
/// reserved for V2 extraction and tolerated by readers for forward
/// compatibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    User,
    Inferred,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::User => "user",
            Source::Inferred => "inferred",
        }
    }

    pub fn parse(value: &str) -> Option<Source> {
        match value {
            "user" => Some(Source::User),
            "inferred" => Some(Source::Inferred),
            _ => None,
        }
    }
}

/// Operational state (D16). V1 persists ACTIVE and SUPERSEDED only;
/// DELETED is reserved for V2 soft-delete — forgetting in V1 removes the
/// line physically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Active,
    Superseded,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Active => "ACTIVE",
            Status::Superseded => "SUPERSEDED",
        }
    }

    pub fn parse(value: &str) -> Option<Status> {
        match value {
            "ACTIVE" => Some(Status::Active),
            "SUPERSEDED" => Some(Status::Superseded),
            _ => None,
        }
    }
}

/// A complete `v:1` memory record (D16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryRecord {
    pub id: String,
    pub key: Option<String>,
    pub kind: Kind,
    pub scope: Scope,
    pub content: String,
    pub source: Source,
    pub status: Status,
    pub pinned: bool,
    /// RFC3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`).
    pub created_at: String,
    /// RFC3339 UTC; set at every write/maintenance touch.
    pub updated_at: String,
    /// Session provenance (required). Opaque to the store.
    pub session_id: String,
    /// Optional best-effort in-session reference; never fabricated.
    pub source_ref: Option<String>,
    /// Optional evidence quote (≤256 chars); never fabricated.
    pub quote: Option<String>,
}

impl MemoryRecord {
    /// Build a record from validated ingestion input plus the store-assigned
    /// scope and timestamps (`now` is `created_at`/`updated_at`).
    pub fn from_new(new: &NewMemory, scope: Scope, now: &str) -> MemoryRecord {
        MemoryRecord {
            id: new.id.clone(),
            key: new.key.clone(),
            kind: new.kind,
            scope,
            content: new.content.clone(),
            source: Source::User,
            status: Status::Active,
            pinned: new.pinned,
            created_at: now.to_owned(),
            updated_at: now.to_owned(),
            session_id: new.session_id.clone(),
            source_ref: new.source_ref.clone(),
            quote: new.quote.clone(),
        }
    }

    /// Deterministic one-line JSONL serialization. Field order is the D16
    /// schema order; `serde_json` sorts object keys (`Map`), which is
    /// stable, so files remain byte-stable for identical records.
    pub fn to_json_line(&self) -> String {
        let mut object = Map::new();
        object.insert("v".into(), Value::from(SCHEMA_VERSION));
        object.insert("id".into(), Value::from(self.id.clone()));
        object.insert(
            "key".into(),
            self.key
                .as_ref()
                .map_or(Value::Null, |k| Value::from(k.clone())),
        );
        object.insert("kind".into(), Value::from(self.kind.as_str()));
        object.insert("scope".into(), Value::from(self.scope.as_str()));
        object.insert("content".into(), Value::from(self.content.clone()));
        object.insert("source".into(), Value::from(self.source.as_str()));
        object.insert("status".into(), Value::from(self.status.as_str()));
        object.insert("pinned".into(), Value::from(self.pinned));
        object.insert("created_at".into(), Value::from(self.created_at.clone()));
        object.insert("updated_at".into(), Value::from(self.updated_at.clone()));
        object.insert("session_id".into(), Value::from(self.session_id.clone()));
        object.insert(
            "source_ref".into(),
            self.source_ref
                .as_ref()
                .map_or(Value::Null, |r| Value::from(r.clone())),
        );
        object.insert(
            "quote".into(),
            self.quote
                .as_ref()
                .map_or(Value::Null, |q| Value::from(q.clone())),
        );
        serde_json::to_string(&Value::Object(object)).expect("record serializes to JSON")
    }
}

/// Ingestion input for a new record. The store assigns timestamps; the
/// caller supplies a globally unique `id` (see `key::generate_id`).
#[derive(Clone, Debug)]
pub struct NewMemory {
    pub id: String,
    pub key: Option<String>,
    pub kind: Kind,
    pub content: String,
    pub pinned: bool,
    pub session_id: String,
    pub source_ref: Option<String>,
    pub quote: Option<String>,
}

impl NewMemory {
    /// Schema-level validation for *ingestion*. Content/quote controls and
    /// sizes, key format, id shape, provenance presence. Empty content is
    /// rejected; content is stored trimmed (command-surface normalization).
    pub fn validate(self) -> Result<NewMemory, MemoryError> {
        let content = self.content.trim().to_owned();
        if content.is_empty() {
            return Err(MemoryError::InvalidMemory("content is required".into()));
        }
        if content.chars().count() > MAX_CONTENT_CHARS {
            return Err(MemoryError::OversizedInput(format!(
                "content exceeds {MAX_CONTENT_CHARS} characters"
            )));
        }
        if has_control_chars(&content) {
            return Err(MemoryError::InvalidMemory(
                "content contains control characters".into(),
            ));
        }
        if let Some(quote) = &self.quote {
            if quote.chars().count() > MAX_QUOTE_CHARS {
                return Err(MemoryError::OversizedInput(format!(
                    "quote exceeds {MAX_QUOTE_CHARS} characters"
                )));
            }
            if has_control_chars(quote) {
                return Err(MemoryError::InvalidMemory(
                    "quote contains control characters".into(),
                ));
            }
        }
        let id = validate_id(&self.id)?;
        let key = self.key.as_deref().map(key::validate_key).transpose()?;
        let session_id = self.session_id.trim().to_owned();
        if session_id.is_empty()
            || has_control_chars(&session_id)
            || session_id.chars().count() > 128
        {
            return Err(MemoryError::InvalidMemory(
                "session_id is required and must be control-free (≤128 chars)".into(),
            ));
        }
        if let Some(source_ref) = &self.source_ref {
            if source_ref.chars().count() > MAX_QUOTE_CHARS || has_control_chars(source_ref) {
                return Err(MemoryError::InvalidMemory(
                    "source_ref must be control-free (≤256 chars)".into(),
                ));
            }
        }
        Ok(NewMemory {
            id,
            key,
            content,
            session_id,
            ..self
        })
    }
}

/// Require the generated id shape: `owt_<millis>_<pid>_<seq>`.
fn validate_id(id: &str) -> Result<String, MemoryError> {
    if id.is_empty() || id.len() > 128 || has_control_chars(id) {
        return Err(MemoryError::InvalidMemory(format!(
            "invalid memory id {id:?}"
        )));
    }
    if key::is_id_shape(id) {
        Ok(id.to_owned())
    } else {
        Err(MemoryError::InvalidMemory(format!(
            "invalid memory id {id:?} (want owt_<millis>_<pid>_<seq>)"
        )))
    }
}

/// True when the string contains NUL or any other C0 control character
/// (U+0000–U+001F). The schema rejects these in content/quote at ingestion
/// and treats records containing them as corrupt on load.
pub fn has_control_chars(text: &str) -> bool {
    text.chars().any(|c| (c as u32) <= 0x1F)
}

/// Canonical content normalization for tombstone hashing (storage §11.2):
/// trim + collapse internal runs of whitespace (including newlines) to a
/// single space. Whitespace variants of the same statement hash equal.
pub fn normalize_content(content: &str) -> String {
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Build a record from one loaded JSONL line. `file_scope` must match the
/// record's `scope` field ("a scope's records live only in its own file").
/// Unknown fields are tolerated; wrong types / bad enums / missing required
/// fields / duplicate ids are reported by the caller as corrupt.
pub fn record_from_line(line: &str, file_scope: Scope) -> Result<MemoryRecord, String> {
    let value: Value =
        serde_json::from_str(line).map_err(|error| format!("not valid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "record is not a JSON object".to_owned())?;

    match object.get("v").and_then(Value::as_i64) {
        Some(SCHEMA_VERSION) => {}
        Some(other) => return Err(format!("unsupported schema version {other}")),
        None => return Err("missing v".to_owned()),
    }

    let required_str = |field: &str| -> Result<String, String> {
        object
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("missing or non-string {field}"))
    };

    let id = required_str("id")?;
    if id.is_empty() || has_control_chars(&id) {
        return Err("invalid id".to_owned());
    }

    let key = match object.get("key") {
        None | Some(Value::Null) => None,
        Some(Value::String(key)) => {
            Some(key::validate_key(key).map_err(|_| "key violates the key pattern".to_owned())?)
        }
        _ => return Err("key is not a string".to_owned()),
    };

    let kind = match object
        .get("kind")
        .and_then(Value::as_str)
        .and_then(Kind::parse)
    {
        Some(kind) => kind,
        _ => return Err("bad kind".to_owned()),
    };
    let scope = match object
        .get("scope")
        .and_then(Value::as_str)
        .and_then(Scope::parse)
    {
        Some(scope) => scope,
        _ => return Err("bad scope".to_owned()),
    };
    if scope != file_scope {
        return Err(format!(
            "scope {scope:?} does not match file scope {file_scope:?}"
        ));
    }
    let source = match object
        .get("source")
        .and_then(Value::as_str)
        .and_then(Source::parse)
    {
        Some(source) => source,
        _ => return Err("bad source".to_owned()),
    };
    let status = match object
        .get("status")
        .and_then(Value::as_str)
        .and_then(Status::parse)
    {
        // DELETED is reserved for V2 and never persisted by V1 writers; a
        // DELETED row in the file is treated as schema-invalid.
        Some(status) => status,
        _ => return Err("bad status".to_owned()),
    };
    let pinned = object
        .get("pinned")
        .and_then(Value::as_bool)
        .ok_or_else(|| "missing or non-bool pinned".to_owned())?;

    let content = required_str("content")?;
    if content.chars().count() > MAX_CONTENT_CHARS {
        return Err("content too long".to_owned());
    }
    if has_control_chars(&content) {
        return Err("content contains control characters".to_owned());
    }

    let created_at = required_str("created_at")?;
    let updated_at = required_str("updated_at")?;
    if parse_rfc3339_utc(&created_at).is_none() || parse_rfc3339_utc(&updated_at).is_none() {
        return Err("bad RFC3339 UTC timestamp".to_owned());
    }

    let session_id = required_str("session_id")?;
    if session_id.is_empty() || has_control_chars(&session_id) {
        return Err("bad session_id".to_owned());
    }

    let source_ref = match object.get("source_ref") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        _ => return Err("source_ref is not a string".to_owned()),
    };
    if source_ref.as_deref().is_some_and(has_control_chars) {
        return Err("source_ref contains control characters".to_owned());
    }

    let quote = match object.get("quote") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        _ => return Err("quote is not a string".to_owned()),
    };
    if let Some(quote) = &quote {
        if quote.chars().count() > MAX_QUOTE_CHARS || has_control_chars(quote) {
            return Err("bad quote".to_owned());
        }
    }

    Ok(MemoryRecord {
        id,
        key,
        kind,
        scope,
        content,
        source,
        status,
        pinned,
        created_at,
        updated_at,
        session_id,
        source_ref,
        quote,
    })
}

/// D24 deterministic ordering: tier partitions then recency, a total order.
///
/// ```text
/// T1 project-pinned → T2 user-pinned → T3 project-recent → T4 user-recent
/// within each tier: updated_at DESC, created_at DESC, id ASC
/// ```
pub fn order_active(records: &mut [MemoryRecord]) {
    records.sort_by(compare_d24);
}

fn tier(record: &MemoryRecord) -> u8 {
    match (record.scope, record.pinned) {
        (Scope::Project, true) => 0,
        (Scope::User, true) => 1,
        (Scope::Project, false) => 2,
        (Scope::User, false) => 3,
    }
}

/// Total order key for sorting: `(tier, updated_at, created_at, -id)`.
/// Timestamps are validated on load; parse failures fall back to epoch so a
/// sort can never panic on external tampering.
fn compare_d24(a: &MemoryRecord, b: &MemoryRecord) -> std::cmp::Ordering {
    tier(a)
        .cmp(&tier(b))
        .then_with(|| timestamp_key(&b.updated_at).cmp(&timestamp_key(&a.updated_at)))
        .then_with(|| timestamp_key(&b.created_at).cmp(&timestamp_key(&a.created_at)))
        .then_with(|| cmp_id(&a.id, &b.id))
}

/// `id ASC` must compare the numeric fields, not the strings: generated ids
/// are `owt_<millis>_<pid>_<seq>` with an *unpadded* seq, so plain string
/// comparison puts `…_12` before `…_5` (and millis/pids vary in width too).
/// Non-conforming ids fall back to string order so a sort can never panic.
fn cmp_id(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |id: &str| -> Option<(u128, u64, u64)> {
        let mut parts = id.split('_');
        if parts.next()? != "owt" {
            return None;
        }
        Some((
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        ))
    };
    match (key(a), key(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

/// Deterministic `show` order: ACTIVE first, then `updated_at DESC`,
/// `created_at DESC`, `id ASC`. Shared by the single-scope store `show` and
/// the API's cross-scope show merge.
pub(crate) fn order_show(records: &mut [MemoryRecord]) {
    records.sort_by(|a, b| {
        show_rank(a)
            .cmp(&show_rank(b))
            .then_with(|| timestamp_key(&b.updated_at).cmp(&timestamp_key(&a.updated_at)))
            .then_with(|| timestamp_key(&b.created_at).cmp(&timestamp_key(&a.created_at)))
            .then_with(|| cmp_id(&a.id, &b.id))
    });
}

fn show_rank(record: &MemoryRecord) -> u8 {
    match record.status {
        Status::Active => 0,
        Status::Superseded => 1,
    }
}

/// `(seconds, nanoseconds)` sort key; invalid timestamps sort as epoch.
pub(crate) fn timestamp_key(stamp: &str) -> (i64, u32) {
    parse_rfc3339_utc(stamp).unwrap_or((0, 0))
}

// ---------------------------------------------------------------------------
// RFC3339 UTC (second precision, `Z` zone) — std-only, no chrono dependency.
// ---------------------------------------------------------------------------

/// Format a `SystemTime` as `YYYY-MM-DDTHH:MM:SSZ` (RFC3339 UTC).
pub fn format_rfc3339_utc(time: std::time::SystemTime) -> String {
    let secs = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let hms = secs.rem_euclid(86_400);
    let hour = hms / 3600;
    let minute = (hms % 3600) / 60;
    let second = hms % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Parse `YYYY-MM-DDTHH:MM:SSZ` (optional fractional seconds, `Z`/`z` zone
/// only). Returns `(seconds_since_epoch, nanos)`.
pub fn parse_rfc3339_utc(text: &str) -> Option<(i64, u32)> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let digits = |start: usize, end: usize| -> Option<i64> {
        let slice = text.get(start..end)?;
        if !slice.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        slice.parse().ok()
    };
    let year = digits(0, 4)?;
    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let month = digits(5, 7)? as u32;
    let day = digits(8, 10)? as u32;
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let hour = digits(11, 13)? as u32;
    let minute = digits(14, 16)? as u32;
    let second = digits(17, 19)? as u32;
    if hour > 23 || minute > 59 || second > 59 || !(1..=12).contains(&month) {
        return None;
    }
    if day < 1 || day > days_in_month(year, month) {
        return None;
    }
    let mut index = 19;
    let mut nanos = 0u32;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let mut scale = 100_000_000u32;
        while let Some(byte) = bytes.get(index) {
            if !byte.is_ascii_digit() {
                break;
            }
            nanos += u32::from(byte - b'0') * scale;
            scale /= 10;
            index += 1;
        }
    }
    if bytes.get(index) != Some(&b'Z') && bytes.get(index) != Some(&b'z') {
        return None;
    }
    if index + 1 != bytes.len() {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + i64::from(hour * 3600 + minute * 60 + second);
    Some((secs, nanos))
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Howard Hinnant's `civil_from_days` (public-domain algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// Howard Hinnant's `days_from_civil` (public-domain algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = i64::from(if month > 2 { month - 3 } else { month + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn fixture(id: &str) -> NewMemory {
        NewMemory {
            id: id.to_owned(),
            key: None,
            kind: Kind::Fact,
            content: "Prefer Rust for new services.".to_owned(),
            pinned: false,
            session_id: "ses_test".to_owned(),
            source_ref: None,
            quote: None,
        }
    }

    #[test]
    fn rfc3339_round_trip() {
        let start = UNIX_EPOCH + Duration::from_secs(1_785_801_600); // 2026-08-04T00:00:00Z
        let formatted = format_rfc3339_utc(start);
        assert_eq!(formatted, "2026-08-04T00:00:00Z");
        let (secs, _) = parse_rfc3339_utc(&formatted).expect("parses");
        assert_eq!(secs, 1_785_801_600);
    }

    #[test]
    fn rfc3339_edge_dates() {
        assert_eq!(format_rfc3339_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        // 2024-02-29 (leap) parses and round-trips.
        let leap = UNIX_EPOCH + Duration::from_secs(1_709_164_800);
        assert_eq!(format_rfc3339_utc(leap), "2024-02-29T00:00:00Z");
        let (secs, _) = parse_rfc3339_utc("2024-02-29T00:00:00Z").unwrap();
        assert_eq!(secs, 1_709_164_800);
        // 1970-02-30 is not a real day.
        assert!(parse_rfc3339_utc("1970-02-30T00:00:00Z").is_none());
        // 2023-02-29 was not a leap year.
        assert!(parse_rfc3339_utc("2023-02-29T00:00:00Z").is_none());
    }

    #[test]
    fn rfc3339_rejects_bad_forms() {
        for bad in [
            "2026-08-04 00:00:00Z",
            "2026-08-04T00:00:00+01:00",
            "2026-08-04T00:00:00",
            "2026-08-04T24:00:00Z",
            "2026-13-01T00:00:00Z",
            "x",
            "2026-08-04T00:00:00Zextra",
        ] {
            assert!(parse_rfc3339_utc(bad).is_none(), "should reject {bad}");
        }
        // Fractional seconds tolerated on read.
        assert!(parse_rfc3339_utc("2026-08-04T00:00:00.123Z").is_some());
    }

    #[test]
    fn new_memory_validation_accepts_minimal() {
        let memory = fixture("owt_1_2_3").validate().expect("valid");
        assert_eq!(memory.content, "Prefer Rust for new services.");
        assert_eq!(memory.id, "owt_1_2_3");
    }

    #[test]
    fn new_memory_validation_trims_content() {
        let mut memory = fixture("owt_1_2_3");
        memory.content = "  spaced out  ".to_owned();
        assert_eq!(memory.validate().unwrap().content, "spaced out");
    }

    #[test]
    fn new_memory_rejects_empty_content() {
        let mut memory = fixture("owt_1_2_3");
        memory.content = "   ".to_owned();
        assert!(matches!(
            memory.validate(),
            Err(MemoryError::InvalidMemory(_))
        ));
    }

    #[test]
    fn new_memory_rejects_oversized() {
        let mut memory = fixture("owt_1_2_3");
        memory.content = "x".repeat(MAX_CONTENT_CHARS + 1);
        assert!(matches!(
            memory.validate(),
            Err(MemoryError::OversizedInput(_))
        ));

        let mut memory = fixture("owt_1_2_3");
        memory.quote = Some("q".repeat(MAX_QUOTE_CHARS + 1));
        assert!(matches!(
            memory.validate(),
            Err(MemoryError::OversizedInput(_))
        ));
    }

    #[test]
    fn new_memory_rejects_controls() {
        for bad in ["nul\u{0}here", "tab\there", "nl\nhere", "esc\u{1B}here"] {
            let mut memory = fixture("owt_1_2_3");
            memory.content = bad.to_owned();
            assert!(
                matches!(memory.validate(), Err(MemoryError::InvalidMemory(_))),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn new_memory_rejects_missing_session() {
        let mut memory = fixture("owt_1_2_3");
        memory.session_id = "  ".to_owned();
        assert!(matches!(
            memory.validate(),
            Err(MemoryError::InvalidMemory(_))
        ));
    }

    #[test]
    fn new_memory_rejects_bad_id() {
        for bad in [
            "",
            "local-1",
            "owt_1_2",
            "owt_1_2_3_4",
            "owt_abc_2_3",
            "owt_1_2_x",
        ] {
            let mut memory = fixture("x");
            memory.id = bad.to_owned();
            assert!(
                matches!(memory.validate(), Err(MemoryError::InvalidMemory(_))),
                "should reject id {bad:?}"
            );
        }
    }

    #[test]
    fn record_json_round_trip() {
        let memory = NewMemory {
            key: Some("lang".to_owned()),
            kind: Kind::Preference,
            content: "Prefer Rust".to_owned(),
            pinned: true,
            source_ref: Some("msg#3".to_owned()),
            quote: Some("I'd rather use Rust".to_owned()),
            ..fixture("owt_1780000000000_1234_7")
        }
        .validate()
        .unwrap();
        let record = MemoryRecord {
            scope: Scope::User,
            source: Source::User,
            status: Status::Active,
            created_at: "2026-09-18T10:00:00Z".to_owned(),
            updated_at: "2026-09-18T10:00:00Z".to_owned(),
            ..MemoryRecord::from_new(&memory, Scope::User, "2026-09-18T10:00:00Z")
        };
        let line = record.to_json_line();
        let parsed = record_from_line(&line, Scope::User).expect("round-trips");
        assert_eq!(parsed, record);
    }

    #[test]
    fn record_from_line_rejects_bad_scope_for_file() {
        let line = r#"{"v":1,"id":"owt_1_2_3","kind":"fact","scope":"user","content":"x","source":"user","status":"ACTIVE","pinned":false,"created_at":"2026-09-18T10:00:00Z","updated_at":"2026-09-18T10:00:00Z","session_id":"ses"}"#;
        assert!(record_from_line(line, Scope::Project).is_err());
        assert!(record_from_line(line, Scope::User).is_ok());
    }

    #[test]
    fn record_from_line_tolerates_unknown_fields() {
        let memory = fixture("owt_1_2_3").validate().unwrap();
        let record = MemoryRecord::from_new(&memory, Scope::User, "2026-09-18T10:00:00Z");
        let mut value: Value = serde_json::from_str(&record.to_json_line()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("future_field".into(), Value::from(42));
        let line = serde_json::to_string(&value).unwrap();
        let parsed = record_from_line(&line, Scope::User).expect("unknown fields tolerated");
        assert_eq!(parsed.id, record.id);
    }

    #[test]
    fn record_from_line_rejects_bad_enums() {
        let memory = fixture("owt_1_2_3").validate().unwrap();
        let record = MemoryRecord::from_new(&memory, Scope::User, "2026-09-18T10:00:00Z");
        for (field, bad) in [
            ("v", "2"),
            ("kind", "rule"),
            ("scope", "session"),
            ("source", "llm"),
            ("status", "DELETED"),
            ("status", "CONFLICT"),
        ] {
            let mut value: Value = serde_json::from_str(&record.to_json_line()).unwrap();
            value
                .as_object_mut()
                .unwrap()
                .insert(field.into(), Value::from(bad));
            let line = serde_json::to_string(&value).unwrap();
            assert!(
                record_from_line(&line, Scope::User).is_err(),
                "should reject {field}={bad}"
            );
        }
    }

    #[test]
    fn record_from_line_tolerates_inferred_source() {
        let memory = fixture("owt_1_2_3").validate().unwrap();
        let record = MemoryRecord::from_new(&memory, Scope::User, "2026-09-18T10:00:00Z");
        let mut value: Value = serde_json::from_str(&record.to_json_line()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("source".into(), Value::from("inferred"));
        let line = serde_json::to_string(&value).unwrap();
        let parsed = record_from_line(&line, Scope::User).expect("inferred tolerated on read");
        assert_eq!(parsed.source, Source::Inferred);
    }

    #[test]
    fn d24_order_matches_design() {
        let make =
            |id: &str, scope: Scope, pinned: bool, updated: &str, created: &str| MemoryRecord {
                id: id.to_owned(),
                key: None,
                kind: Kind::Fact,
                scope,
                content: format!("c{id}"),
                source: Source::User,
                status: Status::Active,
                pinned,
                created_at: created.to_owned(),
                updated_at: updated.to_owned(),
                session_id: "ses".to_owned(),
                source_ref: None,
                quote: None,
            };
        let mut records = vec![
            make(
                "owt_1_2_1",
                Scope::User,
                true,
                "2026-01-01T00:00:00Z",
                "2026-01-01T00:00:00Z",
            ),
            make(
                "owt_1_2_2",
                Scope::Project,
                false,
                "2026-01-02T00:00:00Z",
                "2026-01-02T00:00:00Z",
            ),
            make(
                "owt_1_2_3",
                Scope::Project,
                true,
                "2026-01-03T00:00:00Z",
                "2026-01-03T00:00:00Z",
            ),
            make(
                "owt_1_2_4",
                Scope::User,
                false,
                "2026-01-04T00:00:00Z",
                "2026-01-04T00:00:00Z",
            ),
            make(
                "owt_1_2_5",
                Scope::User,
                false,
                "2026-01-05T00:00:00Z",
                "2026-01-05T00:00:00Z",
            ),
        ];
        order_active(&mut records);
        let ids: Vec<&str> = records.iter().map(|r| r.id.as_str()).collect();
        // T1 project-pinned, then T2 user-pinned, then T3 project-recent,
        // then T4 user-recent; user-recent sorts updated_at DESC (5 before 4).
        assert_eq!(
            ids,
            [
                "owt_1_2_3",
                "owt_1_2_1",
                "owt_1_2_2",
                "owt_1_2_5",
                "owt_1_2_4"
            ]
        );
    }

    #[test]
    fn d24_tie_break_is_id_asc() {
        let make = |id: &str, created: &str| MemoryRecord {
            id: id.to_owned(),
            key: None,
            kind: Kind::Fact,
            scope: Scope::User,
            content: "x".to_owned(),
            source: Source::User,
            status: Status::Active,
            pinned: false,
            created_at: created.to_owned(),
            updated_at: created.to_owned(),
            session_id: "ses".to_owned(),
            source_ref: None,
            quote: None,
        };
        let mut records = vec![
            make("owt_1_2_b", "2026-01-01T00:00:00Z"),
            make("owt_1_2_a", "2026-01-01T00:00:00Z"),
        ];
        order_active(&mut records);
        assert_eq!(records[0].id, "owt_1_2_a");
    }

    #[test]
    fn id_tie_break_is_numeric_not_lexicographic() {
        // Unpadded seqs: string order would put "…_12" before "…_5" and
        // wide millis would break too — the comparator must parse fields.
        let make = |id: &str| MemoryRecord {
            id: id.to_owned(),
            key: None,
            kind: Kind::Fact,
            scope: Scope::User,
            content: "x".to_owned(),
            source: Source::User,
            status: Status::Active,
            pinned: false,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            session_id: "ses".to_owned(),
            source_ref: None,
            quote: None,
        };
        let mut records = vec![make("owt_1_1_12"), make("owt_1_1_5"), make("owt_1_2_5")];
        order_active(&mut records);
        let ids: Vec<&str> = records.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["owt_1_1_5", "owt_1_1_12", "owt_1_2_5"]);
        // `show` shares the same tie-break.
        let mut shown = vec![make("owt_1_1_12"), make("owt_1_1_5")];
        order_show(&mut shown);
        assert_eq!(shown[0].id, "owt_1_1_5");
    }

    #[test]
    fn normalize_content_collapses_whitespace() {
        assert_eq!(
            normalize_content("  Prefer\nRust\tfor   services.  "),
            "Prefer Rust for services."
        );
        assert_eq!(normalize_content(""), "");
    }

    #[test]
    fn control_character_detection() {
        assert!(has_control_chars("a\u{0}b"));
        assert!(has_control_chars("a\u{1F}b"));
        assert!(has_control_chars("a\nb"));
        assert!(!has_control_chars("a b"));
        assert!(!has_control_chars("a\u{7F}b")); // DEL is not C0.
    }
}
