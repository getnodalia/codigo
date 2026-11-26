// SPDX-License-Identifier: MIT OR Apache-2.0
//! The languages Código knows: how to recognise a file, how its comments look, what a language
//! server calls it and, for the ones with a grammar, how to parse and highlight it.

use std::sync::OnceLock;

use tree_sitter::{Language, Query};

use crate::scope::Scope;

/// A compiled grammar: the parser's language, its highlight query and the scope each capture
/// of that query stands for.
pub struct Grammar {
    pub(crate) language: Language,
    pub(crate) query: Query,
    pub(crate) scopes: Vec<Option<Scope>>,
}

struct GrammarSpec {
    language: fn() -> Language,
    /// Highlight queries, concatenated. A dialect (TypeScript, C++) lists its base language
    /// first because its own query only covers what it adds.
    queries: &'static [&'static str],
}

impl GrammarSpec {
    fn compile(&self) -> Option<Grammar> {
        let language = (self.language)();
        let source = self.queries.join("\n");
        let query = Query::new(&language, &source).ok()?;
        let scopes = query
            .capture_names()
            .iter()
            .map(|name| Scope::from_capture(name))
            .collect();
        Some(Grammar {
            language,
            query,
            scopes,
        })
    }
}

/// One language. Everything but the grammar is plain data.
pub struct LanguageDef {
    /// Stable id used in settings and commands: `rust`, `typescript`...
    pub id: &'static str,
    /// The name shown in the status bar.
    pub name: &'static str,
    /// File extensions without the dot, lower case.
    pub extensions: &'static [&'static str],
    /// Whole file names (`Makefile`, `Cargo.lock`).
    pub filenames: &'static [&'static str],
    /// Interpreter names a `#!` line may carry.
    pub interpreters: &'static [&'static str],
    pub line_comment: Option<&'static str>,
    pub block_comment: Option<(&'static str, &'static str)>,
    /// The `languageId` language servers expect in `textDocument/didOpen`.
    pub lsp_id: &'static str,
    /// Spaces per indent level when a file gives no other hint.
    pub indent: usize,
    /// Indent with tabs by default (Go, Makefile).
    pub tabs: bool,
    spec: Option<GrammarSpec>,
    grammar: OnceLock<Option<Grammar>>,
}

impl LanguageDef {
    /// The compiled grammar, built on first use. `None` for a language without one.
    pub fn grammar(&self) -> Option<&Grammar> {
        let spec = self.spec.as_ref()?;
        self.grammar.get_or_init(|| spec.compile()).as_ref()
    }

    pub fn has_grammar(&self) -> bool {
        self.spec.is_some()
    }
}

impl std::fmt::Debug for LanguageDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanguageDef").field("id", &self.id).finish()
    }
}

macro_rules! lang {
    (
        $id:literal, $name:literal,
        ext: [$($ext:literal),* $(,)?],
        $(files: [$($file:literal),* $(,)?],)?
        $(shebang: [$($shebang:literal),* $(,)?],)?
        comment: $line:expr, $block:expr,
        lsp: $lsp:literal,
        indent: $indent:literal $(, tabs: $tabs:literal)?
        $(, grammar: ($lang_fn:expr, [$($query:expr),+ $(,)?]))?
        $(,)?
    ) => {
        LanguageDef {
            id: $id,
            name: $name,
            extensions: &[$($ext),*],
            filenames: &[$($($file),*)?],
            interpreters: &[$($($shebang),*)?],
            line_comment: $line,
            block_comment: $block,
            lsp_id: $lsp,
            indent: $indent,
            tabs: false $(|| $tabs)?,
            spec: lang!(@spec $(($lang_fn, [$($query),+]))?),
            grammar: OnceLock::new(),
        }
    };
    (@spec) => { None };
    (@spec ($lang_fn:expr, [$($query:expr),+])) => {
        Some(GrammarSpec { language: $lang_fn, queries: &[$($query),+] })
    };
}

const C_STYLE: (&str, &str) = ("/*", "*/");

