// SPDX-License-Identifier: MIT OR Apache-2.0
//! GitLab in the window: signing in with a personal access token, the panel with merge requests,
//! issues and projects, the window that shows one merge request or issue (with its diff and
//! conversation), creating a merge request, and cloning. Every call to GitLab runs on a worker
//! thread and comes back as a [`GlMsg`].
//!
//! The token lives in the system keyring (see `codigo_gitlab::secret`), one entry per server.
//! Where there is no keyring it is kept in memory for the session and never written anywhere else.
//! It is only ever sent to the one server it was made for.

use codigo_git::Auth;
use codigo_gitlab::{
    parse_remote, secret, valid_host, Api, Change, Issue, MergeRequest, NewMergeRequest, Note,
    Project, Scope, User, DEFAULT_HOST,
};
use egui::{Align2, FontId, RichText, ScrollArea, Ui};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::{App, Msg, Panel};
use crate::github_ui::{clip, date, first_line, row_frame, CloneSetup, Slot, SlotState};
use crate::i18n::{tr, trf};
use crate::overlays::{heading, modal, Overlay, Shown};
use crate::vcs_ui::section;
use crate::widgets::small_icon_button;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    /// Merge requests of the project the open folder belongs to.
    #[default]
    Merges,
    Issues,
    /// Opened by me, across GitLab.
    Mine,
    /// Waiting for my review, across GitLab.
    Review,
    Projects,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::Merges,
        Tab::Issues,
        Tab::Mine,
        Tab::Review,
        Tab::Projects,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Merges => "Merge requests",
            Tab::Issues => "Issues",
            Tab::Mine => "Mine",
            Tab::Review => "To review",
            Tab::Projects => "Projects",
        }
    }
}

#[derive(Default)]
pub struct GitlabPanel {
    /// The server the token and the lists belong to.
    pub host: String,
    pub token: Option<String>,
    pub user: Option<User>,
    /// The address of the open folder's `origin`, whatever server it is on.
    pub origin: Option<String>,
    /// The `group/project` path of `origin` when it is on this server.
    pub project: Option<String>,
    /// That project as GitLab describes it (its id is what the other calls need).
    pub project_info: Option<Project>,
    pub tab: Tab,
    pub merges: Slot<MergeRequest>,
    pub issues: Slot<Issue>,
    pub mine: Slot<MergeRequest>,
    pub review: Slot<MergeRequest>,
    pub projects: Slot<Project>,
    pub checking: bool,
    pub token_input: String,
    pub host_input: String,
    pub error: Option<String>,
    /// Whether the keyring was asked for a saved token yet.
    pub asked: bool,
}

/// Answers from the worker threads.
pub enum GlMsg {
    /// The saved token for a server, read from the keyring at startup.
    Saved(String, Option<String>),
    Checked {
        host: String,
        token: String,
        persist: bool,
        result: Result<User, String>,
    },
    /// The folder's project with its open merge requests and issues.
    ProjectLists {
        path: String,
        result: Result<Box<ProjectLists>, String>,
    },
    Mine(Result<Vec<MergeRequest>, String>),
    Review(Result<Vec<MergeRequest>, String>),
    Projects(Result<Vec<Project>, String>),
    Detail {
        project_id: u64,
        iid: u64,
        result: Result<Box<Loaded>, String>,
    },
    Posted {
        what: &'static str,
        project_id: u64,
        iid: u64,
        result: Result<(), String>,
    },
    /// The merge request that was just created (its address), or why not.
    MrCreated(Result<String, String>),
}

/// A project with its open merge requests and issues.
pub struct ProjectLists {
    pub project: Project,
    pub merges: Vec<MergeRequest>,
    pub issues: Vec<Issue>,
}

/// What a merge request or issue window loads beyond what the list already had.
#[derive(Clone, Default)]
pub struct Loaded {
    pub changes: Vec<Change>,
    pub notes: Vec<Note>,
    pub mr: Option<MergeRequest>,
}

/// The window of one merge request or issue.
pub struct ItemView {
    pub project_id: u64,
    pub iid: u64,
    pub is_mr: bool,
    pub reference: String,
    pub title: String,
    pub state: String,
    pub author: String,
    pub description: String,
    pub url: String,
    pub labels: Vec<String>,
    pub draft: bool,
    pub source: String,
    pub target: String,
    pub conflicts: bool,
    pub pipeline: Option<String>,
    pub loaded: Option<Result<Box<Loaded>, String>>,
    pub comment: String,
    /// The changed file whose diff is open.
    pub file: Option<usize>,
    pub busy: bool,
}

impl ItemView {
    fn from_mr(m: &MergeRequest) -> ItemView {
        ItemView {
            project_id: m.project_id,
            iid: m.iid,
            is_mr: true,
            reference: m.reference.clone(),
            title: m.title.clone(),
            state: m.state.clone(),
            author: m.author.clone(),
            description: m.description.clone(),
            url: m.web_url.clone(),
            labels: m.labels.clone(),
            draft: m.draft,
            source: m.source_branch.clone(),
            target: m.target_branch.clone(),
            conflicts: m.has_conflicts,
            pipeline: m.pipeline.clone(),
            loaded: None,
            comment: String::new(),
            file: None,
            busy: false,
        }
    }

