// SPDX-License-Identifier: MIT OR Apache-2.0
//! The application: its state, the commands, background work and the frame loop.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use codigo_engine::{
    Appearance, AutoSave, Cmd, DocId, Document, Hub, OpenError, SaveError, SessionState, Settings,
};
use codigo_text::{Query, Selection};
use codigo_workspace::{self as ws, GitInfo, SearchOptions, SearchResult};
use egui::{Event, Key, Modifiers};
use nodalia_theme::creator::Look;
use nodalia_theme::for_egui::{self, Appearance as NdlAppearance};
use nodalia_theme::{Accent, Mode};

use crate::editor::{self, Output};
use crate::explorer::Explorer;
use crate::i18n::{self, tr, trf, Lang};
use crate::overlays::Overlay;
use crate::{chrome, ui, updates};

pub const NAME: &str = "Código";

/// Work that finished off the window thread.
pub enum Msg {
    Files(PathBuf, Vec<String>),
    Search(u64, Result<SearchResult, String>),
    Git(PathBuf, Option<GitInfo>),
    /// A file's changed lines against git, for the gutter.
    Marks(PathBuf, Vec<ws::LineChange>),
    Quality(u64, codigo_quality::Report),
}

pub struct Tab {
    pub doc: Document,
    pub view: editor::View,
}

pub struct Toast {
    pub text: String,
    pub until: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Explorer,
    Search,
    Quality,
    Problems,
}

/// The find bar's state.
pub struct Find {
    pub open: bool,
    pub replace: bool,
    pub query: Query,
    pub with: String,
    /// Take keyboard focus into the field on the next frame.
    pub focus: bool,
    pub matcher: Option<codigo_text::search::Matcher>,
    pub error: Option<String>,
    pub count: usize,
    pub current: Option<usize>,
    /// Revision and query the counts were made for.
    pub stamp: (DocId, u64, Query),
}

impl Default for Find {
    fn default() -> Self {
        Find {
            open: false,
            replace: false,
            query: Query::default(),
            with: String::new(),
            focus: false,
            matcher: None,
            error: None,
            count: 0,
            current: None,
            stamp: (DocId(u64::MAX), 0, Query::default()),
        }
    }
}

/// The project search panel's state.
pub struct ProjectSearch {
    pub query: Query,
    pub focus: bool,
    pub running: bool,
    pub serial: u64,
    pub result: Option<Result<SearchResult, String>>,
    cancel: Arc<AtomicBool>,
}

