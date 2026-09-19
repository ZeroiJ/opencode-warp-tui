//! The `MemoryStore` trait (semantics, not JSONL details) and its V1
//! file-backed implementation `JsonlStore`.
//!
//! Physical model (phase2b-storage D15/D21):
//!
//! ```text
//! <store-dir>/
//!   memory.jsonl          complete current-state snapshot, 0600
//!   tombstones.jsonl      append-only forget hashes, 0600
//!   memory.lock           flock target, 0600
//!   memory.jsonl.tmp.<pid> transient during a write (cleaned on open)
//! ```
//!
//! Mutation protocol: in-process mutex → exclusive file lock → re-read
//! state under the lock → validate → build next state → temp write →
//! `fsync` temp → atomic `rename` → `fsync` directory → release lock.
//! Readers take **no lock**; because writers only swap whole files via
//! rename, a reader always sees a complete old or new file.
//!
//! The store is one scope: user and project stores never share a file
//! (isolation is structural). Replace this implementation with SQLite later
//! behind the same trait (storage §14–§15).

use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use super::record::{
    format_rfc3339_utc, order_show, record_from_line, Kind, MemoryRecord, NewMemory, Scope, Status,
};
use super::tombstone::{self, Tombstone};
use super::MemoryError;

/// Hard load guard: store files larger than this are refused (security §7,
/// D25). Keeps a hostile file out of RAM.
pub const MAX_STORE_BYTES: u64 = 10 * 1024 * 1024;

/// O_NOFOLLOW per platform (security §2/§3: never follow a symlink planted
/// as a store file/dir). Linux (x86_64/aarch64/riscv) and macOS.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) const OPEN_NOFOLLOW: i32 = 0o400000;
#[cfg(target_os = "macos")]
pub(crate) const OPEN_NOFOLLOW: i32 = 0x0100;
#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
pub(crate) const OPEN_NOFOLLOW: i32 = 0; // symlink defense degrades (documented limitation)

/// How a command addresses a record: exact key (ACTIVE) or exact id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Handle {
    Key(String),
    Id(String),
}

impl std::fmt::Display for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Handle::Key(key) => write!(f, "key {key:?}"),
            Handle::Id(id) => write!(f, "id {id:?}"),
        }
    }
}

/// Filters for `active()` reads (scope is intrinsic to the store).
#[derive(Clone, Debug, Default)]
pub struct StoreFilter {
    pub kind: Option<Kind>,
    pub pinned: Option<bool>,
}

/// Result of a same-key write: the new ACTIVE record plus the record it
/// superseded (None on a fresh key).
#[derive(Clone, Debug)]
pub struct WriteOutcome {
    pub created: MemoryRecord,
    pub superseded: Option<MemoryRecord>,
}

/// Result of a forget: what was removed and the tombstone recording it.
#[derive(Clone, Debug)]
pub struct ForgetOutcome {
    pub id: String,
    pub key: Option<String>,
    /// Hash-only tombstone appended before the rewrite (safe residue for
    /// future audit/dedupe); the in-band reply only prints id/key.
    #[allow(dead_code)]
    pub tombstone: Tombstone,
}

/// Memory semantics. The trait intentionally names no storage technology;
/// `JsonlStore` is the V1 file-backed implementation and SQLite (V2) can
/// replace it behind the same surface.
pub trait MemoryStore {
    /// Identity accessor; no caller yet (scopes travel on the records).
    #[allow(dead_code)]
    fn scope(&self) -> Scope;

    /// Create a record; same ACTIVE key in scope → the old record becomes
    /// SUPERSEDED and the new one ACTIVE (never a silent overwrite).
    fn remember(&mut self, new: NewMemory) -> Result<WriteOutcome, MemoryError>;

    /// Update = same-key remember, requiring an existing ACTIVE key.
    fn update(&mut self, key: &str, new: NewMemory) -> Result<WriteOutcome, MemoryError>;

    /// Physically remove the record and append its hash tombstone.
    fn forget(&mut self, handle: &Handle) -> Result<ForgetOutcome, MemoryError>;

    /// Pin/unpin a record (affects D24 ordering); bumps `updated_at`.
    fn set_pinned(&mut self, handle: &Handle, pinned: bool) -> Result<MemoryRecord, MemoryError>;

    /// ACTIVE records in this scope, optionally filtered.
    fn active(&self, filter: &StoreFilter) -> Result<Vec<MemoryRecord>, MemoryError>;

    /// Exact show: by id (any status) or by key (ACTIVE, or the slot
    /// history with `all`).
    fn show(&self, handle: &Handle, all: bool) -> Result<Vec<MemoryRecord>, MemoryError>;

    /// Read-only handle resolution across statuses (ACTIVE for keys, any
    /// status for ids). The API uses it to detect cross-scope ambiguity
    /// before mutating.
    fn contains_handle(&self, handle: &Handle) -> Result<bool, MemoryError>;
}

