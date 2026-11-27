// SPDX-License-Identifier: MIT OR Apache-2.0
//! GitHub in the window: signing in, the panel with pull requests, issues and repositories, the
//! window that shows one pull request or issue (with reviews and comments), and cloning.
//! Every call to GitHub runs on a worker thread and comes back as a [`GhMsg`].
//!
//! The token lives in the system keyring (see `codigo_github::secret`). Where there is no
//! keyring it is kept in memory for the session and never written anywhere else.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use codigo_git::Auth;
use codigo_github::device::{self, DeviceCode, Poll};
use codigo_github::{
    diff_lines, secret, Api, Check, Comment, CommentKind, DiffKind, DiffLine, Issue, LinePlace,
    NewPull, Pull, PullDetail, PullFile, RepoInfo, ReviewComment, ReviewEvent, Scope, User,
};
use egui::{Align, Align2, FontId, Layout, RichText, ScrollArea, Ui};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::{App, Msg, Panel};
use crate::i18n::{tr, trf};
use crate::overlays::{heading, modal, Overlay, Shown};
use crate::vcs_ui::section;
use crate::widgets::small_icon_button;

/// The id of the OAuth app that device sign-in uses. It is not a secret, and Código never ships
/// someone else's: it comes from the build or from the environment.
pub fn client_id() -> Option<String> {
    std::env::var("CODIGO_GITHUB_CLIENT_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| option_env!("CODIGO_GITHUB_CLIENT_ID").map(str::to_owned))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    /// Pull requests of the repository the open folder belongs to.
    #[default]
    Pulls,
    Issues,
    /// Opened by me, across GitHub.
    Mine,
    /// Waiting for my review, across GitHub.
    Review,
    Repos,
}

impl Tab {
    const ALL: [Tab; 5] = [Tab::Pulls, Tab::Issues, Tab::Mine, Tab::Review, Tab::Repos];

    fn icon(self) -> &'static str {
        match self {
            Tab::Pulls => "branch",
            Tab::Issues => "info",
            Tab::Mine => "user",
            Tab::Review => "check-square",
            Tab::Repos => "book",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Pulls => "Pull requests",
            Tab::Issues => "Issues",
            Tab::Mine => "Mine",
            Tab::Review => "To review",
            Tab::Repos => "Repositories",
        }
    }
}

/// One list and how far loading it got.
pub struct Slot<T> {
    pub items: Vec<T>,
    pub state: SlotState,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SlotState {
    Empty,
    Loading,
    Ready,
    Failed(String),
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Slot {
            items: Vec::new(),
            state: SlotState::Empty,
        }
    }
}

/// Where signing in has got to.
#[derive(Default)]
pub enum SignIn {
    #[default]
    Idle,
    /// Asking GitHub for a code.
    Starting,
    /// The person has the code and is authorising in the browser.
    Waiting {
        code: DeviceCode,
        cancel: Arc<AtomicBool>,
    },
    /// A token is being checked.
    Checking,
}

#[derive(Default)]
pub struct GithubPanel {
    pub token: Option<String>,
    pub user: Option<User>,
    /// The GitHub repository the open folder's `origin` points at.
    pub repo: Option<(String, String)>,
    pub tab: Tab,
    pub pulls: Slot<Pull>,
    pub issues: Slot<Issue>,
    pub mine: Slot<Pull>,
    pub review: Slot<Pull>,
    pub repos: Slot<RepoInfo>,
    /// The repository the two repository lists were read for.
    pub listed_for: Option<(String, String)>,
    pub signin: SignIn,
    pub token_input: String,
    pub error: Option<String>,
    /// Whether the keyring was asked for a saved token yet.
    pub asked: bool,
}

/// Answers from the worker threads.
pub enum GhMsg {
    /// The saved token, read from the keyring at startup.
    Saved(Option<String>),
    /// A token was checked: who it belongs to, or why not.
    Checked {
        token: String,
        persist: bool,
        result: Result<User, String>,
    },
    Device(Result<DeviceCode, String>),
    DeviceEnded(Result<String, String>),
    Pulls(Option<(String, String)>, Result<Vec<Pull>, String>),
    Issues(Option<(String, String)>, Result<Vec<Issue>, String>),
    Mine(Result<Vec<Pull>, String>),
    Review(Result<Vec<Pull>, String>),
    Repos(Result<Vec<RepoInfo>, String>),
    Detail {
        repo: String,
        number: u64,
        result: Result<Box<Loaded>, String>,
    },
    Posted {
        what: &'static str,
        repo: String,
        number: u64,
        result: Result<(), String>,
    },
    Cloned(Result<PathBuf, String>),
    /// The branch a repository merges into by default.
    DefaultBranch(Result<String, String>),
    /// The pull request that was just created, or why not.
    PullCreated(Result<Box<Pull>, String>),
}

/// What a pull request or issue window loads beyond what the list already had.
#[derive(Clone, Default)]
pub struct Loaded {
    pub stats: Option<PullDetail>,
    pub files: Vec<PullFile>,
    pub conversation: Vec<Comment>,
    pub checks: Vec<Check>,
    /// Comments left on lines of the diff.
    pub review_comments: Vec<ReviewComment>,
}

/// The window of one pull request or issue.
pub struct ItemView {
    pub repo: String,
    pub number: u64,
    pub is_pull: bool,
    pub title: String,
    pub state: String,
    pub author: String,
    pub body: String,
    pub url: String,
    pub labels: Vec<String>,
    pub draft: bool,
    pub head_ref: String,
    pub base_ref: String,
    pub head_sha: String,
    pub loaded: Option<Result<Box<Loaded>, String>>,
    pub comment: String,
    /// The file whose diff is open.
    pub file: Option<usize>,
    pub busy: bool,
    /// The diff line a review comment is being written on: file, line and whether it is the
    /// old side.
    pub pick: Option<(String, u64, bool)>,
    /// The review comment being written.
    pub inline: String,
    /// The comment on the diff that the one being written answers.
    pub reply_to: Option<u64>,
    /// The parsed patch of the file whose diff is open.
    pub parsed: Option<(usize, std::rc::Rc<Vec<DiffLine>>)>,
}

impl ItemView {
    fn from_pull(p: &Pull) -> ItemView {
        ItemView {
            repo: p.repo.clone(),
            number: p.number,
            is_pull: true,
            title: p.title.clone(),
            state: p.state.clone(),
            author: p.author.clone(),
            body: p.body.clone(),
            url: p.html_url.clone(),
            labels: p.labels.clone(),
            draft: p.draft,
            head_ref: p.head_ref.clone(),
            base_ref: p.base_ref.clone(),
            head_sha: p.head_sha.clone(),
            loaded: None,
            comment: String::new(),
            file: None,
            busy: false,
            pick: None,
            inline: String::new(),
            reply_to: None,
            parsed: None,
        }
    }

