// SPDX-License-Identifier: MIT OR Apache-2.0
//! Source control in the window: the Git panel (changes, staging, commit, branches, push and
//! pull) and the overlays it opens (a file's diff, the branch picker). Git runs on worker
//! threads and answers with [`GitMsg`]; nothing here waits for it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use codigo_git::{Auth, Branch, Change, Commit, Kind, Repo, RepoStatus};
use egui::{Align, Align2, FontId, Layout, RichText, ScrollArea, Sense, Ui};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::{App, Msg};
use crate::i18n::{tr, trf};
use crate::overlays::{heading, modal, Overlay, Shown};
use crate::setup_ui::work_root;
use crate::widgets::small_icon_button;

/// How long the panel trusts what it last read before asking git again.
const REFRESH_EVERY: Duration = Duration::from_secs(4);

/// Answers from the worker threads.
pub enum GitMsg {
    Status {
        serial: u64,
        dir: PathBuf,
        repo: Option<PathBuf>,
        remote: Option<(String, String)>,
        result: Result<RepoStatus, String>,
        log: Vec<Commit>,
    },
    Branches(Vec<Branch>),
    Diff {
        title: String,
        result: Result<String, String>,
    },
    Done {
        what: &'static str,
        result: Result<String, String>,
    },
}

#[derive(Default)]
pub struct GitPanel {
    /// The top of the repository the open folder is in.
    pub root: Option<PathBuf>,
    /// The folder the state below was read for.
    pub for_dir: Option<PathBuf>,
    pub known: bool,
    pub status: Option<RepoStatus>,
    pub log: Vec<Commit>,
    pub branches: Vec<Branch>,
    pub error: Option<String>,
    pub message: String,
    pub busy: Option<&'static str>,
    pub serial: u64,
    pub last: Option<Instant>,
    /// The file whose discard is waiting for a second click.
    pub confirm_discard: Option<String>,
    pub show_log: bool,
}

/// A diff in a window of its own.
pub struct DiffView {
    pub title: String,
    /// `None` while git is still producing it.
    pub text: Option<Result<String, String>>,
    /// A file to open from the window, when the diff is of one on disk.
    pub open: Option<PathBuf>,
}

/// Picking, or making, a branch.
#[derive(Default)]
pub struct BranchSetup {
    pub query: String,
    pub focus: bool,
}

impl BranchSetup {
    pub fn new() -> BranchSetup {
        BranchSetup {
            query: String::new(),
            focus: true,
        }
    }
}

pub fn auth(app: &App) -> Auth {
    match &app.gh.token {
        Some(t) => Auth::GitHubToken(t.clone()),
        None => Auth::None,
    }
}

fn spawn(app: &App, work: impl FnOnce() -> GitMsg + Send + 'static) {
    let tx = app.sender();
    let ctx = app.egui_ctx().clone();
    let _ = std::thread::Builder::new()
        .name("codigo-git".into())
        .spawn(move || {
            let _ = tx.send(Msg::Vcs(work()));
            ctx.request_repaint();
        });
}

/// Read the repository again.
pub fn refresh(app: &mut App) {
    let Some(dir) = work_root(app) else {
        app.vcs = GitPanel::default();
        return;
    };
    app.vcs.serial += 1;
    app.vcs.last = Some(Instant::now());
    let serial = app.vcs.serial;
    spawn(app, move || {
        let Some(repo) = Repo::discover(&dir) else {
            return GitMsg::Status {
                serial,
                dir,
                repo: None,
                remote: None,
                result: Err(String::new()),
                log: Vec::new(),
            };
        };
        let remote = repo
            .remote_url("origin")
            .and_then(|u| codigo_github::parse_remote(&u));
        let result = repo.status().map_err(|e| e.to_string());
        let log = repo.log(30).unwrap_or_default();
        GitMsg::Status {
            serial,
            dir,
            repo: Some(repo.root().to_path_buf()),
            remote,
            result,
            log,
        }
    });
}

