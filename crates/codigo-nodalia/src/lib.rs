// SPDX-License-Identifier: MIT OR Apache-2.0
//! Código's tools for building Nodalia apps. For now: a project template, an egui app in the
//! Nodalia Design Language with the shared theme, the About window, Spanish and English, the
//! lints and the CI the other apps use, so a new app starts coherent with the rest.
//!
//! Nothing here draws anything. Names come from a person and are hostile text: they are checked
//! before a single file is written, and an existing folder is never touched.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use std::fmt;
use std::path::{Path, PathBuf};

pub use nodalia_theme::Accent;

/// The `nodalia-design` commit the template depends on: the one Código itself is built with.
pub const THEME_REV: &str = "8bb4b7dcbf186644ab606b806569c851056cee2c";
/// The `getnodalia/foundation` commit the template's CI calls.
pub const FOUNDATION_REV: &str = "e756876881280ba136ec527be594f616c9a65a5a";

/// Longest name accepted, in characters.
pub const MAX_NAME: usize = 40;

/// What to create.
#[derive(Clone, Debug)]
pub struct AppSpec {
    /// The product name, as shown in the interface: `Mi App`.
    pub name: String,
    pub accent: Accent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The name is empty, too long or has no letters to build a machine name from.
    BadName,
    /// The target folder already exists: nothing is overwritten.
    Exists(PathBuf),
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadName => f.write_str("the name is not valid"),
            Error::Exists(p) => write!(f, "{} already exists", p.display()),
            Error::Io(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for Error {}

/// What was created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Created {
    pub root: PathBuf,
    /// Relative to `root`.
    pub files: Vec<PathBuf>,
    /// The file to open first.
    pub entry: PathBuf,
}

/// The machine name for a product name: lower case ASCII, accents folded, words joined by `-`.
/// `None` when nothing usable is left or it would not start with a letter.
pub fn slug(name: &str) -> Option<String> {
    let mut out = String::new();
    for c in name.trim().chars().take(MAX_NAME * 2) {
        let folded = match c {
            'á' | 'à' | 'ä' | 'â' | 'Á' | 'À' | 'Ä' | 'Â' => 'a',
            'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'Ó' | 'Ò' | 'Ö' | 'Ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => 'u',
            'ñ' | 'Ñ' => 'n',
            c if c.is_ascii_alphanumeric() => c.to_ascii_lowercase(),
            _ => '-',
        };
        if folded == '-' && (out.is_empty() || out.ends_with('-')) {
            continue;
        }
        out.push(folded);
    }
    let out = out.trim_end_matches('-').to_owned();
    out.chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase())
        .then_some(out)
}

/// `mi-app` as `MiApp`, for the app id.
fn pascal(slug: &str) -> String {
    slug.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut cs = w.chars();
            cs.next()
                .map(|f| f.to_ascii_uppercase().to_string() + cs.as_str())
                .unwrap_or_default()
        })
        .collect()
}

/// Whether a product name is acceptable as typed.
pub fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && n.chars().count() <= MAX_NAME
        && n.chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
        && slug(n).is_some()
}

/// The template's files: where they go, and their text with `{{…}}` placeholders.
const FILES: &[(&str, &str)] = &[
    (
        ".github/workflows/ci.yml",
        include_str!("../templates/ndl-app/.github/workflows/ci.yml.tpl"),
    ),
    (
        ".gitignore",
        include_str!("../templates/ndl-app/.gitignore.tpl"),
    ),
    (
        "AGENTS.md",
        include_str!("../templates/ndl-app/AGENTS.md.tpl"),
    ),
    (
        "ATTRIBUTION.md",
        include_str!("../templates/ndl-app/ATTRIBUTION.md.tpl"),
    ),
    (
        "CLAUDE.md",
        include_str!("../templates/ndl-app/CLAUDE.md.tpl"),
    ),
    (
        "Cargo.toml",
        include_str!("../templates/ndl-app/Cargo.toml.tpl"),
    ),
    ("LICENSE-APACHE", include_str!("../../../LICENSE-APACHE")),
    ("LICENSE-MIT", include_str!("../../../LICENSE-MIT")),
    (
        "README.md",
        include_str!("../templates/ndl-app/README.md.tpl"),
    ),
    (
        "ROADMAP.md",
        include_str!("../templates/ndl-app/ROADMAP.md.tpl"),
    ),
    (
        "clippy.toml",
        include_str!("../templates/ndl-app/clippy.toml.tpl"),
    ),
    (
        "src/app.rs",
        include_str!("../templates/ndl-app/src/app.rs.tpl"),
    ),
    (
        "src/i18n.rs",
        include_str!("../templates/ndl-app/src/i18n.rs.tpl"),
    ),
    (
        "src/main.rs",
        include_str!("../templates/ndl-app/src/main.rs.tpl"),
    ),
    (
        "src/spanish.rs",
        include_str!("../templates/ndl-app/src/spanish.rs.tpl"),
    ),
];