static LANGUAGES: [LanguageDef; 38] = [
    lang!("rust", "Rust", ext: ["rs"], comment: Some("//"), Some(C_STYLE), lsp: "rust", indent: 4,
        grammar: (|| tree_sitter_rust::LANGUAGE.into(), [tree_sitter_rust::HIGHLIGHTS_QUERY])),
    lang!("python", "Python", ext: ["py", "pyi", "pyw"], files: ["SConstruct", "SConscript"],
        shebang: ["python", "python3", "python2"],
        comment: Some("#"), None, lsp: "python", indent: 4,
        grammar: (|| tree_sitter_python::LANGUAGE.into(), [tree_sitter_python::HIGHLIGHTS_QUERY])),
    lang!("javascript", "JavaScript", ext: ["js", "mjs", "cjs", "jsx"], shebang: ["node", "deno", "bun"],
        comment: Some("//"), Some(C_STYLE), lsp: "javascript", indent: 2,
        grammar: (|| tree_sitter_javascript::LANGUAGE.into(), [tree_sitter_javascript::HIGHLIGHT_QUERY])),
    lang!("typescript", "TypeScript", ext: ["ts", "mts", "cts"],
        comment: Some("//"), Some(C_STYLE), lsp: "typescript", indent: 2,
        grammar: (|| tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            [tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_typescript::HIGHLIGHTS_QUERY])),
    lang!("tsx", "TypeScript JSX", ext: ["tsx"],
        comment: Some("//"), Some(C_STYLE), lsp: "typescriptreact", indent: 2,
        grammar: (|| tree_sitter_typescript::LANGUAGE_TSX.into(),
            [tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_typescript::HIGHLIGHTS_QUERY])),
    lang!("json", "JSON", ext: ["json", "jsonc", "json5", "geojson", "webmanifest"],
        files: [".babelrc", ".eslintrc", "composer.lock"],
        comment: None, None, lsp: "json", indent: 2,
        grammar: (|| tree_sitter_json::LANGUAGE.into(), [tree_sitter_json::HIGHLIGHTS_QUERY])),
    lang!("toml", "TOML", ext: ["toml"], files: ["Cargo.lock", "Pipfile", "poetry.lock"],
        comment: Some("#"), None, lsp: "toml", indent: 2,
        grammar: (|| tree_sitter_toml_ng::LANGUAGE.into(), [tree_sitter_toml_ng::HIGHLIGHTS_QUERY])),
    lang!("yaml", "YAML", ext: ["yml", "yaml"], comment: Some("#"), None, lsp: "yaml", indent: 2,
        grammar: (|| tree_sitter_yaml::LANGUAGE.into(), [tree_sitter_yaml::HIGHLIGHTS_QUERY])),
    lang!("markdown", "Markdown", ext: ["md", "markdown", "mdx"], comment: None, None,
        lsp: "markdown", indent: 2,
        grammar: (|| tree_sitter_md::LANGUAGE.into(), [tree_sitter_md::HIGHLIGHT_QUERY_BLOCK])),
    lang!("bash", "Shell", ext: ["sh", "bash", "zsh", "ksh"],
        files: [".bashrc", ".zshrc", ".profile", ".bash_profile", "PKGBUILD"],
        shebang: ["sh", "bash", "zsh", "ksh", "dash"],
        comment: Some("#"), None, lsp: "shellscript", indent: 2,
        grammar: (|| tree_sitter_bash::LANGUAGE.into(), [tree_sitter_bash::HIGHLIGHT_QUERY])),
    lang!("c", "C", ext: ["c", "h"], comment: Some("//"), Some(C_STYLE), lsp: "c", indent: 4,
        grammar: (|| tree_sitter_c::LANGUAGE.into(), [tree_sitter_c::HIGHLIGHT_QUERY])),
    lang!("cpp", "C++", ext: ["cc", "cpp", "cxx", "hh", "hpp", "hxx", "ipp", "inl"],
        comment: Some("//"), Some(C_STYLE), lsp: "cpp", indent: 4,
        grammar: (|| tree_sitter_cpp::LANGUAGE.into(),
            [tree_sitter_c::HIGHLIGHT_QUERY, tree_sitter_cpp::HIGHLIGHT_QUERY])),
    lang!("go", "Go", ext: ["go"], files: ["go.mod", "go.sum"],
        comment: Some("//"), Some(C_STYLE), lsp: "go", indent: 4, tabs: true,
        grammar: (|| tree_sitter_go::LANGUAGE.into(), [tree_sitter_go::HIGHLIGHTS_QUERY])),
    lang!("java", "Java", ext: ["java"], comment: Some("//"), Some(C_STYLE), lsp: "java", indent: 4,
        grammar: (|| tree_sitter_java::LANGUAGE.into(), [tree_sitter_java::HIGHLIGHTS_QUERY])),
    lang!("html", "HTML", ext: ["html", "htm", "xhtml", "vue", "svelte"],
        comment: None, Some(("<!--", "-->")), lsp: "html", indent: 2,
        grammar: (|| tree_sitter_html::LANGUAGE.into(), [tree_sitter_html::HIGHLIGHTS_QUERY])),
    lang!("css", "CSS", ext: ["css"], comment: None, Some(C_STYLE), lsp: "css", indent: 2,
        grammar: (|| tree_sitter_css::LANGUAGE.into(), [tree_sitter_css::HIGHLIGHTS_QUERY])),
    // Languages that have no grammar yet still get comments, indentation and a language server.
    lang!("scss", "SCSS", ext: ["scss", "sass"], comment: Some("//"), Some(C_STYLE), lsp: "scss", indent: 2),
    lang!("xml", "XML", ext: ["xml", "svg", "xsd", "xsl", "plist", "csproj"],
        comment: None, Some(("<!--", "-->")), lsp: "xml", indent: 2),
    lang!("sql", "SQL", ext: ["sql"], comment: Some("--"), Some(C_STYLE), lsp: "sql", indent: 2),
    lang!("csharp", "C#", ext: ["cs"], comment: Some("//"), Some(C_STYLE), lsp: "csharp", indent: 4),
    lang!("kotlin", "Kotlin", ext: ["kt", "kts"], comment: Some("//"), Some(C_STYLE), lsp: "kotlin", indent: 4),
    lang!("swift", "Swift", ext: ["swift"], comment: Some("//"), Some(C_STYLE), lsp: "swift", indent: 4),
    lang!("dart", "Dart", ext: ["dart"], comment: Some("//"), Some(C_STYLE), lsp: "dart", indent: 2),
    lang!("scala", "Scala", ext: ["scala", "sc", "sbt"], comment: Some("//"), Some(C_STYLE), lsp: "scala", indent: 2),
    lang!("zig", "Zig", ext: ["zig"], comment: Some("//"), None, lsp: "zig", indent: 4),
    lang!("php", "PHP", ext: ["php", "phtml"], shebang: ["php"], comment: Some("//"), Some(C_STYLE),
        lsp: "php", indent: 4),
    lang!("ruby", "Ruby", ext: ["rb", "rake", "gemspec"], files: ["Gemfile", "Rakefile", "Podfile"],
        shebang: ["ruby"], comment: Some("#"), None, lsp: "ruby", indent: 2),
    lang!("lua", "Lua", ext: ["lua"], shebang: ["lua", "luajit"], comment: Some("--"), Some(("--[[", "]]")),
        lsp: "lua", indent: 2),
    lang!("elixir", "Elixir", ext: ["ex", "exs"], comment: Some("#"), None, lsp: "elixir", indent: 2),
    lang!("haskell", "Haskell", ext: ["hs", "lhs"], comment: Some("--"), Some(("{-", "-}")),
        lsp: "haskell", indent: 2),
    lang!("ocaml", "OCaml", ext: ["ml", "mli"], comment: None, Some(("(*", "*)")), lsp: "ocaml", indent: 2),
    lang!("perl", "Perl", ext: ["pl", "pm"], shebang: ["perl"], comment: Some("#"), None, lsp: "perl", indent: 4),
    lang!("r", "R", ext: ["r"], comment: Some("#"), None, lsp: "r", indent: 2),
    lang!("dockerfile", "Dockerfile", ext: ["dockerfile"], files: ["Dockerfile", "Containerfile"],
        comment: Some("#"), None, lsp: "dockerfile", indent: 2),
    lang!("makefile", "Makefile", ext: ["mk", "mak"], files: ["Makefile", "makefile", "GNUmakefile"],
        comment: Some("#"), None, lsp: "makefile", indent: 4, tabs: true),
    lang!("cmake", "CMake", ext: ["cmake"], files: ["CMakeLists.txt"], comment: Some("#"), None,
        lsp: "cmake", indent: 2),
    lang!("ini", "INI", ext: ["ini", "cfg", "conf", "properties", "desktop", "service", "editorconfig"],
        files: [".gitconfig", ".editorconfig"], comment: Some(";"), None, lsp: "ini", indent: 2),
    lang!("text", "Texto plano", ext: ["txt", "log", "text"], comment: None, None, lsp: "plaintext", indent: 4),
];