/// Run something in the repository, then refresh.
fn op(
    app: &mut App,
    what: &'static str,
    work: impl FnOnce(&Repo, &Auth) -> codigo_git::Result<String> + Send + 'static,
) {
    let Some(root) = app.vcs.root.clone() else {
        app.toast(tr("This folder is not a git repository."));
        return;
    };
    if app.vcs.busy.is_some() {
        app.toast(tr("Git is busy; wait for it to finish."));
        return;
    }
    app.vcs.busy = Some(what);
    let auth = auth(app);
    spawn(app, move || {
        let result = match Repo::discover(&root) {
            Some(repo) => work(&repo, &auth).map_err(|e| e.to_string()),
            None => Err("the repository is gone".to_owned()),
        };
        GitMsg::Done { what, result }
    });
}

pub fn on_msg(app: &mut App, msg: GitMsg) {
    match msg {
        GitMsg::Status {
            serial,
            dir,
            repo,
            remote,
            result,
            log,
        } => {
            if serial != app.vcs.serial {
                return;
            }
            app.vcs.known = true;
            app.vcs.for_dir = Some(dir);
            app.vcs.root = repo;
            app.gh.repo = remote;
            match result {
                Ok(status) => {
                    app.vcs.status = Some(status);
                    app.vcs.error = None;
                }
                Err(e) => {
                    app.vcs.status = None;
                    app.vcs.error = (!e.is_empty()).then_some(e);
                }
            }
            app.vcs.log = log;
        }
        GitMsg::Branches(list) => app.vcs.branches = list,
        GitMsg::Diff { title, result } => {
            if let Overlay::Diff(d) = &mut app.overlay {
                if d.title == title {
                    d.text = Some(result);
                }
            }
        }
        GitMsg::Done { what, result } => {
            app.vcs.busy = None;
            match result {
                Ok(text) => {
                    let text = if text.is_empty() {
                        tr(what).to_owned()
                    } else {
                        text
                    };
                    app.toast(text);
                }
                Err(e) => app.toast(first_line(&e)),
            }
            refresh(app);
            app.refresh_git();
        }
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(text)
        .trim()
        .to_owned()
}

// --- Commands ---------------------------------------------------------------------------------

pub fn show(app: &mut App) {
    app.sidebar_visible = true;
    app.panel = crate::app::Panel::Git;
    refresh(app);
}

pub fn commit(app: &mut App) {
    let message = app.vcs.message.clone();
    if message.trim().is_empty() {
        app.toast(tr("Write a commit message first."));
        return;
    }
    if !app
        .vcs
        .status
        .as_ref()
        .is_some_and(|s| s.staged().next().is_some())
    {
        app.toast(tr("Stage the changes to commit first."));
        return;
    }
    app.save_all();
    app.vcs.message.clear();
    op(app, "Committed", move |repo, _| {
        repo.commit(&message)
            .map(|short| format!("{} {short}", tr("Committed")))
    });
}

pub fn stage_all(app: &mut App) {
    op(app, "Staged everything", |repo, _| {
        repo.stage_all().map(|()| String::new())
    });
}

pub fn pull(app: &mut App) {
    op(app, "Pulled", |repo, auth| {
        repo.pull(auth).map(|()| String::new())
    });
}

pub fn push(app: &mut App) {
    op(app, "Pushed", |repo, auth| {
        repo.push(auth).map(|()| String::new())
    });
}

pub fn fetch(app: &mut App) {
    op(app, "Fetched", |repo, auth| {
        repo.fetch(auth).map(|()| String::new())
    });
}

pub fn init(app: &mut App) {
    let Some(dir) = work_root(app) else {
        app.toast(tr("Open a folder first."));
        return;
    };
    app.vcs.busy = Some("Initialised");
    spawn(app, move || GitMsg::Done {
        what: "Initialised",
        result: Repo::init(&dir)
            .map(|_| String::new())
            .map_err(|e| e.to_string()),
    });
}

fn switch(app: &mut App, name: String) {
    op(app, "Switched branch", move |repo, _| {
        repo.checkout(&name).map(|()| String::new())
    });
}

fn create(app: &mut App, name: String) {
    op(app, "Created the branch", move |repo, _| {
        repo.create_branch(&name).map(|()| String::new())
    });
}

/// Fetch a pull request's branch and switch to it.
pub fn checkout_pull(app: &mut App, number: u64) {
    op(app, "Checked out the pull request", move |repo, auth| {
        let local = format!("codigo-pr-{number}");
        let spec = format!("pull/{number}/head:{local}");
        repo.fetch_ref(auth, &spec)?;
        repo.checkout(&local).map(|()| String::new())
    });
}

pub fn open_branches(app: &mut App) {
    if app.vcs.root.is_none() {
        app.toast(tr("This folder is not a git repository."));
        return;
    }
    if let Some(root) = app.vcs.root.clone() {
        spawn(app, move || {
            GitMsg::Branches(
                Repo::discover(&root)
                    .and_then(|r| r.branches().ok())
                    .unwrap_or_default(),
            )
        });
    }
    app.overlay = Overlay::Branch(BranchSetup::new());
}

fn open_diff(app: &mut App, change: &Change, staged: bool) {
    let Some(root) = app.vcs.root.clone() else {
        return;
    };
    let title = if staged {
        format!("{} ({})", change.path, tr("staged"))
    } else {
        change.path.clone()
    };
    app.overlay = Overlay::Diff(DiffView {
        title: title.clone(),
        text: None,
        open: Some(root.join(&change.path)),
    });
    let change = change.clone();
    spawn(app, move || GitMsg::Diff {
        title,
        result: match Repo::discover(&root) {
            Some(repo) => repo.diff(&change, staged).map_err(|e| e.to_string()),
            None => Err("the repository is gone".to_owned()),
        },
    });
}

// --- The panel --------------------------------------------------------------------------------

fn kind_style(look: &Look, kind: Kind) -> (&'static str, egui::Color32) {
    let p = &look.palette;
    match kind {
        Kind::Modified => ("M", color(p.status_warning_fg)),
        Kind::Added => ("A", color(p.status_success_fg)),
        Kind::Untracked => ("U", color(p.status_success_fg)),
        Kind::Deleted => ("D", color(p.status_danger_fg)),
        Kind::Renamed => ("R", color(p.status_info_fg)),
        Kind::Conflicted => ("!", color(p.status_danger_fg)),
    }
}

pub fn section(ui: &mut Ui, look: &Look, text: &str) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(text)
            .strong()
            .size(font_size::SMALL)
            .color(color(look.palette.text_secondary)),
    );
    ui.add_space(2.0);
}

