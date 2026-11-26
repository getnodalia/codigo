// SPDX-License-Identifier: MIT OR Apache-2.0
//! A program running in a pseudo-terminal, and the screen it has drawn so far.

use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

/// Called from the reader thread whenever the screen changed, so a window can repaint.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Lines of history kept above the screen.
const SCROLLBACK: usize = 5_000;

#[derive(Debug, thiserror::Error)]
pub enum TermError {
    #[error("could not open a terminal: {0}")]
    Pty(String),
    #[error("could not start {program}: {reason}")]
    Spawn { program: String, reason: String },
}

/// What to run, and where.
#[derive(Clone, Debug)]
pub struct Spec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

/// Whether `path` is a program this person could run.
#[cfg(unix)]
fn runnable(path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The login shell the system lists for `user`, from a `passwd` file's text.
#[cfg(unix)]
fn passwd_shell(passwd: &str, user: &str) -> Option<String> {
    passwd.lines().find_map(|line| {
        let mut f = line.split(':');
        let name = f.next()?;
        let shell = f.nth(5)?;
        (name == user && !shell.is_empty()).then(|| shell.to_owned())
    })
}

/// The first shell that exists: the one in `$SHELL`, the one the system lists for the person,
/// then the common ones. A desktop launcher does not always pass `$SHELL` on, and a stale one
/// (a shell that was uninstalled) must not leave the terminal empty.
#[cfg(unix)]
fn choose_shell(
    env_shell: Option<String>,
    listed: Option<String>,
    usable: impl Fn(&str) -> bool,
) -> String {
    env_shell
        .into_iter()
        .chain(listed)
        .chain(
            [
                "/bin/bash",
                "/usr/bin/bash",
                "/bin/zsh",
                "/bin/sh",
                "/usr/bin/sh",
            ]
            .map(String::from),
        )
        .find(|s| !s.is_empty() && usable(s))
        .unwrap_or_else(|| "/bin/sh".to_owned())
}

/// The user's shell, as a [`Spec`] to run in `cwd`.
pub fn shell_command(cwd: PathBuf) -> Spec {
    #[cfg(windows)]
    let program = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_owned());
    #[cfg(not(windows))]
    let program = {
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .ok();
        let listed = user.and_then(|u| {
            let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
            passwd_shell(&passwd, &u)
        });
        choose_shell(std::env::var("SHELL").ok(), listed, runnable)
    };
    let mut env = vec![("SHELL".to_owned(), program.clone())];
    // A session started from a desktop launcher may have no locale; the screen expects UTF-8.
    if std::env::var_os("LANG").is_none() && std::env::var_os("LC_ALL").is_none() {
        env.push(("LANG".to_owned(), "C.UTF-8".to_owned()));
    }
    Spec {
        program,
        args: Vec::new(),
        cwd,
        env,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    /// The program ended with this exit code.
    Exited(u32),
}

struct Shared {
    parser: vt100::Parser,
    /// Bumped each time output arrives, so a view can tell the screen changed.
    generation: u64,
    /// The output stream ended: the program closed its side.
    closed: bool,
}

pub struct Terminal {
    shared: Arc<Mutex<Shared>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    status: Status,
    size: (u16, u16),
}

fn nz(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n).unwrap_or(NonZeroU16::MIN)
}

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Terminal {
    /// Start `spec` in a terminal of `rows` x `cols` cells.
    pub fn spawn(
        spec: &Spec,
        rows: u16,
        cols: u16,
        waker: Option<Waker>,
    ) -> Result<Terminal, TermError> {
        let (rows, cols) = (rows.max(1), cols.max(1));
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| TermError::Pty(e.to_string()))?;
        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        // A project folder that went away must not stop the terminal from opening.
        let cwd = if spec.cwd.is_dir() {
            spec.cwd.clone()
        } else {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|h| h.is_dir())
                .unwrap_or_else(std::env::temp_dir)
        };
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| TermError::Spawn {
                program: spec.program.clone(),
                reason: e.to_string(),
            })?;
        // The child holds the slave end now; keeping ours open would stop us seeing it exit.
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| TermError::Pty(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| TermError::Pty(e.to_string()))?;
        let shared = Arc::new(Mutex::new(Shared {
            parser: vt100::Parser::new(nz(rows), nz(cols), SCROLLBACK),
            generation: 0,
            closed: false,
        }));
        let feed = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("codigo-term".into())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let mut s = lock(&feed);
                            s.parser.process(buf.get(..n).unwrap_or_default());
                            s.generation += 1;
                            drop(s);
                            if let Some(wake) = &waker {
                                wake();
                            }
                        }
                    }
                }
                lock(&feed).closed = true;
                if let Some(wake) = &waker {
                    wake();
                }
            });
        if let Err(e) = spawned {
            return Err(TermError::Pty(e.to_string()));
        }
        Ok(Terminal {
            shared,
            writer,
            master: pair.master,
            child,
            status: Status::Running,
            size: (rows, cols),
        })
    }

    /// Send bytes to the program, as if typed.
    pub fn write(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || self.status != Status::Running {
            return;
        }
        if self
            .writer
            .write_all(bytes)
            .and_then(|()| self.writer.flush())
            .is_err()
        {
            tracing::debug!("terminal: the program is no longer reading");
        }
        // Typing returns the view to the live screen.
        lock(&self.shared).parser.screen_mut().set_scrollback(0);
    }

    /// Resize the screen and tell the program.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(1), cols.max(1));
        if (rows, cols) == self.size {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        lock(&self.shared)
            .parser
            .screen_mut()
            .set_size(nz(rows), nz(cols));
    }

    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    /// Read the screen under the lock. Keep `f` short: output waits while it runs.
    pub fn screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        f(lock(&self.shared).parser.screen())
    }

    /// How output has changed since the last call with the same number.
    pub fn generation(&self) -> u64 {
        lock(&self.shared).generation
    }

    /// Scroll back through history by `delta` lines (positive goes up). Returns the new offset.
    pub fn scroll(&self, delta: i32) -> usize {
        let mut s = lock(&self.shared);
        let now = s.parser.screen().scrollback();
        let next = if delta >= 0 {
            now.saturating_add(delta.unsigned_abs() as usize)
        } else {
            now.saturating_sub(delta.unsigned_abs() as usize)
        };
        s.parser.screen_mut().set_scrollback(next);
        s.parser.screen().scrollback()
    }

    /// Whether the program is still running. Polled, never blocks.
    pub fn status(&mut self) -> Status {
        if self.status == Status::Running {
            if let Ok(Some(done)) = self.child.try_wait() {
                self.status = Status::Exited(done.exit_code());
            }
        }
        self.status
    }

    /// Ask the program to stop.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    /// The output stream has ended, so everything the program printed is on the screen.
    pub fn output_done(&self) -> bool {
        lock(&self.shared).closed
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn run(script: &str) -> Terminal {
        let spec = Spec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            env: Vec::new(),
        };
        Terminal::spawn(&spec, 10, 40, None).expect("spawn")
    }

    fn wait_for(term: &mut Terminal, what: impl Fn(&Terminal) -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if what(term) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out");
    }

    #[test]
    fn output_reaches_the_screen_and_the_exit_code_is_reported() {
        let mut t = run("printf 'hola\\n'; exit 3");
        wait_for(&mut t, |t| t.output_done());
        assert!(t.screen(|s| s.contents()).contains("hola"));
        wait_for(&mut t, |_| true);
        let end = Instant::now() + Duration::from_secs(5);
        while t.status() == Status::Running && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(t.status(), Status::Exited(3));
    }

    #[test]
    fn typing_reaches_the_program() {
        let mut t = run("read line; echo got:$line");
        t.write(b"abc\r");
        wait_for(&mut t, |t| t.screen(|s| s.contents()).contains("got:abc"));
    }

    #[test]
    fn colours_and_resizing_work() {
        let mut t = run("printf '\\033[31mred\\033[0m'");
        wait_for(&mut t, |t| t.output_done());
        let colour = t.screen(|s| s.cell(0, 0).map(|c| c.fgcolor()));
        assert_eq!(colour, Some(vt100::Color::Idx(1)));
        t.resize(20, 80);
        assert_eq!(t.size(), (20, 80));
        assert_eq!(t.screen(|s| s.size()), (nz(20), nz(80)));
    }

    #[test]
    fn a_missing_program_is_an_error() {
        let spec = Spec {
            program: "/no/such/program".into(),
            args: Vec::new(),
            cwd: std::env::temp_dir(),
            env: Vec::new(),
        };
        assert!(Terminal::spawn(&spec, 5, 20, None).is_err());
    }

    #[test]
    fn a_stale_shell_falls_through_to_the_listed_one_and_then_the_common_ones() {
        let usable = |s: &str| s == "/usr/bin/fish" || s == "/bin/sh";
        // $SHELL is gone: the system's listing wins over the common shells.
        assert_eq!(
            choose_shell(
                Some("/gone/zsh".into()),
                Some("/usr/bin/fish".into()),
                usable
            ),
            "/usr/bin/fish"
        );
        // No $SHELL at all (a desktop launcher): the listing is used.
        assert_eq!(
            choose_shell(None, Some("/usr/bin/fish".into()), usable),
            "/usr/bin/fish"
        );
        // Neither: a common shell.
        assert_eq!(choose_shell(None, None, usable), "/bin/sh");
        // Nothing is usable: still a program name, so the error says which.
        assert_eq!(choose_shell(None, None, |_| false), "/bin/sh");
        assert_eq!(choose_shell(Some(String::new()), None, usable), "/bin/sh");
    }

    #[test]
    fn the_system_listing_gives_the_login_shell() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\ndaniel:x:1000:1000:D:/home/daniel:/usr/bin/zsh\nnobody:x:65534:65534::/:\n";
        assert_eq!(
            passwd_shell(passwd, "daniel").as_deref(),
            Some("/usr/bin/zsh")
        );
        assert_eq!(passwd_shell(passwd, "root").as_deref(), Some("/bin/bash"));
        assert_eq!(passwd_shell(passwd, "nobody"), None);
        assert_eq!(passwd_shell(passwd, "nobody2"), None);
        assert_eq!(passwd_shell("junk without colons", "junk"), None);
    }

    #[test]
    fn the_shell_opens_even_when_the_folder_is_gone() {
        let spec = Spec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "printf here".into()],
            cwd: PathBuf::from("/no/such/folder"),
            env: Vec::new(),
        };
        let mut t = Terminal::spawn(&spec, 5, 20, None).expect("spawn");
        wait_for(&mut t, |t| t.output_done());
        assert!(t.screen(|s| s.contents()).contains("here"));
    }

    #[test]
    fn history_scrolls() {
        let mut t = run("seq 1 100");
        wait_for(&mut t, |t| t.output_done());
        assert!(t.scroll(5) > 0);
        assert_eq!(t.scroll(-1000), 0);
    }
}