    fn from_issue(i: &Issue) -> ItemView {
        ItemView {
            project_id: i.project_id,
            iid: i.iid,
            is_mr: false,
            reference: i.reference.clone(),
            title: i.title.clone(),
            state: i.state.clone(),
            author: i.author.clone(),
            description: i.description.clone(),
            url: i.web_url.clone(),
            labels: i.labels.clone(),
            draft: false,
            source: String::new(),
            target: String::new(),
            conflicts: false,
            pipeline: None,
            loaded: None,
            comment: String::new(),
            file: None,
            busy: false,
        }
    }
}

// --- Plumbing ---------------------------------------------------------------------------------

/// The server the settings name, or gitlab.com when that is not a plain server address.
pub fn configured_host(app: &App) -> String {
    let h = app.settings.gitlab_host.trim().to_lowercase();
    if valid_host(&h) {
        h
    } else {
        DEFAULT_HOST.to_owned()
    }
}

fn spawn(app: &App, work: impl FnOnce() -> GlMsg + Send + 'static) {
    let tx = app.sender();
    let ctx = app.egui_ctx().clone();
    let _ = std::thread::Builder::new()
        .name("codigo-gitlab".into())
        .spawn(move || {
            let _ = tx.send(Msg::Gl(work()));
            ctx.request_repaint();
        });
}

fn text(e: codigo_gitlab::Error) -> String {
    e.to_string()
}

fn api(app: &App) -> Option<Api> {
    let token = app.gl.token.clone()?;
    Api::new(&app.gl.host, token).ok()
}

/// The host of an https or ssh address, lower case.
fn url_host(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("ssh://"));
    let host = match rest {
        Some(r) => {
            let r = r.split_once('@').map_or(r, |(_, a)| a);
            r.split(['/', ':']).next()?
        }
        None => {
            // scp-like: git@host:group/project
            let r = url.split_once('@').map_or(url, |(_, a)| a);
            r.split_once(':')?.0
        }
    };
    (!host.is_empty()).then(|| host.to_lowercase())
}

/// How git signs in for the folder's `origin`, when it is on the GitLab server the person is
/// signed in to.
pub fn auth(app: &App) -> Option<Auth> {
    let token = app.gl.token.clone()?;
    app.gl.project.as_ref()?;
    Some(Auth::GitLabToken {
        host: app.gl.host.clone(),
        token,
    })
}

/// How git signs in to clone `url`, when it is on the GitLab server the person is signed in to.
pub fn auth_for_url(app: &App, url: &str) -> Option<Auth> {
    let token = app.gl.token.clone()?;
    let host = url_host(url)?;
    (host == app.gl.host.split(':').next().unwrap_or(&app.gl.host)).then(|| Auth::GitLabToken {
        host: app.gl.host.clone(),
        token,
    })
}

/// The open folder's `origin` changed (or was read for the first time).
pub fn set_origin(app: &mut App, origin: Option<String>) {
    if app.gl.origin == origin {
        return;
    }
    let host = if app.gl.host.is_empty() {
        configured_host(app)
    } else {
        app.gl.host.clone()
    };
    app.gl.project = origin.as_deref().and_then(|u| parse_remote(u, &host));
    app.gl.origin = origin;
    app.gl.project_info = None;
    app.gl.merges = Slot::default();
    app.gl.issues = Slot::default();
    if app.panel == Panel::GitLab {
        ensure(app);
    }
}

/// Read the saved token once, off the window thread (the keyring can be slow to answer).
pub fn init(app: &mut App) {
    if app.gl.asked {
        return;
    }
    app.gl.asked = true;
    app.gl.host = configured_host(app);
    app.gl.host_input = app.gl.host.clone();
    if let Some(origin) = app.gl.origin.clone() {
        app.gl.project = parse_remote(&origin, &app.gl.host);
    }
    let host = app.gl.host.clone();
    spawn(app, move || {
        let token = secret::load(&host);
        GlMsg::Saved(host, token)
    });
}

/// The server in the settings changed: forget this session's sign-in and look for a token for
/// the new one. (A token saved for the old server stays in the keyring.)
pub fn use_host(app: &mut App) {
    let host = configured_host(app);
    if host == app.gl.host && app.gl.asked {
        return;
    }
    let origin = app.gl.origin.take();
    let tab = app.gl.tab;
    app.gl = GitlabPanel {
        tab,
        ..GitlabPanel::default()
    };
    if matches!(app.overlay, Overlay::GlItem(_)) {
        app.overlay = Overlay::None;
    }
    init(app);
    set_origin(app, origin);
}

// --- Signing in -------------------------------------------------------------------------------

/// Check a token against a server and, when it works, keep it.
pub fn verify(app: &mut App, host: String, token: String, persist: bool) {
    app.gl.checking = true;
    app.gl.error = None;
    spawn(app, move || {
        let result = Api::new(&host, token.clone())
            .and_then(|a| a.current_user())
            .map_err(text);
        GlMsg::Checked {
            host,
            token,
            persist,
            result,
        }
    });
}

pub fn sign_in(app: &mut App) {
    show(app);
}

