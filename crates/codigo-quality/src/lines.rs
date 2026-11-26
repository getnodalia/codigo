// SPDX-License-Identifier: MIT OR Apache-2.0
// codigo-ignore-file
//! The line rules: patterns for secrets, unsafe calls and shortcuts, checked against the text
//! with the syntax tree deciding whether a match sits in code, a string or a comment.

use std::sync::OnceLock;

use codigo_syntax::LanguageDef;
use regex::{Captures, Regex};
use tree_sitter::Tree;

use crate::rules::Finding;

/// Where a match counts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Anywhere, comments and strings included (a secret is a secret in a comment).
    Anywhere,
    /// Not in a comment: strings count (a URL, a query).
    Live,
    /// Only real code: not in a string or a comment.
    Code,
    /// Only in a comment.
    Comment,
}

struct Pattern {
    rule: &'static str,
    /// Language ids it applies to; empty means all.
    langs: &'static [&'static str],
    re: &'static str,
    scope: Scope,
    /// A line containing any of these is fine.
    unless: &'static [&'static str],
    /// Use the first capture group as the finding's detail.
    detail: bool,
    /// A last look at the captures: false drops the match.
    check: Option<fn(&Captures) -> bool>,
}

const fn p(rule: &'static str, re: &'static str, scope: Scope) -> Pattern {
    Pattern {
        rule,
        langs: &[],
        re,
        scope,
        unless: &[],
        detail: false,
        check: None,
    }
}

const JS: &[&str] = &["javascript", "typescript", "tsx"];
const C: &[&str] = &["c", "cpp"];
const LOCAL: &[&str] = &[
    "localhost",
    "127.0.0.1",
    "0.0.0.0",
    "[::1]",
    "example.",
    "www.w3.org",
    "xmlns",
    "schemas.",
    "json-schema.org",
    "apache.org/licenses",
    ".local",
    "http://{",
];

fn patterns() -> &'static [Pattern] {
    static PATTERNS: OnceLock<Vec<Pattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            p(
                "private-key",
                r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY",
                Scope::Anywhere,
            ),
            p(
                "cloud-token",
                r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b|\bgh[pousr]_[A-Za-z0-9]{36,}\b|\bgithub_pat_[A-Za-z0-9_]{50,}|\bxox[abprs]-[A-Za-z0-9-]{10,}|\bAIza[0-9A-Za-z_\-]{35}\b|\b[sr]k_live_[0-9A-Za-z]{16,}",
                Scope::Anywhere,
            ),
            Pattern {
                check: Some(looks_like_secret),
                ..p(
                    "hardcoded-secret",
                    r#"(?i)\b[\w.-]*(?:password|passwd|pwd|secret|api[_-]?key|token|auth[_-]?key|private[_-]?key)[\w.-]*["']?\s*[:=]\s*["']([^"'\s]{8,})["']"#,
                    Scope::Anywhere,
                )
            },
            p(
                "sql-concat",
                r#"(?i)["'](?:select\s.+\sfrom|insert\s+into|update\s+\w+\s+set|delete\s+from)\b[^"']*["']\s*(?:\+|%)|(?i)\bf["'](?:select|insert|update|delete)\s[^"']*\{|(?i)format!\(\s*"\s*(?:select|insert|update|delete)\s[^"]*\{"#,
                Scope::Anywhere,
            ),
            Pattern {
                langs: &["python", "javascript", "typescript", "tsx", "php", "ruby"],
                ..p("eval", r"\b(?:eval|exec)\s*\(", Scope::Code)
            },
            Pattern {
                langs: &["python", "ruby", "php"],
                ..p(
                    "shell-command",
                    r"\bshell\s*=\s*True|\bos\.system\s*\(|\bos\.popen\s*\(",
                    Scope::Code,
                )
            },
            Pattern {
                langs: JS,
                ..p(
                    "shell-command",
                    r"\bchild_process\.exec(?:Sync)?\s*\(",
                    Scope::Code,
                )
            },
            Pattern {
                langs: C,
                ..p("shell-command", r"\bsystem\s*\(", Scope::Code)
            },
            p(
                "tls-verification-off",
                r"(?i)verify\s*=\s*False|rejectUnauthorized\s*:\s*false|InsecureSkipVerify\s*:\s*true|danger_accept_invalid_(?:certs|hostnames)\s*\(\s*true|NODE_TLS_REJECT_UNAUTHORIZED\s*=\s*['\x22]?0|CURLOPT_SSL_VERIFYPEER\s*,\s*(?:false|0)",
                Scope::Code,
            ),
            Pattern {
                langs: C,
                detail: true,
                ..p(
                    "unsafe-c-function",
                    r"\b(gets|strcpy|strcat|sprintf|vsprintf)\s*\(",
                    Scope::Code,
                )
            },
            Pattern {
                langs: JS,
                ..p(
                    "html-injection",
                    r"\.(?:innerHTML|outerHTML)\s*=[^=]|\bdocument\.write(?:ln)?\s*\(|dangerouslySetInnerHTML",
                    Scope::Code,
                )
            },
            Pattern {
                langs: &["python"],
                unless: &["SafeLoader", "safe_load"],
                ..p(
                    "unsafe-deserialisation",
                    r"\bpickle\.loads?\s*\(|\byaml\.load\s*\(|\bmarshal\.loads?\s*\(",
                    Scope::Code,
                )
            },
            p(
                "weak-hash",
                r#"(?i)hashlib\.(?:md5|sha1)\b|createHash\(\s*['"](?:md5|sha1)|MessageDigest\.getInstance\(\s*"(?:MD5|SHA-?1)"|\b(?:Md5|Sha1)::|\b(?:md5|sha1)\.New\("#,
                Scope::Anywhere,
            ),
            Pattern {
                unless: LOCAL,
                ..p("insecure-url", r"\bhttp://[A-Za-z0-9]", Scope::Live)
            },
            Pattern {
                langs: &["rust"],
                ..p("unsafe-block", r"\bunsafe\s*(?:\{|fn\b|impl\b|trait\b)", Scope::Code)
            },
            Pattern {
                langs: &["rust"],
                ..p(
                    "may-panic",
                    r"\.unwrap\(\)|\.expect\(|\bpanic!\(|\btodo!\(|\bunimplemented!\(|\bunreachable!\(",
                    Scope::Code,
                )
            },
            Pattern {
                langs: &["rust"],
                ..p("debug-output", r"\bdbg!\(", Scope::Code)
            },
            Pattern {
                langs: JS,
                ..p("debug-output", r"\bconsole\.(?:log|debug)\(", Scope::Code)
            },
            Pattern {
                langs: &["python"],
                ..p("debug-output", r"\bbreakpoint\(\)|\bpdb\.set_trace\(", Scope::Code)
            },
            p("todo-comment", r"\b(?:TODO|FIXME|HACK|XXX)\b", Scope::Comment),
        ]
    })
}

