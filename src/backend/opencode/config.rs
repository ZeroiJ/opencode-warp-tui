//! OpenCode endpoint configuration: explicit override → managed-service
//! discovery → private child server. No secrets in source; the password only
//! ever lives in memory, read from discovery or the environment.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Environment overrides (adapter-defined; the TUI never reads these).
pub const ENV_SERVER_URL: &str = "OPENCODE_SERVER_URL";
pub const ENV_SERVER_PASSWORD: &str = "OPENCODE_SERVER_PASSWORD";
pub const ENV_OPENCODE_BIN: &str = "OPENCODE_BIN";

/// Where OpenCode records its managed background service.
fn service_registration_path() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| {
                let mut path = PathBuf::from(home);
                path.push(".local/state");
                path
            })
        })
        .map(|mut path| {
            path.push("opencode/service.json");
            path
        })
}

/// A reachable OpenCode server. Password stays in memory only.
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub url: String,
    pub password: Option<String>,
}

/// How the endpoint was obtained (for status reporting, never secrets).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointSource {
    Explicit,
    Discovered,
    Spawned,
}

#[derive(Debug)]
pub enum ConfigError {
    NoBinary(String),
    NoPort,
    SpawnFailed(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::NoBinary(b) => write!(f, "opencode binary not found ({b})"),
            ConfigError::NoPort => write!(f, "could not pick a free port"),
            ConfigError::SpawnFailed(e) => write!(f, "could not start opencode server: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Adapter configuration, resolved in `connect()`.
#[derive(Clone, Debug, Default)]
pub struct OpenCodeConfig {
    /// Explicit server URL (`` env). Skips discovery + spawn.
    pub server_url: Option<String>,
    /// Password for an explicitly configured server (`` env).
    pub password: Option<String>,
    /// Project directory the private server works in (defaults to cwd).
    pub project_dir: Option<PathBuf>,
    /// `opencode` binary override (`` env, else `PATH` lookup).
    pub binary: Option<String>,
}

impl OpenCodeConfig {
    pub fn from_env() -> Self {
        Self {
            server_url: std::env::var(ENV_SERVER_URL).ok(),
            password: std::env::var(ENV_SERVER_PASSWORD).ok(),
            project_dir: None,
            binary: std::env::var(ENV_OPENCODE_BIN).ok(),
        }
    }

    fn find_binary(&self) -> Result<PathBuf, ConfigError> {
        if let Some(binary) = &self.binary {
            let path = PathBuf::from(binary);
            if path.exists() {
                return Ok(path);
            }
            return Err(ConfigError::NoBinary(binary.clone()));
        }
        std::env::var_os("PATH")
            .unwrap_or_default()
            .as_os_str()
            .to_string_lossy()
            .split(':')
            .map(|dir| {
                let mut path = PathBuf::from(dir);
                path.push("opencode");
                path
            })
            .find(|path| path.exists())
            .ok_or_else(|| ConfigError::NoBinary("opencode not on PATH".into()))
    }
}

/// A privately spawned server, killed on drop. Never touches the managed
/// `service.json` registration (that belongs to OpenCode's own service).
pub struct PrivateServer {
    child: Child,
    pub endpoint: Endpoint,
}

impl PrivateServer {
    pub fn spawn(config: &OpenCodeConfig) -> Result<Self, ConfigError> {
        let binary = config.find_binary()?;
        let port = free_port()?;
        let mut command = Command::new(&binary);
        command
            .arg("serve")
            .arg("--port")
            .arg(port.to_string())
            .arg("--hostname")
            .arg("127.0.0.1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = config
            .project_dir
            .clone()
            .or_else(|| std::env::current_dir().ok())
        {
            if dir.exists() {
                command.current_dir(dir);
            }
        }
        let mut child = command
            .spawn()
            .map_err(|e| ConfigError::SpawnFailed(e.to_string()))?;
        // The server prints `listening on <url>` once ready.
        let stdout = child.stdout.take();
        let mut url = format!("http://127.0.0.1:{port}");
        if let Some(stdout) = stdout {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while std::time::Instant::now() < deadline {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if let Some(found) = line.split_whitespace().find(|word| {
                            word.starts_with("http://127.0.0.1:")
                                || word.starts_with("http://localhost:")
                        }) {
                            url = found.trim_end_matches([')', '.', ',']).to_owned();
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        Ok(Self {
            child,
            endpoint: Endpoint {
                url,
                password: None,
            },
        })
    }
}

impl Drop for PrivateServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> Result<u16, ConfigError> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|_| ConfigError::NoPort)?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|_| ConfigError::NoPort)
}

/// Read a managed-service registration without claiming it.
pub fn read_service_registration() -> Option<Endpoint> {
    let path = service_registration_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let url = value.get("url")?.as_str()?.to_owned();
    let password = value
        .get("password")?
        .as_str()
        .filter(|password| !password.is_empty())
        .map(str::to_owned);
    Some(Endpoint { url, password })
}
