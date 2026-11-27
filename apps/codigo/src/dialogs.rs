// SPDX-License-Identifier: MIT OR Apache-2.0
//! The system's file and folder choosers.
//!
//! A chooser is asked for on the window's own thread but runs on a worker thread, with the
//! window as its parent, so the window keeps answering the desktop while the chooser is open
//! and the chooser opens in front of it. On Linux the chooser is the desktop's own, through the
//! XDG desktop portal (so it is the default files app, with the person's bookmarks); `zenity` or
//! `kdialog` are the fallback.
//!
//! When no chooser can be opened at all (no portal service and no `zenity`), Código says so and
//! shows a small window where the path is typed, so opening a folder never silently does nothing.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

use codigo_engine::DocId;
use egui::{Align2, FontId, Key, RichText};
use nodalia_theme::creator::{self as cr, Look};
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;

use crate::app::{App, Msg};
use crate::i18n::{tr, trf};
use crate::overlays::{heading, modal, Overlay, Shown};

/// A chooser that closes this fast was not shown to anyone: it failed.
const TOO_FAST: Duration = Duration::from_millis(400);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    OpenFiles,
    OpenFolder,
    /// Choose where to save this untitled file or give it a new name.
    SaveAs(DocId),
    /// The folder a clone goes into, for the Clone window.
    CloneParent,
    /// The folder a new Nodalia app is created in, for the New app window.
    NewAppParent,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub purpose: Purpose,
    /// The folder the chooser starts in.
    pub dir: Option<PathBuf>,
    /// The file name proposed when saving.
    pub name: Option<String>,
}

impl Request {
    pub fn new(purpose: Purpose) -> Request {
        Request {
            purpose,
            dir: None,
            name: None,
        }
    }
}

pub enum Reply {
    /// The person chose these paths (none: they cancelled).
    Picked(Request, Vec<PathBuf>),
    /// No chooser could be shown, and why.
    Failed(Request, String),
}

type Paths = Option<Vec<PathBuf>>;

/// Run the future to its end on this thread.
fn block_on<T>(fut: impl Future<Output = T>) -> T {
    struct Unpark(Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let mut fut = std::pin::pin!(fut);
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        thread::park();
    }
}

/// Whether anything on this computer can show a chooser.
#[cfg(target_os = "linux")]
fn chooser_available() -> bool {
    fn portal() -> bool {
        const NAME: &str = "org.freedesktop.portal.Desktop";
        let Ok(conn) = zbus::blocking::Connection::session() else {
            return false;
        };
        let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&conn) else {
            return false;
        };
        let Ok(name) = zbus::names::BusName::try_from(NAME) else {
            return false;
        };
        dbus.name_has_owner(name).unwrap_or(false)
            || dbus
                .list_activatable_names()
                .is_ok_and(|names| names.iter().any(|n| n.as_str() == NAME))
    }
    fn on_path(program: &str) -> bool {
        std::env::var_os("PATH")
            .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(program).is_file()))
    }
    portal() || on_path("zenity") || on_path("kdialog")
}

#[cfg(not(target_os = "linux"))]
fn chooser_available() -> bool {
    true
}

/// What the portal answered.
#[cfg(target_os = "linux")]
mod portal {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use zbus::zvariant::{OwnedValue, Value};

    use super::{Purpose, Request};
    use crate::i18n::tr;

    const SERVICE: &str = "org.freedesktop.portal.Desktop";
    const ROOT: &str = "/org/freedesktop/portal/desktop";

    /// `Ok(None)`: the person closed the chooser. `Err`: it could not be shown, and why.
    pub type Answer = Result<Option<Vec<PathBuf>>, String>;

    /// Ask the XDG desktop portal for a chooser directly, so that a failure comes with its reason
    /// (the chooser library only says that it failed). No parent window: that is the difference
    /// from the first attempt, and the one most likely to matter when that attempt failed.
    pub fn pick(req: &Request) -> Answer {
        let conn = zbus::blocking::Connection::session()
            .map_err(|e| format!("the session bus is not reachable: {e}"))?;
        pick_on(&conn, req)
    }

