// SPDX-License-Identifier: MIT OR Apache-2.0
//! Snippets: a short word that expands into a block of code with places to fill in. Typing the
//! word and pressing Tab expands it, the first place is selected, and Tab moves to the next.
//!
//! The body syntax is the one language servers and most editors share: `$1`, `${1:name}` for a
//! place to fill in (visited in number order), `$0` for where the caret ends, `\$` for a dollar.
//! Built-in snippets cover the common languages; `snippets.json` in the settings folder adds
//! more, in the same shape as VS Code's (`{"rust": {"Name": {"prefix": "x", "body": ["a", "b"]}}}`).

use std::collections::HashMap;
use std::path::Path;

const MAX_BODY: usize = 20_000;
const MAX_USER: usize = 500;
const MAX_STOPS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub prefix: String,
    pub name: String,
    pub body: String,
}

/// A snippet ready to insert: the text, and the char ranges of its places in the order they are
/// visited. The last is where the caret ends, and always exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expansion {
    pub text: String,
    pub stops: Vec<(usize, usize)>,
}

/// The snippets the person wrote themselves.
#[derive(Clone, Debug, Default)]
pub struct UserSnippets {
    by_language: HashMap<String, Vec<Snippet>>,
}

impl UserSnippets {
    /// Read `snippets.json` from `dir`. A missing or malformed file is simply no snippets.
    pub fn load(dir: &Path) -> UserSnippets {
        let Ok(text) = std::fs::read_to_string(dir.join("snippets.json")) else {
            return UserSnippets::default();
        };
        if text.len() > 1_000_000 {
            return UserSnippets::default();
        }
        UserSnippets::parse(&text)
    }

    pub fn parse(text: &str) -> UserSnippets {
        let Ok(serde_json::Value::Object(languages)) = serde_json::from_str(text) else {
            return UserSnippets::default();
        };
        let mut by_language = HashMap::new();
        for (language, entries) in languages {
            let Some(entries) = entries.as_object() else {
                continue;
            };
            let mut list = Vec::new();
            for (name, entry) in entries.iter().take(MAX_USER) {
                let prefix = match entry.get("prefix") {
                    Some(serde_json::Value::String(p)) => p.clone(),
                    Some(serde_json::Value::Array(a)) => {
                        a.first().and_then(|p| p.as_str()).unwrap_or("").to_owned()
                    }
                    _ => continue,
                };
                let body = match entry.get("body") {
                    Some(serde_json::Value::String(b)) => b.clone(),
                    Some(serde_json::Value::Array(a)) => a
                        .iter()
                        .filter_map(|l| l.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => continue,
                };
                if prefix.is_empty()
                    || !prefix.chars().all(|c| c.is_alphanumeric() || c == '_')
                    || body.len() > MAX_BODY
                {
                    continue;
                }
                list.push(Snippet {
                    prefix,
                    name: name.clone(),
                    body,
                });
            }
            by_language.insert(language, list);
        }
        UserSnippets { by_language }
    }

    fn get(&self, language: &str) -> &[Snippet] {
        self.by_language.get(language).map_or(&[], Vec::as_slice)
    }
}

/// The snippet that `word` names in `language`: the person's own first, then the built-in ones.
pub fn find(language: &str, word: &str, user: &UserSnippets) -> Option<Snippet> {
    if word.is_empty() {
        return None;
    }
    if let Some(s) = user.get(language).iter().find(|s| s.prefix == word) {
        return Some(s.clone());
    }
    builtin(language)
        .iter()
        .find(|(prefix, _, _)| *prefix == word)
        .map(|(prefix, name, body)| Snippet {
            prefix: (*prefix).to_owned(),
            name: (*name).to_owned(),
            body: (*body).to_owned(),
        })
}

/// Every snippet of `language`, for listing.
pub fn all(language: &str, user: &UserSnippets) -> Vec<Snippet> {
    let mine = user.get(language);
    let mut out: Vec<Snippet> = mine.to_vec();
    for (prefix, name, body) in builtin(language) {
        if !mine.iter().any(|s| s.prefix == *prefix) {
            out.push(Snippet {
                prefix: (*prefix).to_owned(),
                name: (*name).to_owned(),
                body: (*body).to_owned(),
            });
        }
    }
    out
}

/// Turn a body into text and places. `indent` is the whitespace of the line the snippet starts
/// on, added to every following line, and `unit` is what a tab in the body becomes.
pub fn expand(body: &str, indent: &str, unit: &str) -> Expansion {
    let chars: Vec<char> = body.chars().collect();
    let mut out = Out {
        text: String::new(),
        len: 0,
        indent,
        unit,
        marks: Vec::new(),
    };
    parse(&chars, &mut 0, &mut out, false);
    let Out {
        text, len, marks, ..
    } = out;
    let mut numbered: Vec<(u32, usize, usize)> = Vec::new();
    for (number, start, end) in marks {
        if !numbered.iter().any(|(n, _, _)| *n == number) {
            numbered.push((number, start, end));
        }
    }
    numbered.sort_by_key(|(n, _, _)| if *n == 0 { u32::MAX } else { *n });
    let mut stops: Vec<(usize, usize)> = numbered
        .iter()
        .filter(|(n, _, _)| *n != 0)
        .take(MAX_STOPS)
        .map(|(_, s, e)| (*s, *e))
        .collect();
    let end = numbered
        .iter()
        .find(|(n, _, _)| *n == 0)
        .map_or((len, len), |(_, s, e)| (*s, *e));
    stops.push(end);
    Expansion { text, stops }
}

struct Out<'a> {
    text: String,
    /// `text`'s length in chars.
    len: usize,
    indent: &'a str,
    unit: &'a str,
    marks: Vec<(u32, usize, usize)>,
}

