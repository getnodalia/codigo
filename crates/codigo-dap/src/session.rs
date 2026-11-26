// SPDX-License-Identifier: MIT OR Apache-2.0
//! A debugging session on its own thread.
//!
//! The window never waits on the adapter: it sends [`Command`]s through a [`Debugger`] and reads
//! the [`Update`]s that come back, once per frame. The thread owns the adapter process, walks the
//! protocol's start-up (initialize, launch, breakpoints, configuration done), and when the program
//! stops it reads the stack and the variables before telling the window, so an `Update::Stopped`
//! is always complete.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::adapters::Adapter;
use crate::client::{Client, ClientError, Incoming};

/// Called from the session's threads after an update was queued, so a window can repaint.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

const REPLY: Duration = Duration::from_secs(10);
const START: Duration = Duration::from_secs(30);
const MAX_FRAMES: usize = 64;
const MAX_VARIABLES: usize = 200;
const MAX_SCOPES: usize = 4;
const MAX_OUTPUT: usize = 64 * 1024;

/// What to debug.
#[derive(Clone, Debug)]
pub struct LaunchSpec {
    pub adapter: Adapter,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Breakpoints to set before the program starts: each file with its 1-based lines.
    pub breakpoints: BTreeMap<PathBuf, Vec<usize>>,
}

/// What the window asks of a running session.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Continue,
    StepOver,
    StepInto,
    StepOut,
    Pause,
    /// Replace the breakpoints of one file (1-based lines).
    SetBreakpoints(PathBuf, Vec<usize>),
    /// Show another frame of the stopped stack (an index into [`StopInfo::frames`]).
    SelectFrame(usize),
    /// Read the children of a variable.
    Expand(i64),
    /// Evaluate an expression in the selected frame.
    Evaluate(String),
    /// End the session and the program with it.
    Stop,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub id: i64,
    pub name: String,
    pub path: Option<PathBuf>,
    /// 1-based.
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub type_name: String,
    /// Not zero when the variable has children to [`Command::Expand`].
    pub reference: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScopeVars {
    pub name: String,
    pub vars: Vec<Variable>,
}

/// The program is stopped: where, and what is in scope there.
#[derive(Clone, Debug, PartialEq)]
pub struct StopInfo {
    pub reason: String,
    pub thread_id: i64,
    pub frames: Vec<Frame>,
    /// The frame the scopes belong to.
    pub selected: usize,
    pub scopes: Vec<ScopeVars>,
}

/// What a session tells the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// The adapter answered and the program is starting.
    Started,
    Running,
    Stopped(StopInfo),
    /// The frame selected by [`Command::SelectFrame`] and what is in scope there.
    Frame {
        selected: usize,
        scopes: Vec<ScopeVars>,
    },
    Children {
        reference: i64,
        vars: Vec<Variable>,
    },
    Evaluated {
        expression: String,
        result: Result<String, String>,
    },
    /// The adapter's answer to a file's breakpoints: each requested line, and whether it landed.
    Breakpoints {
        path: PathBuf,
        lines: Vec<(usize, bool)>,
    },
    /// Output of the program or of the debugger.
    Output {
        category: String,
        text: String,
    },
    /// The session is over. `error` is set when it ended because something went wrong.
    Ended {
        exit_code: Option<i64>,
        error: Option<String>,
    },
}

enum In {
    Cmd(Command),
    Dap(Incoming),
}

/// A session, from the window's side.
pub struct Debugger {
    commands: Sender<In>,
    updates: Receiver<Update>,
}