    fn from_issue(i: &Issue) -> ItemView {
        ItemView {
            repo: i.repo.clone(),
            number: i.number,
            is_pull: false,
            title: i.title.clone(),
            state: i.state.clone(),
            author: i.author.clone(),
            body: i.body.clone(),
            url: i.html_url.clone(),
            labels: i.labels.clone(),
            draft: false,
            head_ref: String::new(),
            base_ref: String::new(),
            head_sha: String::new(),
            loaded: None,
            comment: String::new(),
            file: None,
            busy: false,
            pick: None,
            inline: String::new(),
            reply_to: None,
            parsed: None,
        }
    }
}

/// Cloning a repository.
pub struct CloneSetup {
    pub url: String,
    pub parent: String,
    pub focus: bool,
    pub busy: bool,
    pub error: Option<String>,
    /// The GitLab server `owner/name` is read against; `None` for GitHub.
    pub gitlab: Option<String>,
}

impl CloneSetup {
    pub fn new(_app: &App, url: String) -> CloneSetup {
        CloneSetup {
            gitlab: None,
            url,
            parent: default_parent(),
            focus: true,
            busy: false,
            error: None,
        }
    }
}

pub(crate) fn default_parent() -> String {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let Some(home) = home else {
        return String::new();
    };
    for name in ["Projects", "projects", "Developer", "dev", "src"] {
        let p = home.join(name);
        if p.is_dir() {
            return p.display().to_string();
        }
    }
    home.display().to_string()
}

/// What `owner/name` or a full address means as an address git can clone.
/// [`clone_address`] for a short `group/project` on any server.
pub fn clone_address_on(input: &str, host: &str) -> String {
    let t = input.trim();
    let short = !t.is_empty()
        && !t.contains("://")
        && !t.contains('@')
        && !t.contains(char::is_whitespace)
        && t.matches('/').count() == 1
        && !t.starts_with('/')
        && !t.ends_with('/')
        && !t.starts_with('-');
    if short {
        format!("https://{host}/{t}.git")
    } else {
        t.to_owned()
    }
}

fn spawn(app: &App, work: impl FnOnce() -> GhMsg + Send + 'static) {
    let tx = app.sender();
    let ctx = app.egui_ctx().clone();
    let _ = std::thread::Builder::new()
        .name("codigo-github".into())
        .spawn(move || {
            let _ = tx.send(Msg::Gh(work()));
            ctx.request_repaint();
        });
}

fn text(e: codigo_github::Error) -> String {
    e.to_string()
}

fn api(app: &App) -> Option<Api> {
    app.gh.token.clone().map(Api::new)
}

/// Read the saved token once, off the window thread (the keyring can be slow to answer).
pub fn init(app: &mut App) {
    if app.gh.asked {
        return;
    }
    app.gh.asked = true;
    spawn(app, || GhMsg::Saved(secret::load()));
}

// --- Signing in -------------------------------------------------------------------------------

/// Check a token and, when it works, keep it.
pub fn verify(app: &mut App, token: String, persist: bool) {
    app.gh.signin = SignIn::Checking;
    app.gh.error = None;
    spawn(app, move || {
        let result = Api::new(token.clone()).user().map_err(text);
        GhMsg::Checked {
            token,
            persist,
            result,
        }
    });
}

pub fn sign_in_device(app: &mut App) {
    let Some(id) = client_id() else {
        app.toast(tr(
            "Sign-in from the browser needs a GitHub OAuth app. Use the GitHub CLI or a token instead.",
        ));
        return;
    };
    show(app);
    app.gh.signin = SignIn::Starting;
    app.gh.error = None;
    spawn(app, move || GhMsg::Device(device::start(&id).map_err(text)));
}