impl Out<'_> {
    fn push(&mut self, c: char) {
        match c {
            '\n' => {
                self.text.push('\n');
                self.text.push_str(self.indent);
                self.len += 1 + self.indent.chars().count();
            }
            '\t' => {
                self.text.push_str(self.unit);
                self.len += self.unit.chars().count();
            }
            c => {
                self.text.push(c);
                self.len += 1;
            }
        }
    }
}

/// Read until the end, or until the `}` that closes a placeholder when `inner`.
fn parse(chars: &[char], i: &mut usize, out: &mut Out, inner: bool) {
    let mut depth = 0usize;
    while let Some(&c) = chars.get(*i) {
        match c {
            '\\' if chars.get(*i + 1).is_some() => {
                if let Some(&next) = chars.get(*i + 1) {
                    out.push(next);
                }
                *i += 2;
            }
            '}' if inner && depth == 0 => {
                *i += 1;
                return;
            }
            '$' if chars.get(*i + 1).is_some_and(char::is_ascii_digit) => {
                *i += 1;
                let number = number(chars, i);
                out.marks.push((number, out.len, out.len));
            }
            '$' if chars.get(*i + 1) == Some(&'{')
                && chars.get(*i + 2).is_some_and(char::is_ascii_digit) =>
            {
                *i += 2;
                let number = number(chars, i);
                let start = out.len;
                if chars.get(*i) == Some(&':') {
                    *i += 1;
                    parse(chars, i, out, true);
                } else {
                    // `${1}`: no default text.
                    if chars.get(*i) == Some(&'}') {
                        *i += 1;
                    }
                }
                out.marks.push((number, start, out.len));
            }
            c => {
                if inner && c == '{' {
                    depth += 1;
                }
                if inner && c == '}' {
                    depth = depth.saturating_sub(1);
                }
                out.push(c);
                *i += 1;
            }
        }
    }
}

fn number(chars: &[char], i: &mut usize) -> u32 {
    let mut n: u32 = 0;
    while let Some(d) = chars.get(*i).and_then(|c| c.to_digit(10)) {
        n = n.saturating_mul(10).saturating_add(d);
        *i += 1;
    }
    n
}

