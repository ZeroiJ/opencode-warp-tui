//! The `MemoryApi`: what the OpenCode adapter talks to. Holds both scope
//! stores, resolves `--scope` targets, runs the secret policy *before* any
//! state change, assigns ids/session provenance, and merges reads into the
//! deterministic orders the commands render. No OpenCode types here.

use std::path::PathBuf;

use super::key;
use super::record::{order_active, order_show, Kind, MemoryRecord, NewMemory, Scope};
use super::secret::{scan, SecretVerdict};
use super::store::{ForgetOutcome, Handle, JsonlStore, MemoryStore, StoreFilter, WriteOutcome};
use super::MemoryError;

/// Which store(s) a command addresses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScopeTarget {
    /// Default: `user` for writes; both scopes for reads (`list`, `show`);
    /// one unambiguous store for forget/pin via cross-scope resolution
    /// (retrieval §7.1 — ambiguity is a Conflict listing the candidates).
    #[default]
    Default,
    User,
    Project,
}

/// Ingestion arguments parsed from a `/memory` command. The API assigns the
/// record id and session provenance (Phase 4 verified the adapter only knows
/// the session id at command time).
#[derive(Clone, Debug)]
pub struct NewMemoryArgs {
    pub key: Option<String>,
    pub kind: Kind,
    pub content: String,
    pub pinned: bool,
    pub scope: ScopeTarget,
}

/// A successful write plus any secretary warning to surface in-band.
#[derive(Clone, Debug)]
pub struct WriteResult {
    pub outcome: WriteOutcome,
    pub warning: Option<String>,
}

/// List filters (scope resolution stays in the API).
#[derive(Clone, Debug, Default)]
pub struct ListFilter {
    pub scope: ScopeTarget,
    pub kind: Option<Kind>,
    pub pinned: Option<bool>,
}

/// The memory engine behind one adapter. Infallible to construct on purpose:
/// a store that cannot open is logged and remembered as `disabled`, and
/// every operation then fails with that controlled error — the OpenCode
/// session never fails because memory failed (Phase 5 contract).
pub struct MemoryApi {
    user: Option<JsonlStore>,
    project: Option<JsonlStore>,
    disabled: Option<MemoryError>,
}

impl MemoryApi {
    /// Open both stores (infallible; failures are logged, not propagated).
    pub fn open(user_dir: PathBuf, project_root: Option<PathBuf>) -> MemoryApi {
        let user = match JsonlStore::open(Scope::User, user_dir) {
            Ok(store) => Some(store),
            Err(error) => {
                log::warn!("memory: user store unavailable: {error}; memory commands disabled");
                return MemoryApi {
                    user: None,
                    project: None,
                    disabled: Some(error),
                };
            }
        };
        let project = match project_root {
            Some(root) => {
                let dir = super::store::project_store_dir(&root);
                match JsonlStore::open(Scope::Project, dir) {
                    Ok(store) => Some(store),
                    Err(error) => {
                        log::warn!(
                            "memory: project store unavailable: {error}; project-scope commands will fail"
                        );
                        None
                    }
                }
            }
            None => None,
        };
        MemoryApi {
            user,
            project,
            disabled: None,
        }
    }