pub fn sign_out(app: &mut App) {
    let host = app.gl.host.clone();
    let (origin, tab) = (app.gl.origin.clone(), app.gl.tab);
    app.gl = GitlabPanel {
        host: host.clone(),
        host_input: host.clone(),
        origin,
        tab,
        asked: true,
        ..GitlabPanel::default()
    };
    app.gl.project = app
        .gl
        .origin
        .as_deref()
        .and_then(|u| parse_remote(u, &host));
    if matches!(app.overlay, Overlay::GlItem(_)) {
        app.overlay = Overlay::None;
    }
    let _ = std::thread::Builder::new()
        .name("codigo-gitlab-keyring".into())
        .spawn(move || {
            let _ = secret::delete(&host);
        });
    app.toast(tr("Signed out of GitLab."));
}

// --- Loading ----------------------------------------------------------------------------------

pub fn show(app: &mut App) {
    app.sidebar_visible = true;
    app.panel = Panel::GitLab;
    init(app);
    ensure(app);
}

/// Start loading whatever the open tab shows and does not have yet.
fn ensure(app: &mut App) {
    if app.gl.token.is_none() {
        return;
    }
    let (state, load): (SlotState, fn(&mut App)) = match app.gl.tab {
        Tab::Merges | Tab::Issues => (app.gl.merges.state.clone(), load_project),
        Tab::Mine => (app.gl.mine.state.clone(), load_mine),
        Tab::Review => (app.gl.review.state.clone(), load_review),
        Tab::Projects => (app.gl.projects.state.clone(), load_projects),
    };
    if state == SlotState::Empty
        && !(matches!(app.gl.tab, Tab::Merges | Tab::Issues) && app.gl.project.is_none())
    {
        load(app);
    }
}

pub fn refresh(app: &mut App) {
    if app.gl.token.is_none() {
        show(app);
        return;
    }
    app.gl.merges = Slot::default();
    app.gl.issues = Slot::default();
    app.gl.mine = Slot::default();
    app.gl.review = Slot::default();
    app.gl.projects = Slot::default();
    ensure(app);
}

fn load_project(app: &mut App) {
    let (Some(api), Some(path)) = (api(app), app.gl.project.clone()) else {
        return;
    };
    app.gl.merges.state = SlotState::Loading;
    app.gl.issues.state = SlotState::Loading;
    spawn(app, move || {
        let result = (|| {
            let project = api.project(&path)?;
            let mrs = api.project_merge_requests(project.id)?;
            let issues = api.project_issues(project.id)?;
            Ok(Box::new(ProjectLists {
                project,
                merges: mrs,
                issues,
            }))
        })()
        .map_err(text);
        GlMsg::ProjectLists { path, result }
    });
}

fn load_mine(app: &mut App) {
    let Some(api) = api(app) else { return };
    app.gl.mine.state = SlotState::Loading;
    spawn(app, move || {
        GlMsg::Mine(api.merge_requests(Scope::Mine, "").map_err(text))
    });
}

fn load_review(app: &mut App) {
    let (Some(api), Some(user)) = (api(app), app.gl.user.as_ref().map(|u| u.username.clone()))
    else {
        return;
    };
    app.gl.review.state = SlotState::Loading;
    spawn(app, move || {
        GlMsg::Review(
            api.merge_requests(Scope::ReviewRequested, &user)
                .map_err(text),
        )
    });
}

fn load_projects(app: &mut App) {
    let Some(api) = api(app) else { return };
    app.gl.projects.state = SlotState::Loading;
    spawn(app, move || GlMsg::Projects(api.projects().map_err(text)));
}

fn load_detail(app: &App, project_id: u64, iid: u64, is_mr: bool) {
    let Some(api) = api(app) else { return };
    spawn(app, move || {
        let result = (|| {
            let mut loaded = Loaded::default();
            if is_mr {
                loaded.mr = Some(api.merge_request(project_id, iid)?);
                loaded.changes = api.changes(project_id, iid)?;
                loaded.notes = api.merge_request_notes(project_id, iid)?;
            } else {
                loaded.notes = api.issue_notes(project_id, iid)?;
            }
            Ok(Box::new(loaded))
        })()
        .map_err(text);
        GlMsg::Detail {
            project_id,
            iid,
            result,
        }
    });
}

fn open_mr(app: &mut App, mr: &MergeRequest) {
    let view = ItemView::from_mr(mr);
    load_detail(app, view.project_id, view.iid, true);
    app.overlay = Overlay::GlItem(Box::new(view));
}

fn open_issue(app: &mut App, issue: &Issue) {
    let view = ItemView::from_issue(issue);
    load_detail(app, view.project_id, view.iid, false);
    app.overlay = Overlay::GlItem(Box::new(view));
}

pub fn open_clone(app: &mut App) {
    init(app);
    let mut setup = CloneSetup::new(app, String::new());
    setup.gitlab = Some(app.gl.host.clone());
    app.overlay = Overlay::Clone(setup);
}

fn slot_from<T>(slot: &mut Slot<T>, result: Result<Vec<T>, String>) {
    match result {
        Ok(items) => {
            slot.items = items;
            slot.state = SlotState::Ready;
        }
        Err(e) => slot.state = SlotState::Failed(e),
    }
}

// --- Answers ----------------------------------------------------------------------------------