type Row = (&'static str, &'static str, &'static str);

/// The snippets every install has, per language id.
pub fn builtin(language: &str) -> &'static [Row] {
    match language {
        "rust" => RUST,
        "python" => PYTHON,
        "javascript" | "typescript" | "tsx" => JAVASCRIPT,
        "go" => GO,
        "c" | "cpp" => C,
        "java" => JAVA,
        "bash" => SHELL,
        "html" => HTML,
        _ => &[],
    }
}

const RUST: &[Row] = &[
    ("fn", "Function", "fn ${1:name}(${2}) {\n\t$0\n}"),
    ("pfn", "Public function", "pub fn ${1:name}(${2}) -> ${3:()} {\n\t$0\n}"),
    ("main", "Main function", "fn main() {\n\t$0\n}"),
    ("test", "Test", "#[test]\nfn ${1:name}() {\n\t$0\n}"),
    ("mod", "Test module", "#[cfg(test)]\nmod tests {\n\tuse super::*;\n\n\t#[test]\n\tfn ${1:name}() {\n\t\t$0\n\t}\n}"),
    ("struct", "Struct", "struct ${1:Name} {\n\t${2:field}: ${3:Type},\n}\n$0"),
    ("enum", "Enum", "enum ${1:Name} {\n\t${2:Variant},\n}\n$0"),
    ("impl", "Implementation", "impl ${1:Type} {\n\t$0\n}"),
    ("trait", "Trait", "trait ${1:Name} {\n\t$0\n}"),
    ("match", "Match", "match ${1:value} {\n\t${2:pattern} => ${3:result},\n\t_ => $0,\n}"),
    ("iflet", "If let", "if let ${1:Some(x)} = ${2:value} {\n\t$0\n}"),
    ("for", "For loop", "for ${1:item} in ${2:items} {\n\t$0\n}"),
    ("while", "While loop", "while ${1:condition} {\n\t$0\n}"),
    ("derive", "Derive", "#[derive(${1:Debug, Clone})]\n$0"),
    ("pr", "println!", "println!(\"${1:{}}\", ${2:value});$0"),
];

const PYTHON: &[Row] = &[
    ("def", "Function", "def ${1:name}(${2}):\n\t${3:pass}$0"),
    (
        "class",
        "Class",
        "class ${1:Name}:\n\tdef __init__(self${2}):\n\t\t${3:pass}$0",
    ),
    (
        "main",
        "Entry point",
        "def main():\n\t${1:pass}\n\n\nif __name__ == \"__main__\":\n\tmain()$0",
    ),
    ("ifmain", "If main", "if __name__ == \"__main__\":\n\t$0"),
    ("for", "For loop", "for ${1:item} in ${2:items}:\n\t$0"),
    ("while", "While loop", "while ${1:condition}:\n\t$0"),
    (
        "try",
        "Try",
        "try:\n\t$1\nexcept ${2:Exception} as ${3:error}:\n\t$0",
    ),
    ("with", "With", "with ${1:open(path)} as ${2:file}:\n\t$0"),
    ("test", "Test", "def test_${1:name}():\n\t$0"),
];

const JAVASCRIPT: &[Row] = &[
    ("fn", "Function", "function ${1:name}(${2}) {\n\t$0\n}"),
    (
        "af",
        "Arrow function",
        "const ${1:name} = (${2}) => {\n\t$0\n};",
    ),
    ("log", "console.log", "console.log(${1});$0"),
    (
        "for",
        "For loop",
        "for (let ${1:i} = 0; ${1:i} < ${2:length}; ${1:i}++) {\n\t$0\n}",
    ),
    (
        "forof",
        "For of",
        "for (const ${1:item} of ${2:items}) {\n\t$0\n}",
    ),
    ("if", "If", "if (${1:condition}) {\n\t$0\n}"),
    ("try", "Try", "try {\n\t$1\n} catch (${2:error}) {\n\t$0\n}"),
    (
        "class",
        "Class",
        "class ${1:Name} {\n\tconstructor(${2}) {\n\t\t$0\n\t}\n}",
    ),
    ("imp", "Import", "import { ${2} } from \"${1:module}\";$0"),
    (
        "desc",
        "Test suite",
        "describe(\"${1:name}\", () => {\n\tit(\"${2:works}\", () => {\n\t\t$0\n\t});\n});",
    ),
];