fn compiled() -> &'static [Option<Regex>] {
    static RE: OnceLock<Vec<Option<Regex>>> = OnceLock::new();
    RE.get_or_init(|| patterns().iter().map(|p| Regex::new(p.re).ok()).collect())
}

/// A quoted value after a name like `password` is a secret when it is not an obvious
/// placeholder and looks random enough to be one.
fn looks_like_secret(c: &Captures) -> bool {
    let Some(value) = c.get(1).map(|m| m.as_str()) else {
        return false;
    };
    let lower = value.to_ascii_lowercase();
    const PLACEHOLDERS: &[&str] = &[
        "example",
        "changeme",
        "xxxx",
        "your",
        "dummy",
        "placeholder",
        "password",
        "secret",
        "token",
        "sample",
        "test",
        "fake",
        "none",
        "null",
        "false",
        "true",
        "undefined",
        "env",
    ];
    if PLACEHOLDERS.iter().any(|w| lower.contains(w)) {
        return false;
    }
    if value.contains(['<', '$', '{', '%', '*', '(', ')', '/', '\\']) {
        return false;
    }
    entropy(value) >= 3.0
}

/// Shannon entropy in bits per character.
pub(crate) fn entropy(s: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    let mut n = 0usize;
    for c in s.chars() {
        *counts.entry(c).or_insert(0usize) += 1;
        n += 1;
    }
    if n == 0 {
        return 0.0;
    }
    counts
        .values()
        .map(|&k| {
            let f = k as f64 / n as f64;
            -f * f.log2()
        })
        .sum()
}

/// True when `byte` sits inside a node whose kind says comment or string.
fn inside(tree: &Tree, byte: usize, word: &str) -> bool {
    let mut node = tree
        .root_node()
        .descendant_for_byte_range(byte, byte.saturating_add(1));
    while let Some(n) = node {
        if n.kind().contains(word) {
            return true;
        }
        node = n.parent();
    }
    false
}