pub fn on_msg(app: &mut App, msg: GlMsg) {
    match msg {
        GlMsg::Saved(host, token) => {
            if let Some(token) = token {
                if app.gl.token.is_none() && host == app.gl.host {
                    verify(app, host, token, false);
                }
            }
        }
        GlMsg::Checked {
            host,
            token,
            persist,
            result,
        } => {
            app.gl.checking = false;
            match result {
                Ok(user) => {
                    let name = user.username.clone();
                    let changed = host != app.gl.host;
                    app.gl.token_input.clear();
                    app.gl.error = None;
                    app.gl.user = Some(user);
                    app.gl.token = Some(token.clone());
                    app.gl.host = host.clone();
                    app.gl.host_input = host.clone();
                    if changed {
                        let origin = app.gl.origin.take();
                        set_origin(app, origin);
                    }
                    if app.settings.gitlab_host != host {
                        app.settings.gitlab_host = host.clone();
                        app.save_settings();
                    }
                    if persist {
                        spawn(app, move || {
                            let _ = secret::store(&host, &token);
                            GlMsg::Saved(String::new(), None)
                        });
                    }
                    app.toast(trf("Signed in to GitLab as {}.", &[&name]));
                    ensure(app);
                }
                Err(e) => {
                    // A saved token that no longer works is dropped; one the person just gave
                    // is reported.
                    if persist {
                        app.gl.error = Some(e);
                    }
                }
            }
        }
        GlMsg::ProjectLists { path, result } => {
            if app.gl.project.as_deref() != Some(path.as_str()) {
                return;
            }
            match result {
                Ok(lists) => {
                    let ProjectLists {
                        project,
                        merges,
                        issues,
                    } = *lists;
                    app.gl.project_info = Some(project);
                    slot_from(&mut app.gl.merges, Ok(merges));
                    slot_from(&mut app.gl.issues, Ok(issues));
                }
                Err(e) => {
                    app.gl.merges.state = SlotState::Failed(e.clone());
                    app.gl.issues.state = SlotState::Failed(e);
                }
            }
        }
        GlMsg::Mine(result) => slot_from(&mut app.gl.mine, result),
        GlMsg::Review(result) => slot_from(&mut app.gl.review, result),
        GlMsg::Projects(result) => slot_from(&mut app.gl.projects, result),
        GlMsg::Detail {
            project_id,
            iid,
            result,
        } => {
            if let Overlay::GlItem(v) = &mut app.overlay {
                if v.project_id == project_id && v.iid == iid {
                    if let Ok(l) = &result {
                        if let Some(mr) = &l.mr {
                            v.pipeline = mr.pipeline.clone();
                            v.conflicts = mr.has_conflicts;
                            v.state = mr.state.clone();
                        }
                    }
                    v.loaded = Some(result);
                    v.busy = false;
                }
            }
        }
        GlMsg::Posted {
            what,
            project_id,
            iid,
            result,
        } => {
            let mut reload = None;
            if let Overlay::GlItem(v) = &mut app.overlay {
                if v.project_id == project_id && v.iid == iid {
                    v.busy = false;
                    if result.is_ok() {
                        v.comment.clear();
                        reload = Some(v.is_mr);
                    }
                }
            }
            match result {
                Ok(()) => {
                    app.toast(tr(what));
                    if let Some(is_mr) = reload {
                        load_detail(app, project_id, iid, is_mr);
                    }
                    app.gl.review = Slot::default();
                }
                Err(e) => app.toast(first_line(&e)),
            }
        }
        GlMsg::MrCreated(result) => match result {
            Ok(url) => {
                app.toast(tr("Created the merge request."));
                app.gl.merges = Slot::default();
                app.overlay = Overlay::None;
                app.egui_ctx().open_url(egui::OpenUrl::new_tab(url));
                ensure(app);
            }
            Err(e) => {
                if let Overlay::GlNewMr(n) = &mut app.overlay {
                    n.busy = false;
                    n.error = Some(first_line(&e));
                }
            }
        },
    }
}

fn post_comment(app: &mut App, project_id: u64, iid: u64, is_mr: bool, body: String) {
    let Some(api) = api(app) else { return };
    spawn(app, move || {
        let result = if is_mr {
            api.comment_on_merge_request(project_id, iid, &body)
        } else {
            api.comment_on_issue(project_id, iid, &body)
        }
        .map(|_| ())
        .map_err(text);
        GlMsg::Posted {
            what: "Comment posted.",
            project_id,
            iid,
            result,
        }
    });
}

fn post_approval(app: &mut App, project_id: u64, iid: u64) {
    let Some(api) = api(app) else { return };
    spawn(app, move || GlMsg::Posted {
        what: "Approved.",
        project_id,
        iid,
        result: api.approve(project_id, iid).map_err(text),
    });
}

// --- The panel --------------------------------------------------------------------------------