const GO: &[Row] = &[
    ("main", "Main function", "func main() {\n\t$0\n}"),
    (
        "func",
        "Function",
        "func ${1:name}(${2}) ${3:error} {\n\t$0\n}",
    ),
    (
        "meth",
        "Method",
        "func (${1:r} ${2:Type}) ${3:Name}(${4}) ${5} {\n\t$0\n}",
    ),
    (
        "struct",
        "Struct",
        "type ${1:Name} struct {\n\t${2:Field} ${3:string}\n}\n$0",
    ),
    (
        "iferr",
        "If err",
        "if err != nil {\n\treturn ${1:err}\n}\n$0",
    ),
    (
        "for",
        "For loop",
        "for ${1:i} := 0; ${1:i} < ${2:n}; ${1:i}++ {\n\t$0\n}",
    ),
    (
        "forr",
        "For range",
        "for ${1:i}, ${2:v} := range ${3:items} {\n\t$0\n}",
    ),
    (
        "test",
        "Test",
        "func Test${1:Name}(t *testing.T) {\n\t$0\n}",
    ),
];

const C: &[Row] = &[
    (
        "main",
        "Main function",
        "int main(int argc, char **argv) {\n\t$0\n\treturn 0;\n}",
    ),
    (
        "for",
        "For loop",
        "for (int ${1:i} = 0; ${1:i} < ${2:n}; ${1:i}++) {\n\t$0\n}",
    ),
    ("if", "If", "if (${1:condition}) {\n\t$0\n}"),
    ("while", "While loop", "while (${1:condition}) {\n\t$0\n}"),
    ("struct", "Struct", "struct ${1:name} {\n\t$0\n};"),
    ("inc", "Include", "#include <${1:stdio.h}>$0"),
    (
        "guard",
        "Include guard",
        "#ifndef ${1:NAME_H}\n#define ${1:NAME_H}\n\n$0\n\n#endif",
    ),
];

const JAVA: &[Row] = &[
    (
        "main",
        "Main method",
        "public static void main(String[] args) {\n\t$0\n}",
    ),
    ("class", "Class", "public class ${1:Name} {\n\t$0\n}"),
    (
        "for",
        "For loop",
        "for (int ${1:i} = 0; ${1:i} < ${2:n}; ${1:i}++) {\n\t$0\n}",
    ),
    (
        "fore",
        "For each",
        "for (${1:Type} ${2:item} : ${3:items}) {\n\t$0\n}",
    ),
    ("if", "If", "if (${1:condition}) {\n\t$0\n}"),
    (
        "try",
        "Try",
        "try {\n\t$1\n} catch (${2:Exception} ${3:e}) {\n\t$0\n}",
    ),
    ("sout", "System.out.println", "System.out.println(${1});$0"),
];

const SHELL: &[Row] = &[
    (
        "shebang",
        "Shebang",
        "#!/usr/bin/env bash\nset -euo pipefail\n\n$0",
    ),
    ("if", "If", "if [[ ${1:condition} ]]; then\n\t$0\nfi"),
    (
        "for",
        "For loop",
        "for ${1:item} in ${2:items}; do\n\t$0\ndone",
    ),
    (
        "while",
        "While loop",
        "while ${1:condition}; do\n\t$0\ndone",
    ),
    ("fn", "Function", "${1:name}() {\n\t$0\n}"),
    (
        "case",
        "Case",
        "case \"${1:value}\" in\n\t${2:pattern})\n\t\t$0\n\t\t;;\nesac",
    ),
];