/// What the panel asks the window to do.
#[derive(Default)]
pub struct Outcome {
    pub open: Option<PathBuf>,
}

pub fn panel(ui: &mut Ui, app: &mut App, look: &Look) -> Outcome {
    let mut out = Outcome::default();
    let p = look.palette;
    let dir = work_root(app);
    if dir.is_none() {
        ui.label(
            RichText::new(tr("Open a folder to see its changes."))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
        );
        return out;
    }
    let stale = app.vcs.for_dir != dir || app.vcs.last.is_none_or(|t| t.elapsed() > REFRESH_EVERY);
    if stale && app.vcs.busy.is_none() {
        refresh(app);
    }
    ui.ctx().request_repaint_after(REFRESH_EVERY);

    if !app.vcs.known {
        ui.label(RichText::new(tr("Reading the repository…")).color(color(p.text_secondary)));
        return out;
    }
    if app.vcs.root.is_none() {
        not_a_repo(ui, app, look);
        return out;
    }
    if let Some(e) = app.vcs.error.clone() {
        ui.label(RichText::new(first_line(&e)).color(color(p.status_danger_fg)));
    }
    let Some(status) = app.vcs.status.clone() else {
        return out;
    };
    toolbar(ui, app, look, &status);
    ui.add_space(6.0);
    commit_box(ui, app, look, &status);
    if app.vcs.busy.is_some() {
        ui.add_space(4.0);
        ui.label(
            RichText::new(tr("Git is working…"))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
        );
    }
    ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt("git-panel")
        .show(ui, |ui| {
            let staged: Vec<&Change> = status.staged().collect();
            let unstaged: Vec<&Change> = status.unstaged().collect();
            if !staged.is_empty() {
                section(
                    ui,
                    look,
                    &trf("Staged changes ({})", &[&staged.len().to_string()]),
                );
                for c in staged {
                    change_row(ui, app, look, c, true, &mut out);
                }
            }
            if !unstaged.is_empty() {
                section(
                    ui,
                    look,
                    &trf("Changes ({})", &[&unstaged.len().to_string()]),
                );
                for c in unstaged {
                    change_row(ui, app, look, c, false, &mut out);
                }
            }
            if status.changes.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(tr("Nothing to commit. The working tree is clean."))
                        .size(font_size::SMALL)
                        .color(color(p.text_secondary)),
                );
            }
            history(ui, app, look);
        });
    out
}

