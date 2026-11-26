// SPDX-License-Identifier: MIT OR Apache-2.0
//! An open file: its text, its syntax tree, and what is known about it on disk.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use codigo_syntax::{detect_with_first_line, LanguageDef, Syntax};
use codigo_text::{decode, encode, Buffer, Change, DecodeError, Indent, LineEnding};
use thiserror::Error;

/// Files bigger than this are refused: an editor is for source, and a rope of a 1 GB log is a
/// way to freeze the window.
pub const MAX_OPEN_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocId(pub u64);

#[derive(Debug, Error)]
pub enum OpenError {
    #[error("could not read the file: {0}")]
    Io(#[from] io::Error),
    #[error("the file is binary")]
    Binary,
    #[error("the file is not UTF-8")]
    NotUtf8,
    #[error("the file is larger than {} MB", MAX_OPEN_BYTES / 1024 / 1024)]
    TooLarge,
}

impl From<DecodeError> for OpenError {
    fn from(e: DecodeError) -> Self {
        match e {
            DecodeError::Binary => OpenError::Binary,
            DecodeError::NotUtf8 => OpenError::NotUtf8,
        }
    }
}

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("this document has no file yet")]
    NoPath,
    #[error("could not write the file: {0}")]
    Io(#[from] io::Error),
}

/// What the file looked like when it was last read or written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DiskState {
    modified: Option<SystemTime>,
    len: u64,
}

pub struct Document {
    id: DocId,
    path: Option<PathBuf>,
    buffer: Buffer,
    language: &'static LanguageDef,
    syntax: Option<Syntax>,
    disk: Option<DiskState>,
    /// The file changed on disk while this document had unsaved edits, and the user has not
    /// chosen yet.
    pub conflict: bool,
}

impl Document {
    /// A new empty document with no file.
    pub fn untitled(id: DocId) -> Self {
        let mut doc = Self {
            id,
            path: None,
            buffer: Buffer::new(),
            language: codigo_syntax::plain(),
            syntax: None,
            disk: None,
            conflict: false,
        };
        doc.set_language(doc.language);
        doc
    }

    /// Read a file.
    pub fn open(id: DocId, path: &Path) -> Result<Self, OpenError> {
        let meta = fs::metadata(path)?;
        if meta.len() > MAX_OPEN_BYTES {
            return Err(OpenError::TooLarge);
        }
        let bytes = fs::read(path)?;
        let decoded = decode(&bytes)?;
        let first_line = decoded.text.lines().next().unwrap_or("");
        let name = path
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let language = detect_with_first_line(&name, first_line);
        let mut buffer = Buffer::from_decoded(&decoded);
        buffer.indent = detect_indent(&decoded.text, default_indent(language));
        buffer.tab_width = language.indent.max(1);
        let mut doc = Self {
            id,
            path: Some(path.to_path_buf()),
            buffer,
            language,
            syntax: None,
            disk: Some(DiskState {
                modified: meta.modified().ok(),
                len: meta.len(),
            }),
            conflict: false,
        };
        doc.set_language(language);
        Ok(doc)
    }

    pub fn id(&self) -> DocId {
        self.id
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The file name, or `None` for a document that has no file.
    pub fn file_name(&self) -> Option<String> {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub fn language(&self) -> &'static LanguageDef {
        self.language
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.is_dirty()
    }

    /// Choose the language by hand, e.g. from the status bar.
    pub fn set_language(&mut self, language: &'static LanguageDef) {
        self.language = language;
        self.syntax = Syntax::new(language);
        self.buffer.tab_width = language.indent.max(1);
    }

    /// Run an edit and keep the syntax tree in step with it. The closure is any `Buffer`
    /// operation that reports its changes: typing, undo, a command.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut Buffer) -> (Vec<Change>, R)) -> R {
        let (changes, out) = f(&mut self.buffer);
        self.after_changes(&changes);
        out
    }

    /// Like [`edit`](Document::edit) for operations that only return their changes.
    pub fn apply(&mut self, f: impl FnOnce(&mut Buffer) -> Vec<Change>) {
        self.edit(|b| (f(b), ()));
    }