/// Every language, plain text last.
pub fn languages() -> &'static [LanguageDef] {
    &LANGUAGES
}

/// The language with this id.
pub fn find(id: &str) -> Option<&'static LanguageDef> {
    LANGUAGES.iter().find(|l| l.id == id)
}

/// Plain text, the language of anything unrecognised.
pub fn plain() -> &'static LanguageDef {
    &LANGUAGES[LANGUAGES.len() - 1]
}

/// The language of a file, from its name alone. Unknown files are plain text.
pub fn detect(file_name: &str) -> &'static LanguageDef {
    detect_with_first_line(file_name, "")
}

/// Like [`detect`], and falls back on a `#!` line when the name says nothing.
pub fn detect_with_first_line(file_name: &str, first_line: &str) -> &'static LanguageDef {
    let name = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    if let Some(l) = LANGUAGES.iter().find(|l| l.filenames.contains(&name)) {
        return l;
    }
    if let Some((_, ext)) = name.rsplit_once('.') {
        let ext = ext.to_ascii_lowercase();
        if let Some(l) = LANGUAGES
            .iter()
            .find(|l| l.extensions.contains(&ext.as_str()))
        {
            return l;
        }
    } else {
        let lower = name.to_ascii_lowercase();
        if let Some(l) = LANGUAGES
            .iter()
            .find(|l| l.extensions.contains(&lower.as_str()))
        {
            return l;
        }
    }
    shebang(first_line).unwrap_or_else(plain)
}

