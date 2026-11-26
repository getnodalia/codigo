// SPDX-License-Identifier: MIT OR Apache-2.0
//! A language server client over any pair of byte streams.
//!
//! One reader thread parses what the server sends. Responses go to whoever asked, notifications
//! become [`Event`]s, and the server's own requests (it asks for configuration, wants to show
//! progress) are answered here so that no server stalls waiting for the editor.

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc, Mutex, MutexGuard, PoisonError,
};
use std::time::Duration;

use serde_json::{json, Value};

use crate::diagnostics::{parse_list, Diagnostic};
use crate::framing::{read_message, write_message, FrameError};

type Responses = HashMap<i64, Sender<Result<Value, ResponseError>>>;

/// A server's error reply, or the connection ending before it answered.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{message} (code {code})")]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("could not talk to the server: {0}")]
    Io(#[from] std::io::Error),
    #[error("the server did not answer in time")]
    Timeout,
    #[error("the server refused: {0}")]
    Server(#[from] ResponseError),
}

/// What the server tells the editor without being asked.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The complete list for one file; an empty list clears it.
    Diagnostics {
        uri: String,
        diagnostics: Vec<Diagnostic>,
    },
    /// `window/logMessage` and `window/showMessage`.
    Message { level: u8, text: String },
    /// A `$/progress` report, reduced to the line worth showing.
    Progress { text: String },
    /// Any other notification, untouched.
    Notification { method: String, params: Value },
    /// The connection ended. Nothing follows.
    Exited,
}

/// An answer that has not arrived yet. Poll it from the frame loop.
pub struct Pending {
    rx: Receiver<Result<Value, ResponseError>>,
}

impl Pending {
    /// `None` while the server is still working.
    pub fn poll(&self) -> Option<Result<Value, ResponseError>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(closed())),
        }
    }

    pub fn wait(&self, timeout: Duration) -> Result<Value, ClientError> {
        match self.rx.recv_timeout(timeout) {
            Ok(r) => Ok(r?),
            Err(RecvTimeoutError::Timeout) => Err(ClientError::Timeout),
            Err(RecvTimeoutError::Disconnected) => Err(closed().into()),
        }
    }
}