    pub fn pick_on(conn: &zbus::blocking::Connection, req: &Request) -> Answer {
        let sender = conn
            .unique_name()
            .map(|n| n.as_str().trim_start_matches(':').replace('.', "_"))
            .ok_or_else(|| "the session bus gave no name".to_owned())?;
        let token = format!(
            "codigo{}x{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos())
        );
        let expected = format!("{ROOT}/request/{sender}/{token}");
        let mut responses = listen(conn, &expected)?;

        let (method, title) = match req.purpose {
            Purpose::OpenFiles => ("OpenFile", tr("Open file")),
            Purpose::SaveAs(_) => ("SaveFile", tr("Save as")),
            _ => ("OpenFile", tr("Open folder")),
        };
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("handle_token", Value::from(token.as_str()));
        match req.purpose {
            Purpose::OpenFiles => {
                options.insert("multiple", Value::from(true));
            }
            Purpose::SaveAs(_) => {
                if let Some(name) = &req.name {
                    options.insert("current_name", Value::from(name.as_str()));
                }
            }
            _ => {
                options.insert("directory", Value::from(true));
            }
        }
        if let Some(dir) = req.dir.as_ref().and_then(|d| d.to_str()) {
            let mut bytes = dir.as_bytes().to_vec();
            bytes.push(0);
            options.insert("current_folder", Value::from(bytes));
        }

        let reply = conn
            .call_method(
                Some(SERVICE),
                ROOT,
                Some("org.freedesktop.portal.FileChooser"),
                method,
                &("", title, options),
            )
            .map_err(|e| format!("the portal refused the request: {e}"))?;
        // Portals older than 0.9 answer on a path of their own choosing.
        if let Ok(handle) = reply
            .body()
            .deserialize::<zbus::zvariant::OwnedObjectPath>()
        {
            if handle.as_str() != expected {
                responses = listen(conn, handle.as_str())?;
            }
        }

        let Some(signal) = responses.next() else {
            return Err("the portal closed the connection without an answer".to_owned());
        };
        let (code, results): (u32, HashMap<String, OwnedValue>) = signal
            .body()
            .deserialize()
            .map_err(|e| format!("the portal's answer is unreadable: {e}"))?;
        match code {
            0 => {
                let mut paths = Vec::new();
                if let Some(value) = results.get("uris") {
                    if let Value::Array(uris) = &**value {
                        for uri in uris.iter() {
                            if let Value::Str(uri) = uri {
                                paths.extend(uri_to_path(uri.as_str()));
                            }
                        }
                    }
                }
                Ok(Some(paths).filter(|p| !p.is_empty()))
            }
            1 => Ok(None),
            n => Err(format!("the chooser could not be shown (portal code {n})")),
        }
    }

    fn listen(
        conn: &zbus::blocking::Connection,
        path: &str,
    ) -> Result<zbus::blocking::proxy::SignalIterator<'static>, String> {
        zbus::blocking::Proxy::new(
            conn,
            SERVICE,
            path.to_owned(),
            "org.freedesktop.portal.Request",
        )
        .and_then(|p| p.receive_signal("Response"))
        .map_err(|e| format!("cannot listen for the portal's answer: {e}"))
    }

    /// `file:///home/a%20b` → `/home/a b`. Anything that is not a local file is dropped.
    pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
        use std::os::unix::ffi::OsStringExt;
        let rest = uri.strip_prefix("file://")?;
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        let mut out = Vec::with_capacity(rest.len());
        let bytes = rest.as_bytes();
        let mut i = 0;
        while let Some(&b) = bytes.get(i) {
            if b == b'%' {
                let hex = rest.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            } else {
                out.push(b);
                i += 1;
            }
        }
        Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
    }
}

/// Ask for a chooser; it opens at the start of the next frame.
pub fn request(app: &mut App, req: Request) {
    app.pending_pick = Some(req);
    app.egui_ctx().request_repaint();
}