fn shebang(line: &str) -> Option<&'static LanguageDef> {
    let rest = line.strip_prefix("#!")?.trim();
    let mut words = rest.split_whitespace();
    let mut program = words.next()?.rsplit('/').next()?;
    if program == "env" {
        // `#!/usr/bin/env -S python3 -u`: skip flags, take the first program.
        program = words.find(|w| !w.starts_with('-'))?;
    }
    let program = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    LANGUAGES.iter().find(|l| {
        l.interpreters
            .iter()
            .any(|i| i.trim_end_matches(|c: char| c.is_ascii_digit()) == program)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_plain_text_is_last() {
        for (i, a) in LANGUAGES.iter().enumerate() {
            for b in &LANGUAGES[i + 1..] {
                assert_ne!(a.id, b.id);
            }
        }
        assert_eq!(plain().id, "text");
    }

    #[test]
    fn every_grammar_and_its_highlight_query_compile() {
        for l in languages() {
            if l.has_grammar() {
                assert!(l.grammar().is_some(), "{} does not compile", l.id);
            }
        }
    }

    #[test]
    fn every_grammar_names_some_captures() {
        for l in languages() {
            if let Some(g) = l.grammar() {
                assert!(
                    g.scopes.iter().any(Option::is_some),
                    "{} highlights nothing",
                    l.id
                );
            }
        }
    }

    #[test]
    fn detects_by_extension_name_and_shebang() {
        assert_eq!(detect("src/main.rs").id, "rust");
        assert_eq!(detect("APP.PY").id, "python");
        assert_eq!(detect("a/b/Makefile").id, "makefile");
        assert_eq!(detect("Cargo.lock").id, "toml");
        assert_eq!(detect("Dockerfile").id, "dockerfile");
        assert_eq!(detect("x.tsx").id, "tsx");
        assert_eq!(detect("unknown.zzz").id, "text");
        assert_eq!(detect("script").id, "text");
        assert_eq!(
            detect_with_first_line("script", "#!/usr/bin/env python3").id,
            "python"
        );
        assert_eq!(
            detect_with_first_line("run", "#!/usr/bin/env -S node --x").id,
            "javascript"
        );
        assert_eq!(detect_with_first_line("run", "#!/bin/bash").id, "bash");
        assert_eq!(detect_with_first_line("run", "no shebang").id, "text");
    }

    #[test]
    fn find_by_id() {
        assert_eq!(find("go").map(|l| l.name), Some("Go"));
        assert!(find("nope").is_none());
    }
}