    /// Reject every operation with the stored open error when the user store
    /// failed to open (memory is disabled, the session continues).
    fn guard(&self) -> Result<(), MemoryError> {
        match &self.disabled {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    fn user(&self) -> Result<&JsonlStore, MemoryError> {
        self.guard()?;
        Ok(self.user.as_ref().expect("user store present when enabled"))
    }

    fn user_mut(&mut self) -> Result<&mut JsonlStore, MemoryError> {
        self.guard()?;
        Ok(self.user.as_mut().expect("user store present when enabled"))
    }

    fn project(&self) -> Result<&JsonlStore, MemoryError> {
        self.project
            .as_ref()
            .ok_or_else(|| MemoryError::InvalidScope("no project scope (no project root)".into()))
    }

    fn project_mut(&mut self) -> Result<&mut JsonlStore, MemoryError> {
        self.project
            .as_mut()
            .ok_or_else(|| MemoryError::InvalidScope("no project scope (no project root)".into()))
    }

    /// Single store for a write. Default scope = user (documented decision:
    /// machine-wide by default, `--scope=project` opt-in per project).
    fn write_target(&mut self, target: ScopeTarget) -> Result<&mut JsonlStore, MemoryError> {
        match target {
            ScopeTarget::User | ScopeTarget::Default => self.user_mut(),
            ScopeTarget::Project => self.project_mut(),
        }
    }

    /// Like `write_target`, but for Default resolves a delete/pin across both
    /// scopes: key ACTIVE in both → Conflict listing the candidates; in one →
    /// that store; in neither → NotFound (retrieval §7.1).
    fn mutation_target(
        &mut self,
        handle: &Handle,
        target: ScopeTarget,
    ) -> Result<&mut JsonlStore, MemoryError> {
        match target {
            ScopeTarget::User | ScopeTarget::Project => self.write_target(target),
            ScopeTarget::Default => {
                let user_has = self.user()?.contains_handle(handle)?;
                let project_has = match &self.project {
                    Some(project) => project.contains_handle(handle)?,
                    None => false,
                };
                match (user_has, project_has) {
                    (true, true) => Err(MemoryError::Conflict(format!(
                        "{handle} exists in both scopes — repeat with --scope=user or --scope=project"
                    ))),
                    (true, false) => self.user_mut(),
                    (false, true) => self.project_mut(),
                    (false, false) => Err(MemoryError::NotFound(handle.to_string())),
                }
            }
        }
    }

    fn scan_first(content: &str) -> Result<Option<String>, MemoryError> {
        match scan(content) {
            SecretVerdict::Refused => Err(MemoryError::SecretRefused),
            SecretVerdict::Warn(message) => Ok(Some(message)),
            SecretVerdict::None => Ok(None),
        }
    }

    /// Remember a new memory (Default scope → user store).
    pub fn remember(
        &mut self,
        session_id: &str,
        args: &NewMemoryArgs,
    ) -> Result<WriteResult, MemoryError> {
        let warning = Self::scan_first(&args.content)?;
        let store = self.write_target(args.scope)?;
        let new = NewMemory {
            id: key::generate_id(),
            key: args.key.clone(),
            kind: args.kind,
            content: args.content.clone(),
            pinned: args.pinned,
            session_id: session_id.trim().to_owned(),
            source_ref: None,
            quote: None,
        };
        let outcome = store.remember(new)?;
        Ok(WriteResult { outcome, warning })
    }

    /// Update a memory: same-key remember on an existing ACTIVE key.
    pub fn update(
        &mut self,
        session_id: &str,
        args: &NewMemoryArgs,
    ) -> Result<WriteResult, MemoryError> {
        let Some(key) = args.key.clone() else {
            return Err(MemoryError::InvalidMemory(
                "update needs a key: /memory update <key>: <content>".into(),
            ));
        };
        let warning = Self::scan_first(&args.content)?;
        let store = self.write_target(args.scope)?;
        let new = NewMemory {
            id: key::generate_id(),
            key: Some(key.clone()),
            kind: args.kind,
            content: args.content.clone(),
            pinned: args.pinned,
            session_id: session_id.trim().to_owned(),
            source_ref: None,
            quote: None,
        };
        let outcome = store.update(&key, new)?;
        Ok(WriteResult { outcome, warning })
    }

    /// Forget by exact key (ACTIVE) or exact id (any status); Default scope
    /// resolves unambiguously across both stores or fails with candidates.
    pub fn forget(
        &mut self,
        handle: &Handle,
        scope: ScopeTarget,
    ) -> Result<ForgetOutcome, MemoryError> {
        let store = self.mutation_target(handle, scope)?;
        store.forget(handle)
    }

    /// Pin/unpin (same resolution rules as forget).
    pub fn set_pinned(
        &mut self,
        handle: &Handle,
        pinned: bool,
        scope: ScopeTarget,
    ) -> Result<MemoryRecord, MemoryError> {
        let store = self.mutation_target(handle, scope)?;
        store.set_pinned(handle, pinned)
    }

    /// ACTIVE records matching the filters, in D24 order. Default scope = the
    /// whole corpus (both scopes).
    pub fn list(&self, filter: &ListFilter) -> Result<Vec<MemoryRecord>, MemoryError> {
        self.guard()?;
        let store_filter = StoreFilter {
            kind: filter.kind,
            pinned: filter.pinned,
        };
        let mut records = Vec::new();
        match filter.scope {
            ScopeTarget::User => records.extend(self.user()?.active(&store_filter)?),
            ScopeTarget::Project => records.extend(self.project()?.active(&store_filter)?),
            ScopeTarget::Default => {
                records.extend(self.user()?.active(&store_filter)?);
                if let Some(project) = &self.project {
                    records.extend(project.active(&store_filter)?);
                }
            }
        }
        order_active(&mut records);
        Ok(records)
    }

    /// Exact show: by id (any status/history) or by key (ACTIVE, or the slot
    /// history with `all`). Default scope returns matches from both stores.
    pub fn show(
        &self,
        handle: &Handle,
        all: bool,
        scope: ScopeTarget,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        self.guard()?;
        let mut records = Vec::new();
        match scope {
            ScopeTarget::User => records.extend(self.user()?.show(handle, all)?),
            ScopeTarget::Project => records.extend(self.project()?.show(handle, all)?),
            ScopeTarget::Default => {
                records.extend(self.user()?.show(handle, all)?);
                if let Some(project) = &self.project {
                    records.extend(project.show(handle, all)?);
                }
            }
        }
        order_show(&mut records);
        Ok(records)
    }

    /// The ACTIVE corpus from both scopes in D24 order — the read Phase 6's
    /// session-start builder consumes (via `budget::select`). Nothing writes
    /// it anywhere.
    #[allow(dead_code)] // Phase 6 consumer (session-start injection builder).
    pub fn ordered_active(&self) -> Result<Vec<MemoryRecord>, MemoryError> {
        self.guard()?;
        let mut records = self.user()?.active(&StoreFilter::default())?;
        if let Some(project) = &self.project {
            records.extend(project.active(&StoreFilter::default())?);
        }
        order_active(&mut records);
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory::record::{Kind, Scope};
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("owt-api-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn args(content: &str) -> NewMemoryArgs {
        NewMemoryArgs {
            key: Some("k".to_owned()),
            kind: Kind::Fact,
            content: content.to_owned(),
            pinned: false,
            scope: ScopeTarget::Default,
        }
    }

    fn api_with(name: &str, project: bool) -> MemoryApi {
        let user_dir = temp_dir(&format!("{name}-user"));
        let project_root = if project {
            Some(temp_dir(&format!("{name}-root")))
        } else {
            None
        };
        MemoryApi::open(user_dir, project_root)
    }

    #[test]
    fn remember_default_goes_to_user_scope() {
        let mut api = api_with("default-write", false);
        let result = api
            .remember("ses_1", &args("Prefer Rust for new services."))
            .unwrap();
        assert_eq!(result.outcome.created.scope, Scope::User);
        assert!(result.outcome.created.id.starts_with("owt_"));
        assert!(result.warning.is_none());
        let listed = api.list(&ListFilter::default()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].content, "Prefer Rust for new services.");
    }

    #[test]
    fn project_scope_requires_project_root() {
        let mut api = api_with("no-project", false);
        let mut project_args = args("project fact");
        project_args.scope = ScopeTarget::Project;
        let err = api.remember("ses_1", &project_args).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidScope(_)));
    }