fn wait_for_device(app: &App, code: &DeviceCode, cancel: Arc<AtomicBool>) {
    let Some(id) = client_id() else {
        return;
    };
    let device_code = code.device_code.clone();
    let mut interval = code.interval.max(5);
    let deadline = std::time::Instant::now() + Duration::from_secs(code.expires_in.max(60));
    let (tx, ctx) = (app.sender(), app.egui_ctx().clone());
    let _ = std::thread::Builder::new()
        .name("codigo-github-device".into())
        .spawn(move || {
            let end = loop {
                // Sleep in small steps so cancelling is felt at once.
                for _ in 0..interval * 4 {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
                if std::time::Instant::now() > deadline {
                    break Err("The code expired. Start again.".to_owned());
                }
                match device::poll(&id, &device_code) {
                    Ok(Poll::Pending) => {}
                    Ok(Poll::SlowDown) => interval += 5,
                    Ok(Poll::Token(t)) => break Ok(t),
                    Ok(Poll::Denied) => break Err("GitHub refused the sign-in.".to_owned()),
                    Ok(Poll::Expired) => break Err("The code expired. Start again.".to_owned()),
                    Err(e) => break Err(e.to_string()),
                }
            };
            let _ = tx.send(Msg::Gh(GhMsg::DeviceEnded(end)));
            ctx.request_repaint();
        });
}

pub fn sign_in_cli(app: &mut App) {
    show(app);
    app.gh.signin = SignIn::Checking;
    app.gh.error = None;
    let tx = app.sender();
    let ctx = app.egui_ctx().clone();
    let _ = std::thread::Builder::new()
        .name("codigo-github-cli".into())
        .spawn(move || {
            let msg = match secret::from_gh_cli() {
                Some(token) => {
                    let result = Api::new(token.clone()).user().map_err(text);
                    GhMsg::Checked {
                        token,
                        persist: true,
                        result,
                    }
                }
                None => GhMsg::Device(Err(
                    "The GitHub CLI is not installed or is not signed in (run `gh auth login`)."
                        .to_owned(),
                )),
            };
            let _ = tx.send(Msg::Gh(msg));
            ctx.request_repaint();
        });
}

pub fn sign_out(app: &mut App) {
    if let SignIn::Waiting { cancel, .. } = &app.gh.signin {
        cancel.store(true, Ordering::Relaxed);
    }
    let repo = app.gh.repo.clone();
    app.gh = GithubPanel {
        repo,
        asked: true,
        ..GithubPanel::default()
    };
    if matches!(app.overlay, Overlay::Item(_)) {
        app.overlay = Overlay::None;
    }
    let tx = app.sender();
    let _ = std::thread::Builder::new()
        .name("codigo-github-keyring".into())
        .spawn(move || {
            let _ = secret::delete();
            drop(tx);
        });
    app.toast(tr("Signed out of GitHub."));
}

// --- Loading ----------------------------------------------------------------------------------

pub fn show(app: &mut App) {
    app.sidebar_visible = true;
    app.panel = Panel::GitHub;
    init(app);
    ensure(app);
}

/// Start loading whatever the open tab shows and does not have yet.
fn ensure(app: &mut App) {
    if app.gh.token.is_none() {
        return;
    }
    if app.gh.listed_for != app.gh.repo {
        app.gh.listed_for = app.gh.repo.clone();
        app.gh.pulls = Slot::default();
        app.gh.issues = Slot::default();
    }
    let (state, load): (SlotState, fn(&mut App)) = match app.gh.tab {
        Tab::Pulls => (app.gh.pulls.state.clone(), load_pulls),
        Tab::Issues => (app.gh.issues.state.clone(), load_issues),
        Tab::Mine => (app.gh.mine.state.clone(), load_mine),
        Tab::Review => (app.gh.review.state.clone(), load_review),
        Tab::Repos => (app.gh.repos.state.clone(), load_repos),
    };
    if state == SlotState::Empty {
        load(app);
    }
}

pub fn refresh(app: &mut App) {
    if app.gh.token.is_none() {
        show(app);
        return;
    }
    app.gh.pulls = Slot::default();
    app.gh.issues = Slot::default();
    app.gh.mine = Slot::default();
    app.gh.review = Slot::default();
    app.gh.repos = Slot::default();
    ensure(app);
}

fn load_pulls(app: &mut App) {
    let (Some(api), Some((owner, name))) = (api(app), app.gh.repo.clone()) else {
        return;
    };
    app.gh.pulls.state = SlotState::Loading;
    let key = app.gh.repo.clone();
    spawn(app, move || {
        GhMsg::Pulls(key, api.pulls(&owner, &name).map_err(text))
    });
}

fn load_issues(app: &mut App) {
    let (Some(api), Some((owner, name))) = (api(app), app.gh.repo.clone()) else {
        return;
    };
    app.gh.issues.state = SlotState::Loading;
    let key = app.gh.repo.clone();
    spawn(app, move || {
        GhMsg::Issues(key, api.issues(&owner, &name).map_err(text))
    });
}

fn load_mine(app: &mut App) {
    let Some(api) = api(app) else {
        return;
    };
    app.gh.mine.state = SlotState::Loading;
    spawn(app, move || {
        GhMsg::Mine(api.search_pulls(Scope::Mine).map_err(text))
    });
}

fn load_review(app: &mut App) {
    let Some(api) = api(app) else {
        return;
    };
    app.gh.review.state = SlotState::Loading;
    spawn(app, move || {
        GhMsg::Review(api.search_pulls(Scope::ReviewRequested).map_err(text))
    });
}

fn load_repos(app: &mut App) {
    let Some(api) = api(app) else {
        return;
    };
    app.gh.repos.state = SlotState::Loading;
    spawn(app, move || GhMsg::Repos(api.repos().map_err(text)));
}

fn load_detail(app: &App, repo: &str, number: u64, is_pull: bool, sha: String) {
    let Some(api) = api(app) else {
        return;
    };
    let repo = repo.to_owned();
    spawn(app, move || {
        let result = (|| {
            let (owner, name) = repo
                .split_once('/')
                .ok_or_else(|| "not a repository".to_owned())?;
            let mut loaded = Loaded::default();
            if is_pull {
                loaded.stats = Some(api.pull(owner, name, number).map_err(text)?);
                loaded.files = api.pull_files(owner, name, number).unwrap_or_default();
                let sha = loaded
                    .stats
                    .as_ref()
                    .map(|s| s.pull.head_sha.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(sha);
                loaded.checks = api.checks(owner, name, &sha).unwrap_or_default();
                loaded.review_comments =
                    api.review_comments(owner, name, number).unwrap_or_default();
            }
            loaded.conversation = api
                .conversation(owner, name, number, is_pull)
                .map_err(text)?;
            Ok(Box::new(loaded))
        })();
        GhMsg::Detail {
            repo,
            number,
            result,
        }
    });
}

fn open_pull(app: &mut App, pull: &Pull) {
    let view = ItemView::from_pull(pull);
    load_detail(app, &view.repo, view.number, true, view.head_sha.clone());
    app.overlay = Overlay::Item(Box::new(view));
}

fn open_issue(app: &mut App, issue: &Issue) {
    let view = ItemView::from_issue(issue);
    load_detail(app, &view.repo, view.number, false, String::new());
    app.overlay = Overlay::Item(Box::new(view));
}

pub fn open_clone(app: &mut App) {
    init(app);
    app.overlay = Overlay::Clone(CloneSetup::new(app, String::new()));
}

// --- Answers ----------------------------------------------------------------------------------

fn slot_from<T>(slot: &mut Slot<T>, result: Result<Vec<T>, String>) {
    match result {
        Ok(items) => {
            slot.items = items;
            slot.state = SlotState::Ready;
        }
        Err(e) => slot.state = SlotState::Failed(e),
    }
}

pub fn on_msg(app: &mut App, msg: GhMsg) {
    match msg {
        GhMsg::Saved(token) => {
            if let Some(token) = token {
                if app.gh.token.is_none() {
                    verify(app, token, false);
                }
            }
        }
        GhMsg::Checked {
            token,
            persist,
            result,
        } => match result {
            Ok(user) => {
                app.gh.signin = SignIn::Idle;
                app.gh.token_input.clear();
                app.gh.error = None;
                let name = user.login.clone();
                app.gh.user = Some(user);
                app.gh.token = Some(token.clone());
                if persist {
                    spawn(app, move || {
                        let _ = secret::store(&token);
                        GhMsg::Saved(None)
                    });
                }
                app.toast(trf("Signed in to GitHub as {}.", &[&name]));
                ensure(app);
            }
            Err(e) => {
                app.gh.signin = SignIn::Idle;
                // A saved token that no longer works is dropped; one the person just gave is
                // reported.
                if persist {
                    app.gh.error = Some(e);
                }
            }
        },
        GhMsg::Device(result) => match result {
            Ok(code) => {
                let cancel = Arc::new(AtomicBool::new(false));
                wait_for_device(app, &code, cancel.clone());
                app.egui_ctx()
                    .open_url(egui::OpenUrl::new_tab(code.verification_uri.clone()));
                app.gh.signin = SignIn::Waiting { code, cancel };
            }
            Err(e) => {
                app.gh.signin = SignIn::Idle;
                app.gh.error = Some(e);
            }
        },
        GhMsg::DeviceEnded(result) => {
            if !matches!(app.gh.signin, SignIn::Waiting { .. }) {
                return;
            }
            match result {
                Ok(token) => verify(app, token, true),
                Err(e) => {
                    app.gh.signin = SignIn::Idle;
                    app.gh.error = Some(e);
                }
            }
        }
        GhMsg::Pulls(key, result) => {
            if key == app.gh.repo {
                slot_from(&mut app.gh.pulls, result);
            }
        }
        GhMsg::Issues(key, result) => {
            if key == app.gh.repo {
                slot_from(&mut app.gh.issues, result);
            }
        }
        GhMsg::Mine(result) => slot_from(&mut app.gh.mine, result),
        GhMsg::Review(result) => slot_from(&mut app.gh.review, result),
        GhMsg::Repos(result) => slot_from(&mut app.gh.repos, result),
        GhMsg::Detail {
            repo,
            number,
            result,
        } => {
            if let Overlay::Item(v) = &mut app.overlay {
                if v.repo == repo && v.number == number {
                    v.loaded = Some(result);
                    v.busy = false;
                }
            }
        }
        GhMsg::Posted {
            what,
            repo,
            number,
            result,
        } => {
            let mut reload = None;
            if let Overlay::Item(v) = &mut app.overlay {
                if v.repo == repo && v.number == number {
                    v.busy = false;
                    if result.is_ok() {
                        v.comment.clear();
                        v.inline.clear();
                        v.reply_to = None;
                        reload = Some((v.is_pull, v.head_sha.clone()));
                    }
                }
            }
            match result {
                Ok(()) => {
                    app.toast(tr(what));
                    if let Some((is_pull, sha)) = reload {
                        load_detail(app, &repo, number, is_pull, sha);
                    }
                    app.gh.review = Slot::default();
                }
                Err(e) => app.toast(e),
            }
        }
        GhMsg::DefaultBranch(result) => {
            if let Overlay::NewPull(n) = &mut app.overlay {
                match result {
                    Ok(branch) => {
                        if n.base.is_empty() {
                            n.base = branch;
                        }
                    }
                    Err(e) => n.error = Some(first_line(&e)),
                }
            }
        }
        GhMsg::PullCreated(result) => match result {
            Ok(pull) => {
                app.toast(trf(
                    "Created pull request #{}.",
                    &[&pull.number.to_string()],
                ));
                app.gh.pulls = Slot::default();
                open_pull(app, &pull);
                ensure(app);
            }
            Err(e) => {
                if let Overlay::NewPull(n) = &mut app.overlay {
                    n.busy = false;
                    n.error = Some(first_line(&e));
                }
            }
        },
        GhMsg::Cloned(result) => match result {
            Ok(path) => {
                app.overlay = Overlay::None;
                app.open_project(path, true);
                app.toast(tr("Cloned. The project is open."));
            }
            Err(e) => {
                if let Overlay::Clone(c) = &mut app.overlay {
                    c.busy = false;
                    c.error = Some(first_line(&e));
                }
            }
        },
    }
}

pub(crate) fn first_line(text: &str) -> String {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(text)
        .trim()
        .to_owned()
}

pub(crate) fn date(stamp: &str) -> &str {
    stamp.get(..10).unwrap_or(stamp)
}

// --- Posting ----------------------------------------------------------------------------------

fn post_comment(app: &mut App, repo: String, number: u64, body: String) {
    let Some(api) = api(app) else {
        return;
    };
    spawn(app, move || {
        let result = match repo.split_once('/') {
            Some((o, n)) => api.comment(o, n, number, &body).map_err(text),
            None => Err("not a repository".to_owned()),
        };
        GhMsg::Posted {
            what: "Comment posted.",
            repo,
            number,
            result,
        }
    });
}

fn post_review(app: &mut App, repo: String, number: u64, event: ReviewEvent, body: String) {
    let Some(api) = api(app) else {
        return;
    };
    let what = match event {
        ReviewEvent::Approve => "Approved.",
        ReviewEvent::RequestChanges => "Changes requested.",
        ReviewEvent::Comment => "Review posted.",
    };
    spawn(app, move || {
        let result = match repo.split_once('/') {
            Some((o, n)) => api.review(o, n, number, event, &body).map_err(text),
            None => Err("not a repository".to_owned()),
        };
        GhMsg::Posted {
            what,
            repo,
            number,
            result,
        }
    });
}

/// Comment on a line of the diff, or answer a comment there.
fn post_inline(app: &mut App, view: &ItemView, body: String) {
    let Some(api) = api(app) else {
        return;
    };
    let (repo, number, sha) = (view.repo.clone(), view.number, view.head_sha.clone());
    let (pick, reply_to) = (view.pick.clone(), view.reply_to);
    spawn(app, move || {
        let result = (|| {
            let (owner, name) = repo
                .split_once('/')
                .ok_or_else(|| "not a repository".to_owned())?;
            match (reply_to, pick) {
                (Some(to), _) => api
                    .reply_review_comment(owner, name, number, to, &body)
                    .map_err(text),
                (None, Some((path, line, left))) => api
                    .add_review_comment(
                        owner,
                        name,
                        number,
                        &sha,
                        &LinePlace {
                            path: &path,
                            line,
                            left,
                        },
                        &body,
                    )
                    .map_err(text),
                (None, None) => Err("choose a line of the diff".to_owned()),
            }
        })();
        GhMsg::Posted {
            what: "Comment posted.",
            repo,
            number,
            result,
        }
    });
}

fn start_clone(app: &mut App, url: String, parent: String) {
    let auth = crate::gitlab_ui::auth_for_url(app, &url).unwrap_or_else(|| match &app.gh.token {
        Some(t) => Auth::GitHubToken(t.clone()),
        None => Auth::None,
    });
    spawn(app, move || {
        let name = codigo_git::folder_name(&url);
        let result = codigo_git::clone(&url, std::path::Path::new(&parent), &name, &auth)
            .map_err(|e| e.to_string());
        GhMsg::Cloned(result)
    });
}

// --- The panel --------------------------------------------------------------------------------

#[derive(Default)]
pub struct Outcome {}

pub fn panel(ui: &mut Ui, app: &mut App, look: &Look) -> Outcome {
    let p = look.palette;
    init(app);
    if app.gh.token.is_none() {
        sign_in_view(ui, app, look);
        return Outcome::default();
    }
    ensure(app);

    ui.horizontal(|ui| {
        let who = app
            .gh
            .user
            .as_ref()
            .map(|u| u.login.clone())
            .unwrap_or_default();
        ui.label(RichText::new(who).strong().color(color(p.text_primary)));
        crate::widgets::trailing(ui, |ui| {
            if small_icon_button(ui, look, "log-out", tr("Sign out of GitHub"), false).clicked() {
                sign_out(app);
            }
            if small_icon_button(ui, look, "refresh", tr("Refresh"), false).clicked() {
                refresh(app);
            }
        });
    });
    if let Some((o, n)) = &app.gh.repo {
        ui.label(
            RichText::new(format!("{o}/{n}"))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
        );
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for tab in Tab::ALL {
            if crate::widgets::tab_chip(ui, look, tab.icon(), tr(tab.label()), app.gh.tab == tab)
                .clicked()
            {
                app.gh.tab = tab;
                ensure(app);
            }
        }
    });
    ui.add_space(6.0);

    let needs_repo = matches!(app.gh.tab, Tab::Pulls | Tab::Issues) && app.gh.repo.is_none();
    if needs_repo {
        ui.label(
            RichText::new(tr(
                "This folder does not have a GitHub remote. Open a cloned repository, or look at your own pull requests.",
            ))
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
        );
        return Outcome::default();
    }

    let tab = app.gh.tab;
    let (state, empty) = match tab {
        Tab::Pulls => (&app.gh.pulls.state, app.gh.pulls.items.is_empty()),
        Tab::Issues => (&app.gh.issues.state, app.gh.issues.items.is_empty()),
        Tab::Mine => (&app.gh.mine.state, app.gh.mine.items.is_empty()),
        Tab::Review => (&app.gh.review.state, app.gh.review.items.is_empty()),
        Tab::Repos => (&app.gh.repos.state, app.gh.repos.items.is_empty()),
    };
    match state {
        SlotState::Empty | SlotState::Loading => {
            ui.label(RichText::new(tr("Loading…")).color(color(p.text_secondary)));
            return Outcome::default();
        }
        SlotState::Failed(e) => {
            ui.label(RichText::new(first_line(e)).color(color(p.status_danger_fg)));
            return Outcome::default();
        }
        SlotState::Ready if empty => {
            let none = match tab {
                Tab::Pulls | Tab::Mine => tr("No open pull requests."),
                Tab::Issues => tr("No open issues."),
                Tab::Review => tr("Nothing is waiting for your review."),
                Tab::Repos => tr("No repositories."),
            };
            ui.label(RichText::new(none).color(color(p.text_secondary)));
            return Outcome::default();
        }
        SlotState::Ready => {}
    }

    let mut open_p: Option<Pull> = None;
    let mut open_i: Option<Issue> = None;
    let mut clone: Option<String> = None;
    ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt(("github-list", tab as u8))
        .show(ui, |ui| match tab {
            Tab::Pulls | Tab::Mine | Tab::Review => {
                let items = match tab {
                    Tab::Pulls => &app.gh.pulls.items,
                    Tab::Mine => &app.gh.mine.items,
                    _ => &app.gh.review.items,
                };
                for pull in items {
                    if pull_row(ui, look, pull, tab != Tab::Pulls) {
                        open_p = Some(pull.clone());
                    }
                }
            }
            Tab::Issues => {
                for issue in &app.gh.issues.items {
                    if issue_row(ui, look, issue) {
                        open_i = Some(issue.clone());
                    }
                }
            }
            Tab::Repos => {
                for repo in &app.gh.repos.items {
                    if repo_row(ui, look, repo) {
                        clone = Some(repo.clone_url.clone());
                    }
                }
            }
        });
    if let Some(pull) = open_p {
        open_pull(app, &pull);
    }
    if let Some(issue) = open_i {
        open_issue(app, &issue);
    }
    if let Some(url) = clone {
        app.overlay = Overlay::Clone(CloneSetup::new(app, url));
    }
    Outcome::default()
}

fn sign_in_view(ui: &mut Ui, app: &mut App, look: &Look) {
    let p = look.palette;
    ui.label(
        RichText::new(tr(
            "Sign in to clone repositories and to see and review pull requests and issues without leaving Código.",
        ))
        .size(font_size::SMALL)
        .color(color(p.text_secondary)),
    );
    ui.add_space(8.0);

    match &app.gh.signin {
        SignIn::Starting | SignIn::Checking => {
            ui.label(RichText::new(tr("Connecting to GitHub…")).color(color(p.text_secondary)));
        }
        SignIn::Waiting { code, .. } => {
            ui.label(
                RichText::new(tr("Enter this code on github.com:"))
                    .size(font_size::SMALL)
                    .color(color(p.text_secondary)),
            );
            ui.label(
                RichText::new(&code.user_code)
                    .monospace()
                    .strong()
                    .size(font_size::LARGE)
                    .color(color(p.text_primary)),
            );
            let uri = code.verification_uri.clone();
            let user_code = code.user_code.clone();
            ui.horizontal(|ui| {
                if cr::secondary_button(ui, look, tr("Copy code"), true).clicked() {
                    ui.ctx().copy_text(user_code);
                }
                if cr::secondary_button(ui, look, tr("Open github.com"), true).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(uri));
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr("Waiting for you to authorise…"))
                    .size(font_size::SMALL)
                    .color(color(p.text_secondary)),
            );
            if cr::secondary_button(ui, look, tr("Cancel"), true).clicked() {
                if let SignIn::Waiting { cancel, .. } = &app.gh.signin {
                    cancel.store(true, Ordering::Relaxed);
                }
                app.gh.signin = SignIn::Idle;
            }
        }
        SignIn::Idle => {
            if client_id().is_some()
                && cr::primary_button(ui, look, tr("Sign in with GitHub"), true).clicked()
            {
                sign_in_device(app);
            }
            ui.add_space(4.0);
            if cr::secondary_button(ui, look, tr("Use the GitHub CLI session"), true).clicked() {
                sign_in_cli(app);
            }
            ui.add_space(10.0);
            section(ui, look, tr("Or paste a personal access token"));
            let response = cr::field_frame(look)
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut app.gh.token_input)
                            .frame(egui::Frame::NONE)
                            .password(true)
                            .hint_text("ghp_…")
                            .align(Align2::LEFT_CENTER)
                            .desired_width(f32::INFINITY),
                    )
                })
                .inner;
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(4.0);
            let ready = !app.gh.token_input.trim().is_empty();
            if cr::primary_button(ui, look, tr("Sign in"), ready).clicked() || (enter && ready) {
                let token = app.gh.token_input.trim().to_owned();
                verify(app, token, true);
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(tr(
                    "The token is kept in your system's keyring, never in a file. It needs the repo scope.",
                ))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
            );
        }
    }
    if let Some(e) = &app.gh.error {
        ui.add_space(6.0);
        ui.label(RichText::new(first_line(e)).color(color(p.status_danger_fg)));
    }
}

