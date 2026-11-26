// SPDX-License-Identifier: MIT OR Apache-2.0
//! Language servers as the editor uses them: started on demand per language, kept in step with the
//! open documents, and read for diagnostics. A frontend calls [`Hub::sync`] for each open document
//! every frame and [`Hub::poll`] once; everything slow (starting a server, its handshake) happens
//! on a thread of its own, and a server that is missing or dies is a status, never an error.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use codigo_lsp::{
    char_to_utf16, parse_completion, parse_hover, parse_locations, parse_text_edits,
    parse_workspace_edit, path_to_uri, server_for, spawn_with, uri_to_path, utf16_to_char,
    CompletionItem, Diagnostic, Event, Pending, Position, ServerSpec, Severity, SpawnError,
    Spawned, TextEdit, Waker,
};

use crate::document::Document;

/// Files bigger than this are not sent to a server: the whole text goes over the pipe on every
/// change, and a server would take longer than the person to catch up.
pub const MAX_SYNC_CHARS: usize = 2_000_000;
/// The longest a server may take to answer `initialize`.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// Edits are sent at most this often per document.
const SYNC_EVERY: Duration = Duration::from_millis(150);

/// What a status bar says about one server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerState {
    Starting,
    /// Running; the line is its latest progress report ("indexing"), if it has one.
    Ready(Option<String>),
    /// The program is not on this computer; the line says how to get it.
    Missing(&'static str),
    /// Could not start or died; the line says why.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerStatus {
    pub name: &'static str,
    pub state: ServerState,
}

/// A diagnostic placed on a line of a document, in the editor's own units (chars).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub line: usize,
    /// Char columns within the line, `start..end`; a diagnostic spanning lines ends at the end
    /// of the first one.
    pub start: usize,
    pub end: usize,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub errors: usize,
    pub warnings: usize,
}

/// One diagnostic of any file, for the problems list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub path: PathBuf,
    /// 0-based line.
    pub line: usize,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
}

/// The most problems the list keeps; a project with more than this has bigger worries.
pub const MAX_PROBLEMS: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryKind {
    Definition,
    Hover,
    Completion,
    References,
    Rename,
    Format,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryError {
    /// No server handles this document's language.
    NoServer,
    /// The server is still starting, missing or stopped, or has not been sent this document yet.
    NotReady,
}

/// Somewhere in a file, as the protocol counts it: a line and a UTF-16 column. Turn the column
/// into chars with [`char_in_line`] once the file's text is at hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub path: PathBuf,
    pub line: usize,
    pub character: u32,
}

/// One replacement in a file, positioned as the protocol counts: a line and a UTF-16 column at
/// each end. Turn the columns into chars with [`char_in_line`] against the file's own text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceEdit {
    pub start: (usize, u32),
    pub end: (usize, u32),
    pub new_text: String,
}

/// What a rename or a format asks to change in one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    pub edits: Vec<PlaceEdit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Places(Vec<Place>),
    References(Vec<Place>),
    Edits(Vec<FileChange>),
    Hover(Option<String>),
    Completion(Vec<CompletionItem>),
}

/// A request in flight. Poll it from the frame loop.
pub struct Query {
    pub kind: QueryKind,
    /// The file a formatting answer is about; the protocol's reply does not name it.
    path: Option<PathBuf>,
    pending: Pending,
}

impl Query {
    /// `None` while the server is still working.
    pub fn poll(&self) -> Option<Result<Answer, String>> {
        let reply = self.pending.poll()?;
        Some(reply.map_err(|e| e.message).map(|value| {
            match self.kind {
                QueryKind::Definition => Answer::Places(places(&value)),
                QueryKind::References => Answer::References(places(&value)),
                QueryKind::Rename => Answer::Edits(
                    parse_workspace_edit(&value)
                        .into_iter()
                        .filter_map(|f| {
                            Some(FileChange {
                                path: uri_to_path(&f.uri)?,
                                edits: f.edits.into_iter().map(place_edit).collect(),
                            })
                        })
                        .collect(),
                ),
                QueryKind::Format => Answer::Edits(
                    self.path
                        .iter()
                        .map(|path| FileChange {
                            path: path.clone(),
                            edits: parse_text_edits(&value)
                                .into_iter()
                                .map(place_edit)
                                .collect(),
                        })
                        .collect(),
                ),
                QueryKind::Hover => Answer::Hover(parse_hover(&value)),
                QueryKind::Completion => Answer::Completion(parse_completion(&value)),
            }
        }))
    }
}

