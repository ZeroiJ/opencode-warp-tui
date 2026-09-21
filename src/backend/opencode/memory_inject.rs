//! Phase 6B — Context Builder + OpenCode memory injection.
//!
//! The one bridge from the Phase 5 memory engine into a live session:
//!
//! ```text
//! MemoryApi::ordered_active() → budget::select → budget::encoded_block
//!     → PUT owt.memory (before the first prompt, new sessions only)
//! ```
//!
//! Rules: empty corpus → no entry; any memory failure → warn + continue
//! (never fail session creation); resume paths never call here; `off`
//! disables probe + PUT entirely. No memory *content* in logs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::client::{Client, ClientError};
use crate::backend::memory::budget;
use crate::backend::memory::record::format_rfc3339_utc;
use crate::backend::memory::MemoryApi;

/// The single instruction-entry key (Phase 6A contract).
pub const MEMORY_KEY: &str = "owt.memory";
/// Self-cleaning probe key (same shape, throwaway sessions only).
const PROBE_KEY: &str = "owt.probe";
/// Capability cache TTL (~1 h per Phase 6A).
const CAPABILITY_TTL: Duration = Duration::from_secs(3600);

/// Warn-once latch for capability/injection failures (bounded logging).
static WARNED: AtomicBool = AtomicBool::new(false);

fn warn_once(message: &str) {
    if !WARNED.swap(true, Ordering::SeqCst) {
        log::warn!("{message}");
    }
}

/// Cached capability: which server version the probe result belongs to.
#[derive(Clone, Debug)]
struct CapabilityCache {
    version: String,
    supported: bool,
    probed_at: Instant,
}

impl CapabilityCache {
    fn fresh_for(&self, version: &str) -> Option<bool> {
        if self.version == version && self.probed_at.elapsed() < CAPABILITY_TTL {
            Some(self.supported)
        } else {
            None
        }
    }
}

/// Per-backend injection state: capability cache + config gate.
#[derive(Debug, Default)]
pub struct Injector {
    cache: Option<CapabilityCache>,
}

impl Injector {
    /// Build the encoded `{"text": block}` entry value, or `None` when the
    /// corpus is empty (→ write no entry at all) or unreadable.
    pub fn build_entry(memory: &MemoryApi, budget_chars: usize, now: &str) -> Option<String> {
        let ordered = memory.ordered_active().ok()?;
        let selection = budget::select(&ordered, budget_chars, now);
        budget::encoded_block(&selection.records, now)
    }

    /// Session-start timestamp on the Phase 5 clock abstraction
    /// (`record::format_rfc3339_utc`, hand-rolled civil-date conversion).
    pub fn now_stamp() -> String {
        format_rfc3339_utc(std::time::SystemTime::now())
    }