fn state_tint(look: &Look, state: &str, draft: bool) -> egui::Color32 {
    let p = &look.palette;
    color(match (state, draft) {
        (_, true) => p.text_secondary,
        ("open", _) => p.status_success_fg,
        ("merged", _) => p.accent_text,
        _ => p.status_danger_fg,
    })
}

pub(crate) fn row_frame(ui: &mut Ui, look: &Look, add: impl FnOnce(&mut Ui)) -> bool {
    let p = look.palette;
    let response = ui
        .scope(|ui| {
            ui.set_width(ui.available_width());
            add(ui);
        })
        .response;
    let response = ui.interact(response.rect, response.id.with("row"), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            response.rect,
            egui::CornerRadius::same(6),
            color(p.control_bg_hover).gamma_multiply(0.5),
        );
    }
    ui.add_space(2.0);
    response.clicked()
}

fn pull_row(ui: &mut Ui, look: &Look, pull: &Pull, show_repo: bool) -> bool {
    let p = look.palette;
    row_frame(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(
                RichText::new(if pull.draft { "◌" } else { "●" }).color(state_tint(
                    look,
                    &pull.state,
                    pull.draft,
                )),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(&pull.title)
                        .color(color(p.text_primary))
                        .size(font_size::BODY),
                )
                .truncate(),
            );
        });
        let repo = if show_repo {
            format!("{} ", pull.repo)
        } else {
            String::new()
        };
        ui.label(
            RichText::new(format!(
                "{repo}#{} · {} · {}",
                pull.number,
                pull.author,
                date(&pull.updated_at)
            ))
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
        );
    })
}