fn not_a_repo(ui: &mut Ui, app: &mut App, look: &Look) {
    ui.label(
        RichText::new(tr("This folder is not a git repository."))
            .color(color(look.palette.text_secondary)),
    );
    ui.add_space(8.0);
    if cr::primary_button(ui, look, tr("Initialise repository"), true).clicked() {
        init(app);
    }
    ui.add_space(4.0);
    if cr::secondary_button(ui, look, tr("Clone a repository…"), true).clicked() {
        app.overlay = Overlay::Clone(crate::github_ui::CloneSetup::new(app, String::new()));
    }
}

fn toolbar(ui: &mut Ui, app: &mut App, look: &Look, status: &RepoStatus) {
    let p = look.palette;
    ui.horizontal(|ui| {
        let name = status
            .branch
            .clone()
            .unwrap_or_else(|| tr("(detached)").to_owned());
        if ui
            .add(
                egui::Button::new(
                    RichText::new(format!("⎇ {name}"))
                        .strong()
                        .color(color(p.text_primary)),
                )
                .truncate(),
            )
            .on_hover_text(tr("Switch or create a branch"))
            .clicked()
        {
            open_branches(app);
        }
        crate::widgets::trailing(ui, |ui| {
            if small_icon_button(ui, look, "refresh", tr("Refresh"), false).clicked() {
                refresh(app);
            }
            if small_icon_button(ui, look, "arrow-up", tr("Push"), status.ahead > 0).clicked() {
                push(app);
            }
            if small_icon_button(ui, look, "arrow-down", tr("Pull"), status.behind > 0).clicked() {
                pull(app);
            }
        });
    });
    let mut bits = Vec::new();
    if status.ahead > 0 {
        bits.push(trf("{} to push", &[&status.ahead.to_string()]));
    }
    if status.behind > 0 {
        bits.push(trf("{} to pull", &[&status.behind.to_string()]));
    }
    if status.upstream.is_none() {
        bits.push(tr("not published yet").to_owned());
    }
    if !bits.is_empty() {
        ui.label(
            RichText::new(bits.join(" · "))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
        );
    }
}

fn commit_box(ui: &mut Ui, app: &mut App, look: &Look, status: &RepoStatus) {
    let response = cr::field_frame(look)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut app.vcs.message)
                    .frame(egui::Frame::NONE)
                    .hint_text(tr("Message"))
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .font(FontId::proportional(font_size::BODY)),
            )
        })
        .inner;
    let staged = status.staged().count();
    let ready = staged > 0 && !app.vcs.message.trim().is_empty() && app.vcs.busy.is_none();
    let ctrl_enter = response.has_focus()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let label = if staged > 0 {
            trf("Commit ({})", &[&staged.to_string()])
        } else {
            tr("Commit").to_owned()
        };
        if cr::primary_button(ui, look, &label, ready).clicked() || (ctrl_enter && ready) {
            commit(app);
        }
        if !status.changes.is_empty()
            && cr::secondary_button(ui, look, tr("Stage all"), app.vcs.busy.is_none()).clicked()
        {
            stage_all(app);
        }
    });
}