fn place_edit(e: TextEdit) -> PlaceEdit {
    PlaceEdit {
        start: (e.range.start.line as usize, e.range.start.character),
        end: (e.range.end.line as usize, e.range.end.character),
        new_text: e.new_text,
    }
}

fn places(value: &serde_json::Value) -> Vec<Place> {
    parse_locations(value)
        .into_iter()
        .filter_map(|l| {
            Some(Place {
                path: uri_to_path(&l.uri)?,
                line: l.range.start.line as usize,
                character: l.range.start.character,
            })
        })
        .collect()
}

/// A server's edits as char ranges in `doc`'s text. Positions past the end clamp to it.
pub fn edits_for(doc: &Document, edits: &[PlaceEdit]) -> Vec<codigo_text::Edit> {
    let buffer = doc.buffer();
    let offset = |(line, utf16): (usize, u32)| {
        if line >= buffer.line_count() {
            return buffer.len_chars();
        }
        buffer.line_start(line) + char_in_line(doc, line, utf16)
    };
    edits
        .iter()
        .map(|e| {
            let start = offset(e.start);
            let end = offset(e.end).max(start);
            codigo_text::Edit::new(start..end, e.new_text.clone())
        })
        .collect()
}

/// The char column that sits `utf16` units into `text`'s line `line`.
pub fn char_in_line(doc: &Document, line: usize, utf16: u32) -> usize {
    utf16_to_char(&doc.buffer().line_text(line), utf16)
}

type Started = Result<Spawned, StartError>;

enum StartError {
    Missing(&'static str),
    Failed(String),
}

enum Slot {
    Starting(Receiver<Started>),
    Running(Running),
    Missing(&'static str),
    Failed(String),
}

struct Running {
    server: Spawned,
    progress: Option<String>,
}

struct Synced {
    command: &'static str,
    uri: String,
    version: i32,
    revision: u64,
    sent: Instant,
    /// The text changed since it was last sent.
    dirty: bool,
}

pub struct Hub {
    enabled: bool,
    root: Option<PathBuf>,
    wake: Waker,
    servers: HashMap<&'static str, Slot>,
    docs: HashMap<PathBuf, Synced>,
    diagnostics: HashMap<PathBuf, Vec<Diagnostic>>,
    /// Bumped whenever any diagnostics change, so a view knows to rebuild its marks.
    generation: u64,
}

impl Hub {
    /// `wake` is called from other threads when something happened that is worth a repaint.
    pub fn new(enabled: bool, wake: impl Fn() + Send + Sync + 'static) -> Hub {
        Hub {
            enabled,
            root: None,
            wake: Arc::new(wake),
            servers: HashMap::new(),
            docs: HashMap::new(),
            diagnostics: HashMap::new(),
            generation: 0,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Turning it off stops every server and forgets what they said.
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            if !enabled {
                self.reset();
            }
        }
    }

    /// Servers start in the project folder; a different folder means different servers.
    pub fn set_root(&mut self, root: Option<PathBuf>) {
        if self.root != root {
            self.root = root;
            self.reset();
        }
    }

    fn reset(&mut self) {
        self.servers.clear();
        self.docs.clear();
        if !self.diagnostics.is_empty() {
            self.diagnostics.clear();
            self.generation += 1;
        }
    }