fn closed() -> ResponseError {
    ResponseError {
        code: -32099,
        message: "the server closed the connection".into(),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

pub struct Client {
    writer: SharedWriter,
    next_id: AtomicI64,
    pending: Arc<Mutex<Responses>>,
    events: Receiver<Event>,
}

impl Client {
    /// Start reading from `reader` on its own thread. Messages are written to `writer`.
    pub fn new<R, W>(reader: R, writer: W) -> Client
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        let writer: SharedWriter = Arc::new(Mutex::new(Box::new(writer)));
        let pending: Arc<Mutex<Responses>> = Arc::new(Mutex::new(HashMap::new()));
        let (tx, events) = mpsc::sync_channel(4096);
        let (w, p) = (writer.clone(), pending.clone());
        let spawned = std::thread::Builder::new()
            .name("codigo-lsp-reader".into())
            .spawn(move || read_loop(BufReader::new(reader), w, p, tx));
        if let Err(e) = spawned {
            tracing::warn!("could not start the language server reader: {e}");
        }
        Client {
            writer,
            next_id: AtomicI64::new(1),
            pending,
            events,
        }
    }

    /// What the server has said since the last call. Never blocks.
    pub fn events(&self) -> &Receiver<Event> {
        &self.events
    }

    fn send(&self, message: &Value) -> std::io::Result<()> {
        write_message(&mut **lock(&self.writer), message)
    }

    pub fn request(&self, method: &str, params: Value) -> Pending {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        lock(&self.pending).insert(id, tx);
        let sent =
            self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        if let Err(e) = sent {
            if let Some(tx) = lock(&self.pending).remove(&id) {
                let _ = tx.send(Err(ResponseError {
                    code: -32099,
                    message: format!("could not send: {e}"),
                }));
            }
        }
        Pending { rx }
    }

    pub fn notify(&self, method: &str, params: Value) -> std::io::Result<()> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    /// The handshake. Returns the server's capabilities.
    pub fn initialize(
        &self,
        root_uri: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, ClientError> {
        let folders = root_uri
            .map(|u| json!([{"uri": u, "name": "workspace"}]))
            .unwrap_or(Value::Null);
        let params = json!({
            "processId": std::process::id(),
            "clientInfo": {"name": "Código"},
            "rootUri": root_uri,
            "workspaceFolders": folders,
            "capabilities": client_capabilities(),
        });
        let result = self.request("initialize", params).wait(timeout)?;
        self.notify("initialized", json!({}))?;
        Ok(result.get("capabilities").cloned().unwrap_or(Value::Null))
    }

    pub fn did_open(
        &self,
        uri: &str,
        language_id: &str,
        version: i32,
        text: &str,
    ) -> std::io::Result<()> {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": uri, "languageId": language_id, "version": version, "text": text}}),
        )
    }

    /// Whole-document sync: every server supports it, and it cannot drift out of step.
    pub fn did_change(&self, uri: &str, version: i32, text: &str) -> std::io::Result<()> {
        self.notify(
            "textDocument/didChange",
            json!({"textDocument": {"uri": uri, "version": version}, "contentChanges": [{"text": text}]}),
        )
    }

    pub fn did_save(&self, uri: &str, text: Option<&str>) -> std::io::Result<()> {
        let mut params = json!({"textDocument": {"uri": uri}});
        if let (Some(text), Some(obj)) = (text, params.as_object_mut()) {
            obj.insert("text".into(), json!(text));
        }
        self.notify("textDocument/didSave", params)
    }

    pub fn did_close(&self, uri: &str) -> std::io::Result<()> {
        self.notify(
            "textDocument/didClose",
            json!({"textDocument": {"uri": uri}}),
        )
    }

    /// Ask the server to stop, then tell it to exit. Best effort within `timeout`.
    pub fn shutdown(&self, timeout: Duration) {
        let _ = self.request("shutdown", Value::Null).wait(timeout);
        let _ = self.notify("exit", Value::Null);
    }
}

fn client_capabilities() -> Value {
    json!({
        "general": {"positionEncodings": ["utf-16"]},
        "workspace": {"workspaceFolders": true, "configuration": true},
        "window": {"workDoneProgress": true},
        "textDocument": {
            "synchronization": {"didSave": true, "dynamicRegistration": false},
            "publishDiagnostics": {"relatedInformation": false},
            "hover": {"contentFormat": ["markdown", "plaintext"]},
            "completion": {"completionItem": {"snippetSupport": false, "documentationFormat": ["markdown", "plaintext"]}},
            "definition": {"linkSupport": false},
            "references": {},
            "rename": {"prepareSupport": false},
            "formatting": {}
        }
    })
}

fn read_loop<R: Read>(
    mut reader: BufReader<R>,
    writer: SharedWriter,
    pending: Arc<Mutex<Responses>>,
    events: SyncSender<Event>,
) {
    loop {
        match read_message(&mut reader) {
            Ok(Some(message)) => dispatch(message, &writer, &pending, &events),
            Ok(None) => tracing::debug!("skipped a message that was not JSON"),
            Err(FrameError::Closed) => break,
            Err(e) => {
                tracing::warn!("language server stream broke: {e}");
                break;
            }
        }
    }
    for (_, tx) in lock(&pending).drain() {
        let _ = tx.send(Err(closed()));
    }
    let _ = events.send(Event::Exited);
}

fn dispatch(
    message: Value,
    writer: &SharedWriter,
    pending: &Mutex<Responses>,
    events: &SyncSender<Event>,
) {
    let method = message.get("method").and_then(Value::as_str);
    let id = message.get("id");
    match (method, id) {
        (Some(method), Some(id)) => {
            let reply = server_request_reply(method, message.get("params"));
            let out = match reply {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err(()) => json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": format!("method not supported: {method}")}
                }),
            };
            let _ = write_message(&mut **lock(writer), &out);
        }
        (Some(method), None) => {
            if let Some(event) = notification_event(method, message.get("params")) {
                match event {
                    // Logs and progress can be dropped under pressure; diagnostics cannot.
                    Event::Message { .. } | Event::Progress { .. } => {
                        let _ = events.try_send(event);
                    }
                    other => {
                        let _ = events.send(other);
                    }
                }
            }
        }
        (None, Some(id)) => {
            let Some(id) = id.as_i64() else { return };
            let Some(tx) = lock(pending).remove(&id) else {
                return;
            };
            let answer = match message.get("error") {
                Some(err) => Err(ResponseError {
                    code: err.get("code").and_then(Value::as_i64).unwrap_or(-32603),
                    message: err
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .chars()
                        .take(2000)
                        .collect(),
                }),
                None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = tx.send(answer);
        }
        (None, None) => {}
    }
}