/// Open the chooser asked for. Called with the window, so the chooser can be its child.
pub fn start(app: &mut App, frame: &eframe::Frame, req: Request) {
    if app.picking {
        return;
    }
    let mut dialog = rfd::AsyncFileDialog::new().set_parent(frame);
    if let Some(dir) = &req.dir {
        dialog = dialog.set_directory(dir);
    }
    if let Some(name) = &req.name {
        dialog = dialog.set_file_name(name);
    }
    let paths = |handles: Vec<rfd::FileHandle>| -> Vec<PathBuf> {
        handles
            .into_iter()
            .map(|h| h.path().to_path_buf())
            .collect()
    };
    let one =
        |handle: Option<rfd::FileHandle>| -> Paths { handle.map(|h| vec![h.path().to_path_buf()]) };
    let fut: Pin<Box<dyn Future<Output = Paths> + Send>> = match req.purpose {
        Purpose::OpenFiles => Box::pin(async move { dialog.pick_files().await.map(paths) }),
        Purpose::OpenFolder | Purpose::CloneParent | Purpose::NewAppParent => {
            Box::pin(async move { one(dialog.pick_folder().await) })
        }
        Purpose::SaveAs(_) => Box::pin(async move { one(dialog.save_file().await) }),
    };
    let tx = app.sender();
    let ctx = app.egui_ctx().clone();
    let sent = req.clone();
    let spawned = thread::Builder::new()
        .name("codigo-dialog".into())
        .spawn(move || {
            let began = Instant::now();
            let reply = match block_on(fut) {
                Some(paths) if !paths.is_empty() => Reply::Picked(sent, paths),
                // Closed with nothing chosen: the person cancelled, unless nothing was shown.
                _ if began.elapsed() >= TOO_FAST && chooser_available() => {
                    Reply::Picked(sent, Vec::new())
                }
                _ => second_try(sent),
            };
            let _ = tx.send(Msg::Pick(reply));
            ctx.request_repaint();
        });
    match spawned {
        Ok(_) => app.picking = true,
        Err(e) => {
            tracing::warn!("file chooser: {e}");
            fallback(app, req, e.to_string());
        }
    }
}