    #[test]
    fn project_scope_writes_to_project_store() {
        let mut api = api_with("project-write", true);
        let mut project_args = args("the build is cargo-make driven");
        project_args.scope = ScopeTarget::Project;
        project_args.key = Some("build".to_owned());
        let result = api.remember("ses_1", &project_args).unwrap();
        assert_eq!(result.outcome.created.scope, Scope::Project);
        // Default list = whole corpus (both scopes).
        let listed = api.list(&ListFilter::default()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].scope, Scope::Project);
        // --scope=user list is empty.
        let user_only = api
            .list(&ListFilter {
                scope: ScopeTarget::User,
                ..ListFilter::default()
            })
            .unwrap();
        assert!(user_only.is_empty());
    }

    #[test]
    fn secret_refusal_leaves_no_state_change() {
        let mut api = api_with("secret", false);
        // Fixture shape split at runtime — see `secret::tests::tok`.
        let content = format!("key {}", ["sk-", "abcdefghijklmnopqrstuvwxyz"].concat());
        let err = api.remember("ses_1", &args(&content)).unwrap_err();
        assert_eq!(err, MemoryError::SecretRefused);
        assert!(api.list(&ListFilter::default()).unwrap().is_empty());
    }

    #[test]
    fn forget_resolves_cross_scope_ambiguity() {
        let mut api = api_with("forget", true);
        let mut project_args = args("project k");
        project_args.scope = ScopeTarget::Project;
        api.remember("ses_1", &args("user k")).unwrap();
        api.remember("ses_1", &project_args).unwrap();
        // Default → ambiguous: Conflict listing candidates.
        let err = api
            .forget(&Handle::Key("k".into()), ScopeTarget::Default)
            .unwrap_err();
        assert!(matches!(err, MemoryError::Conflict(_)));
        // Explicit user scope succeeds; project copy survives.
        let removed = api
            .forget(&Handle::Key("k".into()), ScopeTarget::User)
            .unwrap();
        assert!(removed.id.starts_with("owt_"));
        let user = api
            .list(&ListFilter {
                scope: ScopeTarget::User,
                ..ListFilter::default()
            })
            .unwrap();
        assert!(user.is_empty());
        let project = api
            .list(&ListFilter {
                scope: ScopeTarget::Project,
                ..ListFilter::default()
            })
            .unwrap();
        assert_eq!(project.len(), 1);
    }

    #[test]
    fn update_requires_a_key() {
        let mut api = api_with("update", false);
        let mut keyless = args("no key here");
        keyless.key = None;
        let err = api.update("ses_1", &keyless).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidMemory(_)));
        api.remember("ses_1", &args("first")).unwrap();
        let updated = api.update("ses_1", &args("second")).unwrap();
        assert_eq!(updated.outcome.created.content, "second");
        assert_eq!(
            updated.outcome.superseded.as_ref().unwrap().content,
            "first"
        );
    }

    #[test]
    fn show_merges_scopes_and_orders_deterministically() {
        let mut api = api_with("show", true);
        let mut user_args = args("user value");
        user_args.key = Some("shared".to_owned());
        let mut project_args = args("project value");
        project_args.scope = ScopeTarget::Project;
        project_args.key = Some("shared".to_owned());
        api.remember("ses_1", &user_args).unwrap();
        api.remember("ses_1", &project_args).unwrap();
        let shown = api
            .show(&Handle::Key("shared".into()), false, ScopeTarget::Default)
            .unwrap();
        assert_eq!(shown.len(), 2);
        // Deterministic `show` order: ACTIVE first, then recency, then id.
        // The fixed test clock ties every timestamp, so creation/id order
        // decides — user was created first.
        assert!(shown[0].content.starts_with("user"));
        assert!(shown[1].content.starts_with("project"));
        // Stable across calls (the contract the TUI relies on).
        let again = api
            .show(&Handle::Key("shared".into()), false, ScopeTarget::Default)
            .unwrap();
        assert_eq!(shown, again);
    }

    #[test]
    fn pin_bumps_updated_at_and_reorders() {
        let mut api = api_with("pin", false);
        api.remember("ses_1", &args("first")).unwrap();
        let second = {
            let mut a = args("second");
            a.key = Some("other".to_owned());
            a
        };
        api.remember("ses_1", &second).unwrap();
        let pinned = api
            .set_pinned(&Handle::Key("k".into()), true, ScopeTarget::Default)
            .unwrap();
        assert!(pinned.pinned);
        let listed = api.list(&ListFilter::default()).unwrap();
        assert!(listed[0].pinned);
        assert!(listed[0].key.as_deref() == Some("k"));
    }

    #[test]
    fn disabled_api_fails_every_op_with_the_same_error() {
        // A *file* in the way of the store dir prevents creation.
        let blocking = temp_dir("disabled");
        std::fs::create_dir_all(&blocking).unwrap();
        let file_path = blocking.join("user");
        std::fs::write(&file_path, "not a dir").unwrap();
        let api = MemoryApi::open(file_path, None);
        // The open error is remembered; the first operation reports it.
        let err = api.list(&ListFilter::default()).unwrap_err();
        assert!(matches!(err, MemoryError::StoreUnavailable(_)));
    }

    #[test]
    fn ordered_active_is_d24_and_never_persists() {
        let mut api = api_with("ordered", true);
        let mut p = args("project pinned pk");
        p.scope = ScopeTarget::Project;
        p.key = Some("pk".to_owned());
        p.pinned = true;
        // user unpinned (older), then a project pinned → project first.
        api.remember("ses_1", &args("user recent")).unwrap();
        api.remember("ses_1", &p).unwrap();
        let ordered = api.ordered_active().unwrap();
        assert_eq!(ordered.len(), 2);
        assert_eq!(ordered[0].scope, Scope::Project);
        assert!(ordered[0].pinned);
    }
}