pub fn panel(ui: &mut Ui, app: &mut App, look: &Look) {
    let p = look.palette;
    init(app);
    if app.gl.token.is_none() {
        sign_in_view(ui, app, look);
        return;
    }
    ensure(app);

    ui.horizontal(|ui| {
        let who = app
            .gl
            .user
            .as_ref()
            .map(|u| u.username.clone())
            .unwrap_or_default();
        ui.label(RichText::new(who).strong().color(color(p.text_primary)));
        crate::widgets::trailing(ui, |ui| {
            if small_icon_button(ui, look, "log-out", tr("Sign out of GitLab"), false).clicked() {
                sign_out(app);
            }
            if small_icon_button(ui, look, "refresh", tr("Refresh"), false).clicked() {
                refresh(app);
            }
        });
    });
    ui.label(
        RichText::new(match &app.gl.project {
            Some(path) => format!("{} · {path}", app.gl.host),
            None => app.gl.host.clone(),
        })
        .size(font_size::SMALL)
        .color(color(p.text_secondary)),
    );
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for tab in Tab::ALL {
            if crate::widgets::toggle_chip(
                ui,
                look,
                tr(tab.label()),
                tr(tab.label()),
                app.gl.tab == tab,
            )
            .clicked()
            {
                app.gl.tab = tab;
                ensure(app);
            }
        }
    });
    ui.add_space(6.0);

    if matches!(app.gl.tab, Tab::Merges | Tab::Issues) && app.gl.project.is_none() {
        ui.label(
            RichText::new(trf(
                "This folder does not have a remote on {}. Open a cloned project, or look at your own merge requests.",
                &[&app.gl.host],
            ))
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
        );
        return;
    }

    let tab = app.gl.tab;
    let (state, empty) = match tab {
        Tab::Merges => (&app.gl.merges.state, app.gl.merges.items.is_empty()),
        Tab::Issues => (&app.gl.issues.state, app.gl.issues.items.is_empty()),
        Tab::Mine => (&app.gl.mine.state, app.gl.mine.items.is_empty()),
        Tab::Review => (&app.gl.review.state, app.gl.review.items.is_empty()),
        Tab::Projects => (&app.gl.projects.state, app.gl.projects.items.is_empty()),
    };
    match state {
        SlotState::Empty | SlotState::Loading => {
            ui.label(RichText::new(tr("Loading…")).color(color(p.text_secondary)));
            return;
        }
        SlotState::Failed(e) => {
            ui.label(RichText::new(first_line(e)).color(color(p.status_danger_fg)));
            return;
        }
        SlotState::Ready if empty => {
            let none = match tab {
                Tab::Merges | Tab::Mine => tr("No open merge requests."),
                Tab::Issues => tr("No open issues."),
                Tab::Review => tr("Nothing is waiting for your review."),
                Tab::Projects => tr("No projects."),
            };
            ui.label(RichText::new(none).color(color(p.text_secondary)));
            return;
        }
        SlotState::Ready => {}
    }

    let mut open_m: Option<MergeRequest> = None;
    let mut open_i: Option<Issue> = None;
    let mut clone: Option<String> = None;
    ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt(("gitlab-list", tab as u8))
        .show(ui, |ui| match tab {
            Tab::Merges | Tab::Mine | Tab::Review => {
                let items = match tab {
                    Tab::Merges => &app.gl.merges.items,
                    Tab::Mine => &app.gl.mine.items,
                    _ => &app.gl.review.items,
                };
                for mr in items {
                    if mr_row(ui, look, mr, tab != Tab::Merges) {
                        open_m = Some(mr.clone());
                    }
                }
            }
            Tab::Issues => {
                for issue in &app.gl.issues.items {
                    if issue_row(ui, look, issue) {
                        open_i = Some(issue.clone());
                    }
                }
            }
            Tab::Projects => {
                for project in &app.gl.projects.items {
                    if project_row(ui, look, project) {
                        clone = Some(project.http_url.clone());
                    }
                }
            }
        });
    if let Some(mr) = open_m {
        open_mr(app, &mr);
    }
    if let Some(issue) = open_i {
        open_issue(app, &issue);
    }
    if let Some(url) = clone {
        let mut setup = CloneSetup::new(app, url);
        setup.gitlab = Some(app.gl.host.clone());
        app.overlay = Overlay::Clone(setup);
    }
}

fn token_page(host: &str) -> String {
    format!("https://{host}/-/user_settings/personal_access_tokens?name=Codigo&scopes=api")
}

fn sign_in_view(ui: &mut Ui, app: &mut App, look: &Look) {
    let p = look.palette;
    ui.label(
        RichText::new(tr(
            "Sign in to clone projects and to see and review merge requests and issues without leaving Código. Works with gitlab.com and with your own GitLab server.",
        ))
        .size(font_size::SMALL)
        .color(color(p.text_secondary)),
    );
    ui.add_space(8.0);

    if app.gl.checking {
        ui.label(RichText::new(tr("Connecting to GitLab…")).color(color(p.text_secondary)));
    } else {
        section(ui, look, tr("Server"));
        cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.gl.host_input)
                        .frame(egui::Frame::NONE)
                        .hint_text(DEFAULT_HOST)
                        .align(Align2::LEFT_CENTER)
                        .desired_width(f32::INFINITY),
                )
            });
        let host = app.gl.host_input.trim().to_lowercase();
        let host_ok = valid_host(&host);
        ui.add_space(4.0);
        if cr::secondary_button(ui, look, tr("Create a token on the server"), host_ok).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(token_page(&host)));
        }
        ui.add_space(6.0);
        section(ui, look, tr("Personal access token"));
        let response = cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.gl.token_input)
                        .frame(egui::Frame::NONE)
                        .password(true)
                        .hint_text("glpat-…")
                        .align(Align2::LEFT_CENTER)
                        .desired_width(f32::INFINITY),
                )
            })
            .inner;
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(4.0);
        let ready = host_ok && !app.gl.token_input.trim().is_empty();
        if cr::primary_button(ui, look, tr("Sign in"), ready).clicked() || (enter && ready) {
            let token = app.gl.token_input.trim().to_owned();
            verify(app, host.clone(), token, true);
        }
        ui.add_space(4.0);
        ui.label(
            RichText::new(tr(
                "The token is kept in your system's keyring, never in a file, and is sent only to that server. It needs the api scope.",
            ))
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
        );
        if !host.is_empty() && !host_ok {
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr("That is not a server address such as gitlab.com."))
                    .size(font_size::SMALL)
                    .color(color(p.status_danger_fg)),
            );
        }
    }
    if let Some(e) = &app.gl.error {
        ui.add_space(6.0);
        ui.label(RichText::new(first_line(e)).color(color(p.status_danger_fg)));
    }
}

