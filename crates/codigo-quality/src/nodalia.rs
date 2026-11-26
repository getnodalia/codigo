// SPDX-License-Identifier: MIT OR Apache-2.0
// codigo-ignore-file
//! Nodalia's engineering rules that need to look at more than one line or one file: the licence
//! header, and the interface strings that have no Spanish. The single-line ones (colour literals,
//! text that skips `tr()`) are line rules in `lines.rs`. All of them apply only to a project that
//! builds on the Nodalia Design Language.

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::rules::Finding;

/// Whether the folder holds a Nodalia app: a `Cargo.toml` at the root, or one level down in
/// `apps/` or `crates/`, that depends on the Nodalia theme, GTK or update crates.
pub(crate) fn detect(root: &Path) -> bool {
    let mut manifests = vec![root.join("Cargo.toml")];
    for group in ["apps", "crates"] {
        if let Ok(entries) = std::fs::read_dir(root.join(group)) {
            manifests.extend(
                entries
                    .flatten()
                    .take(200)
                    .map(|e| e.path().join("Cargo.toml")),
            );
        }
    }
    manifests.iter().any(|m| {
        std::fs::read_to_string(m).is_ok_and(|t| {
            t.contains("nodalia-theme") || t.contains("nodalia-gtk") || t.contains("nodalia-update")
        })
    })
}

/// A Rust source file of the project.
pub(crate) struct Source<'a> {
    pub path: &'a str,
    pub text: &'a str,
}

fn call_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?:^|[^A-Za-z0-9_])(?:tr|trf|tr_noop)\(\s*"((?:[^"\\]|\\.)*)""#).ok()
    })
    .as_ref()
}

fn pair_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"\(\s*"((?:[^"\\]|\\.)*)"\s*,\s*"(?:[^"\\]|\\.)*"\s*,?\s*\)"#).ok()
    })
    .as_ref()
}

/// The English keys of the project's `SPANISH` table, if it has one.
fn spanish_keys<'a>(files: &[Source<'a>]) -> Option<std::collections::HashSet<&'a str>> {
    let table = files
        .iter()
        .find(|f| f.text.contains("SPANISH: &[(&str, &str)]"))?;
    let start = table.text.find("SPANISH: &[(&str, &str)]")?;
    let body = table.text.get(start..)?;
    let keys: std::collections::HashSet<&str> = pair_re()?
        .captures_iter(body)
        .filter_map(|c| c.get(1).map(|m| m.as_str()))
        .collect();
    (!keys.is_empty()).then_some(keys)
}

/// Findings for the whole project: `(index into files, finding)`.
pub(crate) fn check(files: &[Source]) -> Vec<(usize, Finding)> {
    let mut out = Vec::new();
    let keys = spanish_keys(files);
    for (i, file) in files.iter().enumerate() {
        if file
            .text
            .lines()
            .take(6)
            .any(|l| l.contains("codigo-ignore-file"))
        {
            continue;
        }
        let head_has_licence = file
            .text
            .lines()
            .take(5)
            .any(|l| l.contains("SPDX-License-Identifier"));
        if !head_has_licence {
            out.extend(Finding::new("ndl-missing-spdx", 0, 0, 0).map(|f| (i, f)));
        }
        let (Some(keys), Some(re)) = (&keys, call_re()) else {
            continue;
        };
        if file.path.ends_with("i18n.rs") || file.path.ends_with("spanish.rs") {
            continue;
        }
        let mut line_starts = vec![0usize];
        line_starts.extend(file.text.match_indices('\n').map(|(at, _)| at + 1));
        for caps in re.captures_iter(file.text) {
            let Some(lit) = caps.get(1) else { continue };
            if keys.contains(lit.as_str()) {
                continue;
            }
            let at = lit.start();
            let line = line_starts.partition_point(|s| *s <= at).saturating_sub(1);
            let col = at - line_starts.get(line).copied().unwrap_or(0);
            let raw_line = file.text.lines().nth(line).unwrap_or("");
            if raw_line.contains("codigo-ignore") {
                continue;
            }
            out.extend(
                Finding::new("ndl-missing-spanish", line, col, col + lit.len())
                    .map(|f| (i, f.with_detail(lit.as_str()))),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src<'a>(path: &'a str, text: &'a str) -> Source<'a> {
        Source { path, text }
    }

    #[test]
    fn strings_missing_from_the_spanish_table_are_found() {
        let table = "// SPDX-License-Identifier: MIT OR Apache-2.0\npub const SPANISH: &[(&str, &str)] = &[\n    (\"Open\", \"Abrir\"),\n];\n";
        let app = "// SPDX-License-Identifier: MIT OR Apache-2.0\nfn f() {\n    a(tr(\"Open\"));\n    b(i18n::tr(\"Close\"));\n    c(trf(\"Saved {}\", &[x]));\n    d(str(\"x\"));\n}\n";
        let files = [src("src/spanish.rs", table), src("src/app.rs", app)];
        let found = check(&files);
        let details: Vec<_> = found
            .iter()
            .map(|(i, f)| (*i, f.rule.id, f.detail.as_deref(), f.line))
            .collect();
        assert_eq!(
            details,
            [
                (1, "ndl-missing-spanish", Some("Close"), 3),
                (1, "ndl-missing-spanish", Some("Saved {}"), 4)
            ]
        );
    }

    #[test]
    fn a_missing_licence_header_is_found_and_ignore_works() {
        let files = [
            src("src/a.rs", "fn a() {}\n"),
            src("src/b.rs", "// SPDX-License-Identifier: MIT\nfn b() {}\n"),
            src("src/c.rs", "// codigo-ignore-file\nfn c() {}\n"),
        ];
        let found = check(&files);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 0);
        assert_eq!(found[0].1.rule.id, "ndl-missing-spdx");
    }

    #[test]
    fn a_table_wrapped_by_rustfmt_is_still_read() {
        let table = "// SPDX-License-Identifier: MIT\npub const SPANISH: &[(&str, &str)] = &[\n    (\n        \"Long\",\n        \"Larga\",\n    ),\n];\n";
        let app = "// SPDX-License-Identifier: MIT\nfn f() { tr(\"Long\"); }\n";
        assert!(check(&[src("src/spanish.rs", table), src("src/app.rs", app)]).is_empty());
    }

    #[test]
    fn no_table_means_no_translation_findings() {
        let files = [src(
            "src/a.rs",
            "// SPDX-License-Identifier: MIT\nfn f() { tr(\"X\"); }\n",
        )];
        assert!(check(&files).is_empty());
    }

    #[test]
    fn a_nodalia_project_is_recognised() {
        let d = tempfile::tempdir().expect("tempdir");
        assert!(!detect(d.path()));
        std::fs::create_dir_all(d.path().join("apps/x")).expect("dir");
        std::fs::write(
            d.path().join("apps/x/Cargo.toml"),
            "[dependencies]\nnodalia-theme = {}\n",
        )
        .expect("toml");
        assert!(detect(d.path()));
    }
}
