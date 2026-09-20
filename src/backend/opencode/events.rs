//! SSE subscriber: one blocking thread per connection, plain line parsing.
//!
//! The worker owns no UI state. It forwards raw envelopes to a channel; the
//! adapter translates them to `StreamEvent`s on poll. Reconnects with
//! backoff; subscriptions are live-only (no replay), so state is re-based
//! from REST after a reconnect.

use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::Value;

/// One parsed SSE envelope: `{"id","type","data",...}`.
#[derive(Clone, Debug)]
pub struct RawEvent {
    pub typ: String,
    pub data: Value,
}

/// Parse one SSE `data:` payload into an envelope. Heartbeats, blanks and
/// malformed payloads yield `None` (counted by the caller when batching).
pub fn parse_envelope(payload: &str) -> Option<RawEvent> {
    let value: Value = serde_json::from_str(payload).ok()?;
    if !value.is_object() {
        return None;
    }
    let typ = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let data = value.get("data").cloned().unwrap_or(Value::Null);
    Some(RawEvent { typ, data })
}

/// Parse `data:` lines of an SSE stream into envelopes. Heartbeats (`:`) and
/// blank lines are skipped; malformed payloads are dropped with a count.
pub fn parse_sse_chunked(lines: impl Iterator<Item = String>) -> (Vec<RawEvent>, usize) {
    let mut events = Vec::new();
    let mut dropped = 0;
    let mut data = String::new();
    let flush = |data: &mut String, events: &mut Vec<RawEvent>, dropped: &mut usize| {
        if data.is_empty() {
            return;
        }
        let payload: String = std::mem::take(data);
        match parse_envelope(&payload) {
            Some(event) => events.push(event),
            None => *dropped += 1,
        }
    };
    for line in lines {
        if line.is_empty() || line.starts_with(':') {
            flush(&mut data, &mut events, &mut dropped);
        } else if let Some(payload) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(payload.trim_start());
        } else {
            // `event:`/`id:`/`retry:` lines: terminate the current payload.
            flush(&mut data, &mut events, &mut dropped);
        }
    }
    flush(&mut data, &mut events, &mut dropped);
    (events, dropped)
}

pub struct SseWorker {
    handle: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl SseWorker {
    /// Spawn a subscriber thread. `open` builds a fresh line reader per
    /// attempt (reconnects call it again); returning `None` backs off.
    pub fn spawn<F>(mut open: F, sender: std::sync::mpsc::Sender<RawEvent>) -> Self
    where
        F: FnMut() -> Option<Box<dyn BufRead + Send>> + Send + 'static,
    {
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = shutdown.clone();
        let handle = std::thread::Builder::new()
            .name("owt-opencode-sse".into())
            .spawn(move || {
                let mut backoff = Duration::from_secs(1);
                // Track the live feed: emit a synthetic `connection.lost` when
                // the stream ends (EOF/error) and `connection.restored` once a
                // reconnect succeeds. The adapter reconciles stale busy flags
                // and surfaces the gap in-band (see `mod.rs` ingest).
                let mut lost_announced = false;
                while !flag.load(Ordering::Relaxed) {
                    let Some(reader) = open() else {
                        // Reconnect attempt not yet accepted: just back off.
                        // (A loss is only announced when the stream ends, so
                        // this path never double-announces.)
                        std::thread::sleep(backoff);
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                        continue;
                    };
                    if lost_announced {
                        if send_connection_event(&sender, "connection.restored") {
                            return;
                        }
                        lost_announced = false;
                    }
                    backoff = Duration::from_secs(1);
                    // Buffer data lines and flush whole payloads through the
                    // shared batch parser (single code path with its tests).
                    let mut pending: Vec<String> = Vec::new();
                    let mut stopped = false;
                    for line in reader.lines() {
                        if flag.load(Ordering::Relaxed) {
                            stopped = true;
                            break;
                        }
                        let Ok(line) = line else { break };
                        if line.is_empty() || line.starts_with(':') {
                            let (events, _) =
                                parse_sse_chunked(std::mem::take(&mut pending).into_iter());
                            for event in events {
                                if sender.send(event).is_err() {
                                    stopped = true;
                                    break;
                                }
                            }
                            if stopped {
                                break;
                            }
                        } else {
                            pending.push(line);
                        }
                    }
                    if stopped {
                        return;
                    }
                    // Stream ended (server hiccup or disconnect): announce the
                    // drop once, then reconnect with backoff.
                    if !lost_announced {
                        if send_connection_event(&sender, "connection.lost") {
                            return;
                        }
                        lost_announced = true;
                    }
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            })
            .ok();
        Self { handle, shutdown }
    }
}

/// Send a synthetic connection-lifecycle event (no session attribution).
/// Returns `true` when the consumer is gone and the worker should stop.
fn send_connection_event(sender: &std::sync::mpsc::Sender<RawEvent>, typ: &str) -> bool {
    let event = RawEvent {
        typ: typ.to_owned(),
        data: Value::Null,
    };
    sender.send(event).is_err()
}

impl Drop for SseWorker {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{parse_sse_chunked, SseWorker};

    #[test]
    fn parses_envelopes_skips_heartbeats() {
        let lines = vec![
            ": heartbeat".to_owned(),
            r#"data: {"id":"evt_1","type":"session.created","data":{"sessionID":"ses_1"}}"#
                .to_owned(),
            "".to_owned(),
            "not-an-event".to_owned(),
            r#"data: not json"#.to_owned(),
            "".to_owned(),
        ];
        let (events, dropped) = parse_sse_chunked(lines.into_iter());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].typ, "session.created");
        assert_eq!(dropped, 1);
    }

    #[test]
    fn unknown_types_survive_for_mapper() {
        let lines = vec![
            r#"data: {"type":"future.thing","data":{}}"#.to_owned(),
            "".to_owned(),
        ];
        let (events, dropped) = parse_sse_chunked(lines.into_iter());
        assert_eq!(events.len(), 1);
        assert_eq!(dropped, 0);
    }

    #[test]
    fn worker_delivers_and_joins_on_drop() {
        use std::io::BufRead;
        let body = ": heartbeat\n\ndata: {\"id\":\"evt_1\",\"type\":\"session.created\",\"data\":{\"sessionID\":\"ses_1\"}}\n\n";
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = SseWorker::spawn(
            move || {
                let cursor: Box<dyn BufRead + Send> =
                    Box::new(std::io::Cursor::new(body.as_bytes().to_owned()));
                Some(cursor)
            },
            sender,
        );
        let event = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("worker forwards the envelope");
        assert_eq!(event.typ, "session.created");
        drop(worker);
    }
}