impl Default for ProjectSearch {
    fn default() -> Self {
        ProjectSearch {
            query: Query::default(),
            focus: false,
            running: false,
            serial: 0,
            result: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// The quality panel's state: the last analysis of the folder.
pub struct QualityPanel {
    pub running: bool,
    pub serial: u64,
    pub report: Option<codigo_quality::Report>,
    cancel: Arc<AtomicBool>,
}

impl Default for QualityPanel {
    fn default() -> Self {
        QualityPanel {
            running: false,
            serial: 0,
            report: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub struct Project {
    pub root: PathBuf,
    pub explorer: Explorer,
    pub git: Option<GitInfo>,
}

pub struct App {
    pub settings: Settings,
    pub tabs: Vec<Tab>,
    pub active: usize,
    next_id: u64,
    pub project: Option<Project>,
    /// Language servers and what they reported.
    pub lsp: Hub,
    /// The terminal panel and the run console.
    pub term: crate::terminal_ui::TermPanel,
    /// A language-server question waiting for its answer.
    pub query: Option<crate::lsp_ui::InFlight>,
    /// The hover or completion popup next to the caret.
    pub popup: crate::lsp_ui::Popup,
    pub panel: Panel,
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
    pub overlay: Overlay,
    pub find: Find,
    pub psearch: ProjectSearch,
    pub quality: QualityPanel,
    pub files: Option<(PathBuf, Arc<Vec<String>>)>,
    /// Stops the walk that lists the folder's files when another folder is opened.
    files_cancel: Arc<AtomicBool>,
    pub closed: Vec<PathBuf>,
    pub toasts: Vec<Toast>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub updates: updates::Runtime,
    #[cfg(target_os = "linux")]
    menu: Option<crate::linux_menu::GlobalMenu>,
    applied: Option<(Appearance, bool, bool)>,
    decorated: Option<bool>,
    pub focus_editor: bool,
    pub session_dirty: bool,
    last_session: Instant,
    last_disk_check: Instant,
    last_edit: Option<Instant>,
    ctx: egui::Context,
    ignore_close: bool,
}

pub fn mode_for(dark: bool, high_contrast: bool) -> Mode {
    match (dark, high_contrast) {
        (false, false) => Mode::Light,
        (true, false) => Mode::Dark,
        (false, true) => Mode::LightHc,
        (true, true) => Mode::DarkHc,
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: &[PathBuf]) -> App {
        let settings = Settings::load();
        i18n::set(Lang::from_setting(&settings.language));
        let (tx, rx) = channel();
        let lsp = {
            let ctx = cc.egui_ctx.clone();
            Hub::new(settings.language_servers, move || ctx.request_repaint())
        };
        let mut app = App {
            settings,
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            project: None,
            lsp,
            term: crate::terminal_ui::TermPanel::default(),
            query: None,
            popup: crate::lsp_ui::Popup::None,
            panel: Panel::Explorer,
            sidebar_visible: true,
            sidebar_width: 280.0,
            overlay: Overlay::None,
            find: Find::default(),
            psearch: ProjectSearch::default(),
            quality: QualityPanel::default(),
            files: None,
            files_cancel: Arc::new(AtomicBool::new(false)),
            closed: Vec::new(),
            toasts: Vec::new(),
            tx,
            rx,
            updates: updates::Runtime::default(),
            #[cfg(target_os = "linux")]
            menu: crate::linux_menu::install(
                &cc.egui_ctx,
                std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty()),
            ),
            applied: None,
            decorated: None,
            focus_editor: true,
            session_dirty: false,
            last_session: Instant::now(),
            last_disk_check: Instant::now(),
            last_edit: None,
            ctx: cc.egui_ctx.clone(),
            ignore_close: false,
        };
        app.apply_appearance(&cc.egui_ctx);
        if app.settings.check_updates {
            app.updates.start(false);
        }
        app.restore(SessionState::load());
        for path in open {
            if path.is_dir() {
                app.open_project(path.clone(), true);
            } else if path.is_file() {
                app.open_path(path, None, true);
            }
        }
        app
    }

    pub fn ndl_controls(&self) -> bool {
        chrome::NDL_CONTROLS_POSSIBLE && !self.settings.native_window_controls
    }

    /// The colours to draw with, following the system theme when asked to.
    pub fn look(&self, ctx: &egui::Context) -> (Look, Mode) {
        let dark = match self.settings.appearance {
            Appearance::System => ctx.theme() == egui::Theme::Dark,
            Appearance::Light => false,
            Appearance::Dark => true,
        };
        let mode = mode_for(dark, self.settings.high_contrast);
        (Look::new(mode.palette_with(Accent::Codigo)), mode)
    }

    fn apply_appearance(&mut self, ctx: &egui::Context) {
        let key = (
            self.settings.appearance,
            self.settings.high_contrast,
            self.settings.reduced_motion,
        );
        if self.applied == Some(key) {
            return;
        }
        self.applied = Some(key);
        let appearance = |dark: bool| NdlAppearance {
            mode: mode_for(dark, self.settings.high_contrast),
            accent: Accent::Codigo,
            reduced_motion: self.settings.reduced_motion,
            ..NdlAppearance::default()
        };
        match self.settings.appearance {
            Appearance::System => for_egui::apply_system(ctx, &appearance(false)),
            Appearance::Light => for_egui::apply(ctx, &appearance(false)),
            Appearance::Dark => for_egui::apply(ctx, &appearance(true)),
        }
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        let now = self.ctx.input(|i| i.time);
        self.toasts.push(Toast {
            text: text.into(),
            until: now + 4.0,
        });
        self.ctx.request_repaint();
    }

    pub fn save_settings(&mut self) {
        if let Some(path) = Settings::file() {
            if self.settings.save_to(&path).is_err() {
                self.toast(tr("Could not save the preferences."));
            }
        }
    }

    // --- Session -------------------------------------------------------------------------------

    fn restore(&mut self, session: SessionState) {
        self.sidebar_visible = session.sidebar_visible;
        self.sidebar_width = session.sidebar_width;
        if let Some(root) = session.project.filter(|p| p.is_dir()) {
            self.open_project(root, false);
        }
        for path in &session.tabs {
            if path.is_file() {
                let _ = self.open_path(path, None, false);
            }
        }
        self.active = session.active.min(self.tabs.len().saturating_sub(1));
    }

    fn snapshot(&self) -> SessionState {
        SessionState {
            project: self.project.as_ref().map(|p| p.root.clone()),
            tabs: self
                .tabs
                .iter()
                .filter_map(|t| t.doc.path().map(Path::to_path_buf))
                .collect(),
            active: self.active,
            sidebar_visible: self.sidebar_visible,
            sidebar_width: self.sidebar_width,
        }
    }

    fn save_session(&mut self) {
        let _ = self.snapshot().save();
        self.session_dirty = false;
        self.last_session = Instant::now();
    }

    // --- Documents -----------------------------------------------------------------------------

    fn new_id(&mut self) -> DocId {
        let id = DocId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn active_doc(&self) -> Option<&Document> {
        self.active_tab().map(|t| &t.doc)
    }

    pub fn new_file(&mut self) {
        let id = self.new_id();
        let mut view = editor::View::new();
        view.reveal = true;
        self.tabs.push(Tab {
            doc: Document::untitled(id),
            view,
        });
        self.active = self.tabs.len() - 1;
        self.focus_editor = true;
        self.session_dirty = true;
    }

    pub fn error_text(e: &OpenError) -> String {
        match e {
            OpenError::Io(_) => tr("The file could not be read.").to_owned(),
            OpenError::Binary => {
                tr("This looks like a binary file, so it was not opened.").to_owned()
            }
            OpenError::NotUtf8 => tr("This file is not UTF-8 text.").to_owned(),
            OpenError::TooLarge => {
                tr("The file is too large to open (the limit is 64 MB).").to_owned()
            }
        }
    }

    /// Open a file in a tab, or switch to it when it is already open. `line` is 0-based.
    pub fn open_path(&mut self, path: &Path, line: Option<usize>, focus: bool) -> bool {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(i) = self.tabs.iter().position(|t| {
            t.doc.path().is_some_and(|p| {
                std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()) == path
            })
        }) {
            self.active = i;
        } else {
            let id = self.new_id();
            match Document::open(id, &path) {
                Ok(doc) => {
                    self.tabs.push(Tab {
                        doc,
                        view: editor::View::new(),
                    });
                    self.active = self.tabs.len() - 1;
                    self.load_marks(path.clone());
                }
                Err(e) => {
                    let name = path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    self.toast(format!("{name}: {}", App::error_text(&e)));
                    return false;
                }
            }
        }
        if let Some(project) = self.project.as_mut() {
            if path.starts_with(&project.root) {
                project.explorer.reveal(&path);
            }
        }
        if let Some(line) = line {
            self.go_to_line(line);
        }
        if focus {
            self.focus_editor = true;
        }
        self.session_dirty = true;
        true
    }

    /// Put the caret at the start of a 0-based line, selecting nothing.
    pub fn go_to_line(&mut self, line: usize) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.doc.navigate(|b| {
                let line = line.min(b.line_count().saturating_sub(1));
                b.set_cursor(b.line_start(line));
            });
            tab.view.reveal = true;
        }
    }

    pub fn select_range(&mut self, line: usize, start: usize, end: usize) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.doc.navigate(|b| {
                let line = line.min(b.line_count().saturating_sub(1));
                let base = b.line_start(line);
                let text = b.line_text(line);
                let to_char = |byte: usize| {
                    text.get(..byte.min(text.len()))
                        .map_or(0, |s| s.chars().count())
                };
                let (a, z) = (base + to_char(start), base + to_char(end));
                b.set_selections(codigo_text::Selections::single(Selection::new(a, z)));
            });
            tab.view.reveal = true;
        }
    }

    pub fn save_tab(&mut self, index: usize) -> bool {
        let trim = self.settings.trim_trailing_whitespace_on_save;
        let newline = self.settings.insert_final_newline_on_save;
        let Some(tab) = self.tabs.get_mut(index) else {
            return false;
        };
        if tab.doc.path().is_none() {
            let name = tab.doc.file_name().unwrap_or_else(|| "untitled.txt".into());
            let mut dialog = rfd::FileDialog::new().set_file_name(name);
            if let Some(p) = &self.project {
                dialog = dialog.set_directory(&p.root);
            }
            let Some(path) = dialog.save_file() else {
                return false;
            };
            return self.save_as(index, &path);
        }
        tab.doc.tidy(trim, newline);
        match tab.doc.save() {
            Ok(()) => {
                let saved = tab.doc.path().map(Path::to_path_buf);
                if let Some(p) = &saved {
                    self.lsp.saved(p);
                    self.load_marks(p.clone());
                }
                self.refresh_git();
                true
            }
            Err(e) => {
                self.toast(Self::save_error(&e));
                false
            }
        }
    }

    pub fn save_error(e: &SaveError) -> String {
        match e {
            SaveError::NoPath => tr("This document has no file yet.").to_owned(),
            SaveError::Io(_) => tr("The file could not be saved.").to_owned(),
        }
    }

    pub fn save_as(&mut self, index: usize, path: &Path) -> bool {
        let trim = self.settings.trim_trailing_whitespace_on_save;
        let newline = self.settings.insert_final_newline_on_save;
        let Some(tab) = self.tabs.get_mut(index) else {
            return false;
        };
        tab.doc.tidy(trim, newline);
        let old = tab.doc.path().map(Path::to_path_buf);
        match tab.doc.save_as(path) {
            Ok(()) => {
                if let Some(old) = old.filter(|o| o != path) {
                    self.lsp.closed(&old);
                }
                self.load_marks(path.to_path_buf());
                self.session_dirty = true;
                self.refresh_git();
                true
            }
            Err(e) => {
                self.toast(Self::save_error(&e));
                false
            }
        }
    }

    pub fn save_all(&mut self) -> bool {
        let dirty: Vec<usize> = (0..self.tabs.len())
            .filter(|i| self.tabs.get(*i).is_some_and(|t| t.doc.is_dirty()))
            .collect();
        dirty.into_iter().all(|i| self.save_tab(i))
    }

    pub fn dirty_tabs(&self) -> Vec<usize> {
        (0..self.tabs.len())
            .filter(|i| self.tabs.get(*i).is_some_and(|t| t.doc.is_dirty()))
            .collect()
    }

    /// Close a tab without asking.
    pub fn close_tab_now(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        if let Some(p) = tab.doc.path() {
            self.lsp.closed(p);
            self.closed.push(p.to_path_buf());
            self.closed.truncate(20);
        }
        if self.active > index || self.active >= self.tabs.len() {
            self.active = self
                .active
                .saturating_sub(1)
                .min(self.tabs.len().saturating_sub(1));
        }
        self.focus_editor = true;
        self.session_dirty = true;
    }

    /// Close a tab, asking first when it has unsaved edits.
    pub fn close_tab(&mut self, index: usize) {
        if self.tabs.get(index).is_some_and(|t| t.doc.is_dirty()) {
            self.overlay = Overlay::Confirm(crate::overlays::Confirm::Close(vec![index]));
        } else {
            self.close_tab_now(index);
        }
    }

    pub fn close_many(&mut self, indices: Vec<usize>) {
        let dirty: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|i| self.tabs.get(*i).is_some_and(|t| t.doc.is_dirty()))
            .collect();
        if dirty.is_empty() {
            let mut sorted = indices;
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            for i in sorted {
                self.close_tab_now(i);
            }
        } else {
            self.overlay = Overlay::Confirm(crate::overlays::Confirm::Close(indices));
        }
    }