fn issue_row(ui: &mut Ui, look: &Look, issue: &Issue) -> bool {
    let p = look.palette;
    row_frame(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new("●").color(state_tint(look, &issue.state, false)));
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
                issue.number,
                issue.author,
                date(&issue.updated_at)
            ))
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
        );
    })
}

fn repo_row(ui: &mut Ui, look: &Look, repo: &RepoInfo) -> bool {
    let p = look.palette;
    row_frame(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add(
                egui::Label::new(
                    RichText::new(&repo.full_name)
                        .color(color(p.text_primary))
                        .size(font_size::BODY),
                )
                .truncate(),
            );
            if repo.private {
                ui.label(
                    RichText::new(tr("private"))
                        .size(font_size::SMALL)
                        .color(color(p.text_secondary)),
                );
            }
        });
        if !repo.description.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(&repo.description)
                        .size(font_size::SMALL)
                        .color(color(p.text_secondary)),
                )
                .truncate(),
            );
        }
    })
}

// --- The pull request and issue window --------------------------------------------------------

fn verdict_label(state: &str) -> &'static str {
    match state {
        "APPROVED" => "approved",
        "CHANGES_REQUESTED" => "requested changes",
        "DISMISSED" => "dismissed",
        _ => "reviewed",
    }
}

pub fn item_overlay(
    ctx: &egui::Context,
    app: &mut App,
    look: &Look,
    mut view: Box<ItemView>,
) -> Shown {
    let p = look.palette;
    let mut closed = false;
    let mut post: Option<(&'static str, ReviewEvent)> = None;
    let mut comment = false;
    let mut inline_post: Option<String> = None;
    let mut browser = false;
    let mut checkout = false;
    let same_repo = app
        .gh
        .repo
        .as_ref()
        .is_some_and(|(o, n)| format!("{o}/{n}") == view.repo);
    let dismissed = modal(ctx, look, "github-item", 860.0, true, |ui| {
        closed = heading(
            ui,
            look,
            &format!("{} #{}", view.repo, view.number),
            "github-item",
        );
        ScrollArea::vertical()
            .id_salt("github-item-scroll")
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
                        .color(state_tint(look, &view.state, view.draft)),
                    );
                    ui.label(
                        RichText::new(trf("by {}", &[&view.author]))
                            .size(font_size::SMALL)
                            .color(color(p.text_secondary)),
                    );
                    if view.is_pull && !view.head_ref.is_empty() {
                        ui.label(
                            RichText::new(format!("{} → {}", view.head_ref, view.base_ref))
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
                if let Some(Ok(l)) = &view.loaded {
                    if let Some(s) = &l.stats {
                        ui.label(
                            RichText::new(trf(
                                "{} files changed, +{} −{}",
                                &[
                                    &s.changed_files.to_string(),
                                    &s.additions.to_string(),
                                    &s.deletions.to_string(),
                                ],
                            ))
                            .size(font_size::SMALL)
                            .color(color(p.text_secondary)),
                        );
                    }
                    if !l.checks.is_empty() {
                        section(ui, look, tr("Checks"));
                        for c in &l.checks {
                            let (mark, tint) = match (c.status.as_str(), c.conclusion.as_deref()) {
                                ("completed", Some("success" | "neutral" | "skipped")) => {
                                    ("✓", color(p.status_success_fg))
                                }
                                ("completed", _) => ("✗", color(p.status_danger_fg)),
                                _ => ("●", color(p.status_warning_fg)),
                            };
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(mark).color(tint));
                                ui.label(
                                    RichText::new(&c.name)
                                        .size(font_size::SMALL)
                                        .color(color(p.text_primary)),
                                );
                            });
                        }
                    }
                }
                if !view.body.trim().is_empty() {
                    ui.add_space(8.0);
                    crate::markdown::show(ui, look, &clip(&view.body, 6000));
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
                        if view.is_pull && !l.files.is_empty() {
                            section(ui, look, &trf("Files ({})", &[&l.files.len().to_string()]));
                            for (i, f) in l.files.iter().enumerate() {
                                let on = view.file == Some(i);
                                let label =
                                    format!("{}   +{} −{}", f.filename, f.additions, f.deletions);
                                if ui.selectable_label(on, label).clicked() {
                                    view.file = if on { None } else { Some(i) };
                                }
                                if on {
                                    match &f.patch {
                                        Some(patch) => {
                                            if let Some(body) = review_diff(
                                                ui,
                                                look,
                                                &mut view,
                                                i,
                                                &f.filename,
                                                patch,
                                                &l.review_comments,
                                            ) {
                                                inline_post = Some(body);
                                            }
                                        }
                                        None => {
                                            ui.label(
                                                RichText::new(tr(
                                                    "GitHub has no diff to show for this file.",
                                                ))
                                                .color(color(p.text_secondary)),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        if !l.conversation.is_empty() {
                            section(ui, look, tr("Conversation"));
                        }
                        for c in &l.conversation {
                            ui.add_space(4.0);
                            let head = match &c.kind {
                                CommentKind::Comment => c.author.clone(),
                                CommentKind::Review(s) => {
                                    format!("{} {}", c.author, tr(verdict_label(s)))
                                }
                            };
                            ui.label(
                                RichText::new(format!("{head} · {}", date(&c.created_at)))
                                    .size(font_size::SMALL)
                                    .strong()
                                    .color(color(p.text_secondary)),
                            );
                            if !c.body.trim().is_empty() {
                                crate::markdown::show(ui, look, &clip(&c.body, 4000));
                            }
                        }
                    }
                }
                view.loaded = loaded;
            });
        ui.add_space(8.0);
        let box_response = cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut view.comment)
                        .frame(egui::Frame::NONE)
                        .hint_text(tr("Write a comment…"))
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .font(FontId::proportional(font_size::BODY)),
                )
            })
            .inner;
        let _ = box_response;
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let ready = !view.comment.trim().is_empty() && !view.busy;
            if cr::primary_button(ui, look, tr("Comment"), ready).clicked() {
                comment = true;
            }
            if view.is_pull && view.state == "open" {
                if cr::secondary_button(ui, look, tr("Approve"), !view.busy).clicked() {
                    post = Some(("Approved.", ReviewEvent::Approve));
                }
                if cr::secondary_button(ui, look, tr("Request changes"), ready).clicked() {
                    post = Some(("Changes requested.", ReviewEvent::RequestChanges));
                }
                if same_repo
                    && app.vcs.root.is_some()
                    && cr::secondary_button(ui, look, tr("Check out the branch"), !view.busy)
                        .clicked()
                {
                    checkout = true;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if cr::secondary_button(ui, look, tr("Open on GitHub"), true).clicked() {
                    browser = true;
                }
            });
        });
    });
    if browser {
        ctx.open_url(egui::OpenUrl::new_tab(view.url.clone()));
    }
    if let Some(body) = inline_post {
        view.busy = true;
        post_inline(app, &view, body);
    }
    if comment {
        view.busy = true;
        post_comment(app, view.repo.clone(), view.number, view.comment.clone());
    }
    if let Some((_, event)) = post {
        view.busy = true;
        post_review(
            app,
            view.repo.clone(),
            view.number,
            event,
            view.comment.clone(),
        );
    }
    if checkout {
        crate::vcs_ui::checkout_pull(app, view.number);
        return (Overlay::None, true);
    }
    if closed || dismissed {
        return (Overlay::None, true);
    }
    (Overlay::Item(view), false)
}