/// What the editor answers when the server asks it something. `Err` means "not supported".
fn server_request_reply(method: &str, params: Option<&Value>) -> Result<Value, ()> {
    match method {
        "workspace/configuration" => {
            let n = params
                .and_then(|p| p.get("items"))
                .and_then(Value::as_array)
                .map_or(0, |a| a.len().min(1000));
            Ok(Value::Array(vec![Value::Null; n]))
        }
        "workspace/workspaceFolders" => Ok(Value::Null),
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create"
        | "window/showMessageRequest"
        | "workspace/diagnostic/refresh"
        | "workspace/semanticTokens/refresh"
        | "workspace/inlayHint/refresh" => Ok(Value::Null),
        "workspace/applyEdit" => Ok(json!({"applied": false})),
        _ => Err(()),
    }
}

fn notification_event(method: &str, params: Option<&Value>) -> Option<Event> {
    let params = params.cloned().unwrap_or(Value::Null);
    match method {
        "textDocument/publishDiagnostics" => Some(Event::Diagnostics {
            uri: params.get("uri")?.as_str()?.to_owned(),
            diagnostics: parse_list(params.get("diagnostics")?),
        }),
        "window/logMessage" | "window/showMessage" => Some(Event::Message {
            level: params
                .get("type")
                .and_then(Value::as_u64)
                .unwrap_or(4)
                .min(4) as u8,
            text: params
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .chars()
                .take(2000)
                .collect(),
        }),
        "$/progress" => {
            let value = params.get("value")?;
            let title = value.get("title").and_then(Value::as_str).unwrap_or("");
            let message = value.get("message").and_then(Value::as_str).unwrap_or("");
            let text = [title, message]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" · ");
            Some(Event::Progress { text })
        }
        "$/logTrace" | "$/cancelRequest" => None,
        _ => Some(Event::Notification {
            method: method.to_owned(),
            params,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader as BR;
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    /// A client wired to a loopback socket, and the server end of it.
    fn pair() -> (Client, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let client_end = TcpStream::connect(addr).expect("connect");
        let (server_end, _) = listener.accept().expect("accept");
        let reader = client_end.try_clone().expect("clone");
        (Client::new(reader, client_end), server_end)
    }

    fn reply(w: &mut TcpStream, v: Value) {
        write_message(w, &v).expect("write");
    }

    #[test]
    fn handshake_diagnostics_server_requests_and_shutdown() {
        let (client, server) = pair();
        let mut sw = server.try_clone().expect("clone");
        let handle = thread::spawn(move || {
            let mut r = BR::new(server);
            let init = read_message(&mut r).expect("init").expect("json");
            assert_eq!(init["method"], "initialize");
            assert_eq!(init["params"]["rootUri"], "file:///proj");
            reply(
                &mut sw,
                json!({"jsonrpc":"2.0","id":init["id"],"result":{"capabilities":{"hoverProvider":true}}}),
            );
            let initialized = read_message(&mut r).expect("i").expect("json");
            assert_eq!(initialized["method"], "initialized");
            let open = read_message(&mut r).expect("open").expect("json");
            assert_eq!(open["method"], "textDocument/didOpen");
            assert_eq!(open["params"]["textDocument"]["languageId"], "rust");
            // The server asks for configuration before it will report anything.
            reply(
                &mut sw,
                json!({"jsonrpc":"2.0","id":"cfg-1","method":"workspace/configuration","params":{"items":[{},{}]}}),
            );
            let answered = read_message(&mut r).expect("cfg").expect("json");
            assert_eq!(answered["id"], "cfg-1");
            assert_eq!(answered["result"], json!([null, null]));
            reply(
                &mut sw,
                json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{
                    "uri":"file:///proj/a.rs",
                    "diagnostics":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},
                                    "severity":1,"message":"nope"}]}}),
            );
            let shutdown = read_message(&mut r).expect("sd").expect("json");
            assert_eq!(shutdown["method"], "shutdown");
            reply(
                &mut sw,
                json!({"jsonrpc":"2.0","id":shutdown["id"],"result":null}),
            );
            let exit = read_message(&mut r).expect("exit").expect("json");
            assert_eq!(exit["method"], "exit");
        });

        let caps = client
            .initialize(Some("file:///proj"), Duration::from_secs(5))
            .expect("initialize");
        assert_eq!(caps["hoverProvider"], true);
        client
            .did_open("file:///proj/a.rs", "rust", 1, "fn x() {}")
            .expect("open");
        let event = client
            .events()
            .recv_timeout(Duration::from_secs(5))
            .expect("event");
        match event {
            Event::Diagnostics { uri, diagnostics } => {
                assert_eq!(uri, "file:///proj/a.rs");
                assert_eq!(diagnostics.len(), 1);
                assert_eq!(diagnostics[0].message, "nope");
            }
            other => panic!("unexpected {other:?}"),
        }
        client.shutdown(Duration::from_secs(5));
        handle.join().expect("server thread");
    }

    #[test]
    fn a_dead_server_fails_pending_requests_and_reports_exit() {
        let (client, server) = pair();
        let pending = client.request("textDocument/hover", json!({}));
        drop(server);
        let err = pending.wait(Duration::from_secs(5));
        assert!(matches!(err, Err(ClientError::Server(_))), "{err:?}");
        let mut saw_exit = false;
        while let Ok(e) = client.events().recv_timeout(Duration::from_secs(5)) {
            if e == Event::Exited {
                saw_exit = true;
                break;
            }
        }
        assert!(saw_exit);
    }

    #[test]
    fn error_replies_and_junk_do_not_break_the_stream() {
        let (client, mut server) = pair();
        let pending = client.request("textDocument/definition", json!({}));
        // Not JSON, then a reply for an id nobody asked, then the real error reply.
        server
            .write_all(b"Content-Length: 3\r\n\r\n{{{")
            .expect("junk");
        reply(&mut server, json!({"jsonrpc":"2.0","id":9999,"result":1}));
        reply(
            &mut server,
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-32801,"message":"content modified"}}),
        );
        match pending.wait(Duration::from_secs(5)) {
            Err(ClientError::Server(e)) => assert_eq!(e.code, -32801),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn server_requests_we_do_not_know_get_an_error_reply() {
        let (_client, server) = pair();
        let mut sw = server.try_clone().expect("clone");
        let mut r = BR::new(server);
        reply(
            &mut sw,
            json!({"jsonrpc":"2.0","id":7,"method":"custom/thing","params":{}}),
        );
        let answer = read_message(&mut r).expect("a").expect("json");
        assert_eq!(answer["id"], 7);
        assert_eq!(answer["error"]["code"], -32601);
    }

    #[test]
    fn progress_and_logs_become_events() {
        let e = notification_event(
            "$/progress",
            Some(
                &json!({"token": 1, "value": {"kind": "report", "title": "Indexing", "message": "3/10"}}),
            ),
        );
        assert_eq!(
            e,
            Some(Event::Progress {
                text: "Indexing · 3/10".into()
            })
        );
        let e = notification_event(
            "window/logMessage",
            Some(&json!({"type": 2, "message": "hi"})),
        );
        assert_eq!(
            e,
            Some(Event::Message {
                level: 2,
                text: "hi".into()
            })
        );
    }
}
