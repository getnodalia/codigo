// SPDX-License-Identifier: MIT OR Apache-2.0
//! A Debug Adapter Protocol client over any pair of byte streams.
//!
//! One reader thread parses what the adapter sends. Responses go to whoever asked, events are
//! handed to a callback, and the adapter's own requests are refused, since Código does not
//! advertise any of the optional ones.

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc, Mutex, MutexGuard, PoisonError,
};
use std::time::Duration;

use codigo_lsp::framing::{read_message, write_message, FrameError};
use serde_json::{json, Value};

type Responses = HashMap<i64, Sender<Result<Value, String>>>;
type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// What the adapter says without being asked.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    Event {
        name: String,
        body: Value,
    },
    /// The connection ended. Nothing follows.
    Exited,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("could not talk to the debug adapter: {0}")]
    Io(#[from] std::io::Error),
    #[error("the debug adapter did not answer in time")]
    Timeout,
    #[error("{0}")]
    Refused(String),
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An answer that has not arrived yet.
pub struct Pending {
    rx: Receiver<Result<Value, String>>,
}

impl Pending {
    /// The `body` of the response, or why the adapter refused.
    pub fn wait(&self, timeout: Duration) -> Result<Value, ClientError> {
        match self.rx.recv_timeout(timeout) {
            Ok(r) => r.map_err(ClientError::Refused),
            Err(RecvTimeoutError::Timeout) => Err(ClientError::Timeout),
            Err(RecvTimeoutError::Disconnected) => Err(ClientError::Refused(
                "the debug adapter closed the connection".into(),
            )),
        }
    }
}

pub struct Client {
    writer: SharedWriter,
    seq: AtomicI64,
    pending: Arc<Mutex<Responses>>,
}

impl Client {
    /// Start reading from `reader` on its own thread; `on_incoming` runs there for every event.
    pub fn new<R, W, F>(reader: R, writer: W, on_incoming: F) -> Client
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
        F: Fn(Incoming) + Send + 'static,
    {
        let writer: SharedWriter = Arc::new(Mutex::new(Box::new(writer)));
        let pending: Arc<Mutex<Responses>> = Arc::new(Mutex::new(HashMap::new()));
        let (w, p) = (writer.clone(), pending.clone());
        let spawned = std::thread::Builder::new()
            .name("codigo-dap-reader".into())
            .spawn(move || read_loop(BufReader::new(reader), w, p, on_incoming));
        if let Err(e) = spawned {
            tracing::warn!("could not start the debug adapter reader: {e}");
        }
        Client {
            writer,
            seq: AtomicI64::new(1),
            pending,
        }
    }

    pub fn request(&self, command: &str, arguments: Value) -> Pending {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        lock(&self.pending).insert(seq, tx);
        let message =
            json!({"seq": seq, "type": "request", "command": command, "arguments": arguments});
        if let Err(e) = write_message(&mut **lock(&self.writer), &message) {
            if let Some(tx) = lock(&self.pending).remove(&seq) {
                let _ = tx.send(Err(format!("could not send {command}: {e}")));
            }
        }
        Pending { rx }
    }
}

fn read_loop<R: Read, F: Fn(Incoming)>(
    mut reader: BufReader<R>,
    writer: SharedWriter,
    pending: Arc<Mutex<Responses>>,
    on_incoming: F,
) {
    loop {
        match read_message(&mut reader) {
            Ok(Some(message)) => dispatch(&message, &writer, &pending, &on_incoming),
            Ok(None) => tracing::debug!("skipped a message that was not JSON"),
            Err(FrameError::Closed) => break,
            Err(e) => {
                tracing::warn!("debug adapter stream broke: {e}");
                break;
            }
        }
    }
    for (_, tx) in lock(&pending).drain() {
        let _ = tx.send(Err("the debug adapter closed the connection".into()));
    }
    on_incoming(Incoming::Exited);
}

fn dispatch<F: Fn(Incoming)>(
    message: &Value,
    writer: &SharedWriter,
    pending: &Mutex<Responses>,
    on_incoming: &F,
) {
    match message.get("type").and_then(Value::as_str) {
        Some("response") => {
            let Some(seq) = message.get("request_seq").and_then(Value::as_i64) else {
                return;
            };
            let Some(tx) = lock(pending).remove(&seq) else {
                return;
            };
            let ok = message
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let result = if ok {
                Ok(message.get("body").cloned().unwrap_or(Value::Null))
            } else {
                Err(message
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("the debug adapter refused")
                    .to_owned())
            };
            let _ = tx.send(result);
        }
        Some("event") => {
            if let Some(name) = message.get("event").and_then(Value::as_str) {
                on_incoming(Incoming::Event {
                    name: name.to_owned(),
                    body: message.get("body").cloned().unwrap_or(Value::Null),
                });
            }
        }
        Some("request") => {
            let reply = json!({
                "seq": 0, "type": "response",
                "request_seq": message.get("seq").cloned().unwrap_or(Value::Null),
                "success": false,
                "command": message.get("command").cloned().unwrap_or(Value::Null),
                "message": "not supported"
            });
            let _ = write_message(&mut **lock(writer), &reply);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    /// A pipe: what one end writes the other reads.
    fn pipe() -> (impl Write + Send, impl Read + Send) {
        struct W(Sender<Vec<u8>>);
        impl Write for W {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                let _ = self.0.send(buf.to_vec());
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        struct R(Receiver<Vec<u8>>, Vec<u8>);
        impl Read for R {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                if self.1.is_empty() {
                    match self.0.recv() {
                        Ok(b) => self.1 = b,
                        Err(_) => return Ok(0),
                    }
                }
                let n = out.len().min(self.1.len());
                out[..n].copy_from_slice(&self.1[..n]);
                self.1.drain(..n);
                Ok(n)
            }
        }
        let (tx, rx) = channel();
        (W(tx), R(rx, Vec::new()))
    }

    #[test]
    fn answers_events_and_the_adapters_own_requests_are_routed() {
        // client -> adapter and adapter -> client
        let (to_adapter, adapter_in) = pipe();
        let (from_adapter, client_in) = pipe();
        let (events_tx, events) = channel();
        let client = Client::new(client_in, to_adapter, move |i| {
            let _ = events_tx.send(i);
        });

        // A fake adapter on a thread.
        std::thread::spawn(move || {
            let mut r = BufReader::new(adapter_in);
            let mut w = from_adapter;
            while let Ok(Some(m)) = read_message(&mut r) {
                let seq = m["seq"].clone();
                match m["command"].as_str() {
                    Some("initialize") => {
                        let _ =
                            write_message(&mut w, &json!({"type":"event","event":"initialized"}));
                        let _ = write_message(
                            &mut w,
                            &json!({"type":"request","seq":7,"command":"runInTerminal"}),
                        );
                        let _ = write_message(
                            &mut w,
                            &json!({"type":"response","request_seq":seq,"success":true,"body":{"supportsConfigurationDoneRequest":true}}),
                        );
                    }
                    _ => {
                        let _ = write_message(
                            &mut w,
                            &json!({"type":"response","request_seq":seq,"success":false,"message":"nope"}),
                        );
                    }
                }
            }
        });

        let body = client
            .request("initialize", json!({}))
            .wait(Duration::from_secs(5))
            .expect("answer");
        assert_eq!(body["supportsConfigurationDoneRequest"], true);
        assert_eq!(
            events.recv_timeout(Duration::from_secs(5)).expect("event"),
            Incoming::Event {
                name: "initialized".into(),
                body: Value::Null
            }
        );
        let err = client
            .request("launch", json!({}))
            .wait(Duration::from_secs(5))
            .expect_err("refused");
        assert!(matches!(err, ClientError::Refused(m) if m == "nope"));
    }

    #[test]
    fn a_closed_connection_fails_waiting_requests_and_reports_exit() {
        let (to_adapter, _adapter_in) = pipe();
        let (from_adapter, client_in) = pipe();
        let (events_tx, events) = channel();
        let client = Client::new(client_in, to_adapter, move |i| {
            let _ = events_tx.send(i);
        });
        let pending = client.request("launch", json!({}));
        drop(from_adapter);
        assert!(pending.wait(Duration::from_secs(5)).is_err());
        assert_eq!(
            events.recv_timeout(Duration::from_secs(5)).expect("exit"),
            Incoming::Exited
        );
    }
}