/// File-backed store for one scope (storage §3: one file per scope).
pub struct JsonlStore {
    scope: Scope,
    dir: PathBuf,
    /// Serializes threads within this process. flock is per-open-file-
    /// description, so a mutex is mandatory even alongside the file lock.
    mutex: Mutex<()>,
    /// Wall clock; tests swap in a fixed clock via `open_with_clock`.
    now: Box<dyn Fn() -> SystemTime + Send + Sync>,
}

/// User store: `$XDG_DATA_HOME/owt/`, default `~/.local/share/owt/`
/// (storage §9.1). Windows `%APPDATA%` is specified but not implemented.
pub fn user_store_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(dir).join("owt");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/owt");
    }
    // No HOME: keep the store visible and clearly named rather than hidden.
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".owt-user")
}

/// Project store: `<project-root>/.owt/` (D20, storage §9.2).
pub fn project_store_dir(project_root: &Path) -> PathBuf {
    project_root.join(".owt")
}

impl JsonlStore {
    /// Open (creating) the store for one scope. Fails on a symlinked dir,
    /// permission errors, or unreadable/oversized/non-UTF-8 state — the
    /// caller treats this as *memory unavailable*, never a session failure.
    pub fn open(scope: Scope, dir: PathBuf) -> Result<JsonlStore, MemoryError> {
        ensure_dir(&dir)?;
        let store = JsonlStore {
            scope,
            dir,
            mutex: Mutex::new(()),
            now: Box::new(SystemTime::now),
        };
        store.prepare()?;
        Ok(store)
    }

    /// Test-only: fixed clock for deterministic timestamps.
    #[cfg(test)]
    pub fn open_with_clock(
        scope: Scope,
        dir: PathBuf,
        now: SystemTime,
    ) -> Result<JsonlStore, MemoryError> {
        ensure_dir(&dir)?;
        let store = JsonlStore {
            scope,
            dir,
            mutex: Mutex::new(()),
            now: Box::new(move || now),
        };
        store.prepare()?;
        Ok(store)
    }

    /// Connect-time preparation under the write lock: clean stale temp
    /// files, ensure the tombstone/lock files exist with private modes, and
    /// surface hard load failures (oversized/non-UTF-8) up front.
    fn prepare(&self) -> Result<(), MemoryError> {
        let guard = self.mutex.lock().unwrap_or_else(|p| p.into_inner());
        let lock = lock_file(&self.dir)?;
        let result = (|| {
            clean_stale_temps(&self.dir)?;
            ensure_data_file(&self.dir, "tombstones.jsonl")?;
            load_records(&self.dir, self.scope)?;
            Ok(())
        })();
        drop(lock);
        drop(guard);
        result
    }

    /// Inspect the store directory (diagnostics/tests).
    #[allow(dead_code)]
    pub fn store_dir(&self) -> &Path {
        &self.dir
    }

    /// Exclusive in-process + cross-process mutation runner:
    /// lock → re-read under the lock → `action` builds the next state →
    /// atomic rewrite. `action` may also append tombstones (forget does,
    /// before the rewrite — the safe residue on partial failure).
    fn run_exclusive<T>(
        &self,
        action: impl FnOnce(&Path, &mut Vec<MemoryRecord>, &str) -> Result<T, MemoryError>,
    ) -> Result<T, MemoryError> {
        let guard = self.mutex.lock().unwrap_or_else(|p| p.into_inner());
        let lock = lock_file(&self.dir)?;
        let now = format_rfc3339_utc((self.now)());
        let result = (|| {
            let mut records = load_records(&self.dir, self.scope)?;
            let outcome = action(&self.dir, &mut records, &now)?;
            write_state(&self.dir, &records)?;
            Ok(outcome)
        })();
        drop(lock);
        drop(guard);
        result
    }
}

impl MemoryStore for JsonlStore {
    fn scope(&self) -> Scope {
        self.scope
    }

    fn remember(&mut self, new: NewMemory) -> Result<WriteOutcome, MemoryError> {
        let new = new.validate()?;
        self.run_exclusive(|_dir, records, now| {
            if records.iter().any(|record| record.id == new.id) {
                return Err(MemoryError::Conflict(format!(
                    "memory id {:?} already exists",
                    new.id
                )));
            }
            let mut superseded = None;
            if let Some(key) = new.key.as_deref() {
                if let Some(index) = records.iter().position(|record| {
                    record.status == Status::Active && record.key.as_deref() == Some(key)
                }) {
                    let old = records
                        .get_mut(index)
                        .expect("index just found within bounds");
                    old.status = Status::Superseded;
                    old.updated_at = now.to_owned();
                    superseded = Some(old.clone());
                }
            }
            let created = MemoryRecord::from_new(&new, self.scope, now);
            records.push(created.clone());
            Ok(WriteOutcome {
                created,
                superseded,
            })
        })
    }