const HTML: &[Row] = &[
    ("html5", "HTML page", "<!doctype html>\n<html lang=\"${1:en}\">\n<head>\n\t<meta charset=\"utf-8\">\n\t<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\t<title>${2:Title}</title>\n</head>\n<body>\n\t$0\n</body>\n</html>"),
    ("link", "Stylesheet", "<link rel=\"stylesheet\" href=\"${1:style.css}\">$0"),
    ("script", "Script", "<script src=\"${1:main.js}\"></script>$0"),
    ("a", "Link", "<a href=\"${1:#}\">${2:text}</a>$0"),
    ("img", "Image", "<img src=\"${1}\" alt=\"${2}\">$0"),
    ("ul", "List", "<ul>\n\t<li>${1}</li>\n\t$0\n</ul>"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_are_visited_in_number_order_and_zero_comes_last() {
        let e = expand("fn ${1:name}(${2}) {\n\t$0\n}", "", "    ");
        assert_eq!(e.text, "fn name() {\n    \n}");
        assert_eq!(e.stops, vec![(3, 7), (8, 8), (16, 16)]);
    }

    #[test]
    fn following_lines_keep_the_indent_of_the_first() {
        let e = expand("if x {\n\t$0\n}", "    ", "\t");
        assert_eq!(e.text, "if x {\n    \t\n    }");
        assert_eq!(e.stops.last(), Some(&(12, 12)));
    }

    #[test]
    fn a_snippet_without_zero_ends_at_its_end() {
        let e = expand("a${1:b}c", "", "  ");
        assert_eq!(e.text, "abc");
        assert_eq!(e.stops, vec![(1, 2), (3, 3)]);
        assert_eq!(expand("plain", "", " ").stops, vec![(5, 5)]);
    }

    #[test]
    fn a_repeated_place_is_visited_once_and_escapes_work() {
        let e = expand("${1:i} < ${1:i} \\$x \\\\", "", " ");
        assert_eq!(e.text, "i < i $x \\");
        assert_eq!(e.stops.len(), 2);
        assert_eq!(e.stops[0], (0, 1));
    }

    #[test]
    fn nested_placeholders_and_broken_bodies_do_not_panic() {
        let e = expand("${1:a ${2:b}} end", "", " ");
        assert_eq!(e.text, "a b end");
        for body in [
            "${",
            "$",
            "${1:unclosed",
            "}}}",
            "\\",
            "${1",
            "$$1",
            "${99999999999999:x}",
        ] {
            let _ = expand(body, "  ", "\t");
        }
    }

    #[test]
    fn every_builtin_snippet_expands_with_a_final_stop() {
        for lang in [
            "rust",
            "python",
            "javascript",
            "typescript",
            "go",
            "c",
            "cpp",
            "java",
            "bash",
            "html",
        ] {
            assert!(!builtin(lang).is_empty(), "{lang}");
            let mut seen = Vec::new();
            for (prefix, name, body) in builtin(lang) {
                assert!(!seen.contains(prefix), "{lang}: {prefix} twice");
                seen.push(prefix);
                assert!(!name.is_empty());
                let e = expand(body, "", "    ");
                assert!(!e.stops.is_empty(), "{lang}: {prefix}");
                for (s, t) in &e.stops {
                    assert!(s <= t && *t <= e.text.chars().count(), "{lang}: {prefix}");
                }
            }
        }
        assert!(builtin("brainfuck").is_empty());
    }

    #[test]
    fn user_snippets_win_and_bad_entries_are_skipped() {
        let user = UserSnippets::parse(
            r#"{"rust": {"Mine": {"prefix": "fn", "body": ["fn mine() {", "\t$0", "}"]},
                         "Bad": {"prefix": "has space", "body": "x"},
                         "NoBody": {"prefix": "nb"}},
                "python": 5}"#,
        );
        let found = find("rust", "fn", &user).expect("snippet");
        assert_eq!(found.name, "Mine");
        assert_eq!(found.body, "fn mine() {\n\t$0\n}");
        assert!(find("rust", "has", &user).is_none());
        assert!(find("rust", "main", &user).is_some());
        assert!(find("rust", "", &user).is_none());
        assert!(UserSnippets::parse("not json").by_language.is_empty());
        let listed = all("rust", &user);
        assert_eq!(listed.iter().filter(|s| s.prefix == "fn").count(), 1);
    }
}