/// The diff of one file with a number beside each line. Click a line to comment on it; lines
/// that already have comments carry a dot, and the comments of the chosen line show below.
/// Returns the text of a review comment the person asked to post.
fn review_diff(
    ui: &mut Ui,
    look: &Look,
    view: &mut ItemView,
    index: usize,
    path: &str,
    patch: &str,
    comments: &[ReviewComment],
) -> Option<String> {
    let p = look.palette;
    let lines = match &view.parsed {
        Some((i, lines)) if *i == index => lines.clone(),
        _ => {
            let parsed: Vec<DiffLine> = diff_lines(patch).into_iter().take(20_000).collect();
            let lines = std::rc::Rc::new(parsed);
            view.parsed = Some((index, lines.clone()));
            lines
        }
    };
    let here: Vec<&ReviewComment> = comments.iter().filter(|c| c.path == path).collect();
    let has = |line: u64, left: bool| {
        here.iter()
            .any(|c| c.line == line && (c.side == "LEFT") == left)
    };
    let font = FontId::monospace(font_size::SMALL);
    let row = ui.fonts_mut(|f| f.row_height(&font)).max(18.0);
    let mut picked: Option<(u64, bool)> = None;
    ScrollArea::both()
        .id_salt(("pr-diff", index))
        .max_height(360.0)
        .auto_shrink([false, true])
        .show_rows(ui, row, lines.len(), |ui, range| {
            for i in range {
                let Some(line) = lines.get(i) else { continue };
                let place = line.place();
                let commented = place.is_some_and(|(n, left)| has(n, left));
                let chosen = place.is_some_and(|(n, left)| {
                    view.pick
                        .as_ref()
                        .is_some_and(|(f, pn, pl)| f == path && *pn == n && *pl == left)
                });
                let tint = match line.kind {
                    DiffKind::Added => color(p.status_success_fg),
                    DiffKind::Removed => color(p.status_danger_fg),
                    DiffKind::Hunk => color(p.status_info_fg),
                    _ => color(p.text_secondary),
                };
                let number = |n: Option<u64>| n.map_or("    ".to_owned(), |n| format!("{n:>4}"));
                let text = format!(
                    "{} {} {} {}",
                    if commented { "●" } else { " " },
                    number(line.old),
                    number(line.new),
                    line.text
                );
                let mut rich = RichText::new(text).font(font.clone()).color(tint);
                if chosen {
                    rich = rich.strong().background_color(color(p.control_bg));
                }
                let response = ui.add(egui::Label::new(rich).extend().sense(if place.is_some() {
                    egui::Sense::click()
                } else {
                    egui::Sense::hover()
                }));
                if response.clicked() {
                    picked = place;
                }
                if place.is_some() {
                    response.on_hover_text(tr("Click to comment on this line"));
                }
            }
        });
    if let Some((line, left)) = picked {
        view.pick = Some((path.to_owned(), line, left));
        view.reply_to = None;
    }
    let mut out = None;
    let (pick_path, line, left) = view.pick.clone()?;
    if pick_path != path {
        return None;
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new(trf(
            if left {
                "Line {} (before the change)"
            } else {
                "Line {}"
            },
            &[&line.to_string()],
        ))
        .strong()
        .size(font_size::SMALL)
        .color(color(p.text_secondary)),
    );
    for c in here
        .iter()
        .filter(|c| c.line == line && (c.side == "LEFT") == left)
    {
        ui.label(
            RichText::new(format!("{} · {}", c.author, date(&c.created_at)))
                .size(font_size::SMALL)
                .strong()
                .color(color(p.text_secondary)),
        );
        crate::markdown::show(ui, look, &clip(&c.body, 4000));
        if c.reply_to.is_none() && ui.small_button(tr("Reply")).clicked() {
            view.reply_to = Some(c.id);
        }
    }
    cr::field_frame(look)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut view.inline)
                    .frame(egui::Frame::NONE)
                    .hint_text(if view.reply_to.is_some() {
                        tr("Write a reply…")
                    } else {
                        tr("Comment on this line…")
                    })
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .font(FontId::proportional(font_size::BODY)),
            )
        });
    ui.horizontal(|ui| {
        let ready = !view.inline.trim().is_empty() && !view.busy;
        let label = if view.reply_to.is_some() {
            tr("Reply")
        } else {
            tr("Comment on the line")
        };
        if cr::primary_button(ui, look, label, ready).clicked() {
            out = Some(view.inline.clone());
        }
        if view.reply_to.is_some() && ui.small_button(tr("Start a new thread")).clicked() {
            view.reply_to = None;
        }
    });
    out
}