    fn update(&mut self, key: &str, new: NewMemory) -> Result<WriteOutcome, MemoryError> {
        let key = super::key::validate_key(key)?;
        let mut new = new.validate()?;
        new.key = Some(key.clone()); // the update target is authoritative
        self.run_exclusive(|_dir, records, now| {
            let index = records.iter().position(|record| {
                record.status == Status::Active && record.key.as_deref() == Some(key.as_str())
            });
            let Some(index) = index else {
                return Err(MemoryError::NotFound(format!("active key {key:?}")));
            };
            if records.iter().any(|record| record.id == new.id) {
                return Err(MemoryError::Conflict(format!(
                    "memory id {:?} already exists",
                    new.id
                )));
            }
            let old = records
                .get_mut(index)
                .expect("index just found within bounds");
            old.status = Status::Superseded;
            old.updated_at = now.to_owned();
            let superseded = old.clone();
            let created = MemoryRecord::from_new(&new, self.scope, now);
            records.push(created.clone());
            Ok(WriteOutcome {
                created,
                superseded: Some(superseded),
            })
        })
    }

    fn forget(&mut self, handle: &Handle) -> Result<ForgetOutcome, MemoryError> {
        self.run_exclusive(|dir, records, now| {
            let index = resolve_index(records, handle, "forget")?;
            let removed = records.remove(index);
            let tombstone = Tombstone {
                v: 1,
                hash: tombstone::canonical_hash(removed.kind, self.scope, &removed.content),
                scope: self.scope,
                kind: removed.kind,
                forgotten_at: now.to_owned(),
            };
            // Tombstone first: if the rewrite then fails, the worst case is
            // a tombstone with the memory still present (V1 explicit
            // re-remember is always allowed) — never a deletion without a
            // tombstone.
            tombstone::append_tombstone(&dir.join("tombstones.jsonl"), &tombstone)?;
            Ok(ForgetOutcome {
                id: removed.id,
                key: removed.key,
                tombstone,
            })
        })
    }

    fn set_pinned(&mut self, handle: &Handle, pinned: bool) -> Result<MemoryRecord, MemoryError> {
        self.run_exclusive(|_dir, records, now| {
            let index = resolve_index(records, handle, "pin")?;
            let record = records
                .get_mut(index)
                .expect("index just resolved within bounds");
            record.pinned = pinned;
            record.updated_at = now.to_owned();
            Ok(record.clone())
        })
    }

    fn active(&self, filter: &StoreFilter) -> Result<Vec<MemoryRecord>, MemoryError> {
        let records = load_records(&self.dir, self.scope)?;
        Ok(records
            .into_iter()
            .filter(|record| record.status == Status::Active)
            .filter(|record| filter.kind.is_none_or(|kind| record.kind == kind))
            .filter(|record| filter.pinned.is_none_or(|pinned| record.pinned == pinned))
            .collect())
    }

    fn show(&self, handle: &Handle, all: bool) -> Result<Vec<MemoryRecord>, MemoryError> {
        let records = load_records(&self.dir, self.scope)?;
        let mut matched: Vec<MemoryRecord> = match handle {
            Handle::Id(id) => records
                .into_iter()
                .filter(|record| record.id == *id)
                .collect(),
            Handle::Key(key) => records
                .into_iter()
                .filter(|record| record.key.as_deref() == Some(key.as_str()))
                .filter(|record| all || record.status == Status::Active)
                .collect(),
        };
        // ACTIVE first, then SUPERSEDED by recency: deterministic `show`.
        order_show(&mut matched);
        Ok(matched)
    }

    fn contains_handle(&self, handle: &Handle) -> Result<bool, MemoryError> {
        let records = load_records(&self.dir, self.scope)?;
        Ok(records.iter().any(|record| match handle {
            Handle::Key(key) => {
                record.status == Status::Active && record.key.as_deref() == Some(key)
            }
            Handle::Id(id) => record.id == *id,
        }))
    }
}