fn change_row(
    ui: &mut Ui,
    app: &mut App,
    look: &Look,
    change: &Change,
    staged: bool,
    out: &mut Outcome,
) {
    let p = look.palette;
    let kind = if staged && change.index == Some('A') {
        Kind::Added
    } else {
        change.kind()
    };
    let (letter, tint) = kind_style(look, kind);
    let (dir, name) = match change.path.rsplit_once('/') {
        Some((d, n)) => (d.to_owned(), n.to_owned()),
        None => (String::new(), change.path.clone()),
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new(letter).monospace().strong().color(tint));
        let room = (ui.available_width() - 56.0).max(40.0);
        let label = ui
            .allocate_ui(egui::vec2(room, 20.0), |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&name)
                            .color(color(p.text_primary))
                            .size(font_size::BODY),
                    )
                    .truncate()
                    .sense(Sense::click()),
                )
            })
            .inner;
        let label = if dir.is_empty() {
            label
        } else {
            label.on_hover_text(&change.path)
        };
        if label.clicked() {
            open_diff(app, change, staged);
        }
        if label.double_clicked() {
            if let Some(root) = &app.vcs.root {
                out.open = Some(root.join(&change.path));
            }
        }
        if staged {
            if small_icon_button(ui, look, "minus", tr("Unstage"), false).clicked() {
                let path = vec![change.path.clone()];
                op(app, "Unstaged", move |repo, _| {
                    repo.unstage(&path).map(|()| String::new())
                });
            }
        } else {
            let armed = app.vcs.confirm_discard.as_deref() == Some(change.path.as_str());
            if small_icon_button(
                ui,
                look,
                if armed { "check" } else { "delete" },
                if armed {
                    tr("Click again to discard this file's changes")
                } else {
                    tr("Discard changes")
                },
                armed,
            )
            .clicked()
            {
                if armed {
                    app.vcs.confirm_discard = None;
                    let change = change.clone();
                    op(app, "Discarded", move |repo, _| {
                        repo.discard(&change).map(|()| String::new())
                    });
                } else {
                    app.vcs.confirm_discard = Some(change.path.clone());
                }
            }
            if small_icon_button(ui, look, "add", tr("Stage"), false).clicked() {
                let path = vec![change.path.clone()];
                op(app, "Staged", move |repo, _| {
                    repo.stage(&path).map(|()| String::new())
                });
            }
        }
    });
    if !dir.is_empty() {
        ui.label(
            RichText::new(dir)
                .size(font_size::SMALL)
                .color(color(p.text_tertiary)),
        );
    }
}

fn history(ui: &mut Ui, app: &mut App, look: &Look) {
    if app.vcs.log.is_empty() {
        return;
    }
    let p = look.palette;
    ui.add_space(8.0);
    let open = app.vcs.show_log;
    let head = ui.add(
        egui::Label::new(
            RichText::new(format!(
                "{} {}",
                if open { "▾" } else { "▸" },
                tr("History")
            ))
            .strong()
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
        )
        .sense(Sense::click()),
    );
    if head.clicked() {
        app.vcs.show_log = !open;
    }
    if !app.vcs.show_log {
        return;
    }
    for c in app.vcs.log.iter().take(30) {
        ui.add(
            egui::Label::new(
                RichText::new(format!("{}  {}", c.short, c.subject))
                    .size(font_size::SMALL)
                    .color(color(p.text_primary)),
            )
            .truncate(),
        )
        .on_hover_text(format!("{} · {}", c.author, c.hash));
    }
}

// --- Overlays ---------------------------------------------------------------------------------

pub fn diff_overlay(ctx: &egui::Context, app: &mut App, look: &Look, view: DiffView) -> Shown {
    let p = look.palette;
    let mut closed = false;
    let mut open = false;
    let dismissed = modal(ctx, look, "diff", 920.0, true, |ui| {
        closed = heading(ui, look, &view.title, "diff");
        match &view.text {
            None => {
                ui.label(RichText::new(tr("Reading the changes…")).color(color(p.text_secondary)));
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e).color(color(p.status_danger_fg)));
            }
            Some(Ok(text)) if text.trim().is_empty() => {
                ui.label(RichText::new(tr("No changes to show.")).color(color(p.text_secondary)));
            }
            Some(Ok(text)) => diff_text(
                ui,
                look,
                text,
                (ctx.content_rect().height() - 260.0).max(200.0),
            ),
        }
        if view.open.as_ref().is_some_and(|p| p.is_file()) {
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if cr::secondary_button(ui, look, tr("Open file"), true).clicked() {
                    open = true;
                }
            });
        }
    });
    if open {
        if let Some(path) = &view.open {
            app.open_path(path, None, true);
        }
        return (Overlay::None, true);
    }
    if closed || dismissed {
        return (Overlay::None, true);
    }
    (Overlay::Diff(view), false)
}