    // --- Project -------------------------------------------------------------------------------

    pub fn open_project(&mut self, root: PathBuf, save: bool) {
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        self.settings.remember_project(&root);
        if save {
            self.save_settings();
        }
        self.files = None;
        self.files_cancel.store(true, Ordering::Relaxed);
        self.psearch = ProjectSearch::default();
        self.quality
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.quality = QualityPanel::default();
        self.project = Some(Project {
            explorer: Explorer::new(root.clone(), {
                let ctx = self.ctx.clone();
                Arc::new(move || ctx.request_repaint())
            }),
            root: root.clone(),
            git: None,
        });
        self.sidebar_visible = true;
        self.panel = Panel::Explorer;
        self.refresh_git();
        self.list_files();
        self.session_dirty = true;
    }

    pub fn close_project(&mut self) {
        self.quality
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.quality = QualityPanel::default();
        self.project = None;
        self.files = None;
        self.files_cancel.store(true, Ordering::Relaxed);
        self.session_dirty = true;
    }

    pub fn refresh_git(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let info = ws::git_status(&root);
            let _ = tx.send(Msg::Git(root, info));
            ctx.request_repaint();
        });
    }

    /// Read a file's changed lines from git on a worker thread: `git diff` on a big repository
    /// takes long enough to be felt, so the window never waits for it.
    pub fn load_marks(&self, path: PathBuf) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let marks = ws::line_changes(&path);
            let _ = tx.send(Msg::Marks(path, marks));
            ctx.request_repaint();
        });
    }

    pub fn list_files(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        // A walk of the folder that was open before has nothing left to say.
        self.files_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.files_cancel = cancel.clone();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let files = ws::list_files(&root, 200_000, &cancel)
                .into_iter()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .collect();
            let _ = tx.send(Msg::Files(root, files));
            ctx.request_repaint();
        });
    }

    pub fn run_project_search(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        self.psearch
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if self.psearch.query.pattern.is_empty() {
            self.psearch.result = None;
            self.psearch.running = false;
            return;
        }
        self.psearch.serial += 1;
        let serial = self.psearch.serial;
        let cancel = Arc::new(AtomicBool::new(false));
        self.psearch.cancel = cancel.clone();
        self.psearch.running = true;
        let options = SearchOptions::new(self.psearch.query.clone());
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let result = ws::search(&root, &options, &cancel).map_err(|e| e.to_string());
            let _ = tx.send(Msg::Search(serial, result));
            ctx.request_repaint();
        });
    }

    /// Analyse the folder for quality and security problems, off the UI thread.
    pub fn run_quality(&mut self) {
        let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        self.quality
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.quality.serial += 1;
        let serial = self.quality.serial;
        let cancel = Arc::new(AtomicBool::new(false));
        self.quality.cancel = cancel.clone();
        self.quality.running = true;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let report = codigo_quality::analyze_project(&root, &cancel);
            let _ = tx.send(Msg::Quality(serial, report));
            ctx.request_repaint();
        });
    }

    /// Keep the language servers in step with the open files and read what they reported.
    fn sync_language_servers(&mut self) {
        self.lsp.set_enabled(self.settings.language_servers);
        self.lsp
            .set_root(self.project.as_ref().map(|p| p.root.clone()));
        for tab in &self.tabs {
            self.lsp.sync(&tab.doc);
        }
        self.lsp.poll();
        let generation = self.lsp.generation();
        for tab in &mut self.tabs {
            let key = (generation, tab.doc.buffer().revision());
            if tab.view.diagnostics_key != Some(key) {
                tab.view.diagnostics = self.lsp.marks(&tab.doc);
                tab.view.diagnostics_key = Some(key);
            }
        }
        if self.lsp.has_pending_changes() {
            self.ctx.request_repaint_after(Hub::sync_interval());
        }
    }

    fn poll(&mut self) {
        if let Some(p) = self.project.as_mut() {
            p.explorer.poll();
        }
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Files(root, files) => {
                    if self.project.as_ref().is_some_and(|p| p.root == root) {
                        self.files = Some((root, Arc::new(files)));
                    }
                }
                Msg::Marks(path, marks) => {
                    for tab in &mut self.tabs {
                        if tab.doc.path() == Some(path.as_path()) {
                            tab.view.marks = marks.clone();
                        }
                    }
                }
                Msg::Git(root, info) => {
                    if let Some(p) = self.project.as_mut().filter(|p| p.root == root) {
                        p.git = info;
                    }
                }
                Msg::Quality(serial, report) => {
                    if serial == self.quality.serial {
                        self.quality.running = false;
                        self.quality.report = Some(report);
                    }
                }
                Msg::Search(serial, result) => {
                    if serial == self.psearch.serial {
                        self.psearch.running = false;
                        self.psearch.result = Some(result);
                    }
                }
            }
        }
    }

    // --- Commands ------------------------------------------------------------------------------

    pub fn run(&mut self, ctx: &egui::Context, cmd: Cmd) {
        use Cmd::*;
        match cmd {
            NewFile => self.new_file(),
            OpenFile => {
                let mut dialog = rfd::FileDialog::new();
                if let Some(p) = &self.project {
                    dialog = dialog.set_directory(&p.root);
                }
                if let Some(paths) = dialog.pick_files() {
                    for p in paths {
                        self.open_path(&p, None, true);
                    }
                }
            }
            OpenFolder => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.open_project(dir, true);
                }
            }
            Save => {
                let i = self.active;
                self.save_tab(i);
            }
            SaveAs => {
                let i = self.active;
                if let Some(name) = self.tabs.get(i).and_then(|t| t.doc.file_name()) {
                    let mut dialog = rfd::FileDialog::new().set_file_name(name);
                    if let Some(p) = &self.project {
                        dialog = dialog.set_directory(&p.root);
                    }
                    if let Some(path) = dialog.save_file() {
                        self.save_as(i, &path);
                    }
                } else {
                    self.save_tab(i);
                }
            }
            SaveAll => {
                self.save_all();
            }
            CloseTab => {
                let i = self.active;
                self.close_tab(i);
            }
            CloseOtherTabs => {
                let keep = self.active;
                let others: Vec<usize> = (0..self.tabs.len()).filter(|i| *i != keep).collect();
                self.close_many(others);
            }
            CloseAllTabs => {
                let all: Vec<usize> = (0..self.tabs.len()).collect();
                self.close_many(all);
            }
            ReopenClosedTab => {
                while let Some(path) = self.closed.pop() {
                    if path.is_file() {
                        self.open_path(&path, None, true);
                        break;
                    }
                }
            }
            RevealInFiles => {
                if let Some(path) = self
                    .active_doc()
                    .and_then(|d| d.path().map(Path::to_path_buf))
                {
                    if let Some(dir) = path.parent() {
                        ctx.open_url(egui::OpenUrl::new_tab(format!("file://{}", dir.display())));
                    }
                }
            }
            Quit => self.request_quit(ctx),

            Undo => self.edit(|b| b.undo()),
            Redo => self.edit(|b| b.redo()),
            Cut | Copy => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    let text = tab.doc.buffer().selected_text(true);
                    ctx.copy_text(text);
                    if cmd == Cut {
                        if tab.doc.buffer().selections().iter().all(|s| s.is_empty()) {
                            tab.doc.apply(codigo_text::Buffer::delete_lines);
                        } else {
                            tab.doc.apply(|b| b.insert_text(""));
                        }
                    }
                }
            }
            Paste => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            Find => self.open_find(false),
            Replace => self.open_find(true),
            AnalyzeProject => {
                if self.project.is_some() {
                    self.panel = Panel::Quality;
                    self.sidebar_visible = true;
                    self.run_quality();
                } else {
                    self.toast(tr("Open a folder first."));
                }
            }
            FindNext => self.find_step(true),
            FindPrevious => self.find_step(false),
            FindInProject => {
                if self.project.is_some() {
                    self.panel = Panel::Search;
                    self.sidebar_visible = true;
                    self.psearch.focus = true;
                    let selected = self
                        .active_tab()
                        .map(|t| t.doc.buffer().selected_text(false))
                        .filter(|s| !s.is_empty() && !s.contains('\n'));
                    if let Some(s) = selected {
                        self.psearch.query.pattern = s;
                        self.run_project_search();
                    }
                } else {
                    self.toast(tr("Open a folder first."));
                }
            }
            ToggleLineComment => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    if let Some(token) = tab.doc.language().line_comment {
                        tab.doc.apply(|b| b.toggle_line_comment(token));
                    } else {
                        self.toast(tr("This language has no line comments."));
                    }
                }
            }
            Indent => self.edit(|b| b.indent()),
            Outdent => self.edit(|b| b.outdent()),
            MoveLineUp => self.edit(|b| b.move_lines(true)),
            MoveLineDown => self.edit(|b| b.move_lines(false)),
            DuplicateLineUp => self.edit(|b| b.duplicate_lines(true)),
            DuplicateLineDown => self.edit(|b| b.duplicate_lines(false)),
            DeleteLine => self.edit(|b| b.delete_lines()),
            JoinLines => self.edit(|b| b.join_lines()),
            OpenLineBelow => self.edit(|b| b.open_line(true)),
            OpenLineAbove => self.edit(|b| b.open_line(false)),
            TrimTrailingWhitespace => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.doc.tidy(true, false);
                }
            }

            SelectAll => self.select(|b| b.select_all()),
            SelectLine => self.select(|b| b.select_lines()),
            SelectNextOccurrence => self.select(|b| b.select_next_occurrence()),
            SelectAllOccurrences => self.select(|b| b.select_all_occurrences()),
            AddCursorAbove => self.select(|b| b.add_cursor_vertical(true)),
            AddCursorBelow => self.select(|b| b.add_cursor_vertical(false)),
            CollapseSelections => self.select(|b| b.collapse_selections()),

            CommandPalette => self.overlay = Overlay::palette(),
            ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                self.session_dirty = true;
            }
            ToggleWordWrap => {
                self.settings.word_wrap = !self.settings.word_wrap;
                self.save_settings();
            }
            ToggleWhitespace => {
                self.settings.show_whitespace = !self.settings.show_whitespace;
                self.save_settings();
            }
            ZoomIn => self.zoom(1.0),
            ZoomOut => self.zoom(-1.0),
            ZoomReset => {
                self.settings.font_size = 14.0;
                self.save_settings();
            }
            ShowProblems => {
                self.sidebar_visible = true;
                self.panel = Panel::Problems;
            }
            FocusEditor => self.focus_editor = true,
            FocusExplorer => {
                if self.project.is_some() {
                    self.sidebar_visible = true;
                    self.panel = Panel::Explorer;
                } else {
                    self.toast(tr("Open a folder first."));
                }
            }
            ChangeLanguage => {
                if self.active_tab().is_some() {
                    self.overlay = Overlay::language();
                }
            }

            QuickOpen => {
                if self.project.is_some() {
                    self.overlay = Overlay::quick_open();
                } else {
                    self.toast(tr("Open a folder first."));
                }
            }
            NewNodaliaApp => self.overlay = Overlay::NewApp(crate::nodalia_ui::NewApp::new(self)),
            GoToLine => {
                if self.active_tab().is_some() {
                    self.overlay = Overlay::go_to_line();
                }
            }
            GoToDefinition => crate::lsp_ui::ask(self, codigo_engine::QueryKind::Definition, false),
            ShowHover => crate::lsp_ui::ask(self, codigo_engine::QueryKind::Hover, false),
            TriggerCompletion => {
                crate::lsp_ui::ask(self, codigo_engine::QueryKind::Completion, false)
            }
            ToggleTerminal => crate::terminal_ui::toggle(self, ctx),
            NewTerminal => crate::terminal_ui::new_shell(self, ctx),
            CloseTerminal => crate::terminal_ui::close_active(self),
            RunProject => crate::terminal_ui::run_task(self, ctx, codigo_engine::TaskKind::Run),
            BuildProject => crate::terminal_ui::run_task(self, ctx, codigo_engine::TaskKind::Build),
            TestProject => crate::terminal_ui::run_task(self, ctx, codigo_engine::TaskKind::Test),
            CheckProject => crate::terminal_ui::run_task(self, ctx, codigo_engine::TaskKind::Check),
            StopRun => crate::terminal_ui::stop(self),
            NextTab => {
                if !self.tabs.is_empty() {
                    self.active = (self.active + 1) % self.tabs.len();
                    self.focus_editor = true;
                    self.session_dirty = true;
                }
            }
            PreviousTab => {
                if !self.tabs.is_empty() {
                    self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
                    self.focus_editor = true;
                    self.session_dirty = true;
                }
            }

            Preferences => self.overlay = Overlay::Preferences,
            KeyboardShortcuts => self.overlay = Overlay::Shortcuts(String::new()),
            About => self.overlay = Overlay::About,
        }
    }

    fn edit(&mut self, f: impl FnOnce(&mut codigo_text::Buffer) -> Vec<codigo_text::Change>) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.doc.apply(f);
            tab.view.reveal = true;
            self.last_edit = Some(Instant::now());
        }
    }

    fn select(&mut self, f: impl FnOnce(&mut codigo_text::Buffer)) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.doc.navigate(f);
            tab.view.reveal = true;
        }
    }

    fn zoom(&mut self, delta: f32) {
        self.settings.font_size = (self.settings.font_size + delta).clamp(
            codigo_engine::settings::MIN_FONT_SIZE,
            codigo_engine::settings::MAX_FONT_SIZE,
        );
        self.save_settings();
    }

    pub fn request_quit(&mut self, ctx: &egui::Context) {
        let dirty = self.dirty_tabs();
        if dirty.is_empty() {
            self.save_session();
            self.ignore_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.overlay = Overlay::Confirm(crate::overlays::Confirm::Quit);
        }
    }

    pub fn quit_now(&mut self, ctx: &egui::Context) {
        self.save_session();
        self.ignore_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    // --- Find ----------------------------------------------------------------------------------

    pub fn open_find(&mut self, replace: bool) {
        if self.active_tab().is_none() {
            return;
        }
        self.find.open = true;
        self.find.replace = replace || self.find.replace && self.find.open;
        self.find.focus = true;
        let selected = self
            .active_tab()
            .map(|t| t.doc.buffer().selected_text(false))
            .filter(|s| !s.is_empty() && !s.contains('\n'));
        if let Some(s) = selected {
            self.find.query.pattern = s;
        }
    }

    pub fn close_find(&mut self) {
        self.find.open = false;
        self.focus_editor = true;
    }

    /// Bring the find bar's matcher and counts up to date with its query and the text.
    pub fn refresh_find(&mut self) {
        if !self.find.open {
            self.find.matcher = None;
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let stamp = (
            tab.doc.id(),
            tab.doc.buffer().revision(),
            self.find.query.clone(),
        );
        let sel = tab.doc.buffer().primary();
        if self.find.stamp == stamp && self.find.matcher.is_some() == !stamp.2.pattern.is_empty() {
            // Only the current match can have moved.
            self.find.current = None;
            if let Some(m) = &self.find.matcher {
                let all = m.find_all(tab.doc.buffer().rope(), 100_000);
                self.find.current = all
                    .iter()
                    .position(|f| f.start == sel.start() && f.end == sel.end());
            }
            return;
        }
        match codigo_text::search::Matcher::new(&self.find.query) {
            Ok(m) => {
                let all = m.find_all(tab.doc.buffer().rope(), 100_000);
                self.find.count = all.len();
                self.find.current = all
                    .iter()
                    .position(|f| f.start == sel.start() && f.end == sel.end());
                self.find.error = None;
                self.find.matcher = Some(m);
            }
            Err(e) => {
                self.find.matcher = None;
                self.find.count = 0;
                self.find.current = None;
                self.find.error = (!e.is_empty()).then_some(e);
            }
        }
        self.find.stamp = stamp;
    }

    /// Select the next or previous match, wrapping around.
    pub fn find_step(&mut self, forward: bool) {
        if self.find.query.pattern.is_empty() {
            self.open_find(false);
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Ok(matcher) = codigo_text::search::Matcher::new(&self.find.query) else {
            return;
        };
        let all = matcher.find_all(tab.doc.buffer().rope(), 100_000);
        if all.is_empty() {
            return;
        }
        let sel = tab.doc.buffer().primary();
        let pick = if forward {
            all.iter()
                .find(|m| m.start >= sel.end())
                .or_else(|| all.first())
        } else {
            all.iter()
                .rev()
                .find(|m| m.end <= sel.start())
                .or_else(|| all.last())
        };
        if let Some(m) = pick.copied() {
            tab.doc.navigate(|b| {
                b.set_selections(codigo_text::Selections::single(Selection::new(
                    m.start, m.end,
                )));
            });
            tab.view.reveal = true;
        }
    }

    pub fn replace_current(&mut self) {
        let with = self.find.with.clone();
        let regex = self.find.query.regex;
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Ok(matcher) = codigo_text::search::Matcher::new(&self.find.query) else {
            return;
        };
        let sel = tab.doc.buffer().primary();
        let all = matcher.find_all(tab.doc.buffer().rope(), 100_000);
        if let Some(m) = all
            .iter()
            .find(|m| m.start == sel.start() && m.end == sel.end())
            .copied()
        {
            let text = matcher.replacement(tab.doc.buffer().rope(), m, &with, regex);
            tab.doc.apply(|b| {
                b.apply_edits(vec![codigo_text::Edit::new(m.start..m.end, text)])
                    .unwrap_or_default()
            });
        }
        self.find_step(true);
    }

    pub fn replace_all(&mut self) {
        let with = self.find.with.clone();
        let regex = self.find.query.regex;
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Ok(matcher) = codigo_text::search::Matcher::new(&self.find.query) else {
            return;
        };
        let all = matcher.find_all(tab.doc.buffer().rope(), 1_000_000);
        if all.is_empty() {
            return;
        }
        let count = all.len();
        let edits = matcher.replace_edits(tab.doc.buffer().rope(), &all, &with, regex);
        tab.doc.apply(|b| b.apply_edits(edits).unwrap_or_default());
        self.toast(trf("Replaced {} matches.", &[&count.to_string()]));
    }

    // --- Frame ---------------------------------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context, editor_has_focus: bool) {
        if !matches!(self.overlay, Overlay::None) {
            return;
        }
        let text_focus = ctx.egui_wants_keyboard_input() && !editor_has_focus;
        let mut commands: Vec<Cmd> = Vec::new();
        // Most specific first: egui matches a shortcut even when Shift or Alt are also held.
        let mut table: Vec<(Cmd, codigo_engine::Shortcut)> = Cmd::ALL
            .iter()
            .filter_map(|c| c.shortcut().map(|s| (*c, s)))
            .collect();
        table.sort_by_key(|(_, s)| std::cmp::Reverse(u8::from(s.shift) + u8::from(s.alt)));
        ctx.input_mut(|i| {
            for (cmd, s) in &table {
                let text_only = matches!(
                    cmd,
                    Cmd::Undo
                        | Cmd::Redo
                        | Cmd::Cut
                        | Cmd::Copy
                        | Cmd::Paste
                        | Cmd::SelectAll
                        | Cmd::SelectLine
                        | Cmd::Indent
                        | Cmd::Outdent
                        | Cmd::MoveLineUp
                        | Cmd::MoveLineDown
                        | Cmd::CollapseSelections
                );
                if text_focus && text_only {
                    continue;
                }
                if *cmd == Cmd::CollapseSelections {
                    // Escape closes things first; only collapse when nothing else wants it.
                    continue;
                }
                let mut mods = Modifiers::NONE;
                mods.shift = s.shift;
                mods.alt = s.alt;
                if s.primary {
                    mods.command = true;
                }
                if let Some(key) = to_egui_key(s.key) {
                    if i.consume_key(mods, key) {
                        commands.push(*cmd);
                    }
                }
            }
            // Zoom in is "Ctrl+=" on most keyboards.
            if i.consume_key(Modifiers::COMMAND, Key::Equals) {
                commands.push(Cmd::ZoomIn);
            }
        });
        for cmd in commands {
            self.run(ctx, cmd);
        }
        // Escape: close the find bar, otherwise collapse to one cursor.
        if ctx.input(|i| i.key_pressed(Key::Escape)) && !text_focus {
            if self.find.open {
                self.close_find();
            } else if editor_has_focus {
                self.run(ctx, Cmd::CollapseSelections);
            }
        } else if ctx.input(|i| i.key_pressed(Key::Escape)) && self.find.open {
            self.close_find();
        }
    }

    fn external_changes(&mut self) {
        if self.last_disk_check.elapsed() < Duration::from_millis(1500) {
            return;
        }
        self.last_disk_check = Instant::now();
        let mut toasts = Vec::new();
        let mut reloaded = None;
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if !tab.doc.conflict && tab.doc.changed_on_disk() {
                if tab.doc.is_dirty() {
                    tab.doc.conflict = true;
                } else if tab.doc.reload().is_ok() {
                    reloaded = tab.doc.path().map(Path::to_path_buf);
                    toasts.push(tr("The file changed on disk and was reloaded."));
                }
            }
        }
        if let Some(path) = reloaded {
            self.load_marks(path);
        }
        for t in toasts {
            self.toast(t);
        }
    }

    fn autosave(&mut self, ctx: &egui::Context, focus_lost: bool) {
        match self.settings.auto_save {
            AutoSave::Off => {}
            AutoSave::OnFocusLoss => {
                if focus_lost {
                    let dirty: Vec<usize> = self
                        .dirty_tabs()
                        .into_iter()
                        .filter(|i| self.tabs.get(*i).is_some_and(|t| t.doc.path().is_some()))
                        .collect();
                    for i in dirty {
                        self.save_tab(i);
                    }
                }
            }
            AutoSave::AfterDelay => {
                let delay = Duration::from_millis(self.settings.auto_save_delay_ms);
                if let Some(at) = self.last_edit {
                    if at.elapsed() >= delay {
                        self.last_edit = None;
                        let dirty: Vec<usize> = self
                            .dirty_tabs()
                            .into_iter()
                            .filter(|i| self.tabs.get(*i).is_some_and(|t| t.doc.path().is_some()))
                            .collect();
                        for i in dirty {
                            self.save_tab(i);
                        }
                    } else {
                        ctx.request_repaint_after(delay - at.elapsed());
                    }
                }
            }
        }
    }

    pub fn editor_options(&self) -> editor::Options {
        editor::Options {
            font_size: self.settings.font_size,
            word_wrap: self.settings.word_wrap,
            line_numbers: self.settings.line_numbers,
            highlight_line: self.settings.highlight_active_line,
            whitespace: self.settings.show_whitespace,
            auto_close_pairs: self.settings.auto_close_pairs,
        }
    }

    /// Carry out what the editor asked for.
    pub fn editor_output(&mut self, ctx: &egui::Context, out: Output) {
        if let Some(text) = out.copied {
            ctx.copy_text(text);
        }
        if out.edited {
            self.last_edit = Some(Instant::now());
        }
        crate::lsp_ui::update(self, out.edited);
        for cmd in out.commands {
            self.run(ctx, cmd);
        }
    }
}