fn tint(look: &Look, state: &str, draft: bool) -> egui::Color32 {
    let p = &look.palette;
    color(match (state, draft) {
        (_, true) => p.text_secondary,
        ("opened", _) => p.status_success_fg,
        ("merged", _) => p.accent_text,
        _ => p.status_danger_fg,
    })
}

fn pipeline_mark(look: &Look, status: &str) -> (&'static str, egui::Color32) {
    let p = &look.palette;
    match status {
        "success" => ("✓", color(p.status_success_fg)),
        "failed" | "canceled" => ("✗", color(p.status_danger_fg)),
        _ => ("●", color(p.status_warning_fg)),
    }
}

fn mr_row(ui: &mut Ui, look: &Look, mr: &MergeRequest, show_ref: bool) -> bool {
    let p = look.palette;
    row_frame(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(
                RichText::new(if mr.draft { "◌" } else { "●" })
                    .color(tint(look, &mr.state, mr.draft)),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(&mr.title)
                        .color(color(p.text_primary))
                        .size(font_size::BODY),
                )
                .truncate(),
            );
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let head = if show_ref {
                mr.reference.clone()
            } else {
                format!("!{}", mr.iid)
            };
            ui.label(
                RichText::new(format!("{head} · {} · {}", mr.author, date(&mr.updated_at)))
                    .size(font_size::SMALL)
                    .color(color(p.text_secondary)),
            );
            if let Some(status) = &mr.pipeline {
                let (mark, c) = pipeline_mark(look, status);
                ui.label(RichText::new(mark).size(font_size::SMALL).color(c));
            }
        });
    })
}

fn issue_row(ui: &mut Ui, look: &Look, issue: &Issue) -> bool {
    let p = look.palette;
    row_frame(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new("●").color(tint(look, &issue.state, false)));
            ui.add(
                egui::Label::new(
                    RichText::new(&issue.title)
                        .color(color(p.text_primary))
                        .size(font_size::BODY),
                )
                .truncate(),
            );
        });
        ui.label(
            RichText::new(format!(
                "#{} · {} · {}",
                issue.iid,
                issue.author,
                date(&issue.updated_at)
            ))
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
        );
    })
}

fn project_row(ui: &mut Ui, look: &Look, project: &Project) -> bool {
    let p = look.palette;
    row_frame(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add(
                egui::Label::new(
                    RichText::new(&project.path)
                        .color(color(p.text_primary))
                        .size(font_size::BODY),
                )
                .truncate(),
            );
            if project.visibility != "public" {
                ui.label(
                    RichText::new(tr(if project.visibility == "internal" {
                        "internal"
                    } else {
                        "private"
                    }))
                    .size(font_size::SMALL)
                    .color(color(p.text_secondary)),
                );
            }
        });
        if !project.description.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(&project.description)
                        .size(font_size::SMALL)
                        .color(color(p.text_secondary)),
                )
                .truncate(),
            );
        }
    })
}

// --- The merge request and issue window -------------------------------------------------------