    /// Capability probe: throwaway session → PUT probe → GET → DELETE key →
    /// DELETE session. Any failure ⇒ unsupported. Self-cleaning on the
    /// happy path; failures attempt session cleanup best-effort.
    /// ponytail: O(1) synchronous roundtrips on the adapter thread; no
    /// retry — one probe per cache lifetime, skip on failure.
    fn probe(client: &Client) -> bool {
        let created = match client.create_session("owt-capability-probe") {
            Ok(value) => value,
            Err(_) => return false,
        };
        let id = created
            .get("data")
            .and_then(|data| data.get("id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned();
        if id.is_empty() {
            return false;
        }
        let ok = Self::probe_roundtrip(client, &id);
        // Best-effort cleanup; the entry is session-scoped so an orphan dies
        // with its session anyway.
        let _ = client.delete_session(&id);
        ok
    }

    fn probe_roundtrip(client: &Client, id: &str) -> bool {
        let value = serde_json::json!({"text": "probe"});
        if client.put_instruction_entry(id, PROBE_KEY, &value).is_err() {
            return false;
        }
        let seen = client
            .get_instruction_entry(id, PROBE_KEY)
            .ok()
            .and_then(|got| got.get("data").and_then(|data| data.get("value")).cloned());
        if seen.as_ref() != Some(&value) {
            // Some servers wrap single-key GETs differently; fall back to
            // accepting any 2xx read as presence.
            if client.get_instruction_entry(id, PROBE_KEY).is_err() {
                return false;
            }
        }
        client.delete_instruction_entry(id, PROBE_KEY).is_ok()
    }

    /// Resolve capability, using the version-keyed cache. `None` version
    /// (unreachable `/api/info`) still probes once — version-agnostic.
    fn supported(&mut self, client: &Client) -> bool {
        let version = client
            .info()
            .ok()
            .and_then(|info| {
                info.get("data")
                    .and_then(|data| data.get("version"))
                    .or_else(|| info.get("version"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        if let Some(cached) = self.cache.as_ref().and_then(|c| c.fresh_for(&version)) {
            return cached;
        }
        let supported = Self::probe(client);
        self.cache = Some(CapabilityCache {
            version,
            supported,
            probed_at: Instant::now(),
        });
        supported
    }

    /// Inject into a brand-new session: capability → snapshot → PUT
    /// `owt.memory`. Must be called inline in the create path (code
    /// placement is the PUT-before-first-prompt guarantee). Every failure
    /// mode warns at most once and returns normally — the session proceeds.
    pub fn inject_new_session(&mut self, client: &Client, memory: &MemoryApi, session_id: &str) {
        if session_id.starts_with("local-") {
            return; // server create failed; nothing to write to.
        }
        let now = Self::now_stamp();
        let Some(encoded) = Self::build_entry(memory, budget::DEFAULT_BUDGET_CHARS, &now) else {
            return; // empty corpus → no entry.
        };
        if !self.supported(client) {
            warn_once("memory: instruction entries unsupported; session continues without memory");
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&encoded) else {
            return;
        };
        match client.put_instruction_entry(session_id, MEMORY_KEY, &value) {
            Ok(_) => {}
            Err(ClientError::Status(413, _)) => {
                warn_once(
                    "memory: entry rejected as too large (413); session continues without memory",
                );
            }
            Err(error) => {
                warn_once(&format!(
                    "memory: injection skipped ({}); session continues without memory",
                    short_error(&error)
                ));
            }
        }
    }
}

/// 413 bodies name the limit; other bodies are truncated by the client to
/// 300 chars already — never log payload content, only the status/reason.
fn short_error(error: &ClientError) -> String {
    match error {
        ClientError::Transport(reason) => format!("transport {reason}"),
        ClientError::Status(code, _) => format!("http {code}"),
        ClientError::Json(reason) => format!("invalid json {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory::api::{NewMemoryArgs, ScopeTarget};
    use crate::backend::memory::record::Kind;
    use crate::backend::opencode::config::{Endpoint, MemoryInjection};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// Minimal stub OpenCode server on std only. Each handler decides the
    /// status + body from the request line + body; hits are recorded.
    type Handler = dyn Fn(&str, &str) -> (u16, String) + Send + Sync;
    struct Stub {
        addr: std::net::SocketAddr,
        hits: Arc<Mutex<Vec<String>>>,
        #[allow(dead_code)]
        handler: Arc<Handler>,
    }

    impl Stub {
        fn spawn(handler: impl Fn(&str, &str) -> (u16, String) + Send + Sync + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let hits = Arc::new(Mutex::new(Vec::new()));
            let handler = Arc::new(handler);
            let hits_thread = hits.clone();
            let handler_thread = handler.clone();
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
                    let body = String::from_utf8_lossy(&body).into_owned();
                    hits_thread
                        .lock()
                        .unwrap()
                        .push(format!("{} | {body}", request_line.trim_end()));
                    let (status, response_body) = handler_thread(&request_line, &body);
                    let reason = match status {
                        200 => "OK",
                        204 => "No Content",
                        400 => "Bad Request",
                        404 => "Not Found",
                        413 => "Content Too Large",
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
            Stub {
                addr,
                hits,
                handler,
            }
        }

        fn client(&self) -> Client {
            Client::new(&Endpoint {
                url: format!("http://{}", self.addr),
                password: None,
            })
        }

        fn hits(&self) -> Vec<String> {
            self.hits.lock().unwrap().clone()
        }
    }

    fn memory_with_fact(name: &str) -> MemoryApi {
        let user_dir =
            std::env::temp_dir().join(format!("owt-inject-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user_dir);
        let mut api = MemoryApi::open(user_dir, None);
        api.remember(
            "ses_1",
            &NewMemoryArgs {
                key: Some("lang".into()),
                kind: Kind::Fact,
                content: "Prefer Rust for new services.".into(),
                pinned: false,
                scope: ScopeTarget::Default,
                method: None,
                quote: None,
                session_id: None,
            },
        )
        .unwrap();
        api
    }

    fn full_capable_stub() -> Stub {
        Stub::spawn(|request, _| {
            if request.starts_with("GET /api/info") {
                (200, r#"{"data":{"version":"9.9.9-test"}}"#.into())
            } else if request.starts_with("POST /api/session ") {
                (200, r#"{"data":{"id":"ses_probe1"}}"#.into())
            } else if request.contains("/instructions/entries/") {
                if request.starts_with("GET") {
                    (200, r#"{"data":{"value":{"text":"probe"}}}"#.into())
                } else {
                    (204, String::new())
                }
            } else if request.starts_with("DELETE /api/session/") {
                (204, String::new())
            } else {
                (404, "{}".into())
            }
        })
    }

    #[test]
    fn empty_corpus_writes_no_entry() {
        let stub = full_capable_stub();
        let user_dir =
            std::env::temp_dir().join(format!("owt-inject-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user_dir);
        let memory = MemoryApi::open(user_dir, None);
        let mut injector = Injector::default();
        injector.inject_new_session(&stub.client(), &memory, "ses_new1");
        // Probe traffic only; no owt.memory PUT for the real session.
        assert!(!stub
            .hits()
            .iter()
            .any(|hit| hit.contains("ses_new1") && hit.contains(MEMORY_KEY)));
    }

    #[test]
    fn new_session_puts_exact_key_and_shape() {
        let stub = full_capable_stub();
        let memory = memory_with_fact("empty");
        let mut injector = Injector::default();
        injector.inject_new_session(&stub.client(), &memory, "ses_new2");
        let put = stub
            .hits()
            .into_iter()
            .find(|hit| hit.contains("ses_new2") && hit.contains(MEMORY_KEY))
            .expect("owt.memory PUT happened");
        assert!(put
            .starts_with("PUT /api/experimental/session/ses_new2/instructions/entries/owt.memory"));
        assert!(put.contains(r#"\"text\":"#) || put.contains("\"text\":"));
    }

    #[test]
    fn unsupported_server_disables_injection() {
        let stub = Stub::spawn(|request, _| {
            if request.starts_with("GET /api/info") {
                (200, r#"{"data":{"version":"0.0.0"}}"#.into())
            } else if request.starts_with("POST /api/session ") {
                (200, r#"{"data":{"id":"ses_probe9"}}"#.into())
            } else if request.starts_with("DELETE /api/session/") {
                (204, String::new())
            } else {
                (404, "{}".into()) // no entries surface
            }
        });
        let memory = memory_with_fact("shape");
        let mut injector = Injector::default();
        injector.inject_new_session(&stub.client(), &memory, "ses_new3");
        assert!(!stub
            .hits()
            .iter()
            .any(|hit| hit.contains("ses_new3") && hit.contains(MEMORY_KEY)));
    }

    #[test]
    fn low_limit_413_skips_without_failing() {
        let stub = Stub::spawn(|request, _| {
            if request.starts_with("GET /api/info") {
                (200, r#"{"data":{"version":"9.9.9-test"}}"#.into())
            } else if request.starts_with("POST /api/session ") {
                (200, r#"{"data":{"id":"ses_probe1"}}"#.into())
            } else if request.contains("/instructions/entries/") {
                if request.starts_with("GET") {
                    (200, r#"{"data":{"value":{"text":"probe"}}}"#.into())
                } else if request.contains("ses_tiny") {
                    (
                        413,
                        r#"{"_tag":"InstructionEntryValueTooLargeError","maxBytes":10}"#.into(),
                    )
                } else {
                    (204, String::new())
                }
            } else if request.starts_with("DELETE /api/session/") {
                (204, String::new())
            } else {
                (404, "{}".into())
            }
        });
        let memory = memory_with_fact("unsup");
        let mut injector = Injector::default();
        // Does not panic / propagate: session creation "succeeds" regardless.
        injector.inject_new_session(&stub.client(), &memory, "ses_tiny");
        assert!(stub
            .hits()
            .iter()
            .any(|hit| hit.contains("ses_tiny") && hit.contains(MEMORY_KEY)));
    }

    #[test]
    fn offline_server_never_fails_session() {
        let client = Client::new(&Endpoint {
            url: "http://127.0.0.1:1".into(), // nothing listens here
            password: None,
        });
        let memory = memory_with_fact("low413");
        let mut injector = Injector::default();
        injector.inject_new_session(&client, &memory, "ses_offline");
    }

    #[test]
    fn probe_is_self_cleaning_and_cached() {
        let stub = full_capable_stub();
        let memory = memory_with_fact("offline");
        let mut injector = Injector::default();
        injector.inject_new_session(&stub.client(), &memory, "ses_a");
        injector.inject_new_session(&stub.client(), &memory, "ses_b");
        let hits = stub.hits();
        // One probe (one throwaway create), two real PUTs — cache hit on #2.
        assert_eq!(
            hits.iter()
                .filter(|h| h.contains("POST /api/session "))
                .count(),
            1,
            "{hits:?}"
        );
        assert!(hits
            .iter()
            .any(|h| h.contains("ses_a") && h.contains(MEMORY_KEY)));
        assert!(hits
            .iter()
            .any(|h| h.contains("ses_b") && h.contains(MEMORY_KEY)));
        // Cleanup: probe key deleted + throwaway session deleted.
        assert!(hits
            .iter()
            .any(|h| h.contains("DELETE") && h.contains("owt.probe")));
        assert!(hits
            .iter()
            .any(|h| h.contains("DELETE /api/session/ses_probe1")));
    }

    #[test]
    fn context_builder_preserves_d24_and_budget() {
        let memory = memory_with_fact("cached");
        let now = Injector::now_stamp();
        let encoded = Injector::build_entry(&memory, budget::DEFAULT_BUDGET_CHARS, &now).unwrap();
        assert!(encoded.len() <= budget::MAX_ENCODED_BYTES);
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        let text = value.get("text").and_then(|t| t.as_str()).unwrap();
        assert!(text.contains("Prefer Rust for new services."));
        assert!(text.ends_with("[owt-memory end]\n"));
        // Empty corpus → None.
        let user_dir = std::env::temp_dir().join(format!("owt-inject-mt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user_dir);
        let empty = MemoryApi::open(user_dir, None);
        assert!(Injector::build_entry(&empty, budget::DEFAULT_BUDGET_CHARS, &now).is_none());
    }

    #[test]
    fn injection_gate_parses() {
        assert_eq!(MemoryInjection::from_env_value(None), MemoryInjection::Auto);
        assert_eq!(
            MemoryInjection::from_env_value(Some("off")),
            MemoryInjection::Off
        );
        assert_eq!(
            MemoryInjection::from_env_value(Some("OFF")),
            MemoryInjection::Off
        );
        assert_eq!(
            MemoryInjection::from_env_value(Some("auto")),
            MemoryInjection::Auto
        );
    }
}