    /// Keep the server for this document's language in step with its text. Cheap to call every
    /// frame: it sends nothing unless the text changed.
    pub fn sync(&mut self, doc: &Document) {
        if !self.enabled {
            return;
        }
        let Some(path) = doc.path() else { return };
        let Some(spec) = server_for(doc.language().id) else {
            return;
        };
        let buffer = doc.buffer();
        if buffer.len_chars() > MAX_SYNC_CHARS {
            return;
        }
        self.ensure_started(spec, path);
        let Some(Slot::Running(running)) = self.servers.get(spec.command) else {
            return;
        };
        let revision = buffer.revision();
        match self.docs.get_mut(path) {
            None => {
                let Some(uri) = path_to_uri(path) else { return };
                let text = buffer.text();
                if running
                    .server
                    .client
                    .did_open(&uri, doc.language().lsp_id, 1, &text)
                    .is_ok()
                {
                    self.docs.insert(
                        path.to_path_buf(),
                        Synced {
                            command: spec.command,
                            uri,
                            version: 1,
                            revision,
                            sent: Instant::now(),
                            dirty: false,
                        },
                    );
                }
            }
            Some(synced) => {
                if synced.revision != revision {
                    synced.revision = revision;
                    synced.dirty = true;
                }
                if synced.dirty && synced.sent.elapsed() >= SYNC_EVERY {
                    synced.version = synced.version.saturating_add(1);
                    let sent = running.server.client.did_change(
                        &synced.uri,
                        synced.version,
                        &buffer.text(),
                    );
                    synced.sent = Instant::now();
                    synced.dirty = sent.is_err();
                }
            }
        }
    }

    /// Whether some document has edits waiting for their turn to be sent; the frontend should
    /// repaint after [`SYNC_EVERY`] so they go out.
    pub fn has_pending_changes(&self) -> bool {
        self.docs.values().any(|d| d.dirty)
    }

    pub fn sync_interval() -> Duration {
        SYNC_EVERY
    }

    /// The file was saved.
    pub fn saved(&mut self, path: &Path) {
        if let Some(synced) = self.docs.get(path) {
            if let Some(Slot::Running(r)) = self.servers.get(synced.command) {
                let _ = r.server.client.did_save(&synced.uri, None);
            }
        }
    }

    /// The document was closed, or its file renamed.
    pub fn closed(&mut self, path: &Path) {
        if let Some(synced) = self.docs.remove(path) {
            if let Some(Slot::Running(r)) = self.servers.get(synced.command) {
                let _ = r.server.client.did_close(&synced.uri);
            }
        }
        if self.diagnostics.remove(path).is_some() {
            self.generation += 1;
        }
    }

    fn ensure_started(&mut self, spec: &'static ServerSpec, doc_path: &Path) {
        if self.servers.contains_key(spec.command) {
            return;
        }
        let cwd = self
            .root
            .clone()
            .or_else(|| doc_path.parent().map(Path::to_path_buf));
        let Some(cwd) = cwd else { return };
        let (tx, rx) = mpsc::channel();
        let wake = self.wake.clone();
        let started = std::thread::Builder::new()
            .name("codigo-lsp-start".into())
            .spawn(move || {
                let _ = tx.send(start(spec, &cwd, wake.clone()));
                wake();
            });
        let slot = match started {
            Ok(_) => Slot::Starting(rx),
            Err(e) => Slot::Failed(format!(
                "could not start a thread for {}: {e}",
                spec.command
            )),
        };
        self.servers.insert(spec.command, slot);
    }

    /// Take in what the servers said. Returns whether anything a view shows changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let mut finished: Vec<(&'static str, Slot)> = Vec::new();
        let mut gone: Vec<(&'static str, String)> = Vec::new();
        for (name, slot) in &mut self.servers {
            match slot {
                Slot::Starting(rx) => {
                    if let Ok(result) = rx.try_recv() {
                        let next = match result {
                            Ok(server) => Slot::Running(Running {
                                server,
                                progress: None,
                            }),
                            Err(StartError::Missing(install)) => Slot::Missing(install),
                            Err(StartError::Failed(why)) => Slot::Failed(why),
                        };
                        finished.push((*name, next));
                    }
                }
                Slot::Running(r) => {
                    let mut batch = Vec::new();
                    while let Ok(event) = r.server.client.events().try_recv() {
                        batch.push(event);
                        if batch.len() >= 512 {
                            break;
                        }
                    }
                    for event in batch {
                        match event {
                            Event::Diagnostics { uri, diagnostics } => {
                                if let Some(path) = uri_to_path(&uri) {
                                    if diagnostics.is_empty() {
                                        changed |= self.diagnostics.remove(&path).is_some();
                                    } else {
                                        self.diagnostics.insert(path, diagnostics);
                                        changed = true;
                                    }
                                }
                            }
                            Event::Progress { text } => {
                                let text = (!text.is_empty()).then_some(text);
                                if r.progress != text {
                                    r.progress = text;
                                    changed = true;
                                }
                            }
                            Event::Exited => gone.push((*name, "the server stopped".into())),
                            Event::Message { .. } | Event::Notification { .. } => {}
                        }
                    }
                    if r.server.exited() && !gone.iter().any(|(n, _)| n == name) {
                        gone.push((*name, "the server stopped".into()));
                    }
                }
                Slot::Missing(_) | Slot::Failed(_) => {}
            }
        }
        for (name, slot) in finished {
            self.servers.insert(name, slot);
            changed = true;
        }
        for (name, why) in gone {
            self.servers.insert(name, Slot::Failed(why));
            self.docs.retain(|_, d| d.command != name);
            changed = true;
        }
        if changed {
            self.generation += 1;
        }
        changed
    }