pub fn item_overlay(
    ctx: &egui::Context,
    app: &mut App,
    look: &Look,
    mut view: Box<ItemView>,
) -> Shown {
    let p = look.palette;
    let mut closed = false;
    let mut comment = false;
    let mut approve = false;
    let mut browser = false;
    let dismissed = modal(ctx, look, "gitlab-item", 860.0, true, |ui| {
        closed = heading(
            ui,
            look,
            &format!(
                "{}{}{}",
                view.reference,
                if view.reference.is_empty() { "" } else { " " },
                if view.is_mr {
                    format!("!{}", view.iid)
                } else {
                    format!("#{}", view.iid)
                }
            ),
            "gitlab-item",
        );
        ScrollArea::vertical()
            .id_salt("gitlab-item-scroll")
            .max_height((ctx.content_rect().height() - 240.0).max(240.0))
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&view.title)
                            .strong()
                            .size(font_size::LARGE)
                            .color(color(p.text_primary)),
                    )
                    .wrap(),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(if view.draft {
                            tr("draft").to_owned()
                        } else {
                            view.state.clone()
                        })
                        .strong()
                        .color(tint(look, &view.state, view.draft)),
                    );
                    ui.label(
                        RichText::new(trf("by {}", &[&view.author]))
                            .size(font_size::SMALL)
                            .color(color(p.text_secondary)),
                    );
                    if view.is_mr && !view.source.is_empty() {
                        ui.label(
                            RichText::new(format!("{} → {}", view.source, view.target))
                                .monospace()
                                .size(font_size::SMALL)
                                .color(color(p.text_secondary)),
                        );
                    }
                    for l in &view.labels {
                        ui.label(
                            RichText::new(l)
                                .size(font_size::SMALL)
                                .color(color(p.accent_text)),
                        );
                    }
                });
                if view.is_mr {
                    ui.horizontal(|ui| {
                        if let Some(status) = &view.pipeline {
                            let (mark, c) = pipeline_mark(look, status);
                            ui.label(RichText::new(mark).color(c));
                            ui.label(
                                RichText::new(trf("Pipeline {}", &[status]))
                                    .size(font_size::SMALL)
                                    .color(color(p.text_secondary)),
                            );
                        }
                        if view.conflicts {
                            ui.label(
                                RichText::new(tr("Has merge conflicts"))
                                    .size(font_size::SMALL)
                                    .color(color(p.status_danger_fg)),
                            );
                        }
                    });
                }
                if !view.description.trim().is_empty() {
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(clip(&view.description, 6000))
                                .color(color(p.text_primary)),
                        )
                        .wrap(),
                    );
                }
                let loaded = view.loaded.take();
                match &loaded {
                    None => {
                        ui.add_space(8.0);
                        ui.label(RichText::new(tr("Loading…")).color(color(p.text_secondary)));
                    }
                    Some(Err(e)) => {
                        ui.add_space(8.0);
                        ui.label(RichText::new(first_line(e)).color(color(p.status_danger_fg)));
                    }
                    Some(Ok(l)) => {
                        if view.is_mr && !l.changes.is_empty() {
                            section(
                                ui,
                                look,
                                &trf("Files ({})", &[&l.changes.len().to_string()]),
                            );
                            for (i, c) in l.changes.iter().enumerate() {
                                let on = view.file == Some(i);
                                let name = if c.new_path.is_empty() {
                                    &c.old_path
                                } else {
                                    &c.new_path
                                };
                                if ui.selectable_label(on, name).clicked() {
                                    view.file = if on { None } else { Some(i) };
                                }
                                if on {
                                    diff_text(ui, look, &c.diff);
                                }
                            }
                        }
                        let talk: Vec<&Note> = l.notes.iter().filter(|n| !n.system).collect();
                        if !talk.is_empty() {
                            section(
                                ui,
                                look,
                                &trf("Conversation ({})", &[&talk.len().to_string()]),
                            );
                            for n in talk {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&n.author)
                                            .strong()
                                            .size(font_size::SMALL)
                                            .color(color(p.text_primary)),
                                    );
                                    ui.label(
                                        RichText::new(date(&n.created_at))
                                            .size(font_size::SMALL)
                                            .color(color(p.text_secondary)),
                                    );
                                });
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(clip(&n.body, 3000))
                                            .color(color(p.text_primary)),
                                    )
                                    .wrap(),
                                );
                                ui.add_space(6.0);
                            }
                        }
                    }
                }
                view.loaded = loaded;
            });
        ui.add_space(8.0);
        cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut view.comment)
                        .frame(egui::Frame::NONE)
                        .hint_text(tr("Leave a comment"))
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .font(FontId::proportional(font_size::BODY)),
                )
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let can = !view.busy && !view.comment.trim().is_empty();
            if cr::primary_button(ui, look, tr("Comment"), can).clicked() {
                comment = true;
            }
            if view.is_mr
                && view.state == "opened"
                && cr::secondary_button(ui, look, tr("Approve"), !view.busy).clicked()
            {
                approve = true;
            }
            if cr::secondary_button(ui, look, tr("Open on GitLab"), !view.url.is_empty()).clicked()
            {
                browser = true;
            }
        });
    });
    if browser {
        ctx.open_url(egui::OpenUrl::new_tab(view.url.clone()));
    }
    if comment {
        view.busy = true;
        let body = view.comment.trim().to_owned();
        post_comment(app, view.project_id, view.iid, view.is_mr, body);
    }
    if approve {
        view.busy = true;
        post_approval(app, view.project_id, view.iid);
    }
    if closed || dismissed {
        return (Overlay::None, true);
    }
    (Overlay::GlItem(view), false)
}

/// A diff as text, with added and removed lines tinted. Long diffs are cut.
fn diff_text(ui: &mut Ui, look: &Look, diff: &str) {
    let p = &look.palette;
    const MAX_LINES: usize = 400;
    if diff.is_empty() {
        ui.label(
            RichText::new(tr("No text changes to show."))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
        );
        return;
    }
    egui::Frame::NONE
        .fill(color(p.control_bg_hover).gamma_multiply(0.35))
        .corner_radius(6)
        .inner_margin(6)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for line in diff.lines().take(MAX_LINES) {
                let ink = if line.starts_with("@@") {
                    color(p.accent_text)
                } else if line.starts_with('+') {
                    color(p.status_success_fg)
                } else if line.starts_with('-') {
                    color(p.status_danger_fg)
                } else {
                    color(p.text_primary)
                };
                ui.add(
                    egui::Label::new(
                        RichText::new(line)
                            .monospace()
                            .size(font_size::SMALL)
                            .color(ink),
                    )
                    .extend(),
                );
            }
            if diff.lines().count() > MAX_LINES {
                ui.label(
                    RichText::new(tr("The rest of this diff is on GitLab."))
                        .size(font_size::SMALL)
                        .color(color(p.text_secondary)),
                );
            }
        });
}