pub(crate) fn to_egui_key(key: codigo_engine::Key) -> Option<Key> {
    use codigo_engine::Key as K;
    Some(match key {
        K::A => Key::A,
        K::B => Key::B,
        K::C => Key::C,
        K::D => Key::D,
        K::E => Key::E,
        K::F => Key::F,
        K::G => Key::G,
        K::H => Key::H,
        K::I => Key::I,
        K::J => Key::J,
        K::K => Key::K,
        K::L => Key::L,
        K::M => Key::M,
        K::N => Key::N,
        K::O => Key::O,
        K::P => Key::P,
        K::Q => Key::Q,
        K::R => Key::R,
        K::S => Key::S,
        K::T => Key::T,
        K::U => Key::U,
        K::V => Key::V,
        K::W => Key::W,
        K::X => Key::X,
        K::Y => Key::Y,
        K::Z => Key::Z,
        K::Num0 => Key::Num0,
        K::Num1 => Key::Num1,
        K::Num2 => Key::Num2,
        K::Num3 => Key::Num3,
        K::Num4 => Key::Num4,
        K::Num5 => Key::Num5,
        K::Num6 => Key::Num6,
        K::Num7 => Key::Num7,
        K::Num8 => Key::Num8,
        K::Num9 => Key::Num9,
        K::F1 => Key::F1,
        K::F2 => Key::F2,
        K::F3 => Key::F3,
        K::F4 => Key::F4,
        K::F5 => Key::F5,
        K::F6 => Key::F6,
        K::F7 => Key::F7,
        K::F8 => Key::F8,
        K::F9 => Key::F9,
        K::F10 => Key::F10,
        K::F11 => Key::F11,
        K::F12 => Key::F12,
        K::Enter => Key::Enter,
        K::Escape => Key::Escape,
        K::Tab => Key::Tab,
        K::Backspace => Key::Backspace,
        K::Delete => Key::Delete,
        K::Space => Key::Space,
        K::Up => Key::ArrowUp,
        K::Down => Key::ArrowDown,
        K::Left => Key::ArrowLeft,
        K::Right => Key::ArrowRight,
        K::Home => Key::Home,
        K::End => Key::End,
        K::PageUp => Key::PageUp,
        K::PageDown => Key::PageDown,
        K::Plus => Key::Plus,
        K::Minus => Key::Minus,
        K::Comma => Key::Comma,
        K::Period => Key::Period,
        K::Slash => Key::Slash,
        K::Backslash => Key::Backslash,
        K::Backtick => Key::Backtick,
        K::OpenBracket => Key::OpenBracket,
        K::CloseBracket => Key::CloseBracket,
        K::Semicolon => Key::Semicolon,
        K::Quote => Key::Quote,
    })
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_appearance(&ctx);
        self.updates.tick(&ctx);
        self.poll();
        self.sync_language_servers();
        crate::lsp_ui::poll(self);
        self.external_changes();

        let decorated = !self.ndl_controls();
        if self.decorated != Some(decorated) {
            if self.decorated.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(decorated));
            }
            self.decorated = Some(decorated);
        }

        // Closing the window asks about unsaved work first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.ignore_close {
            if !self.dirty_tabs().is_empty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.overlay = Overlay::Confirm(crate::overlays::Confirm::Quit);
            } else {
                self.save_session();
            }
        }

        let editor_focus = self.tabs.get(self.active).is_some_and(|t| {
            ctx.memory(|m| m.has_focus(egui::Id::unique(("codigo-editor", t.doc.id().0))))
        });
        crate::lsp_ui::keys(self, &ctx);
        crate::terminal_ui::input(self, &ctx);
        self.shortcuts(&ctx, editor_focus);
        self.refresh_find();

        ui::show(ui, self);

        #[cfg(target_os = "linux")]
        if let Some(menu) = &self.menu {
            let chosen = menu.take();
            for id in chosen {
                if id == crate::menu::UPDATES {
                    self.updates.start(true);
                } else if let Some(cmd) = Cmd::from_id(id) {
                    self.run(&ctx, cmd);
                }
            }
        }

        let focus_lost = ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, Event::WindowFocused(false)))
        });
        self.autosave(&ctx, focus_lost);
        let idle = self.dirty_tabs().is_empty();
        self.updates.show(&ctx, idle);

        let now = ctx.input(|i| i.time);
        self.toasts.retain(|t| t.until > now);
        if self.session_dirty && self.last_session.elapsed() > Duration::from_secs(2) {
            self.save_session();
        }
        ctx.request_repaint_after(Duration::from_millis(1500));
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.window_fill.to_normalized_gamma_f32()
    }
}