    /// Run an operation that moves or selects and never edits text: clicks, arrows, selecting.
    /// If it edits anyway, the syntax tree is rebuilt from scratch rather than left wrong.
    pub fn navigate<R>(&mut self, f: impl FnOnce(&mut Buffer) -> R) -> R {
        let revision = self.buffer.revision();
        let out = f(&mut self.buffer);
        if self.buffer.revision() != revision {
            if let Some(s) = self.syntax.as_mut() {
                s.reset();
            }
        }
        out
    }

    fn after_changes(&mut self, changes: &[Change]) {
        if changes.is_empty() {
            return;
        }
        if let Some(s) = self.syntax.as_mut() {
            s.edit(changes);
        }
    }

    /// Bring the syntax tree up to date with the text. Cheap when nothing changed; the editor
    /// calls it before drawing.
    pub fn ensure_parsed(&mut self) {
        if let Some(s) = self.syntax.as_mut() {
            if s.is_stale() {
                s.reparse(self.buffer.rope());
            }
        }
    }

    pub fn syntax_mut(&mut self) -> Option<&mut Syntax> {
        self.syntax.as_mut()
    }

    // --- Disk -----------------------------------------------------------------------------

    /// Write to the document's own file.
    pub fn save(&mut self) -> Result<(), SaveError> {
        let path = self.path.clone().ok_or(SaveError::NoPath)?;
        self.write_to(&path)
    }

    /// Write to a new file and make it the document's file. The language follows the new name.
    pub fn save_as(&mut self, path: &Path) -> Result<(), SaveError> {
        self.write_to(path)?;
        self.path = Some(path.to_path_buf());
        let name = path
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let first = self.buffer.line_text(0);
        let language = detect_with_first_line(&name, &first);
        if language.id != self.language.id {
            self.set_language(language);
        }
        Ok(())
    }

    fn write_to(&mut self, path: &Path) -> Result<(), SaveError> {
        let bytes = encode(
            &self.buffer.text(),
            self.buffer.line_ending(),
            self.buffer.bom(),
        );
        write_atomic(path, &bytes)?;
        let meta = fs::metadata(path)?;
        self.disk = Some(DiskState {
            modified: meta.modified().ok(),
            len: meta.len(),
        });
        self.buffer.mark_saved();
        self.conflict = false;
        Ok(())
    }

    /// True when the file on disk is not the one this document last read or wrote: another
    /// program saved it, or it was deleted.
    pub fn changed_on_disk(&self) -> bool {
        let (Some(path), Some(known)) = (self.path.as_ref(), self.disk) else {
            return false;
        };
        match fs::metadata(path) {
            Ok(m) => m.modified().ok() != known.modified || m.len() != known.len,
            Err(_) => true,
        }
    }

    /// Replace the text with what is on disk, as one undoable edit when the file still exists,
    /// keeping the cursors where they were.
    pub fn reload(&mut self) -> Result<(), OpenError> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| OpenError::Io(io::Error::new(io::ErrorKind::NotFound, "no file")))?;
        let meta = fs::metadata(&path)?;
        if meta.len() > MAX_OPEN_BYTES {
            return Err(OpenError::TooLarge);
        }
        let decoded = decode(&fs::read(&path)?)?;
        self.buffer.set_line_ending(decoded.line_ending);
        let changes = self.buffer.replace_all_text(&decoded.text);
        self.after_changes(&changes);
        self.buffer.mark_saved();
        self.disk = Some(DiskState {
            modified: meta.modified().ok(),
            len: meta.len(),
        });
        self.conflict = false;
        Ok(())
    }

    /// Remember the file as it is now, so an edit made elsewhere is no longer reported as news
    /// (the user chose to keep their own version).
    pub fn accept_disk_state(&mut self) {
        if let Some(meta) = self.path.as_ref().and_then(|p| fs::metadata(p).ok()) {
            self.disk = Some(DiskState {
                modified: meta.modified().ok(),
                len: meta.len(),
            });
        }
        self.conflict = false;
    }

    /// Strip trailing whitespace from every line and make the text end in one line break, the
    /// usual tidy-on-save. One undoable edit.
    pub fn tidy(&mut self, trim_trailing: bool, final_newline: bool) {
        let text = self.buffer.text();
        let mut out = String::with_capacity(text.len() + 1);
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(if trim_trailing {
                line.trim_end_matches([' ', '\t'])
            } else {
                line
            });
        }
        if final_newline && !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        let changes = self.buffer.replace_all_text(&out);
        self.after_changes(&changes);
    }

    pub fn line_ending(&self) -> LineEnding {
        self.buffer.line_ending()
    }
}