/// A unified diff, coloured by line, in a scroll area that only lays out what is visible.
pub fn diff_text(ui: &mut Ui, look: &Look, text: &str, max_height: f32) {
    let p = look.palette;
    let lines: Vec<&str> = text.lines().take(20_000).collect();
    let font = FontId::monospace(font_size::SMALL);
    let row = ui.fonts_mut(|f| f.row_height(&font));
    ScrollArea::both()
        .id_salt(("diff-scroll", lines.len()))
        .max_height(max_height)
        .auto_shrink([false, true])
        .show_rows(ui, row, lines.len(), |ui, range| {
            for line in lines.iter().skip(range.start).take(range.len()) {
                let tint = match line.chars().next() {
                    Some('+') if !line.starts_with("+++") => color(p.status_success_fg),
                    Some('-') if !line.starts_with("---") => color(p.status_danger_fg),
                    Some('@') => color(p.status_info_fg),
                    _ => color(p.text_secondary),
                };
                ui.add(
                    egui::Label::new(RichText::new(*line).font(font.clone()).color(tint)).extend(),
                );
            }
        });
}

pub fn branch_overlay(
    ctx: &egui::Context,
    app: &mut App,
    look: &Look,
    mut state: BranchSetup,
) -> Shown {
    let p = look.palette;
    let query = state.query.trim().to_owned();
    let lower = query.to_lowercase();
    let shown: Vec<Branch> = app
        .vcs
        .branches
        .iter()
        .filter(|b| lower.is_empty() || b.name.to_lowercase().contains(&lower))
        .take(30)
        .cloned()
        .collect();
    let exists = app.vcs.branches.iter().any(|b| b.name == query);
    let can_create = !query.is_empty() && !exists && !query.contains(char::is_whitespace);
    let mut pick: Option<String> = None;
    let mut make: Option<String> = None;
    let mut closed = false;
    let dismissed = modal(ctx, look, "branch", 460.0, true, |ui| {
        closed = heading(ui, look, tr("Switch branch"), "branch");
        let response = cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut state.query)
                        .frame(egui::Frame::NONE)
                        .hint_text(tr("Branch name"))
                        .align(Align2::LEFT_CENTER)
                        .desired_width(f32::INFINITY)
                        .font(FontId::proportional(font_size::LARGE)),
                )
            })
            .inner;
        if state.focus {
            response.request_focus();
            state.focus = false;
        }
        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        ui.add_space(6.0);
        ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            if can_create {
                let label = trf("Create branch “{}”", &[&query]);
                if ui
                    .selectable_label(false, RichText::new(label).color(color(p.accent_text)))
                    .clicked()
                {
                    make = Some(query.clone());
                }
            }
            for b in &shown {
                let text = if b.current {
                    format!("● {}", b.name)
                } else if b.remote {
                    format!("☁ {}", b.name)
                } else {
                    b.name.clone()
                };
                if ui.selectable_label(b.current, text).clicked() && !b.current {
                    pick = Some(b.name.clone());
                }
            }
            if shown.is_empty() && !can_create {
                ui.label(RichText::new(tr("No branches match.")).color(color(p.text_secondary)));
            }
        });
        if enter {
            if let Some(first) = shown.first().filter(|b| !b.current) {
                pick = Some(first.name.clone());
            } else if can_create {
                make = Some(query.clone());
            }
        }
    });
    if closed || dismissed {
        return (Overlay::None, true);
    }
    if let Some(name) = pick {
        switch(app, name);
        return (Overlay::None, true);
    }
    if let Some(name) = make {
        create(app, name);
        return (Overlay::None, true);
    }
    (Overlay::Branch(state), false)
}