// --- Creating a merge request -----------------------------------------------------------------

pub struct MrSetup {
    pub title: String,
    pub description: String,
    pub source: String,
    pub target: String,
    pub draft: bool,
    pub focus: bool,
    pub busy: bool,
    pub error: Option<String>,
}

/// Start a merge request for the current branch: the folder must be a project on the server the
/// person is signed in to, and on a branch.
pub fn open_create_merge_request(app: &mut App) {
    init(app);
    if app.gl.token.is_none() {
        app.toast(tr("Sign in to GitLab first."));
        show(app);
        return;
    }
    let Some(project) = app.gl.project_info.clone() else {
        if app.gl.project.is_some() {
            app.toast(tr("Still reading the project; try again in a moment."));
            show(app);
        } else {
            app.toast(tr("This folder is not a GitLab project."));
        }
        return;
    };
    let Some(branch) = app.vcs.status.as_ref().and_then(|s| s.branch.clone()) else {
        app.toast(tr("Switch to a branch first."));
        return;
    };
    let title = app
        .vcs
        .log
        .first()
        .map(|c| c.subject.clone())
        .unwrap_or_default();
    app.overlay = Overlay::GlNewMr(MrSetup {
        title,
        description: String::new(),
        source: branch,
        target: if project.default_branch.is_empty() {
            "main".to_owned()
        } else {
            project.default_branch
        },
        draft: false,
        focus: true,
        busy: false,
        error: None,
    });
}

fn create_merge_request(app: &mut App, setup: &MrSetup) {
    let (Some(api), Some(project), Some(root)) =
        (api(app), app.gl.project_info.clone(), app.vcs.root.clone())
    else {
        return;
    };
    let auth = crate::vcs_ui::auth(app);
    let (title, description, source, target, draft) = (
        setup.title.clone(),
        setup.description.clone(),
        setup.source.clone(),
        setup.target.clone(),
        setup.draft,
    );
    spawn(app, move || {
        let result = (|| {
            let repo = codigo_git::Repo::discover(&root)
                .ok_or_else(|| "this folder is not a git repository".to_owned())?;
            // The branch has to be on GitLab before a merge request can name it.
            repo.push(&auth).map_err(|e| e.to_string())?;
            api.create_merge_request(
                project.id,
                NewMergeRequest {
                    title: &title,
                    description: &description,
                    source_branch: &source,
                    target_branch: &target,
                    draft,
                },
            )
            .map_err(text)
        })();
        GlMsg::MrCreated(result)
    });
}

pub fn mr_overlay(ctx: &egui::Context, app: &mut App, look: &Look, mut state: MrSetup) -> Shown {
    let p = look.palette;
    let mut closed = false;
    let mut go = false;
    let dismissed = modal(ctx, look, "create-mr", 620.0, true, |ui| {
        closed = heading(ui, look, tr("Create a merge request"), "create-mr");
        ui.label(
            RichText::new(trf("{} into {}", &[&state.source, &state.target]))
                .monospace()
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
        );
        ui.add_space(8.0);
        let response = cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut state.title)
                        .frame(egui::Frame::NONE)
                        .hint_text(tr("Title"))
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
        ui.add_space(6.0);
        cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut state.description)
                        .frame(egui::Frame::NONE)
                        .hint_text(tr("Describe the change"))
                        .desired_rows(6)
                        .desired_width(f32::INFINITY)
                        .font(FontId::proportional(font_size::BODY)),
                )
            });
        ui.add_space(6.0);
        ui.checkbox(&mut state.draft, tr("Open as a draft"));
        ui.label(
            RichText::new(tr("The branch is pushed to GitLab first."))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
        );
        if let Some(e) = &state.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(color(p.status_danger_fg)));
        }
        ui.add_space(10.0);
        let ready = !state.busy && !state.title.trim().is_empty() && !state.target.is_empty();
        let label = if state.busy {
            tr("Creating…")
        } else {
            tr("Create merge request")
        };
        if cr::primary_button(ui, look, label, ready).clicked() {
            go = true;
        }
    });
    if go {
        state.busy = true;
        state.error = None;
        create_merge_request(app, &state);
    }
    if closed || dismissed {
        return (Overlay::None, true);
    }
    (Overlay::GlNewMr(state), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_come_out_of_every_spelling_of_an_address() {
        for (url, host) in [
            ("https://gitlab.com/a/b.git", "gitlab.com"),
            (
                "https://oauth2:tok@GitLab.Example.org:8443/a/b",
                "gitlab.example.org",
            ),
            ("git@gitlab.com:a/b.git", "gitlab.com"),
            ("ssh://git@git.example.org:2222/a/b.git", "git.example.org"),
        ] {
            assert_eq!(url_host(url).as_deref(), Some(host), "{url}");
        }
        assert_eq!(url_host(""), None);
        assert_eq!(url_host("not an address"), None);
    }

    #[test]
    fn the_token_page_names_the_server_and_the_scope() {
        let url = token_page("gitlab.example.org");
        assert!(url.starts_with("https://gitlab.example.org/"));
        assert!(url.contains("scopes=api"));
    }

    #[test]
    fn tabs_are_labelled_once() {
        let mut labels: Vec<_> = Tab::ALL.iter().map(|t| t.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), Tab::ALL.len());
    }
}