/// The first attempt showed nothing. Try the portal directly, without a parent window, which says
/// why when it fails; otherwise report the reason.
#[cfg(target_os = "linux")]
fn second_try(req: Request) -> Reply {
    match portal::pick(&req) {
        Ok(Some(paths)) => Reply::Picked(req, paths),
        Ok(None) => Reply::Picked(req, Vec::new()),
        Err(why) => {
            let why = if chooser_available() {
                why
            } else {
                format!("{why}; no portal service and no zenity or kdialog were found")
            };
            Reply::Failed(req, why)
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn second_try(req: Request) -> Reply {
    Reply::Failed(req, "the system chooser did not open".to_owned())
}

/// What a chooser's answer does.
pub fn on_reply(app: &mut App, reply: Reply) {
    app.picking = false;
    match reply {
        Reply::Failed(req, why) => {
            tracing::warn!("no system file chooser could be shown: {why}");
            fallback(app, req, why);
        }
        Reply::Picked(req, paths) => chosen(app, req.purpose, paths),
    }
}

fn chosen(app: &mut App, purpose: Purpose, paths: Vec<PathBuf>) {
    let Some(first) = paths.first().cloned() else {
        return;
    };
    match purpose {
        Purpose::OpenFiles => {
            for p in paths {
                app.open_path(&p, None, true);
            }
        }
        Purpose::OpenFolder => app.open_project(first, true),
        Purpose::SaveAs(id) => {
            if let Some(i) = app.tabs.iter().position(|t| t.doc.id() == id) {
                app.save_as(i, &first);
            }
        }
        Purpose::CloneParent => {
            if let Overlay::Clone(state) = &mut app.overlay {
                state.parent = first.display().to_string();
            }
        }
        Purpose::NewAppParent => {
            if let Overlay::NewApp(state) = &mut app.overlay {
                state.parent = first;
            }
        }
    }
}

/// No chooser opened: say so, and let the path be typed where that makes sense.
fn fallback(app: &mut App, req: Request, why: String) {
    match req.purpose {
        Purpose::CloneParent | Purpose::NewAppParent => {
            app.toast(tr(
                "No file chooser opened. Type the folder in the field instead.",
            ));
        }
        _ => {
            app.toast(tr(
                "No file chooser opened. Install xdg-desktop-portal or zenity, or type the path here.",
            ));
            app.overlay = Overlay::Path(PathPrompt::new(req, why));
        }
    }
}

/// The typed-path window shown when no chooser can open.
#[derive(Clone)]
pub struct PathPrompt {
    req: Request,
    text: String,
    error: Option<String>,
    /// Why no chooser opened, for the person to read (and to report).
    why: String,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

impl PathPrompt {
    pub fn new(req: Request, why: String) -> PathPrompt {
        let dir = req.dir.clone().or_else(home).unwrap_or_default();
        let mut start = dir.display().to_string();
        match (&req.purpose, &req.name) {
            (Purpose::SaveAs(_), Some(name)) => {
                start = dir.join(name).display().to_string();
            }
            _ => {
                if !start.ends_with(std::path::MAIN_SEPARATOR) {
                    start.push(std::path::MAIN_SEPARATOR);
                }
            }
        }
        PathPrompt {
            req,
            text: start,
            error: None,
            why,
        }
    }

    fn title(&self) -> String {
        match self.req.purpose {
            Purpose::OpenFiles => tr("Open file").to_owned(),
            Purpose::SaveAs(_) => tr("Save as").to_owned(),
            _ => tr("Open folder").to_owned(),
        }
    }
}

/// `~` and `~/…` mean the home folder.
fn expand(text: &str) -> PathBuf {
    let text = text.trim();
    if text == "~" {
        return home().unwrap_or_else(|| PathBuf::from(text));
    }
    match (text.strip_prefix("~/"), home()) {
        (Some(rest), Some(h)) => h.join(rest),
        _ => PathBuf::from(text),
    }
}

/// Check what was typed against what the purpose needs.
fn accept(prompt: &PathPrompt) -> Result<PathBuf, String> {
    let path = expand(&prompt.text);
    if prompt.text.trim().is_empty() {
        return Err(tr("Type a path.").to_owned());
    }
    match prompt.req.purpose {
        Purpose::OpenFolder | Purpose::CloneParent | Purpose::NewAppParent => {
            if path.is_dir() {
                Ok(path)
            } else {
                Err(trf("{} is not a folder.", &[&path.display().to_string()]))
            }
        }
        Purpose::OpenFiles => {
            if path.is_file() {
                Ok(path)
            } else {
                Err(trf("{} is not a file.", &[&path.display().to_string()]))
            }
        }
        Purpose::SaveAs(_) => match path.parent() {
            _ if path.is_dir() => Err(trf("{} is a folder.", &[&path.display().to_string()])),
            Some(dir) if dir.as_os_str().is_empty() || dir.is_dir() => Ok(path),
            _ => Err(trf(
                "The folder of {} does not exist.",
                &[&path.display().to_string()],
            )),
        },
    }
}

pub fn overlay(ctx: &egui::Context, app: &mut App, look: &Look, mut prompt: PathPrompt) -> Shown {
    let p = look.palette;
    let mut go = false;
    let mut closed = false;
    let dismissed = modal(ctx, look, "path-prompt", 640.0, true, |ui| {
        closed = heading(ui, look, &prompt.title(), "path-prompt");
        ui.label(
            RichText::new(tr(
                "The system file chooser is not available, so type the path.",
            ))
            .size(font_size::SMALL)
            .color(color(p.text_secondary)),
        );
        if !prompt.why.is_empty() {
            ui.label(
                RichText::new(trf("Reason: {}", &[&prompt.why]))
                    .size(font_size::SMALL)
                    .color(color(p.text_secondary)),
            );
        }
        ui.add_space(8.0);
        let response = cr::field_frame(look)
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut prompt.text)
                        .frame(egui::Frame::NONE)
                        .align(Align2::LEFT_CENTER)
                        .desired_width(f32::INFINITY)
                        .font(FontId::monospace(font_size::BODY)),
                )
            })
            .inner;
        if !response.has_focus() && !response.lost_focus() {
            response.request_focus();
        }
        if let Some(e) = &prompt.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(color(p.status_danger_fg)));
        }
        ui.add_space(10.0);
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
            go = true;
        }
        if cr::primary_button(ui, look, tr("Choose"), true).clicked() {
            go = true;
        }
    });
    if closed || dismissed {
        return (Overlay::None, true);
    }
    if go {
        match accept(&prompt) {
            Ok(path) => {
                chosen(app, prompt.req.purpose.clone(), vec![path]);
                return (Overlay::None, true);
            }
            Err(e) => prompt.error = Some(e),
        }
    }
    (Overlay::Path(prompt), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(purpose: Purpose, text: &str) -> PathPrompt {
        PathPrompt {
            req: Request::new(purpose),
            text: text.to_owned(),
            error: None,
            why: String::new(),
        }
    }

    #[test]
    fn block_on_finishes_a_future_that_needs_waking() {
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            let _ = tx.send(7);
        });
        let got = block_on(std::future::poll_fn(|cx| match rx.try_recv() {
            Ok(v) => Poll::Ready(v),
            Err(_) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }));
        let _ = worker.join();
        assert_eq!(got, 7);
    }

    #[test]
    fn a_typed_folder_must_be_a_folder_and_a_file_a_file() {
        let d = tempfile::tempdir().expect("tempdir");
        let file = d.path().join("a.txt");
        std::fs::write(&file, "x").expect("write");
        let dir = d.path().display().to_string();
        assert!(accept(&prompt(Purpose::OpenFolder, &dir)).is_ok());
        assert!(accept(&prompt(Purpose::OpenFolder, &file.display().to_string())).is_err());
        assert!(accept(&prompt(Purpose::OpenFiles, &file.display().to_string())).is_ok());
        assert!(accept(&prompt(Purpose::OpenFiles, &dir)).is_err());
        assert!(accept(&prompt(Purpose::OpenFolder, "   ")).is_err());
        assert!(accept(&prompt(Purpose::OpenFolder, "/no/such/place/at/all")).is_err());
    }

    #[test]
    fn saving_needs_an_existing_folder_and_not_a_folder_as_the_name() {
        let d = tempfile::tempdir().expect("tempdir");
        let new = d.path().join("new.txt").display().to_string();
        assert!(accept(&prompt(Purpose::SaveAs(DocId(1)), &new)).is_ok());
        assert!(accept(&prompt(
            Purpose::SaveAs(DocId(1)),
            &d.path().display().to_string()
        ))
        .is_err());
        let lost = d.path().join("nope/new.txt").display().to_string();
        assert!(accept(&prompt(Purpose::SaveAs(DocId(1)), &lost)).is_err());
    }

    #[test]
    fn a_tilde_means_the_home_folder() {
        if let Some(h) = home() {
            assert_eq!(expand("~"), h);
            assert_eq!(expand("~/x"), h.join("x"));
        }
        assert_eq!(expand("/a/b"), PathBuf::from("/a/b"));
    }

    #[cfg(unix)]
    #[test]
    fn the_prompt_starts_in_the_folder_asked_for() {
        let mut req = Request::new(Purpose::OpenFolder);
        req.dir = Some(PathBuf::from("/srv/project"));
        assert!(PathPrompt::new(req, String::new())
            .text
            .starts_with("/srv/project"));
        let mut req = Request::new(Purpose::SaveAs(DocId(2)));
        req.dir = Some(PathBuf::from("/srv/project"));
        req.name = Some("notes.md".into());
        assert_eq!(
            PathPrompt::new(req, String::new()).text,
            "/srv/project/notes.md"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn portal_uris_become_paths() {
        use std::path::Path;
        let p = portal::uri_to_path;
        assert_eq!(
            p("file:///home/a%20b/c"),
            Some(Path::new("/home/a b/c").into())
        );
        assert_eq!(
            p("file://localhost/tmp/x"),
            Some(Path::new("/tmp/x").into())
        );
        assert_eq!(p("file:///caf%C3%A9"), Some(Path::new("/café").into()));
        assert_eq!(p("https://example.com/x"), None);
        assert_eq!(p("file:///bad%2"), None);
        assert_eq!(p("file:///bad%zz"), None);
    }

    /// A stand-in for the desktop's portal on a private bus: it answers `OpenFile` with a fixed
    /// result and records what it was asked.
    #[cfg(target_os = "linux")]
    mod fake_portal {
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

        pub struct Chooser {
            pub code: u32,
            pub uris: Vec<String>,
            pub asked: Arc<Mutex<Vec<String>>>,
        }

        #[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
        impl Chooser {
            fn open_file(
                &self,
                parent: String,
                title: String,
                options: HashMap<String, OwnedValue>,
                #[zbus(connection)] conn: &zbus::Connection,
                #[zbus(header)] header: zbus::message::Header<'_>,
            ) -> OwnedObjectPath {
                let token = options
                    .get("handle_token")
                    .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
                    .unwrap_or_default();
                let mut keys: Vec<_> = options.keys().cloned().collect();
                keys.sort();
                if let Ok(mut asked) = self.asked.lock() {
                    asked.push(format!("{parent}|{title}|{}", keys.join(",")));
                }
                let sender = header
                    .sender()
                    .map(|s| s.as_str().trim_start_matches(':').replace('.', "_"))
                    .unwrap_or_default();
                let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
                let mut results: HashMap<&str, Value<'_>> = HashMap::new();
                results.insert("uris", Value::from(self.uris.clone()));
                let _ = zbus::block_on(conn.emit_signal(
                    None::<&str>,
                    path.as_str(),
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(self.code, results),
                ));
                OwnedObjectPath::try_from(path).unwrap_or_default()
            }
        }
    }

    /// Start a private session bus, or `None` when this machine has no `dbus-daemon`.
    #[cfg(target_os = "linux")]
    fn private_bus() -> Option<(std::process::Child, String)> {
        use std::io::{BufRead, BufReader};
        let mut daemon = std::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .ok()?;
        let mut address = String::new();
        BufReader::new(daemon.stdout.take()?)
            .read_line(&mut address)
            .ok()?;
        Some((daemon, address.trim().to_owned()))
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_portal_is_asked_directly_and_its_answers_are_read() {
        use std::sync::{Arc, Mutex};
        use zbus::blocking::connection::Builder;
        let Some((mut daemon, address)) = private_bus() else {
            eprintln!("no dbus-daemon: skipped");
            return;
        };
        let asked = Arc::new(Mutex::new(Vec::new()));
        let run = |code: u32, uris: Vec<String>, req: &Request| {
            let service = Builder::address(address.as_str())
                .and_then(|b| b.name("org.freedesktop.portal.Desktop"))
                .and_then(|b| {
                    b.serve_at(
                        "/org/freedesktop/portal/desktop",
                        fake_portal::Chooser {
                            code,
                            uris,
                            asked: asked.clone(),
                        },
                    )
                })
                .and_then(|b| b.build())
                .expect("fake portal");
            let client = Builder::address(address.as_str())
                .and_then(|b| b.build())
                .expect("client");
            let answer = portal::pick_on(&client, req);
            drop(service);
            answer
        };

        let folder = run(
            0,
            vec!["file:///home/a%20b".to_owned()],
            &Request::new(Purpose::OpenFolder),
        );
        assert_eq!(folder, Ok(Some(vec![PathBuf::from("/home/a b")])));
        let mut start = Request::new(Purpose::OpenFiles);
        start.dir = Some(PathBuf::from("/srv/p"));
        assert_eq!(run(1, Vec::new(), &start), Ok(None));
        assert!(run(2, Vec::new(), &start).is_err());

        let asked = asked.lock().expect("log").clone();
        // The title is translated, so only its position and the options are pinned.
        assert!(asked[0].ends_with("|directory,handle_token"), "{asked:?}");
        assert!(
            asked[1].ends_with("|current_folder,handle_token,multiple"),
            "{asked:?}"
        );
        let _ = daemon.kill();
        let _ = daemon.wait();
    }
}