fn in_scope(
    scope: Scope,
    tree: Option<&Tree>,
    lang: &LanguageDef,
    line: &str,
    column: usize,
    byte: usize,
) -> bool {
    match (scope, tree) {
        (Scope::Anywhere, _) => true,
        (Scope::Live, Some(t)) => !inside(t, byte, "comment"),
        (Scope::Code, Some(t)) => !inside(t, byte, "comment") && !inside(t, byte, "string"),
        (Scope::Comment, Some(t)) => inside(t, byte, "comment"),
        (scope, None) => {
            // No grammar: a line that starts with the comment marker is a comment.
            let commented = lang.line_comment.is_some_and(|m| {
                line.trim_start().starts_with(m)
                    || line
                        .find(m)
                        .is_some_and(|at| at < column && scope != Scope::Comment)
            });
            if scope == Scope::Comment {
                commented
            } else {
                !commented
            }
        }
    }
}

/// `#[cfg(test)]`, `#[cfg(all(test, unix))]` and the like: what follows is test code.
fn is_cfg_test(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("#[cfg(") && line.contains("test") && !line.contains("not(test")
}

/// Rust files and folders where `unwrap` and friends are how a test is written.
fn is_test_path(path: &str) -> bool {
    let p = path.replace('\\', "/");
    p.starts_with("tests/")
        || p.contains("/tests/")
        || p.ends_with("/tests.rs")
        || p == "tests.rs"
        || p.starts_with("benches/")
        || p.contains("/benches/")
        || p.starts_with("examples/")
        || p.contains("/examples/")
        || p.ends_with("build.rs")
        || p.ends_with("_test.go")
        || p.ends_with(".test.ts")
        || p.ends_with(".test.js")
        || p.ends_with(".spec.ts")
        || p.ends_with(".spec.js")
        || p.rsplit('/').next().is_some_and(|f| f.starts_with("test_"))
}