/// Opening a pull request from the branch that is checked out.
pub struct PullSetup {
    pub title: String,
    pub body: String,
    pub head: String,
    pub base: String,
    pub draft: bool,
    pub focus: bool,
    pub busy: bool,
    pub error: Option<String>,
}

/// Start a pull request for the current branch: the repository must be on GitHub, the person
/// signed in and on a branch.
pub fn open_create_pull(app: &mut App) {
    init(app);
    let Some((owner, name)) = app.gh.repo.clone() else {
        app.toast(tr("This folder is not a GitHub repository."));
        return;
    };
    let Some(api) = api(app) else {
        app.toast(tr("Sign in to GitHub first."));
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
    app.overlay = Overlay::NewPull(PullSetup {
        title,
        body: String::new(),
        head: branch,
        base: String::new(),
        draft: false,
        focus: true,
        busy: false,
        error: None,
    });
    spawn(app, move || {
        GhMsg::DefaultBranch(api.default_branch(&owner, &name).map_err(text))
    });
}

fn create_pull(app: &mut App, setup: &PullSetup) {
    let (Some(api), Some((owner, name)), Some(root)) =
        (api(app), app.gh.repo.clone(), app.vcs.root.clone())
    else {
        return;
    };
    let auth = vcs_auth(app);
    let (title, body, head, base, draft) = (
        setup.title.clone(),
        setup.body.clone(),
        setup.head.clone(),
        setup.base.clone(),
        setup.draft,
    );
    spawn(app, move || {
        let result = (|| {
            let repo = codigo_git::Repo::discover(&root)
                .ok_or_else(|| "this folder is not a git repository".to_owned())?;
            // The branch has to be on GitHub before a pull request can name it.
            repo.push(&auth).map_err(|e| e.to_string())?;
            api.create_pull(
                &owner,
                &name,
                &NewPull {
                    title: &title,
                    body: &body,
                    head: &head,
                    base: &base,
                    draft,
                },
            )
            .map(Box::new)
            .map_err(text)
        })();
        GhMsg::PullCreated(result)
    });
}

fn vcs_auth(app: &App) -> Auth {
    crate::vcs_ui::auth(app)
}

pub fn pull_overlay(
    ctx: &egui::Context,
    app: &mut App,
    look: &Look,
    mut state: PullSetup,
) -> Shown {
    let p = look.palette;
    let mut closed = false;
    let mut go = false;
    let dismissed = modal(ctx, look, "create-pull", 620.0, true, |ui| {
        closed = heading(ui, look, tr("Create a pull request"), "create-pull");
        ui.label(
            RichText::new(trf("{} into {}", &[&state.head, &state.base]))
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
                    egui::TextEdit::multiline(&mut state.body)
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
            RichText::new(tr("The branch is pushed to GitHub first."))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
        );
        if let Some(e) = &state.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(color(p.status_danger_fg)));
        }
        ui.add_space(10.0);
        let ready = !state.busy && !state.title.trim().is_empty() && !state.base.is_empty();
        let label = if state.busy {
            tr("Creating…")
        } else {
            tr("Create pull request")
        };
        if cr::primary_button(ui, look, label, ready).clicked() {
            go = true;
        }
    });
    if go {
        state.busy = true;
        state.error = None;
        create_pull(app, &state);
    }
    if closed || dismissed {
        return (Overlay::None, true);
    }
    (Overlay::NewPull(state), false)
}