    /// Changes whenever diagnostics or server states do.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn diagnostics(&self, path: &Path) -> &[Diagnostic] {
        self.diagnostics.get(path).map_or(&[], Vec::as_slice)
    }

    /// The errors and warnings of every file the servers reported on.
    pub fn totals(&self) -> Counts {
        let mut c = Counts::default();
        for list in self.diagnostics.values() {
            for d in list {
                match d.severity {
                    Severity::Error => c.errors += 1,
                    Severity::Warning => c.warnings += 1,
                    _ => {}
                }
            }
        }
        c
    }

    /// Errors and warnings of every file, errors first, then by file and line, cut at
    /// [`MAX_PROBLEMS`]. Hints and notes are left out: they belong in the editor, not in a list.
    pub fn problems(&self) -> Vec<Problem> {
        let mut out: Vec<Problem> = self
            .diagnostics
            .iter()
            .flat_map(|(path, list)| {
                list.iter()
                    .filter(|d| matches!(d.severity, Severity::Error | Severity::Warning))
                    .map(move |d| Problem {
                        path: path.clone(),
                        line: d.range.start.line as usize,
                        severity: d.severity,
                        message: d.message.clone(),
                        source: d.source.clone(),
                    })
            })
            .collect();
        out.sort_by(|a, b| (a.severity, &a.path, a.line).cmp(&(b.severity, &b.path, b.line)));
        out.truncate(MAX_PROBLEMS);
        out
    }

    pub fn counts(&self, path: &Path) -> Counts {
        let mut c = Counts::default();
        for d in self.diagnostics(path) {
            match d.severity {
                Severity::Error => c.errors += 1,
                Severity::Warning => c.warnings += 1,
                _ => {}
            }
        }
        c
    }

    /// The diagnostics of `doc` placed in its lines and char columns.
    pub fn marks(&self, doc: &Document) -> Vec<Mark> {
        let Some(path) = doc.path() else {
            return Vec::new();
        };
        let buffer = doc.buffer();
        let lines = buffer.line_count();
        let mut marks: Vec<Mark> = self
            .diagnostics(path)
            .iter()
            .filter_map(|d| {
                let line = d.range.start.line as usize;
                if line >= lines {
                    return None;
                }
                let text = buffer.line_text(line);
                let len = text.chars().count();
                let start = utf16_to_char(&text, d.range.start.character);
                let end = if d.range.end.line as usize == line {
                    utf16_to_char(&text, d.range.end.character)
                } else {
                    len
                };
                // An empty range still has to be seen: underline the char it sits on.
                let end = end.max(start + 1).min(len.max(start + 1));
                Some(Mark {
                    line,
                    start,
                    end,
                    severity: d.severity,
                    message: d.message.clone(),
                    source: d.source.clone(),
                })
            })
            .collect();
        marks.sort_by_key(|m| (m.line, m.start));
        marks
    }

    /// Ask the server for `kind` of answer at char offset `at` in `doc`. Edits not yet sent are
    /// sent first, so the server answers about the text the person sees. A rename takes its new
    /// name from `name`; a format has no position and takes the indent from `indent`.
    pub fn query(
        &mut self,
        doc: &Document,
        at: usize,
        kind: QueryKind,
    ) -> Result<Query, QueryError> {
        self.ask(doc, at, kind, "", (4, true))
    }

