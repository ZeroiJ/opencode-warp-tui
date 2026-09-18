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

    pub fn list_messages(&self, session: &str) -> Result<Value, ClientError> {
        self.get(&format!("/api/session/{session}/message?limit=200"))
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

    pub fn list_commands(&self) -> Result<Value, ClientError> {
        self.get("/api/command")
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
    use super::basic_auth;

    #[test]
    fn basic_header_matches_rfc_vector() {
        // "Aladdin:open sesame" is the RFC 7617 example vector.
        assert_eq!(
            basic_auth("Aladdin", "open sesame"),
            "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
        assert_eq!(basic_auth("opencode", "x"), "b3BlbmNvZGU6eA==");
    }
}
