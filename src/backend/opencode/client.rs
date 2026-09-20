//! Minimal blocking OpenCode HTTP client (JSON over HTTP/1.1, localhost).
//!
//! Only the endpoints the adapter needs; every response is parsed as
//! `serde_json::Value` (tolerant reader — see `mapper`). Timeouts keep the UI
//! thread responsive: adapter trait methods perform at most one quick
//! roundtrip each.

use serde_json::Value;
use std::time::Duration;

use super::config::Endpoint;

const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub enum ClientError {
    Transport(String),
    Status(u16, String),
    Json(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Transport(e) => write!(f, "transport: {e}"),
            ClientError::Status(code, body) => write!(f, "http {code}: {body}"),
            ClientError::Json(e) => write!(f, "invalid json: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Sync client: an agent that returns error bodies instead of raising, so
/// OpenCode's `{"_tag",...}` envelopes stay visible.
#[derive(Clone, Debug)]
pub struct Client {
    agent: ureq::Agent,
    base: String,
    auth: Option<String>,
}

impl Client {
    pub fn new(endpoint: &Endpoint) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            agent,
            base: endpoint.url.trim_end_matches('/').to_owned(),
            // Local servers use HTTP Basic `opencode:<password>` (verified
            // live against 1.18.31); unsecured servers send no auth at all.
            auth: endpoint
                .password
                .as_deref()
                .map(|password| format!("Basic {}", basic_auth("opencode", password))),
        }
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, ClientError> {
        let url = format!("{}{path}", self.base);
        let auth = self.auth.clone();
        let with_auth =
            |request: ureq::RequestBuilder<ureq::typestate::WithBody>| match auth.as_deref() {
                Some(auth) => request.header("Authorization", auth),
                None => request,
            };
        let with_auth_nobody =
            |request: ureq::RequestBuilder<ureq::typestate::WithoutBody>| match auth.as_deref() {
                Some(auth) => request.header("Authorization", auth),
                None => request,
            };
        let transport = |error: ureq::Error| ClientError::Transport(error.to_string());
        let mut response = match (method, body) {
            ("GET", None) => with_auth_nobody(self.agent.get(&url))
                .call()
                .map_err(transport)?,
            ("DELETE", None) => with_auth_nobody(self.agent.delete(&url))
                .call()
                .map_err(transport)?,
            ("POST", Some(body)) => with_auth(self.agent.post(&url))
                .send_json(body)
                .map_err(transport)?,
            ("POST", None) => with_auth(self.agent.post(&url))
                .send_empty()
                .map_err(transport)?,
            ("PATCH", body) => {
                let builder = with_auth(self.agent.patch(&url));
                match body {
                    Some(body) => builder.send_json(body).map_err(transport)?,
                    None => builder.send_empty().map_err(transport)?,
                }
            }
            ("PUT", body) => {
                let builder = with_auth(self.agent.put(&url));
                match body {
                    Some(body) => builder.send_json(body).map_err(transport)?,
                    None => builder.send_empty().map_err(transport)?,
                }
            }
            (other, _) => return Err(ClientError::Transport(format!("bad method {other}"))),
        };
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().map_err(transport)?;
        if !(200..300).contains(&status) {
            let short: String = text.chars().take(300).collect();
            return Err(ClientError::Status(status, short));
        }
        // 204 No Content (instruction PUT/DELETE, session DELETE) carries
        // no body — report Null rather than a parse error.
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|error| ClientError::Json(error.to_string()))
    }

    fn get(&self, path: &str) -> Result<Value, ClientError> {
        self.request("GET", path, None)
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value, ClientError> {
        self.request("POST", path, Some(body))
    }

    pub fn health(&self) -> Result<Value, ClientError> {
        self.get("/api/health")
    }

    pub fn list_sessions(&self) -> Result<Value, ClientError> {
        self.get("/api/session")
    }

    pub fn create_session(&self, title: &str) -> Result<Value, ClientError> {
        self.post("/api/session", &serde_json::json!({"title": title}))
    }

    pub fn get_session(&self, session: &str) -> Result<Value, ClientError> {
        self.get(&format!("/api/session/{session}"))
    }

    /// Page of session messages, oldest-first. Live 2.0.8 pagination is
    /// cursor-based (`limit`/`order`/`cursor`/`type` — **no `offset`**);
    /// the default wire order is `desc` (newest first), so history
    /// hydration requests `order=asc` explicitly and follows `cursor.next`.
    pub fn list_messages_paged(
        &self,
        session: &str,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Value, ClientError> {
        let mut query = String::from("limit=200&order=asc");
        if let Some(limit) = limit {
            query = format!("limit={limit}&order=asc");
        }
        if let Some(cursor) = cursor {
            query.push_str("&cursor=");
            query.push_str(cursor);
        }
        self.get(&format!("/api/session/{session}/message?{query}"))
    }

    /// Send a user prompt. Wire shape is version-skewed: current servers
    /// take a flat `{"text"}` (published V2 contract); 1.18.x servers want
    /// `{"prompt": {"text"}}` (verified live). Try flat first, fall back to
    /// wrapped when validation names the missing `prompt` key.
    pub fn send_prompt(&self, session: &str, text: &str) -> Result<Value, ClientError> {
        let flat = serde_json::json!({"text": text});
        match self.post(&format!("/api/session/{session}/prompt"), &flat) {
            Err(ClientError::Status(400, body)) if body.contains("Missing key") => {
                log::warn!("server wants wrapped prompt body; retrying");
                self.post(
                    &format!("/api/session/{session}/prompt"),
                    &serde_json::json!({"prompt": {"text": text}}),
                )
            }
            other => other,
        }
    }

    pub fn interrupt(&self, session: &str) -> Result<Value, ClientError> {
        self.post(
            &format!("/api/session/{session}/interrupt"),
            &serde_json::json!({}),
        )
    }

    /// Reply to a pending permission request. `accept` maps to `once`,
    /// deny maps to `reject` (`always` reserved for a future allow-list UI).
    pub fn reply_permission(
        &self,
        session: &str,
        request: &str,
        accept: bool,
    ) -> Result<Value, ClientError> {
        self.post(
            &format!("/api/session/{session}/permission/{request}/reply"),
            &serde_json::json!({"decision": if accept { "once" } else { "reject" }}),
        )
    }

    /// Reply to a question form (live 2.0.8: the `question` tool call
    /// materializes as `form.created` with `metadata.kind == "question"`;
    /// `question.v2.asked` never fires on that build). Body shape verified
    /// live: `{"answer": {field_key: option_value}}` → 204. `key` is the
    /// form field key (`q0`), `value` the chosen option's **value** (which
    /// may differ from its label).
    pub fn reply_form(
        &self,
        session: &str,
        form_id: &str,
        field_key: &str,
        value: &str,
    ) -> Result<Value, ClientError> {
        let mut answers = serde_json::Map::new();
        answers.insert(field_key.to_owned(), serde_json::json!(value));
        self.post(
            &format!("/api/session/{session}/form/{form_id}/reply"),
            &serde_json::json!({ "answer": answers }),
        )
    }

    pub fn list_commands(&self) -> Result<Value, ClientError> {
        self.get("/api/command")
    }

    /// Phase 7C wrappers: thin, verified contracts only (see
    /// `research/memory/phase7c-r5-probes.md`). No invented routes.
    /// Admit a compaction: `POST …/compact {}`. Async — the `compaction`
    /// message + `session.compaction.*` events carry the outcome; a 200
    /// here is admission, not completion.
    pub fn compact_session(&self, session: &str) -> Result<Value, ClientError> {
        self.post(
            &format!("/api/session/{session}/compact"),
            &serde_json::json!({}),
        )
    }

    /// Read-only turn-scoped file snapshots. Empty → `{"data":[]}`;
    /// populated → per-file `{file, patch, additions, deletions, status}`.
    pub fn session_diff(&self, session: &str) -> Result<Value, ClientError> {
        self.get(&format!("/api/session/{session}/diff"))
    }

    /// Revert trio. Stage restores files immediately + records the
    /// boundary; commit deletes post-boundary messages (irreversible);
    /// delete abandons the staged boundary (may append `idle`).
    pub fn stage_revert(&self, session: &str, message_id: &str) -> Result<Value, ClientError> {
        self.post(
            &format!("/api/session/{session}/revert/stage"),
            &serde_json::json!({"messageID": message_id}),
        )
    }

    pub fn commit_revert(&self, session: &str) -> Result<Value, ClientError> {
        self.post(
            &format!("/api/session/{session}/revert/commit"),
            &serde_json::json!({}),
        )
    }

    pub fn delete_revert(&self, session: &str) -> Result<Value, ClientError> {
        self.request("DELETE", &format!("/api/session/{session}/revert"), None)
    }

    /// Fork: omit `before` for full history, or pass a message id for
    /// history-before. Returns the child session (new id).
    pub fn fork_session(&self, session: &str, before: Option<&str>) -> Result<Value, ClientError> {
        let body = match before {
            Some(message_id) => serde_json::json!({"before": message_id}),
            None => serde_json::json!({}),
        };
        self.post(&format!("/api/session/{session}/fork"), &body)
    }

    /// Model/agent discovery (client-side validation source — the switch
    /// routes blindly accept bogus values, so invalid selections must
    /// never be sent).
    pub fn list_models(&self) -> Result<Value, ClientError> {
        self.get("/api/model")
    }

    pub fn switch_model(
        &self,
        session: &str,
        model_id: &str,
        provider_id: &str,
    ) -> Result<Value, ClientError> {
        self.post(
            &format!("/api/session/{session}/model"),
            &serde_json::json!({"model": {"id": model_id, "providerID": provider_id}}),
        )
    }

    pub fn list_agents(&self) -> Result<Value, ClientError> {
        self.get("/api/agent")
    }

    pub fn switch_agent(&self, session: &str, agent: &str) -> Result<Value, ClientError> {
        self.post(
            &format!("/api/session/{session}/agent"),
            &serde_json::json!({"agent": agent}),
        )
    }

    /// Slash-command execution: 204 admits; the run materializes as a
    /// synthetic user message + normal agent turn (existing pump covers
    /// it — no separate event architecture).
    pub fn execute_command(
        &self,
        session: &str,
        name: &str,
        text: &str,
    ) -> Result<Value, ClientError> {
        self.post(
            &format!("/api/session/{session}/command"),
            &serde_json::json!({"name": name, "text": text}),
        )
    }

    /// Tolerant instruction-entry read (7C-R2): 200 → parsed value, 404 →
    /// empty (fork children with no entries 404 instead of 200 `[]`),
    /// other failures surface. PUT/DELETE semantics unchanged.
    pub fn try_get_instruction_entry(
        &self,
        session: &str,
        key: &str,
    ) -> Result<Option<Value>, ClientError> {
        match self.get_instruction_entry(session, key) {
            Ok(value) => Ok(Some(value)),
            Err(ClientError::Status(404, _)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn info(&self) -> Result<Value, ClientError> {
        self.get("/api/info")
    }

    pub fn delete_session(&self, session: &str) -> Result<Value, ClientError> {
        self.request("DELETE", &format!("/api/session/{session}"), None)
    }

    /// Thin instruction-entry wrappers (Phase 6B): the experimental entries
    /// surface verified live on OpenCode 2.0.8. `value` is any JSON; the
    /// memory path passes the parsed `{"text": block}` object.
    pub fn put_instruction_entry(
        &self,
        session: &str,
        key: &str,
        value: &Value,
    ) -> Result<Value, ClientError> {
        self.request(
            "PUT",
            &format!("/api/experimental/session/{session}/instructions/entries/{key}"),
            Some(&serde_json::json!({"value": value})),
        )
    }

    pub fn get_instruction_entry(&self, session: &str, key: &str) -> Result<Value, ClientError> {
        self.get(&format!(
            "/api/experimental/session/{session}/instructions/entries/{key}"
        ))
    }

    pub fn delete_instruction_entry(&self, session: &str, key: &str) -> Result<Value, ClientError> {
        self.request(
            "DELETE",
            &format!("/api/experimental/session/{session}/instructions/entries/{key}"),
            None,
        )
    }
}

/// Minimal base64 (RFC 4648, padded) for the Basic auth header — avoids a
/// dependency for twelve lines.
pub(crate) fn basic_auth(user: &str, password: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = format!("{user}:{password}").into_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut word = [0u8; 3];
        word[..chunk.len()].copy_from_slice(chunk);
        let bits = ((word[0] as u32) << 16) | ((word[1] as u32) << 8) | word[2] as u32;
        out.push(ALPHABET[(bits >> 18) as usize & 63] as char);
        out.push(ALPHABET[(bits >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(bits >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[bits as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{basic_auth, Client};
    use crate::backend::opencode::config::Endpoint;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn basic_header_matches_rfc_vector() {
        // "Aladdin:open sesame" is the RFC 7617 example vector.
        assert_eq!(
            basic_auth("Aladdin", "open sesame"),
            "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
        assert_eq!(basic_auth("opencode", "x"), "b3BlbmNvZGU6eA==");
    }

    /// One-shot stub server: serves one request (handler decides status +
    /// body from the raw request line/body), records the wire shape.
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
                    let body = String::from_utf8_lossy(&body).into_owned();
                    seen_thread
                        .lock()
                        .unwrap()
                        .push(format!("{} | {body}", request_line.trim_end()));
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

    #[test]
    fn reply_form_posts_dynamic_key_to_form_reply() {
        let stub = Stub::spawn((204, String::new()));
        let result = stub
            .client()
            .reply_form("ses_1", "frm_abc", "q0", "Dark")
            .expect("204 maps to Ok");
        // 204 No Content parses as JSON null; the envelope is tolerant.
        assert!(result.is_null() || result.is_object());
        let seen = stub.seen();
        assert_eq!(seen.len(), 1, "exactly one request");
        let request = &seen[0];
        assert!(request.starts_with("POST /api/session/ses_1/form/frm_abc/reply"));
        let body = request
            .split_once('|')
            .map(|(_, body)| body.trim())
            .unwrap_or("");
        let parsed: serde_json::Value = serde_json::from_str(body)
            .unwrap_or_else(|_| panic!("reply body must be JSON, got: {body}"));
        let answer = parsed.get("answer").and_then(|answer| answer.get("q0"));
        assert_eq!(answer, Some(&serde_json::json!("Dark")), "got: {request}");
    }

    #[test]
    fn reply_form_is_rejected_visibly() {
        let stub = Stub::spawn((400, r#"{"error":"FormInvalidAnswerError"}"#.to_owned()));
        let err = stub
            .client()
            .reply_form("ses_1", "frm_abc", "q0", "nope")
            .expect_err("400 surfaces as an error");
        assert!(err.to_string().contains("400"));
    }

    #[test]
    fn list_messages_paged_builds_cursor_query() {
        let stub = Stub::spawn((
            200,
            r#"{"data":[],"cursor":{"previous":null,"next":null}}"#.to_owned(),
        ));
        let _ = stub
            .client()
            .list_messages_paged("ses_1", Some("cur_9"), Some(50))
            .expect("parsed");
        let seen = stub.seen();
        assert_eq!(seen.len(), 1);
        let request = &seen[0];
        assert!(
            request.starts_with("GET /api/session/ses_1/message?limit=50&order=asc&cursor=cur_9"),
            "got: {request}"
        );
    }

    #[test]
    fn list_messages_paged_defaults_order_asc() {
        let stub = Stub::spawn((
            200,
            r#"{"data":[],"cursor":{"previous":null,"next":null}}"#.to_owned(),
        ));
        let _ = stub
            .client()
            .list_messages_paged("ses_1", None, None)
            .expect("parsed");
        let seen = stub.seen();
        assert_eq!(seen.len(), 1);
        let request = &seen[0];
        assert!(
            request.starts_with("GET /api/session/ses_1/message?limit=200&order=asc"),
            "got: {request}"
        );
    }

    fn body_of(request: &str) -> serde_json::Value {
        let body = request
            .split_once('|')
            .map(|(_, body)| body.trim())
            .unwrap_or("");
        serde_json::from_str(body)
            .unwrap_or_else(|_| panic!("request body must be JSON, got: {body}"))
    }

    #[test]
    fn compact_posts_empty_body() {
        let stub = Stub::spawn((200, r#"{"data":{"id":"ses_1"}}"#.to_owned()));
        stub.client().compact_session("ses_1").expect("admitted");
        let seen = stub.seen();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].starts_with("POST /api/session/ses_1/compact"),
            "got: {}",
            seen[0]
        );
        assert_eq!(body_of(&seen[0]), serde_json::json!({}));
    }

    #[test]
    fn diff_gets_session_diff() {
        let stub = Stub::spawn((200, r#"{"data":[]}"#.to_owned()));
        stub.client().session_diff("ses_1").expect("parsed");
        let seen = stub.seen();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].starts_with("GET /api/session/ses_1/diff"),
            "got: {}",
            seen[0]
        );
    }

    #[test]
    fn revert_trio_hits_verified_routes() {
        let stub = Stub::spawn((
            200,
            r#"{"data":{"messageID":"msg_1","files":[]}}"#.to_owned(),
        ));
        stub.client()
            .stage_revert("ses_1", "msg_1")
            .expect("staged");
        let seen = stub.seen();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].starts_with("POST /api/session/ses_1/revert/stage"),
            "got: {}",
            seen[0]
        );
        assert_eq!(body_of(&seen[0]), serde_json::json!({"messageID": "msg_1"}));

        let stub = Stub::spawn((204, String::new()));
        stub.client().commit_revert("ses_1").expect("committed");
        assert!(stub.seen()[0].starts_with("POST /api/session/ses_1/revert/commit"));

        let stub = Stub::spawn((204, String::new()));
        stub.client().delete_revert("ses_1").expect("abandoned");
        assert!(stub.seen()[0].starts_with("DELETE /api/session/ses_1/revert"));
    }

    #[test]
    fn revert_stage_bad_id_surfaces() {
        let stub = Stub::spawn((404, r#"{"_tag":"MessageNotFoundError"}"#.to_owned()));
        let err = stub
            .client()
            .stage_revert("ses_1", "msg_bogus")
            .expect_err("404 surfaces");
        assert!(err.to_string().contains("404"));
    }

    #[test]
    fn fork_posts_before_only_when_given() {
        let stub = Stub::spawn((200, r#"{"data":{"id":"ses_2"}}"#.to_owned()));
        stub.client().fork_session("ses_1", None).expect("forked");
        assert_eq!(body_of(&stub.seen()[0]), serde_json::json!({}));

        let stub = Stub::spawn((200, r#"{"data":{"id":"ses_3"}}"#.to_owned()));
        stub.client()
            .fork_session("ses_1", Some("msg_9"))
            .expect("forked");
        let seen = stub.seen();
        assert!(
            seen[0].starts_with("POST /api/session/ses_1/fork"),
            "got: {}",
            seen[0]
        );
        assert_eq!(body_of(&seen[0]), serde_json::json!({"before": "msg_9"}));
    }

    #[test]
    fn model_switch_posts_verified_shape() {
        let stub = Stub::spawn((204, String::new()));
        stub.client()
            .switch_model("ses_1", "mimo-v2.5-free", "opencode")
            .expect("switched");
        let seen = stub.seen();
        assert!(
            seen[0].starts_with("POST /api/session/ses_1/model"),
            "got: {}",
            seen[0]
        );
        assert_eq!(
            body_of(&seen[0]),
            serde_json::json!({"model": {"id": "mimo-v2.5-free", "providerID": "opencode"}})
        );

        let stub = Stub::spawn((200, r#"{"data":[]}"#.to_owned()));
        stub.client().list_models().expect("listed");
        assert!(
            stub.seen()[0].starts_with("GET /api/model"),
            "got: {}",
            stub.seen()[0]
        );
    }

    #[test]
    fn agent_switch_posts_verified_shape() {
        let stub = Stub::spawn((204, String::new()));
        stub.client()
            .switch_agent("ses_1", "general")
            .expect("switched");
        let seen = stub.seen();
        assert!(
            seen[0].starts_with("POST /api/session/ses_1/agent"),
            "got: {}",
            seen[0]
        );
        assert_eq!(body_of(&seen[0]), serde_json::json!({"agent": "general"}));

        let stub = Stub::spawn((200, r#"{"data":[]}"#.to_owned()));
        stub.client().list_agents().expect("listed");
        assert!(
            stub.seen()[0].starts_with("GET /api/agent"),
            "got: {}",
            stub.seen()[0]
        );
    }

    #[test]
    fn command_exec_posts_name_and_text() {
        let stub = Stub::spawn((204, String::new()));
        stub.client()
            .execute_command("ses_1", "review", "")
            .expect("executed");
        let seen = stub.seen();
        assert!(
            seen[0].starts_with("POST /api/session/ses_1/command"),
            "got: {}",
            seen[0]
        );
        assert_eq!(
            body_of(&seen[0]),
            serde_json::json!({"name": "review", "text": ""})
        );
    }

    #[test]
    fn instruction_get_404_means_empty() {
        let stub = Stub::spawn((404, "not found".to_owned()));
        assert_eq!(
            stub.client()
                .try_get_instruction_entry("ses_1", "owt.memory")
                .expect("tolerant"),
            None
        );
        let stub = Stub::spawn((200, r#"{"data":{"value":{"text":"hi"}}}"#.to_owned()));
        assert!(stub
            .client()
            .try_get_instruction_entry("ses_1", "owt.memory")
            .expect("parsed")
            .is_some());
    }
}