    /// Rename the symbol at `at` to `name`.
    pub fn rename(&mut self, doc: &Document, at: usize, name: &str) -> Result<Query, QueryError> {
        self.ask(doc, at, QueryKind::Rename, name, (4, true))
    }

    /// Format the whole of `doc`.
    pub fn format(
        &mut self,
        doc: &Document,
        tab_size: u32,
        insert_spaces: bool,
    ) -> Result<Query, QueryError> {
        self.ask(doc, 0, QueryKind::Format, "", (tab_size, insert_spaces))
    }

    fn ask(
        &mut self,
        doc: &Document,
        at: usize,
        kind: QueryKind,
        name: &str,
        indent: (u32, bool),
    ) -> Result<Query, QueryError> {
        let path = doc.path().ok_or(QueryError::NoServer)?;
        let spec = server_for(doc.language().id).ok_or(QueryError::NoServer)?;
        let Some(Slot::Running(running)) = self.servers.get(spec.command) else {
            return Err(QueryError::NotReady);
        };
        let synced = self.docs.get_mut(path).ok_or(QueryError::NotReady)?;
        let buffer = doc.buffer();
        if synced.dirty {
            synced.version = synced.version.saturating_add(1);
            let sent =
                running
                    .server
                    .client
                    .did_change(&synced.uri, synced.version, &buffer.text());
            synced.sent = Instant::now();
            synced.dirty = sent.is_err();
        }
        let (line, col) = buffer.line_col(at);
        let position = Position {
            line: u32::try_from(line).unwrap_or(u32::MAX),
            character: char_to_utf16(&buffer.line_text(line), col),
        };
        let client = &running.server.client;
        let pending = match kind {
            QueryKind::Definition => {
                client.at_position("textDocument/definition", &synced.uri, position)
            }
            QueryKind::Hover => client.at_position("textDocument/hover", &synced.uri, position),
            QueryKind::Completion => {
                client.at_position("textDocument/completion", &synced.uri, position)
            }
            QueryKind::References => client.references(&synced.uri, position),
            QueryKind::Rename => client.rename(&synced.uri, position, name),
            QueryKind::Format => client.formatting(&synced.uri, indent.0, indent.1),
        };
        Ok(Query {
            kind,
            path: Some(path.to_path_buf()),
            pending,
        })
    }

    /// One entry per server that was asked for, for the status bar.
    pub fn statuses(&self) -> Vec<ServerStatus> {
        let mut out: Vec<ServerStatus> = self
            .servers
            .iter()
            .map(|(name, slot)| ServerStatus {
                name,
                state: match slot {
                    Slot::Starting(_) => ServerState::Starting,
                    Slot::Running(r) => ServerState::Ready(r.progress.clone()),
                    Slot::Missing(install) => ServerState::Missing(install),
                    Slot::Failed(why) => ServerState::Failed(why.clone()),
                },
            })
            .collect();
        out.sort_by_key(|s| s.name);
        out
    }