/// At most `max` characters of a text, cut on a character boundary.
pub(crate) fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

// --- The clone window -------------------------------------------------------------------------

pub fn clone_overlay(
    ctx: &egui::Context,
    app: &mut App,
    look: &Look,
    mut state: CloneSetup,
) -> Shown {
    let p = look.palette;
    let mut closed = false;
    let mut go = false;
    let mut browse = false;
    let on = state.gitlab.clone();
    let address = clone_address_on(&state.url, on.as_deref().unwrap_or("github.com"));
    let name = codigo_git::folder_name(&address);
    let dismissed = modal(ctx, look, "clone", 560.0, true, |ui| {
        closed = heading(ui, look, tr("Clone a repository"), "clone");
        ui.label(
            RichText::new(if on.is_some() {
                tr("Address, or group/project on GitLab")
            } else {
                tr("Address, or owner/name on GitHub")
            })
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
        );
        let response = cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut state.url)
                        .frame(egui::Frame::NONE)
                        .hint_text(if on.is_some() {
                            "group/project"
                        } else {
                            "getnodalia/codigo"
                        })
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
        ui.add_space(8.0);
        ui.label(
            RichText::new(tr("Into this folder"))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
        );
        ui.horizontal(|ui| {
            let room = (ui.available_width() - 96.0).max(120.0);
            cr::field_frame(look)
                .inner_margin(egui::Margin::symmetric(10, 4))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.parent)
                            .frame(egui::Frame::NONE)
                            .align(Align2::LEFT_CENTER)
                            .desired_width(room),
                    )
                });
            if cr::secondary_button(ui, look, tr("Browse…"), !state.busy).clicked() {
                browse = true;
            }
        });
        if !name.is_empty() && !state.parent.trim().is_empty() {
            ui.add_space(4.0);
            ui.label(
                RichText::new(trf(
                    "It will be cloned to {}",
                    &[&std::path::Path::new(state.parent.trim())
                        .join(&name)
                        .display()
                        .to_string()],
                ))
                .size(font_size::SMALL)
                .color(color(p.text_secondary)),
            );
        }
        if let Some(e) = &state.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(color(p.status_danger_fg)));
        }
        ui.add_space(10.0);
        let ready = !state.busy
            && !name.is_empty()
            && !state.url.trim().is_empty()
            && !state.parent.trim().is_empty();
        let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            let label = if state.busy {
                tr("Cloning…")
            } else {
                tr("Clone")
            };
            if cr::primary_button(ui, look, label, ready).clicked() || (enter && ready) {
                go = true;
            }
        });
    });
    if browse {
        crate::dialogs::request(
            app,
            crate::dialogs::Request::new(crate::dialogs::Purpose::CloneParent),
        );
    }
    if go {
        state.busy = true;
        state.error = None;
        start_clone(app, address, state.parent.trim().to_owned());
    }
    if closed || dismissed {
        return (Overlay::None, true);
    }
    (Overlay::Clone(state), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_and_name_become_an_https_address() {
        assert_eq!(
            clone_address_on("getnodalia/codigo", "github.com"),
            "https://github.com/getnodalia/codigo.git"
        );
        assert_eq!(
            clone_address_on("  getnodalia/codigo \n", "github.com"),
            "https://github.com/getnodalia/codigo.git"
        );
    }

    #[test]
    fn a_short_name_on_gitlab_uses_that_server() {
        assert_eq!(
            clone_address_on("group/sub", "gitlab.example.org"),
            "https://gitlab.example.org/group/sub.git"
        );
    }

    #[test]
    fn full_addresses_and_odd_input_pass_through() {
        for s in [
            "https://github.com/a/b.git",
            "git@github.com:a/b.git",
            "-oProxyCommand=x/y",
            "a/b/c",
            "/a",
            "a b/c",
        ] {
            assert_eq!(clone_address_on(s, "github.com"), s.trim(), "{s}");
        }
    }

    #[test]
    fn clip_cuts_on_characters() {
        assert_eq!(clip("héllo", 10), "héllo");
        assert_eq!(clip("héllo", 2), "hé…");
    }

    #[test]
    fn dates_are_cut_safely() {
        assert_eq!(date("2026-11-26T10:00:00Z"), "2026-11-26");
        assert_eq!(date("é"), "é");
        assert_eq!(date(""), "");
    }
}