/// Resolve an exact-key-ACTIVE or exact-id handle to an index. Key path
/// only matches ACTIVE; id path prefers ACTIVE but also resolves
/// SUPERSEDED (history cleanup). Deterministic precedence: key first.
fn resolve_index(
    records: &[MemoryRecord],
    handle: &Handle,
    verb: &str,
) -> Result<usize, MemoryError> {
    match handle {
        Handle::Key(key) => records
            .iter()
            .position(|record| {
                record.status == Status::Active && record.key.as_deref() == Some(key)
            })
            .ok_or_else(|| MemoryError::NotFound(format!("{verb}: active key {key:?}"))),
        Handle::Id(id) => {
            let prefer_active = records
                .iter()
                .position(|record| record.status == Status::Active && record.id == *id);
            match prefer_active {
                Some(index) => Ok(index),
                None => records
                    .iter()
                    .position(|record| record.id == *id)
                    .ok_or_else(|| MemoryError::NotFound(format!("{verb}: id {id:?}"))),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Filesystem primitives
// ---------------------------------------------------------------------------

/// Create the store directory private (0700); refuse a symlinked or
/// non-directory path (security §2/§3).
fn ensure_dir(dir: &Path) -> Result<(), MemoryError> {
    match std::fs::symlink_metadata(dir) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(MemoryError::StoreUnavailable(format!(
                    "refusing symlinked store directory {}",
                    dir.display()
                )));
            }
            if !meta.is_dir() {
                return Err(MemoryError::StoreUnavailable(format!(
                    "store path {} is not a directory",
                    dir.display()
                )));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder
                .create(dir)
                .map_err(|error| io_error("create store directory", dir, error))?;
            // Re-check what we just created is not a symlink (TOCTOU guard).
            match std::fs::symlink_metadata(dir) {
                Ok(meta) if !meta.file_type().is_symlink() => Ok(()),
                _ => Err(MemoryError::StoreUnavailable(format!(
                    "refusing symlinked store directory {}",
                    dir.display()
                ))),
            }
        }
        Err(error) => Err(io_error("inspect store directory", dir, error)),
    }
}

/// Create a data file (0600, no-follow) if missing; warn (never chmod) when
/// an existing file has loose permission bits.
fn ensure_data_file(dir: &Path, name: &str) -> Result<(), MemoryError> {
    let path = dir.join(name);
    let exists = std::fs::symlink_metadata(&path).is_ok();
    if !exists {
        let file =
            open_data_file(&path, true, true).map_err(|error| io_error("create", &path, error))?;
        drop(file);
    }
    check_loose_mode(&path);
    Ok(())
}

/// Open a store file: readable, 0600 on create, `O_NOFOLLOW` (and std's
/// O_CLOEXEC) so a planted symlink is refused with ELOOP→controlled error.
fn open_data_file(path: &Path, create: bool, writable: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    if create {
        options.create(true);
    }
    if writable {
        options.write(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        if OPEN_NOFOLLOW != 0 {
            options.custom_flags(OPEN_NOFOLLOW);
        }
    }
    options.open(path)
}

/// Warn when a store file's permission bits are looser than 0600. We open
/// it anyway (user-owned, may predate the policy) — security §2.
#[cfg(unix)]
fn check_loose_mode(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.file_type().is_symlink() && meta.permissions().mode() & 0o077 != 0 {
            log::warn!(
                "memory: {} has loose permissions {:#o}; consider chmod 600",
                path.display(),
                meta.permissions().mode() & 0o7777
            );
        }
    }
}

#[cfg(not(unix))]
fn check_loose_mode(_path: &Path) {}

/// Exclusive cross-process lock (`flock` via `File::lock`). The kernel
/// releases it when the fd closes, so a crashed writer can never deadlock a
/// later one (storage §7, D21).
fn lock_file(dir: &Path) -> Result<File, MemoryError> {
    let path = dir.join("memory.lock");
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        if OPEN_NOFOLLOW != 0 {
            options.custom_flags(OPEN_NOFOLLOW);
        }
    }
    let file = options
        .open(&path)
        .map_err(|error| io_error("open lock file", &path, error))?;
    file.lock()
        .map_err(|error| io_error("lock", &path, error))?;
    Ok(file)
}

/// Remove `memory.jsonl.tmp*` leftovers from crashed writers; the canonical
/// `memory.jsonl` is authoritative (storage §6).
fn clean_stale_temps(dir: &Path) -> Result<(), MemoryError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error("scan store directory", dir, error)),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("memory.jsonl.tmp") {
            let path = entry.path();
            if let Err(error) = std::fs::remove_file(&path) {
                log::warn!(
                    "memory: failed to clean stale temp {}: {error}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

/// Atomic rewrite of `memory.jsonl` (storage §6, D15):
/// temp → fsync → rename → directory fsync.
fn write_state(dir: &Path, records: &[MemoryRecord]) -> Result<(), MemoryError> {
    let temp = dir.join(format!("memory.jsonl.tmp.{}", std::process::id()));
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        options.mode(0o600);
        if OPEN_NOFOLLOW != 0 {
            options.custom_flags(OPEN_NOFOLLOW);
        }
        options
            .open(&temp)
            .map_err(|error| io_error("create temp file", &temp, error))?
    };
    #[cfg(not(unix))]
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|error| io_error("create temp file", &temp, error))?;
    for record in records {
        let mut line = record.to_json_line();
        line.push('\n');
        file.write_all(line.as_bytes())
            .map_err(|error| io_error("write temp file", &temp, error))?;
    }
    // fsync before rename: the new content is durable before it becomes
    // visible under the canonical name.
    file.sync_all()
        .map_err(|error| io_error("fsync temp file", &temp, error))?;
    drop(file);
    std::fs::rename(&temp, dir.join("memory.jsonl"))
        .map_err(|error| io_error("rename into place", &temp, error))?;
    // Directory fsync narrows the power-loss window for the rename itself.
    if let Ok(dir_file) = File::open(dir) {
        let _ = dir_file.sync_all();
    }
    Ok(())
}

/// Load and validate the memory file. Missing/empty files are a valid empty
/// corpus; malformed lines are skipped + counted + warned (never fatal,
/// never auto-repaired at read time — storage §8). Hard failures (too
/// large, not UTF-8, symlink) make memory unavailable.
fn load_records(dir: &Path, scope: Scope) -> Result<Vec<MemoryRecord>, MemoryError> {
    let path = dir.join("memory.jsonl");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = match open_data_file(&path, false, false) {
        Ok(file) => file,
        // Vanished between the existence check and the open: treat as an
        // empty corpus (another process may have removed it).
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error("open", &path, error)),
    };
    let meta = file
        .metadata()
        .map_err(|error| io_error("stat", &path, error))?;
    if meta.len() > MAX_STORE_BYTES {
        return Err(MemoryError::StoreTooLarge(meta.len()));
    }
    check_loose_mode(&path);
    let mut text = String::new();
    BufReader::new(file)
        .read_to_string(&mut text)
        .map_err(|error| {
            MemoryError::StoreUnavailable(format!(
                "memory store {} is not valid UTF-8: {error}",
                path.display()
            ))
        })?;
    let mut records = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    let mut malformed = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match record_from_line(line, scope) {
            Ok(record) => {
                if seen_ids.insert(record.id.clone()) {
                    records.push(record);
                } else {
                    malformed += 1; // duplicate id → later occurrence malformed
                }
            }
            Err(_) => malformed += 1,
        }
    }
    // Duplicate ACTIVE keys: warned, both kept (the loader never silently
    // picks one — the *writer* is responsible for preventing this).
    let mut key_slots = std::collections::HashMap::<String, usize>::new();
    for record in records.iter().filter(|r| r.status == Status::Active) {
        if let Some(key) = &record.key {
            *key_slots.entry(key.clone()).or_insert(0) += 1;
        }
    }
    for (key, count) in key_slots {
        if count > 1 {
            log::warn!(
                "memory: {} active records share key {key:?} in {} (corrupt state)",
                count,
                path.display()
            );
        }
    }
    if malformed > 0 {
        log::warn!(
            "memory: {malformed} malformed record(s) ignored in {}",
            path.display()
        );
    }
    Ok(records)
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> MemoryError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        return MemoryError::PermissionDenied;
    }
    MemoryError::StoreUnavailable(format!("{action} {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_785_801_600)
    } // 2026-08-04T00:00:00Z

    fn test_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("owt-store-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn open_at(scope: Scope, dir: &Path) -> JsonlStore {
        JsonlStore::open_with_clock(scope, dir.to_path_buf(), now())
            .unwrap_or_else(|error| panic!("open {dir:?}: {error}"))
    }

    fn open_at_result(scope: Scope, dir: &Path) -> Result<JsonlStore, MemoryError> {
        JsonlStore::open_with_clock(scope, dir.to_path_buf(), now())
    }

    fn new_memory(id: &str, content: &str) -> NewMemory {
        NewMemory {
            id: id.to_owned(),
            key: None,
            kind: Kind::Fact,
            content: content.to_owned(),
            pinned: false,
            session_id: "ses_test".to_owned(),
            source_ref: None,
            quote: None,
        }
    }

    fn file_text(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("memory.jsonl")).unwrap_or_default()
    }

    #[test]
    fn first_open_creates_private_layout() {
        let dir = test_dir("layout");
        let store = open_at(Scope::User, &dir);
        assert_eq!(store.scope(), Scope::User);
        assert!(dir.join("memory.lock").exists());
        assert!(dir.join("tombstones.jsonl").exists());
        assert!(!dir.join("memory.jsonl").exists()); // missing file = empty corpus
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(dir_mode & 0o777, 0o700, "store dir must be 0700");
            let lock_mode = std::fs::metadata(dir.join("memory.lock"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(lock_mode & 0o777, 0o600, "lock file must be 0600");
        }
    }

    #[test]
    fn empty_store_round_trip() {
        let dir = test_dir("empty");
        let store = open_at(Scope::User, &dir);
        assert_eq!(store.active(&StoreFilter::default()).unwrap(), Vec::new());
    }

    #[test]
    fn remember_then_rewrite_is_atomic_and_complete() {
        let dir = test_dir("rewrite");
        let mut store = open_at(Scope::User, &dir);
        store
            .remember(new_memory("owt_1_1_1", "first memory"))
            .unwrap();
        store
            .remember(new_memory("owt_1_1_2", "second memory"))
            .unwrap();
        let text = file_text(&dir);
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("first memory"));
        assert!(text.contains("second memory"));
        // No temp file survives a completed write.
        let temps: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(temps.is_empty(), "leftover temp files: {temps:?}");
        // The file is always a complete parse.
        let records = store.active(&StoreFilter::default()).unwrap();
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn remember_same_key_supersedes_not_overwrites() {
        let dir = test_dir("supersede");
        let mut store = open_at(Scope::User, &dir);
        let first = store
            .remember(NewMemory {
                key: Some("lang".into()),
                ..new_memory("owt_1_1_1", "Prefer Rust.")
            })
            .unwrap();
        assert!(first.superseded.is_none());
        let second = store
            .remember(NewMemory {
                key: Some("lang".into()),
                ..new_memory("owt_1_1_2", "Prefer Python.")
            })
            .unwrap();
        assert_eq!(second.superseded.as_ref().unwrap().id, "owt_1_1_1");
        assert_eq!(
            second.superseded.as_ref().unwrap().status,
            Status::Superseded
        );
        // Old line retained, new ACTIVE — no silent overwrite.
        let text = file_text(&dir);
        assert!(text.contains("Prefer Rust.") && text.contains("Prefer Python."));
        let active = store.active(&StoreFilter::default()).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].content, "Prefer Python.");
        assert_eq!(active[0].status, Status::Active);
        // Superseded visible only via show.
        let shown = store.show(&Handle::Key("lang".into()), false).unwrap();
        assert_eq!(shown.len(), 1);
        let history = store.show(&Handle::Key("lang".into()), true).unwrap();
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn update_requires_existing_key() {
        let dir = test_dir("update");
        let mut store = open_at(Scope::User, &dir);
        let err = store
            .update("missing", new_memory("owt_1_1_1", "x"))
            .unwrap_err();
        assert!(matches!(err, MemoryError::NotFound(_)));
        store
            .remember(NewMemory {
                key: Some("lang".into()),
                ..new_memory("owt_1_1_1", "Prefer Rust.")
            })
            .unwrap();
        let outcome = store
            .update("lang", new_memory("owt_1_1_2", "Prefer Go."))
            .unwrap();
        assert_eq!(outcome.superseded.unwrap().content, "Prefer Rust.");
        assert_eq!(outcome.created.content, "Prefer Go.");
    }

    #[test]
    fn different_scopes_may_share_a_key() {
        let user_dir = test_dir("scope-user");
        let project_dir = test_dir("scope-project");
        let mut user = open_at(Scope::User, &user_dir);
        let mut project = open_at(Scope::Project, &project_dir);
        user.remember(NewMemory {
            key: Some("database".into()),
            ..new_memory("owt_1_1_1", "user db memory")
        })
        .unwrap();
        project
            .remember(NewMemory {
                key: Some("database".into()),
                ..new_memory("owt_2_1_1", "project db memory")
            })
            .unwrap();
        let user_active = user.active(&StoreFilter::default()).unwrap();
        let project_active = project.active(&StoreFilter::default()).unwrap();
        assert_eq!(user_active.len(), 1);
        assert_eq!(project_active.len(), 1);
        assert_eq!(user_active[0].scope, Scope::User);
        assert_eq!(project_active[0].scope, Scope::Project);
    }

    #[test]
    fn forget_removes_content_and_tombstones() {
        let dir = test_dir("forget");
        let mut store = open_at(Scope::User, &dir);
        store
            .remember(NewMemory {
                key: Some("db".into()),
                content: "Project uses PostgreSQL.".to_owned(),
                ..new_memory("owt_1_1_1", "")
            })
            .unwrap();
        let outcome = store.forget(&Handle::Key("db".into())).unwrap();
        assert_eq!(outcome.id, "owt_1_1_1");
        // Content is physically gone from the record file.
        let text = file_text(&dir);
        assert!(!text.contains("PostgreSQL"));
        // The tombstone contains the hash, never the content.
        let tombstones = std::fs::read_to_string(dir.join("tombstones.jsonl")).unwrap();
        assert!(!tombstones.contains("PostgreSQL"));
        assert!(tombstones.contains(&outcome.tombstone.hash));
        assert!(store.active(&StoreFilter::default()).unwrap().is_empty());
        // Repeated forget → not found.
        let err = store.forget(&Handle::Key("db".into())).unwrap_err();
        assert!(matches!(err, MemoryError::NotFound(_)));
    }

    #[test]
    fn forget_by_id_releases_key_but_history_dies_with_it() {
        let dir = test_dir("forget-id");
        let mut store = open_at(Scope::User, &dir);
        store
            .remember(NewMemory {
                key: Some("k".into()),
                ..new_memory("owt_1_1_1", "one")
            })
            .unwrap();
        store
            .remember(NewMemory {
                key: Some("k".into()),
                ..new_memory("owt_1_1_2", "two")
            })
            .unwrap();
        // Forget the SUPERSEDED predecessor by id (history cleanup).
        store.forget(&Handle::Id("owt_1_1_1".into())).unwrap();
        assert_eq!(store.show(&Handle::Key("k".into()), true).unwrap().len(), 1);
    }

    #[test]
    fn pin_unpin_rewrites_and_reorders() {
        let dir = test_dir("pin");
        let mut store = open_at(Scope::User, &dir);
        store.remember(new_memory("owt_1_1_1", "older")).unwrap();
        store.remember(new_memory("owt_1_1_2", "newer")).unwrap();
        let pinned = store
            .set_pinned(&Handle::Id("owt_1_1_1".into()), true)
            .unwrap();
        assert!(pinned.pinned);
        let unpinned = store
            .set_pinned(&Handle::Id("owt_1_1_1".into()), false)
            .unwrap();
        assert!(!unpinned.pinned);
        let active = store.active(&StoreFilter::default()).unwrap();
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn contains_handle_distinguishes_scope_and_status() {
        let dir = test_dir("contains");
        let mut store = open_at(Scope::User, &dir);
        store
            .remember(NewMemory {
                key: Some("k".into()),
                ..new_memory("owt_1_1_1", "one")
            })
            .unwrap();
        store
            .remember(NewMemory {
                key: Some("k".into()),
                ..new_memory("owt_1_1_2", "two")
            })
            .unwrap();
        // ACTIVE key matches; superseded key does not (for resolve purposes).
        assert!(store.contains_handle(&Handle::Key("k".into())).unwrap());
        assert!(store
            .contains_handle(&Handle::Id("owt_1_1_1".into()))
            .unwrap());
        assert!(store
            .contains_handle(&Handle::Id("owt_1_1_2".into()))
            .unwrap());
        assert!(!store.contains_handle(&Handle::Key("nope".into())).unwrap());
    }

    #[test]
    fn malformed_lines_are_skipped_and_warned_not_fatal() {
        let dir = test_dir("corrupt");
        open_at(Scope::User, &dir);
        std::fs::write(
            dir.join("memory.jsonl"),
            "not json\n{\"v\":1,\"id\":\"owt_1_1_1\",\"kind\":\"fact\",\"scope\":\"user\",\"content\":\"good one\",\"source\":\"user\",\"status\":\"ACTIVE\",\"pinned\":false,\"created_at\":\"2026-08-04T00:00:00Z\",\"updated_at\":\"2026-08-04T00:00:00Z\",\"session_id\":\"ses\"}\n{\"v\":1,\"id\":\"owt_1_1_1\",\"kind\":\"fact\",\"scope\":\"user\",\"content\":\"dup id\",\"source\":\"user\",\"status\":\"ACTIVE\",\"pinned\":false,\"created_at\":\"2026-08-04T00:00:00Z\",\"updated_at\":\"2026-08-04T00:00:00Z\",\"session_id\":\"ses\"}\n",
        )
        .unwrap();
        let store = open_at(Scope::User, &dir);
        let records = store.active(&StoreFilter::default()).unwrap();
        // Non-JSON line skipped, duplicate-id line skipped; valid line kept.
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].content, "good one");
        // Reads never rewrite the file (recovery discipline).
        assert!(file_text(&dir).contains("not json"));
    }

    #[test]
    fn scope_mismatch_lines_are_skipped() {
        let dir = test_dir("scope-mismatch");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("memory.jsonl"),
            "{\"v\":1,\"id\":\"owt_1_1_1\",\"kind\":\"fact\",\"scope\":\"project\",\"content\":\"x\",\"source\":\"user\",\"status\":\"ACTIVE\",\"pinned\":false,\"created_at\":\"2026-08-04T00:00:00Z\",\"updated_at\":\"2026-08-04T00:00:00Z\",\"session_id\":\"ses\"}\n",
        )
        .unwrap();
        let store = open_at(Scope::User, &dir);
        assert!(store.active(&StoreFilter::default()).unwrap().is_empty());
    }

    #[test]
    fn non_utf8_store_is_refused() {
        let dir = test_dir("non-utf8");
        open_at(Scope::User, &dir);
        std::fs::write(dir.join("memory.jsonl"), [0xFF, 0xFE, 0x00, 0x41]).unwrap();
        let err = open_at_result(Scope::User, &dir);
        assert!(matches!(err, Err(MemoryError::StoreUnavailable(_))));
    }

    #[test]
    fn oversized_store_is_refused() {
        let dir = test_dir("oversized");
        open_at(Scope::User, &dir);
        // Just over the 10 MiB guard: 10 MiB + 1 byte.
        let size = MAX_STORE_BYTES + 1;
        let mut data = vec![b' '; size as usize];
        data[0] = b'{';
        data[1] = b'}';
        std::fs::write(dir.join("memory.jsonl"), data).unwrap();
        let err = open_at_result(Scope::User, &dir);
        assert!(matches!(err, Err(MemoryError::StoreTooLarge(_))));
    }

    #[test]
    fn stale_temp_files_are_cleaned_on_open() {
        let dir = test_dir("stale");
        open_at(Scope::User, &dir);
        std::fs::write(
            dir.join("memory.jsonl.tmp.9999"),
            "partial garbage".as_bytes(),
        )
        .unwrap();
        std::fs::write(dir.join("memory.jsonl.tmp.1234"), "more garbage").unwrap();
        let mut store = open_at(Scope::User, &dir);
        store
            .remember(new_memory("owt_1_1_1", "canonical"))
            .unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        // Canonical file stays authoritative.
        assert!(file_text(&dir).contains("canonical"));
    }

    #[test]
    fn symlinked_store_dir_is_refused() {
        #[cfg(unix)]
        {
            let dir = test_dir("symlink");
            std::fs::create_dir_all(dir.join("real")).unwrap();
            std::os::unix::fs::symlink(dir.join("real"), dir.join("linked")).unwrap();
            let err = open_at_result(Scope::User, &dir.join("linked"));
            assert!(matches!(err, Err(MemoryError::StoreUnavailable(_))));
        }
    }

    #[test]
    fn reader_never_sees_torn_state_during_writes() {
        let dir = test_dir("reader-torn");
        let mut initial = open_at(Scope::User, &dir);
        for i in 0..20 {
            initial
                .remember(new_memory(
                    &format!("owt_1_1_{i:02}"),
                    &format!("value {i}"),
                ))
                .unwrap();
        }
        drop(initial);
        let writer = std::thread::spawn({
            let mut store = open_at(Scope::User, &dir);
            move || {
                for i in 0..30 {
                    store
                        .remember(new_memory(
                            &format!("owt_1_2_{i:02}"),
                            &format!("writer {i}"),
                        ))
                        .unwrap();
                }
            }
        });
        let reader = std::thread::spawn({
            let store = open_at(Scope::User, &dir);
            move || {
                // Reader takes no lock; must always observe a complete file.
                for _ in 0..200 {
                    let records = store.active(&StoreFilter::default()).unwrap();
                    for record in &records {
                        assert!(
                            record.content.starts_with("value ")
                                || record.content.starts_with("writer "),
                            "incomplete content observed"
                        );
                    }
                }
            }
        });
        writer.join().unwrap();
        reader.join().unwrap();
        let store = open_at(Scope::User, &dir);
        assert_eq!(store.active(&StoreFilter::default()).unwrap().len(), 50);
    }

    #[test]
    fn concurrent_writers_do_not_lose_updates() {
        let dir = test_dir("lost-update");
        // Four independent store instances (separate fds → real flock
        // contention like four processes) append records on distinct keys.
        let mut handles = Vec::new();
        for writer in 0..4u32 {
            let dir = dir.clone();
            handles.push(std::thread::spawn(move || {
                let mut store = open_at(Scope::User, &dir);
                for i in 0..25 {
                    let key = format!("writer{writer}-{i}");
                    store
                        .remember(NewMemory {
                            key: Some(key),
                            ..new_memory(
                                &format!("owt_{writer}_1_{i:02}"),
                                &format!("writer {writer} value {i}"),
                            )
                        })
                        .unwrap();
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        let store = open_at(Scope::User, &dir);
        let active = store.active(&StoreFilter::default()).unwrap();
        // 4 writers × 25 distinct-key mutations: every mutation must
        // survive — read-under-lock prevents lost updates.
        assert_eq!(
            active.len(),
            100,
            "lost update: {}/100 survivors",
            active.len()
        );
        assert!(active
            .iter()
            .any(|record| record.content == "writer 3 value 24"));
    }

    #[test]
    fn two_real_processes_serialize_on_the_lock() {
        // Fork self with the fork-gate env var; each child writes 30
        // distinct keys, so no mutation may disappear under contention.
        if std::env::var("OWT_FORK_WRITER").is_ok() {
            let dir = std::path::PathBuf::from(std::env::var("OWT_FORK_DIR").unwrap());
            let mut store = JsonlStore::open_with_clock(Scope::User, dir.clone(), now()).unwrap();
            for i in 0..30 {
                store
                    .remember(NewMemory {
                        key: Some(format!("fork-{}-{i}", std::process::id())),
                        ..new_memory(
                            // Unique per child: id embeds the child pid.
                            &format!("owt_9_{}_{i:02}", std::process::id()),
                            &format!("fork {} value {i}", std::process::id()),
                        )
                    })
                    .unwrap();
            }
            return;
        }
        let dir = test_dir("two-process");
        JsonlStore::open_with_clock(Scope::User, dir.clone(), now()).unwrap();
        let exe = std::env::current_exe().expect("test binary path");
        let n_children = 2usize;
        for child in 0..n_children {
            let status = std::process::Command::new(&exe)
                .arg("--exact")
                .arg("backend::memory::store::tests::two_real_processes_serialize_on_the_lock")
                .arg("--nocapture")
                .env("OWT_FORK_WRITER", "1")
                .env("OWT_FORK_DIR", &dir)
                .env("OWT_FORK_CHILD", child.to_string())
                .status()
                .expect("spawn fork writer child");
            assert!(status.success(), "fork writer child {child} failed");
        }
        let store = JsonlStore::open_with_clock(Scope::User, dir.clone(), now()).unwrap();
        let active = store.active(&StoreFilter::default()).unwrap();
        assert_eq!(active.len(), 60, "lost update across processes");
    }

    #[test]
    fn permission_denied_is_controlled() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Root bypasses permission checks; the test is a no-op then.
            if current_uid() == 0 {
                return;
            }
            let dir = test_dir("perm");
            let mut store = open_at(Scope::User, &dir);
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
            let result = store.remember(new_memory("owt_1_1_1", "x"));
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
            let error = result.expect_err("write into a read-only dir must fail");
            assert!(
                matches!(
                    error,
                    MemoryError::PermissionDenied | MemoryError::StoreUnavailable(_)
                ),
                "unexpected error: {error}"
            );
        }
    }

    /// Real uid via /proc/self/status (no libc dependency in tests either).
    #[cfg(unix)]
    fn current_uid() -> u32 {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| {
                status.lines().find_map(|line| {
                    line.strip_prefix("Uid:")
                        .and_then(|rest| rest.split_whitespace().next())
                        .and_then(|value| value.parse().ok())
                })
            })
            .unwrap_or(u32::MAX)
    }
}
