// SPDX-License-Identifier: MIT OR Apache-2.0
//! The fonts installed on this computer, and the one the code is drawn in.
//!
//! The scan reads each font's own name table, so a family shows under the name its designer gave
//! it, and runs off the window thread: a computer can have thousands of font files.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{FontData, FontFamily};
use read_fonts::tables::name::NameId;
use read_fonts::{FileRef, FontRef, TableProvider};

/// One family that can draw code, from one regular-weight file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Family {
    pub name: String,
    pub file: PathBuf,
    /// Which font of a collection file.
    pub index: u32,
    /// Whether every letter has the same width, which is what code wants.
    pub mono: bool,
}

/// Files larger than this are skipped: they are the big CJK faces, which are not for code.
const MAX_FILE: u64 = 40 * 1024 * 1024;
const MAX_FILES: usize = 6000;
const MAX_DEPTH: usize = 6;

const STYLED: [&str; 12] = [
    "bold",
    "italic",
    "oblique",
    "light",
    "thin",
    "black",
    "heavy",
    "medium",
    "semibold",
    "extra",
    "condensed",
    "demi",
];

/// Where fonts live on each system.
pub fn directories() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
        dirs.extend(home.iter().map(|h| h.join("Library/Fonts")));
    } else if cfg!(windows) {
        if let Some(root) = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot")) {
            dirs.push(PathBuf::from(root).join("Fonts"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
        }
    } else {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        dirs.extend(home.iter().map(|h| h.join(".fonts")));
        dirs.extend(home.iter().map(|h| h.join(".local/share/fonts")));
        if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
            dirs.push(PathBuf::from(data).join("fonts"));
        }
    }
    dirs
}

fn is_font_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "ttf" | "otf" | "ttc" | "otc"
        )
    })
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            walk(&path, depth + 1, out);
        } else if is_font_file(&path) && out.len() < MAX_FILES {
            out.push(path);
        }
    }
}

fn name_of(font: &FontRef, id: NameId) -> Option<String> {
    let name = font.name().ok()?;
    let data = name.string_data();
    let mut fallback = None;
    for record in name.name_record() {
        if record.name_id() != id {
            continue;
        }
        let Ok(text) = record.string(data) else {
            continue;
        };
        let text: String = text.chars().collect();
        if text.trim().is_empty() {
            continue;
        }
        if record.is_unicode() {
            return Some(text);
        }
        fallback.get_or_insert(text);
    }
    fallback
}

/// The family of one font, if it is a regular-weight one worth listing.
fn family_of(font: &FontRef, file: &Path, index: u32) -> Option<Family> {
    let name = name_of(font, NameId::TYPOGRAPHIC_FAMILY_NAME)
        .or_else(|| name_of(font, NameId::FAMILY_NAME))?;
    let style = name_of(font, NameId::TYPOGRAPHIC_SUBFAMILY_NAME)
        .or_else(|| name_of(font, NameId::SUBFAMILY_NAME))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(style.as_str(), "regular" | "book" | "roman" | "normal" | "") {
        return None;
    }
    // Emoji and symbol fonts set the fixed-pitch flag too, but they are not for code.
    let lower = name.to_lowercase();
    let mono = font.post().is_ok_and(|p| p.is_fixed_pitch() != 0)
        && !lower.contains("emoji")
        && !lower.contains("symbols");
    Some(Family {
        name: name.trim().to_owned(),
        file: file.to_path_buf(),
        index,
        mono,
    })
}

/// Read the families a font file holds.
pub fn families_in(path: &Path) -> Vec<Family> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Vec::new();
    };
    if meta.len() > MAX_FILE {
        return Vec::new();
    }
    let Ok(data) = std::fs::read(path) else {
        return Vec::new();
    };
    match FileRef::new(&data) {
        Ok(FileRef::Font(font)) => family_of(&font, path, 0).into_iter().collect(),
        Ok(FileRef::Collection(c)) => (0..c.len().min(32))
            .filter_map(|i| c.get(i).ok().and_then(|f| family_of(&f, path, i)))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Every family on this computer, monospaced ones first, each name once.
pub fn scan() -> Vec<Family> {
    let mut files = Vec::new();
    for dir in directories() {
        walk(&dir, 0, &mut files);
    }
    files.sort();
    let mut out: Vec<Family> = Vec::new();
    for path in files {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if STYLED.iter().any(|s| stem.contains(s)) {
            continue;
        }
        for family in families_in(&path) {
            if !out.iter().any(|f| f.name == family.name) {
                out.push(family);
            }
        }
    }
    out.sort_by(|a, b| {
        b.mono
            .cmp(&a.mono)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// The egui font name the chosen code font is registered under.
const CODE_FONT: &str = "codigo-code-font";

/// Install the Nodalia fonts with `family` in front of the monospaced ones. An empty or unknown
/// family leaves the built-in code font. Returns whether the family was found and loaded.
pub fn install(ctx: &egui::Context, family: &str, known: &[Family]) -> bool {
    let mut defs = nodalia_theme::engineering::font_definitions();
    let mut loaded = false;
    if !family.is_empty() {
        if let Some(found) = known.iter().find(|f| f.name == family) {
            if let Ok(bytes) = std::fs::read(&found.file) {
                let mut data = FontData::from_owned(bytes);
                data.index = found.index;
                defs.font_data.insert(CODE_FONT.to_owned(), Arc::new(data));
                defs.families
                    .entry(FontFamily::Monospace)
                    .or_default()
                    .insert(0, CODE_FONT.to_owned());
                loaded = true;
            }
        }
    }
    ctx.set_fonts(defs);
    nodalia_theme::engineering::apply_text_styles(ctx);
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_font_files_are_listed() {
        for (name, yes) in [
            ("a.ttf", true),
            ("A.OTF", true),
            ("b.ttc", true),
            ("c.woff2", false),
            ("d", false),
        ] {
            assert_eq!(is_font_file(Path::new(name)), yes, "{name}");
        }
    }

    #[test]
    fn a_file_that_is_not_a_font_gives_nothing() {
        let d = tempfile::tempdir().expect("tempdir");
        let p = d.path().join("fake.ttf");
        std::fs::write(&p, b"this is not a font").expect("write");
        assert!(families_in(&p).is_empty());
        assert!(families_in(&d.path().join("missing.ttf")).is_empty());
        std::fs::write(&p, []).expect("write");
        assert!(families_in(&p).is_empty());
    }

    #[test]
    fn walking_stops_at_the_depth_limit_and_finds_nested_fonts() {
        let d = tempfile::tempdir().expect("tempdir");
        let nested = d.path().join("a/b");
        std::fs::create_dir_all(&nested).expect("dirs");
        std::fs::write(nested.join("x.ttf"), b"").expect("write");
        std::fs::write(d.path().join("readme.txt"), b"").expect("write");
        let mut out = Vec::new();
        walk(d.path(), 0, &mut out);
        assert_eq!(out, vec![nested.join("x.ttf")]);
    }

    /// The scan finds what this machine has installed; only checked where fonts exist.
    #[test]
    fn the_scan_reads_real_names_and_flags_monospaced_ones() {
        let known = scan();
        if known.is_empty() {
            return;
        }
        assert!(known.iter().all(|f| !f.name.is_empty()));
        let mut names: Vec<&str> = known.iter().map(|f| f.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), known.len(), "a family is listed twice");
        // Monospaced families come first.
        let first_proportional = known.iter().position(|f| !f.mono).unwrap_or(known.len());
        assert!(known[first_proportional..].iter().all(|f| !f.mono));
    }
}