pub(crate) fn check(
    lang: &'static LanguageDef,
    path: &str,
    text: &str,
    tree: Option<&Tree>,
) -> Vec<Finding> {
    let test_file = is_test_path(path);
    let mut out = Vec::new();
    let mut line_start = 0usize;
    let mut in_test_module = false;
    for (line_no, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let start_of_line = line_start;
        line_start += raw.len() + 1;
        if lang.id == "rust" && is_cfg_test(line) {
            in_test_module = true;
        }
        if line.len() > 2000 || line.contains("codigo-ignore") {
            continue;
        }
        for (pattern, re) in patterns().iter().zip(compiled()) {
            let Some(re) = re else { continue };
            if !pattern.langs.is_empty() && !pattern.langs.contains(&lang.id) {
                continue;
            }
            if (test_file || in_test_module) && matches!(pattern.rule, "may-panic" | "debug-output")
            {
                continue;
            }
            if pattern.unless.iter().any(|u| line.contains(u)) {
                continue;
            }
            for caps in re.captures_iter(line) {
                let Some(m) = caps.get(0) else { continue };
                if pattern.check.is_some_and(|f| !f(&caps)) {
                    continue;
                }
                if !in_scope(
                    pattern.scope,
                    tree,
                    lang,
                    line,
                    m.start(),
                    start_of_line + m.start(),
                ) {
                    continue;
                }
                let Some(mut f) = Finding::new(pattern.rule, line_no, m.start(), m.end()) else {
                    continue;
                };
                if pattern.detail {
                    if let Some(d) = caps.get(1) {
                        f = f.with_detail(d.as_str());
                    }
                }
                if pattern.rule == "insecure-url"
                    && out
                        .iter()
                        .any(|o: &Finding| o.rule.id == "insecure-url" && o.line == line_no)
                {
                    continue;
                }
                out.push(f);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles_and_names_a_rule() {
        for (p, re) in patterns().iter().zip(compiled()) {
            assert!(re.is_some(), "{} does not compile", p.rule);
            assert!(crate::rule(p.rule).is_some(), "{} has no rule", p.rule);
        }
    }

    #[test]
    fn test_modules_are_recognised() {
        assert!(is_cfg_test("#[cfg(test)]"));
        assert!(is_cfg_test("  #[cfg(all(test, unix))]"));
        assert!(!is_cfg_test("#[cfg(not(test))]"));
        assert!(!is_cfg_test("#[derive(Debug)]"));
        assert!(is_test_path("crates/x/src/buffer/tests.rs"));
    }

    #[test]
    fn entropy_tells_words_from_keys() {
        assert!(entropy("aaaaaaaa") < 1.0);
        assert!(entropy("x9Fq2LmZp7Wk") > 3.0);
    }

    fn ids(lang: &str, path: &str, text: &str) -> Vec<&'static str> {
        let lang = codigo_syntax::find(lang).unwrap_or_else(codigo_syntax::plain);
        crate::analyze_text(lang, path, text)
            .into_iter()
            .map(|f| f.rule.id)
            .collect()
    }

    #[test]
    fn secrets_are_found_but_placeholders_are_not() {
        assert_eq!(
            ids("python", "a.py", "key = \"AKIAIOSFODNN7EXAMPLE\"\n"),
            ["cloud-token"]
        );
        assert_eq!(
            ids("bash", "a.sh", "-----BEGIN RSA PRIVATE KEY-----\n"),
            ["private-key"]
        );
        assert_eq!(
            ids("python", "a.py", "db_password = \"Zq8f2Lm9Xw4Kp7\"\n"),
            ["hardcoded-secret"]
        );
        for fine in [
            "password = \"changeme-please\"\n",
            "password = os.environ[\"DB_PASSWORD\"]\n",
            "api_key = \"${API_KEY}\"\n",
            "token = \"aaaaaaaaaaaa\"\n",
        ] {
            assert!(ids("python", "a.py", fine).is_empty(), "{fine}");
        }
    }

    #[test]
    fn rust_shortcuts_count_in_code_but_not_in_tests_strings_or_comments() {
        let src = "fn f() { let x = a.unwrap(); }\n";
        assert_eq!(ids("rust", "src/a.rs", src), ["may-panic"]);
        assert!(ids("rust", "tests/a.rs", src).is_empty());
        assert!(ids("rust", "src/a.rs", "// call .unwrap() here\n").is_empty());
        assert!(ids("rust", "src/a.rs", "let s = \"x.unwrap()\";\n").is_empty());
        let with_tests = "fn f() {}\n#[cfg(test)]\nmod tests { fn t() { a.unwrap(); } }\n";
        assert!(ids("rust", "src/a.rs", with_tests).is_empty());
        assert_eq!(
            ids("rust", "src/a.rs", "fn f() { unsafe { g() } }\n"),
            ["unsafe-block"]
        );
    }

    #[test]
    fn dangerous_calls_are_found_per_language() {
        assert_eq!(ids("python", "a.py", "eval(user_input)\n"), ["eval"]);
        assert_eq!(
            ids("python", "a.py", "subprocess.run(cmd, shell=True)\n"),
            ["shell-command"]
        );
        assert_eq!(
            ids("javascript", "a.js", "el.innerHTML = html;\n"),
            ["html-injection"]
        );
        assert_eq!(
            ids("c", "a.c", "void f(char *s) { strcpy(buf, s); }\n"),
            ["unsafe-c-function"]
        );
        assert_eq!(
            ids("python", "a.py", "yaml.load(text)\n"),
            ["unsafe-deserialisation"]
        );
        assert!(ids(
            "python",
            "a.py",
            "yaml.load(text, Loader=yaml.SafeLoader)\n"
        )
        .is_empty());
        assert_eq!(
            ids("go", "a.go", "c := &tls.Config{InsecureSkipVerify: true}\n"),
            ["tls-verification-off"]
        );
        assert_eq!(
            ids("python", "a.py", "h = hashlib.md5(data)\n"),
            ["weak-hash"]
        );
        assert_eq!(
            ids(
                "python",
                "a.py",
                "q = \"SELECT * FROM t WHERE id = \" + uid\n"
            ),
            ["sql-concat"]
        );
    }

    #[test]
    fn urls_and_todos_follow_their_scope() {
        assert_eq!(
            ids("python", "a.py", "u = \"http://api.company.org/x\"\n"),
            ["insecure-url"]
        );
        assert!(ids("python", "a.py", "u = \"http://localhost:8080/x\"\n").is_empty());
        assert!(ids("python", "a.py", "# see http://api.company.org\n").is_empty());
        assert_eq!(ids("python", "a.py", "# TODO: later\n"), ["todo-comment"]);
        assert!(ids("python", "a.py", "s = \"TODO\"\n").is_empty());
    }

    #[test]
    fn the_ignore_marker_and_hostile_input_are_safe() {
        assert!(ids("python", "a.py", "eval(x)  # codigo-ignore\n").is_empty());
        assert!(ids("python", "a.py", "# codigo-ignore-file\neval(x)\n").is_empty());
        let long = "x".repeat(5000);
        assert!(ids("python", "a.py", &long).is_empty());
        assert!(ids("python", "a.py", "").is_empty());
        assert!(ids("plain", "notes.txt", "password = \"Zq8f2Lm9Xw4Kp7\"\n")
            .contains(&"hardcoded-secret"));
    }
}