impl Debugger {
    /// Start the adapter and the program. Returns at once; everything that follows arrives as
    /// updates.
    pub fn start(spec: LaunchSpec, waker: Option<Waker>) -> Debugger {
        let (commands, inbox) = channel();
        let (out, updates) = channel();
        let tx = commands.clone();
        let spawned = std::thread::Builder::new()
            .name("codigo-debugger".into())
            .spawn(move || {
                let worker = Worker {
                    out,
                    waker,
                    inbox,
                    inbox_tx: tx,
                    thread: 0,
                    frames: Vec::new(),
                    selected: 0,
                    entry_stop: false,
                    output_bytes: 0,
                    ready: false,
                };
                worker.run(spec);
            });
        if spawned.is_err() {
            // The window will see the channel close; nothing else can be said without a thread.
            tracing::warn!("could not start the debugger thread");
        }
        Debugger { commands, updates }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(In::Cmd(command));
    }

    /// The next update, if there is one. Never blocks.
    pub fn poll(&self) -> Option<Update> {
        self.updates.try_recv().ok()
    }
}

struct Worker {
    out: Sender<Update>,
    waker: Option<Waker>,
    inbox: Receiver<In>,
    inbox_tx: Sender<In>,
    thread: i64,
    frames: Vec<Frame>,
    /// The frame the person selected in the stack.
    selected: usize,
    /// The adapter was asked to stop at the start of `main` so the breakpoints land before the
    /// program gets anywhere; that first stop is ours and is not shown.
    entry_stop: bool,
    output_bytes: usize,
    /// The adapter said `initialized`; what it printed before that is its own banner.
    ready: bool,
}

/// How the start-up ended.
enum Flow {
    Go,
    Over,
}

impl Worker {
    fn tell(&self, update: Update) {
        let _ = self.out.send(update);
        if let Some(w) = &self.waker {
            w();
        }
    }

    fn end(&self, exit_code: Option<i64>, error: Option<String>) {
        self.tell(Update::Ended { exit_code, error });
    }