/// Write through a temporary file and rename it over the target, so a crash or a full disk
/// leaves the old file whole. Permissions are kept, and a link is written through, not replaced.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = target
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = target
        .file_name()
        .map_or(String::from("file"), |n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(".{name}.codigo-tmp"));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Ok(meta) = fs::metadata(&target) {
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        fs::rename(&tmp, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn default_indent(language: &LanguageDef) -> Indent {
    if language.tabs {
        Indent::Tabs
    } else {
        Indent::Spaces(language.indent.clamp(1, 8))
    }
}

/// The indentation a text already uses: tabs when most indented lines start with one, otherwise
/// the commonest step between a line's indent and the previous indented line's. `default` when
/// the text has too little indentation to tell.
pub fn detect_indent(text: &str, default: Indent) -> Indent {
    let (mut tabs, mut spaces) = (0usize, 0usize);
    let mut steps = [0usize; 9];
    let mut previous = 0usize;
    for line in text.lines().take(2000) {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('\t') {
            tabs += 1;
            continue;
        }
        let width = line.len() - line.trim_start_matches(' ').len();
        if width > 0 {
            spaces += 1;
            let step = width.abs_diff(previous);
            if let Some(count) = steps.get_mut(step) {
                *count += 1;
            }
        }
        previous = width;
    }
    if tabs + spaces < 3 {
        return default;
    }
    if tabs > spaces {
        return Indent::Tabs;
    }
    let best = steps
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(step, count)| **count > 0 && matches!(step, 2 | 3 | 4 | 8))
        .max_by_key(|(step, count)| (**count, std::cmp::Reverse(*step)));
    match best {
        Some((step, _)) => Indent::Spaces(step),
        None => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, bytes).expect("write");
        p
    }

    #[test]
    fn opens_detects_the_language_and_round_trips_crlf_and_bom() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = write(
            d.path(),
            "a.rs",
            b"\xEF\xBB\xBFfn main() {\r\n    1;\r\n}\r\n",
        );
        let mut doc = Document::open(DocId(1), &p).expect("open");
        assert_eq!(doc.language().id, "rust");
        assert_eq!(doc.line_ending(), LineEnding::CrLf);
        assert!(doc.buffer().bom());
        assert_eq!(doc.buffer().text(), "fn main() {\n    1;\n}\n");
        assert!(!doc.is_dirty());
        doc.apply(|b| b.insert_text("// hi\n"));
        assert!(doc.is_dirty());
        doc.save().expect("save");
        assert!(!doc.is_dirty());
        assert_eq!(
            fs::read(&p).expect("read"),
            b"\xEF\xBB\xBF// hi\r\nfn main() {\r\n    1;\r\n}\r\n"
        );
    }

    #[test]
    fn refuses_binary_non_utf8_missing_and_huge_files() {
        let d = tempfile::tempdir().expect("tempdir");
        let bin = write(d.path(), "b.dat", b"ab\0cd");
        assert!(matches!(
            Document::open(DocId(1), &bin),
            Err(OpenError::Binary)
        ));
        let latin = write(d.path(), "l.txt", b"caf\xe9");
        assert!(matches!(
            Document::open(DocId(1), &latin),
            Err(OpenError::NotUtf8)
        ));
        assert!(matches!(
            Document::open(DocId(1), &d.path().join("missing")),
            Err(OpenError::Io(_))
        ));
        assert!(matches!(
            Document::open(DocId(1), d.path()),
            Err(OpenError::Io(_)) | Err(OpenError::NotUtf8) | Err(OpenError::Binary)
        ));
    }

    #[test]
    fn untitled_saves_as_and_takes_the_new_language() {
        let d = tempfile::tempdir().expect("tempdir");
        let mut doc = Document::untitled(DocId(2));
        assert!(matches!(doc.save(), Err(SaveError::NoPath)));
        assert_eq!(doc.language().id, "text");
        doc.apply(|b| b.insert_text("def f():\n    pass\n"));
        doc.save_as(&d.path().join("x.py")).expect("save as");
        assert_eq!(doc.language().id, "python");
        assert_eq!(doc.file_name().as_deref(), Some("x.py"));
        assert!(!doc.is_dirty());
        assert!(doc.syntax_mut().is_some());
        doc.ensure_parsed();
        assert!(!doc.syntax_mut().is_some_and(|s| s.is_stale()));
    }

    #[test]
    fn syntax_follows_edits_and_undo() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = write(d.path(), "a.rs", b"fn a() {}\n");
        let mut doc = Document::open(DocId(1), &p).expect("open");
        doc.ensure_parsed();
        doc.apply(|b| {
            b.set_cursor(0);
            b.insert_text("// c\n")
        });
        assert!(doc.syntax_mut().is_some_and(|s| s.is_stale()));
        doc.ensure_parsed();
        doc.apply(|b| b.undo());
        doc.ensure_parsed();
        assert_eq!(doc.buffer().text(), "fn a() {}\n");
        let n = doc.buffer().line_count();
        let rope = doc.buffer().rope().clone();
        let spans = doc
            .syntax_mut()
            .map(|s| s.spans(&rope, 0..n))
            .unwrap_or_default();
        assert!(spans.iter().any(|l| !l.is_empty()));
    }

    #[test]
    fn notices_changes_made_elsewhere_and_reloads_them() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = write(d.path(), "a.txt", b"one\n");
        let mut doc = Document::open(DocId(1), &p).expect("open");
        assert!(!doc.changed_on_disk());
        fs::write(&p, b"two two\n").expect("external");
        assert!(doc.changed_on_disk());
        doc.reload().expect("reload");
        assert_eq!(doc.buffer().text(), "two two\n");
        assert!(!doc.is_dirty() && !doc.changed_on_disk());
        doc.apply(|b| b.undo());
        assert_eq!(doc.buffer().text(), "one\n");
        fs::remove_file(&p).expect("remove");
        assert!(doc.changed_on_disk());
    }

    #[test]
    fn saving_keeps_permissions_and_leaves_no_temp_file() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = write(d.path(), "run.sh", b"#!/bin/sh\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let mut doc = Document::open(DocId(1), &p).expect("open");
        assert_eq!(doc.language().id, "bash");
        doc.apply(|b| b.insert_text("echo hi\n"));
        doc.save().expect("save");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&p).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        }
        let names: Vec<_> = fs::read_dir(d.path())
            .expect("dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["run.sh"]);
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_a_link_keeps_the_link() {
        let d = tempfile::tempdir().expect("tempdir");
        let real = write(d.path(), "real.txt", b"a\n");
        let link = d.path().join("link.txt");
        std::os::unix::fs::symlink(&real, &link).expect("link");
        let mut doc = Document::open(DocId(1), &link).expect("open");
        doc.apply(|b| b.insert_text("b"));
        doc.save().expect("save");
        assert!(fs::symlink_metadata(&link)
            .expect("meta")
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&real).expect("read"), "ba\n");
    }

    #[test]
    fn tidy_trims_and_adds_one_final_newline() {
        let mut doc = Document::untitled(DocId(1));
        doc.apply(|b| b.insert_text("a  \n\tb\t\nc"));
        doc.tidy(true, true);
        assert_eq!(doc.buffer().text(), "a\n\tb\nc\n");
        doc.tidy(true, true);
        assert_eq!(doc.buffer().text(), "a\n\tb\nc\n");
        doc.apply(|b| b.undo());
        assert_eq!(doc.buffer().text(), "a  \n\tb\t\nc");
    }

    #[test]
    fn indentation_is_read_from_the_file() {
        let four = "fn a() {\n    if x {\n        y;\n    }\n}\n";
        assert_eq!(detect_indent(four, Indent::Tabs), Indent::Spaces(4));
        let two = "a:\n  b:\n    c: 1\n  d: 2\n";
        assert_eq!(detect_indent(two, Indent::Spaces(4)), Indent::Spaces(2));
        let tabs = "a {\n\tb;\n\tc {\n\t\td;\n\t}\n}\n";
        assert_eq!(detect_indent(tabs, Indent::Spaces(4)), Indent::Tabs);
        assert_eq!(
            detect_indent("x\ny\n", Indent::Spaces(3)),
            Indent::Spaces(3)
        );
        assert_eq!(detect_indent("", Indent::Tabs), Indent::Tabs);
    }
}