    /// The status of the server that handles `language`, if one was asked for.
    pub fn status_for(&self, language: &str) -> Option<ServerStatus> {
        let spec = server_for(language)?;
        self.statuses().into_iter().find(|s| s.name == spec.command)
    }
}

/// Spawn, then do the handshake. Runs on its own thread.
fn start(spec: &'static ServerSpec, cwd: &Path, wake: Waker) -> Started {
    let server = spawn_with(spec, cwd, Some(wake)).map_err(|e| match e {
        SpawnError::NotFound { install, .. } => StartError::Missing(install),
        other => StartError::Failed(other.to_string()),
    })?;
    let root = path_to_uri(cwd);
    server
        .client
        .initialize(root.as_deref(), START_TIMEOUT)
        .map_err(|e| StartError::Failed(format!("{} did not start: {e}", spec.command)))?;
    Ok(server)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{DocId, Document};

    fn doc_in(dir: &Path, name: &str, text: &str) -> Document {
        let path = dir.join(name);
        std::fs::write(&path, text).expect("write");
        Document::open(DocId(1), &path).expect("open")
    }

    #[test]
    fn a_servers_edits_become_char_ranges() {
        let d = tempfile::tempdir().expect("tempdir");
        let doc = doc_in(d.path(), "a.rs", "let é = 1;\nlet b = 2;\n");
        let edits = vec![
            PlaceEdit {
                start: (0, 4),
                end: (0, 5),
                new_text: "e".into(),
            },
            PlaceEdit {
                start: (1, 4),
                end: (99, 0),
                new_text: "c".into(),
            },
        ];
        let got = edits_for(&doc, &edits);
        assert_eq!(got[0].range, 4..5);
        assert_eq!(got[1].range, 15..doc.buffer().len_chars());
        let mut text = doc.buffer().text();
        text = text.replacen('é', "e", 1);
        assert!(text.starts_with("let e"));
    }

    #[test]
    fn a_disabled_hub_starts_nothing() {
        let d = tempfile::tempdir().expect("tempdir");
        let doc = doc_in(d.path(), "a.rs", "fn main() {}\n");
        let mut hub = Hub::new(false, || {});
        hub.sync(&doc);
        assert!(hub.statuses().is_empty());
        assert!(!hub.poll());
    }

    #[test]
    fn a_language_without_a_server_is_ignored() {
        let d = tempfile::tempdir().expect("tempdir");
        let doc = doc_in(d.path(), "notes.txt", "hello\n");
        let mut hub = Hub::new(true, || {});
        hub.sync(&doc);
        assert!(hub.statuses().is_empty());
    }

    #[test]
    fn untitled_documents_are_not_synced() {
        let doc = Document::untitled(DocId(2));
        let mut hub = Hub::new(true, || {});
        hub.sync(&doc);
        assert!(hub.statuses().is_empty());
    }

    #[test]
    fn diagnostics_become_marks_in_chars_and_counts() {
        let d = tempfile::tempdir().expect("tempdir");
        let doc = doc_in(d.path(), "a.rs", "let é = 1;\nlet 😀x = 2;\n");
        let path = doc.path().expect("path").to_path_buf();
        let mut hub = Hub::new(true, || {});
        let at = |l, c| codigo_lsp::Position {
            line: l,
            character: c,
        };
        let diag = |s, e, sev| Diagnostic {
            range: codigo_lsp::Range { start: s, end: e },
            severity: sev,
            message: "m".into(),
            source: None,
            code: None,
        };
        hub.diagnostics.insert(
            path.clone(),
            vec![
                diag(at(1, 4), at(1, 6), Severity::Error),
                diag(at(0, 4), at(0, 4), Severity::Warning),
                diag(at(99, 0), at(99, 1), Severity::Error),
            ],
        );
        let marks = hub.marks(&doc);
        assert_eq!(marks.len(), 2, "a line past the end is dropped");
        // Sorted by line; the empty range still covers one char.
        assert_eq!((marks[0].line, marks[0].start, marks[0].end), (0, 4, 5));
        // The emoji is two UTF-16 units but one char: 4..6 covers the emoji only.
        assert_eq!((marks[1].line, marks[1].start, marks[1].end), (1, 4, 5));
        let c = hub.counts(&path);
        assert_eq!((c.errors, c.warnings), (2, 1));
    }

    #[test]
    fn problems_span_every_file_errors_first_and_without_hints() {
        let mut hub = Hub::new(true, || {});
        let at = |l| codigo_lsp::Position {
            line: l,
            character: 0,
        };
        let diag = |l, sev, m: &str| Diagnostic {
            range: codigo_lsp::Range {
                start: at(l),
                end: at(l),
            },
            severity: sev,
            message: m.into(),
            source: Some("rustc".into()),
            code: None,
        };
        hub.diagnostics.insert(
            PathBuf::from("/p/b.rs"),
            vec![
                diag(7, Severity::Warning, "unused"),
                diag(2, Severity::Hint, "style"),
            ],
        );
        hub.diagnostics.insert(
            PathBuf::from("/p/a.rs"),
            vec![
                diag(9, Severity::Error, "late"),
                diag(1, Severity::Error, "early"),
                diag(3, Severity::Information, "note"),
            ],
        );
        let list = hub.problems();
        let got: Vec<_> = list.iter().map(|p| p.message.as_str()).collect();
        assert_eq!(got, ["early", "late", "unused"]);
        let t = hub.totals();
        assert_eq!((t.errors, t.warnings), (2, 1));
        assert!(Hub::new(true, || {}).problems().is_empty());
    }
}