/// The file the person starts in.
const ENTRY: &str = "src/app.rs";

/// Fill the placeholders. Names are checked first, so nothing hostile reaches a source file.
fn render(text: &str, values: &[(&str, &str)]) -> String {
    let mut out = text.to_owned();
    for (key, value) in values {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    out
}

/// Create the app in `parent/<slug>`. Fails, touching nothing, when the name is not valid or the
/// folder is already there; a failure half way removes what it had written.
pub fn create_app(parent: &Path, spec: &AppSpec) -> Result<Created, Error> {
    if !valid_name(&spec.name) {
        return Err(Error::BadName);
    }
    let name = spec.name.trim();
    let slug = slug(name).ok_or(Error::BadName)?;
    let root = parent.join(&slug);
    if root.exists() {
        return Err(Error::Exists(root));
    }
    let app_id = format!("dev.nodalia.{}", pascal(&slug));
    let accent = format!("{:?}", spec.accent);
    let values = [
        ("name", name),
        ("slug", slug.as_str()),
        ("app_id", app_id.as_str()),
        ("accent", accent.as_str()),
        ("rev", THEME_REV),
        ("foundation", FOUNDATION_REV),
    ];
    let mut files = Vec::new();
    let result = (|| -> std::io::Result<()> {
        for (rel, text) in FILES {
            let path = root.join(rel);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, render(text, &values))?;
            files.push(PathBuf::from(rel));
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&root);
        return Err(Error::Io(e.to_string()));
    }
    Ok(Created {
        entry: PathBuf::from(ENTRY),
        root,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str) -> AppSpec {
        AppSpec {
            name: name.into(),
            accent: Accent::Codigo,
        }
    }

    #[test]
    fn names_become_machine_names() {
        assert_eq!(slug("Mi App").as_deref(), Some("mi-app"));
        assert_eq!(slug("  Órbita Ñu ").as_deref(), Some("orbita-nu"));
        assert_eq!(slug("a__b").as_deref(), Some("a-b"));
        assert_eq!(slug("1app"), None);
        assert_eq!(slug("---"), None);
        assert_eq!(slug(""), None);
    }

    #[test]
    fn hostile_names_are_refused() {
        for bad in ["", "   ", "../x", "a/b", "a\"b", "{{name}}", "x\ny", "1abc"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        assert!(!valid_name(&"a".repeat(MAX_NAME + 1)));
        assert!(valid_name("Mi App"));
        assert!(valid_name("Brújula"));
    }

    #[test]
    fn the_template_pins_the_theme_this_workspace_uses() {
        let root = include_str!("../../../Cargo.toml");
        assert!(
            root.contains(THEME_REV),
            "bump THEME_REV with the workspace"
        );
        assert!(include_str!("../../../.github/workflows/ci.yml").contains(FOUNDATION_REV));
    }

    #[test]
    fn an_app_is_created_with_every_file_filled_in() {
        let dir = tempfile::tempdir().expect("tempdir");
        let made = create_app(dir.path(), &spec("Mi App")).expect("created");
        assert_eq!(made.root, dir.path().join("mi-app"));
        assert_eq!(made.files.len(), FILES.len());
        assert!(made.root.join(&made.entry).is_file());
        for rel in &made.files {
            let text = std::fs::read_to_string(made.root.join(rel)).expect("read");
            assert!(
                !text.contains("{{name}}") && !text.contains("{{slug}}"),
                "{} has an unfilled placeholder",
                rel.display()
            );
        }
        let cargo = std::fs::read_to_string(made.root.join("Cargo.toml")).expect("cargo");
        assert!(cargo.contains("name = \"mi-app\""));
        assert!(cargo.contains(THEME_REV));
        let main = std::fs::read_to_string(made.root.join("src/main.rs")).expect("main");
        assert!(main.contains("dev.nodalia.MiApp"));
        let app = std::fs::read_to_string(made.root.join("src/app.rs")).expect("app");
        assert!(app.contains("Accent::Codigo"));
    }

    #[test]
    fn nothing_existing_is_overwritten() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("mi-app")).expect("dir");
        std::fs::write(dir.path().join("mi-app/keep.txt"), "mine").expect("file");
        assert_eq!(
            create_app(dir.path(), &spec("Mi App")),
            Err(Error::Exists(dir.path().join("mi-app")))
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("mi-app/keep.txt")).expect("keep"),
            "mine"
        );
        assert_eq!(create_app(dir.path(), &spec("../..")), Err(Error::BadName));
    }

    #[test]
    fn the_app_id_is_pascal_case() {
        assert_eq!(pascal("mi-app"), "MiApp");
        assert_eq!(pascal("orbita"), "Orbita");
    }
}