    fn run(mut self, spec: LaunchSpec) {
        let mut child = match spawn(&spec.adapter) {
            Ok(c) => c,
            Err(e) => {
                self.end(None, Some(e));
                return;
            }
        };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            self.end(None, Some("could not talk to the debug adapter".into()));
            kill(&mut child);
            return;
        };
        let tx = self.inbox_tx.clone();
        let waker = self.waker.clone();
        let client = Client::new(stdout, stdin, move |incoming| {
            let _ = tx.send(In::Dap(incoming));
            if let Some(w) = &waker {
                w();
            }
        });
        let (code, error) = self.session(&client, &spec);
        // Be polite, then make sure the adapter and the program are gone.
        let _ = client
            .request("disconnect", json!({"terminateDebuggee": true}))
            .wait(Duration::from_secs(2));
        kill(&mut child);
        self.end(code, error);
    }

    /// The whole session. Returns the exit code of the program and the error, if any.
    fn session(&mut self, client: &Client, spec: &LaunchSpec) -> (Option<i64>, Option<String>) {
        let caps = match client
            .request(
                "initialize",
                json!({
                    "clientID": "codigo", "clientName": "Código", "adapterID": spec.adapter.name,
                    "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path",
                    "supportsVariableType": true, "supportsRunInTerminalRequest": false
                }),
            )
            .wait(REPLY)
        {
            Ok(c) => c,
            Err(e) => return (None, Some(explain(&e, spec))),
        };
        let launch = client.request(
            "launch",
            spec.adapter
                .launch_arguments(&spec.program, &spec.args, &spec.cwd),
        );
        let pipelined = spec.adapter.kind.pipelined();
        self.entry_stop = pipelined;
        // Breakpoints are only accepted after `initialized`, except where the adapter starts the
        // program at once.
        if !pipelined {
            match self.wait_for_initialized() {
                Ok(Flow::Go) => {}
                Ok(Flow::Over) => return (None, None),
                Err(e) => return (None, Some(e)),
            }
        }
        for (path, lines) in &spec.breakpoints {
            self.set_breakpoints(client, path, lines);
        }
        if caps
            .get("supportsConfigurationDoneRequest")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let done = client.request("configurationDone", json!({})).wait(REPLY);
            if let (Err(e), false) = (done, pipelined) {
                return (None, Some(explain(&e, spec)));
            }
        }
        if let Err(e) = launch.wait(START) {
            return (None, Some(explain(&e, spec)));
        }
        self.tell(Update::Started);
        self.events(client, spec)
    }

    fn wait_for_initialized(&mut self) -> Result<Flow, String> {
        let deadline = std::time::Instant::now() + START;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.inbox.recv_timeout(left) {
                Ok(In::Dap(Incoming::Event { name, .. })) if name == "initialized" => {
                    self.ready = true;
                    return Ok(Flow::Go);
                }
                Ok(In::Dap(Incoming::Event { name, body })) => self.relay_output(&name, &body),
                Ok(In::Dap(Incoming::Exited)) => {
                    return Err("the debug adapter stopped before the program started".into())
                }
                Ok(In::Cmd(Command::Stop)) => return Ok(Flow::Over),
                Ok(In::Cmd(_)) => {}
                Err(RecvTimeoutError::Timeout) => {
                    return Err("the debug adapter did not get ready in time".into())
                }
                Err(RecvTimeoutError::Disconnected) => return Ok(Flow::Over),
            }
        }
    }

    fn relay_output(&mut self, name: &str, body: &Value) {
        if name != "output" || !self.ready {
            return;
        }
        let category = body
            .get("category")
            .and_then(Value::as_str)
            .unwrap_or("console");
        let Some(text) = body.get("output").and_then(Value::as_str) else {
            return;
        };
        // A program that prints without end must not fill the memory.
        if self.output_bytes > MAX_OUTPUT * 64 {
            return;
        }
        self.output_bytes += text.len();
        let text: String = text.chars().take(MAX_OUTPUT).collect();
        self.tell(Update::Output {
            category: category.to_owned(),
            text,
        });
    }

    fn set_breakpoints(&mut self, client: &Client, path: &Path, lines: &[usize]) {
        let wanted: Vec<Value> = lines.iter().map(|l| json!({"line": l})).collect();
        let reply = client
            .request(
                "setBreakpoints",
                json!({
                    "source": {"path": path.to_string_lossy()},
                    "breakpoints": wanted, "sourceModified": false
                }),
            )
            .wait(REPLY);
        let verified: Vec<(usize, bool)> = match reply {
            Ok(body) => {
                let got = body
                    .get("breakpoints")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                lines
                    .iter()
                    .enumerate()
                    .map(|(i, line)| {
                        let b = got.get(i);
                        let ok = b
                            .and_then(|b| b.get("verified"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let at = b
                            .and_then(|b| b.get("line"))
                            .and_then(Value::as_u64)
                            .and_then(|l| usize::try_from(l).ok())
                            .unwrap_or(*line);
                        (at, ok)
                    })
                    .collect()
            }
            Err(_) => lines.iter().map(|l| (*l, false)).collect(),
        };
        self.tell(Update::Breakpoints {
            path: path.to_path_buf(),
            lines: verified,
        });
    }

    /// Everything after the program started, until it ends.
    fn events(&mut self, client: &Client, spec: &LaunchSpec) -> (Option<i64>, Option<String>) {
        let mut exit_code = None;
        loop {
            let Ok(message) = self.inbox.recv() else {
                return (exit_code, None);
            };
            match message {
                In::Dap(Incoming::Exited) => {
                    return (
                        exit_code,
                        exit_code.is_none().then(|| {
                            format!(
                                "the debug adapter ({}) stopped unexpectedly",
                                spec.adapter.name
                            )
                        }),
                    )
                }
                In::Dap(Incoming::Event { name, body }) => match name.as_str() {
                    "stopped" => {
                        if std::mem::take(&mut self.entry_stop)
                            && !self.at_breakpoint(client, &body, spec)
                        {
                            let thread = body.get("threadId").and_then(Value::as_i64).unwrap_or(1);
                            let _ = client.request("continue", json!({"threadId": thread}));
                        } else {
                            self.stopped(client, &body);
                        }
                    }
                    "initialized" => self.ready = true,
                    "continued" => self.tell(Update::Running),
                    "exited" => {
                        exit_code = body.get("exitCode").and_then(Value::as_i64);
                    }
                    "terminated" => return (exit_code, None),
                    _ => self.relay_output(&name, &body),
                },
                In::Cmd(Command::Stop) => return (exit_code, None),
                In::Cmd(command) => self.command(client, command),
            }
        }
    }

    /// Whether a stop is on a line the person put a breakpoint on.
    fn at_breakpoint(&self, client: &Client, body: &Value, spec: &LaunchSpec) -> bool {
        let thread = body.get("threadId").and_then(Value::as_i64).unwrap_or(1);
        let top = client
            .request("stackTrace", json!({"threadId": thread, "levels": 1}))
            .wait(REPLY)
            .map(|b| parse_frames(&b))
            .ok()
            .and_then(|f| f.into_iter().next());
        let Some(frame) = top else { return false };
        let Some(path) = frame.path else { return false };
        spec.breakpoints
            .iter()
            .any(|(p, lines)| same_file(p, &path) && lines.contains(&frame.line))
    }

    fn command(&mut self, client: &Client, command: Command) {
        let thread = json!({"threadId": self.thread});
        let go = |name: &str| {
            // The answer matters only when it is a refusal; the stop arrives as an event.
            let _ = client.request(name, thread.clone());
        };
        match command {
            Command::Continue => {
                go("continue");
                self.tell(Update::Running);
            }
            Command::StepOver => {
                go("next");
                self.tell(Update::Running);
            }
            Command::StepInto => {
                go("stepIn");
                self.tell(Update::Running);
            }
            Command::StepOut => {
                go("stepOut");
                self.tell(Update::Running);
            }
            Command::Pause => go("pause"),
            Command::SetBreakpoints(path, lines) => self.set_breakpoints(client, &path, &lines),
            Command::SelectFrame(i) => {
                if let Some(frame) = self.frames.get(i) {
                    self.selected = i;
                    let scopes = self.scopes(client, frame.id);
                    self.tell(Update::Frame {
                        selected: i,
                        scopes,
                    });
                }
            }
            Command::Expand(reference) => {
                let vars = self.variables(client, reference);
                self.tell(Update::Children { reference, vars });
            }
            Command::Evaluate(expression) => {
                let frame = self.frames.get(self.selected).map(|f| f.id);
                let mut args = json!({"expression": expression, "context": "repl"});
                if let (Some(frame), Some(obj)) = (frame, args.as_object_mut()) {
                    obj.insert("frameId".into(), json!(frame));
                }
                let result = client
                    .request("evaluate", args)
                    .wait(REPLY)
                    .map(|b| {
                        b.get("result")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned()
                    })
                    .map_err(|e| e.to_string());
                self.tell(Update::Evaluated { expression, result });
            }
            Command::Stop => {}
        }
    }

    fn stopped(&mut self, client: &Client, body: &Value) {
        let reason = body
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("stopped")
            .to_owned();
        let thread = match body.get("threadId").and_then(Value::as_i64) {
            Some(t) => t,
            None => client
                .request("threads", json!({}))
                .wait(REPLY)
                .ok()
                .and_then(|b| b.get("threads")?.as_array()?.first()?.get("id")?.as_i64())
                .unwrap_or(1),
        };
        self.thread = thread;
        let frames = client
            .request(
                "stackTrace",
                json!({"threadId": thread, "levels": MAX_FRAMES}),
            )
            .wait(REPLY)
            .map(|b| parse_frames(&b))
            .unwrap_or_default();
        let scopes = frames
            .first()
            .map(|f| self.scopes(client, f.id))
            .unwrap_or_default();
        self.frames = frames.clone();
        self.selected = 0;
        self.tell(Update::Stopped(StopInfo {
            reason,
            thread_id: thread,
            frames,
            selected: 0,
            scopes,
        }));
    }

    fn scopes(&self, client: &Client, frame: i64) -> Vec<ScopeVars> {
        let Ok(body) = client
            .request("scopes", json!({"frameId": frame}))
            .wait(REPLY)
        else {
            return Vec::new();
        };
        let list = body
            .get("scopes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for scope in list.iter().take(MAX_SCOPES) {
            let name = scope
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Scope")
                .to_owned();
            let expensive = scope
                .get("expensive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let reference = scope
                .get("variablesReference")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let vars = if expensive || reference == 0 {
                Vec::new()
            } else {
                self.variables(client, reference)
            };
            out.push(ScopeVars { name, vars });
        }
        out
    }

    fn variables(&self, client: &Client, reference: i64) -> Vec<Variable> {
        client
            .request(
                "variables",
                json!({"variablesReference": reference, "count": MAX_VARIABLES}),
            )
            .wait(REPLY)
            .map(|b| parse_variables(&b))
            .unwrap_or_default()
    }
}

fn parse_frames(body: &Value) -> Vec<Frame> {
    body.get("stackFrames")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(MAX_FRAMES)
                .map(|f| Frame {
                    id: f.get("id").and_then(Value::as_i64).unwrap_or(0),
                    name: f
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_owned(),
                    path: f
                        .get("source")
                        .and_then(|s| s.get("path"))
                        .and_then(Value::as_str)
                        .map(PathBuf::from),
                    line: usize_of(f.get("line")),
                    column: usize_of(f.get("column")),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_variables(body: &Value) -> Vec<Variable> {
    body.get("variables")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(MAX_VARIABLES)
                .map(|v| Variable {
                    name: v
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_owned(),
                    value: v
                        .get("value")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    type_name: v
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    reference: v
                        .get("variablesReference")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn usize_of(v: Option<&Value>) -> usize {
    v.and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0)
}

fn explain(e: &ClientError, spec: &LaunchSpec) -> String {
    format!("{} ({})", e, spec.adapter.name)
}

fn spawn(adapter: &Adapter) -> Result<Child, String> {
    Process::new(&adapter.program)
        .args(&adapter.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", adapter.name))
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_and_variables_are_read_defensively() {
        let frames = parse_frames(&json!({"stackFrames": [
            {"id": 3, "name": "main", "source": {"path": "/p/a.c"}, "line": 7, "column": 1},
            {"id": 4}, 5, null
        ]}));
        assert_eq!(frames.len(), 4);
        assert_eq!(frames[0].line, 7);
        assert_eq!(frames[0].path.as_deref(), Some(Path::new("/p/a.c")));
        assert_eq!(frames[1].name, "?");
        assert_eq!(frames[1].path, None);
        assert!(parse_frames(&json!("junk")).is_empty());

        let vars = parse_variables(&json!({"variables": [
            {"name": "x", "value": "5", "type": "int", "variablesReference": 0},
            {"name": "s", "value": "{...}", "variablesReference": 9}
        ]}));
        assert_eq!(vars[0].type_name, "int");
        assert_eq!(vars[1].reference, 9);
        assert!(parse_variables(&Value::Null).is_empty());
    }

    #[test]
    fn a_missing_adapter_ends_the_session_with_the_reason() {
        let spec = LaunchSpec {
            adapter: Adapter {
                kind: crate::AdapterKind::Gdb,
                name: "gdb",
                program: PathBuf::from("/definitely/not/here"),
                args: Vec::new(),
            },
            program: PathBuf::from("/bin/true"),
            args: Vec::new(),
            cwd: PathBuf::from("/"),
            breakpoints: BTreeMap::new(),
        };
        let d = Debugger::start(spec, None);
        let end = (0..100).find_map(|_| {
            std::thread::sleep(Duration::from_millis(50));
            d.poll()
        });
        match end {
            Some(Update::Ended { error: Some(e), .. }) => assert!(e.contains("could not start")),
            other => panic!("{other:?}"),
        }
    }
}
