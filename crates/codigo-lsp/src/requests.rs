// SPDX-License-Identifier: MIT OR Apache-2.0
//! The answers to hover, definition and completion requests, read defensively: a server's reply
//! can take any of several shapes, and none of it is trusted.

use serde_json::Value;

use crate::diagnostics::parse_range;
use crate::position::Range;

const MAX_EDITS: usize = 20_000;
const MAX_FILES: usize = 200;
const MAX_EDIT_TEXT: usize = 2_000_000;

const MAX_LOCATIONS: usize = 200;
const MAX_HOVER_CHARS: usize = 8_000;
const MAX_ITEMS: usize = 2_000;
const MAX_TEXT: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

/// `Location`, `Location[]` or `LocationLink[]`, whichever the server sent. Anything else is empty.
pub fn parse_locations(value: &Value) -> Vec<Location> {
    let one = |v: &Value| -> Option<Location> {
        // A LocationLink names the target; its selection range is the identifier itself.
        if let Some(uri) = v.get("targetUri") {
            let range = v
                .get("targetSelectionRange")
                .or_else(|| v.get("targetRange"))?;
            return Some(Location {
                uri: uri.as_str()?.to_owned(),
                range: parse_range(range)?,
            });
        }
        Some(Location {
            uri: v.get("uri")?.as_str()?.to_owned(),
            range: parse_range(v.get("range")?)?,
        })
    };
    match value {
        Value::Array(items) => items.iter().filter_map(one).take(MAX_LOCATIONS).collect(),
        Value::Object(_) => one(value).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// The text of a hover, as markdown or plain text. `None` when the server had nothing to say.
pub fn parse_hover(value: &Value) -> Option<String> {
    fn text_of(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(a) => a.iter().for_each(|x| text_of(x, out)),
            Value::Object(o) => {
                let Some(body) = o.get("value").and_then(Value::as_str) else {
                    return;
                };
                // A MarkedString with a language is code.
                match o.get("language").and_then(Value::as_str) {
                    Some(lang) => out.push(format!("```{lang}\n{body}\n```")),
                    None => out.push(body.to_owned()),
                }
            }
            _ => {}
        }
    }
    let mut parts = Vec::new();
    text_of(value.get("contents")?, &mut parts);
    let joined = parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if joined.is_empty() {
        return None;
    }
    Some(joined.chars().take(MAX_HOVER_CHARS).collect())
}

/// One replacement a server asks for: the range, and the text that takes its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range,
    pub new_text: String,
}

/// The edits a server wants made to one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEdits {
    pub uri: String,
    pub edits: Vec<TextEdit>,
}

fn parse_edit(v: &Value) -> Option<TextEdit> {
    let text = v.get("newText")?.as_str()?;
    if text.len() > MAX_EDIT_TEXT {
        return None;
    }
    Some(TextEdit {
        range: parse_range(v.get("range")?)?,
        new_text: text.to_owned(),
    })
}

/// `TextEdit[]`, the answer to a formatting request. Anything else is no edits.
pub fn parse_text_edits(value: &Value) -> Vec<TextEdit> {
    value
        .as_array()
        .map(|a| a.iter().filter_map(parse_edit).take(MAX_EDITS).collect())
        .unwrap_or_default()
}

/// A `WorkspaceEdit`, in either of its shapes (`changes` or `documentChanges`). Creating,
/// renaming and deleting files is not supported: those entries are skipped.
pub fn parse_workspace_edit(value: &Value) -> Vec<FileEdits> {
    let mut out: Vec<FileEdits> = Vec::new();
    let mut add = |uri: &str, edits: Vec<TextEdit>| {
        if edits.is_empty() || out.len() >= MAX_FILES {
            return;
        }
        match out.iter_mut().find(|f| f.uri == uri) {
            Some(f) => f.edits.extend(edits),
            None => out.push(FileEdits {
                uri: uri.to_owned(),
                edits,
            }),
        }
    };
    if let Some(changes) = value.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            add(uri, parse_text_edits(edits));
        }
    }
    if let Some(docs) = value.get("documentChanges").and_then(Value::as_array) {
        for d in docs {
            let uri = d
                .get("textDocument")
                .and_then(|t| t.get("uri"))
                .and_then(Value::as_str);
            let edits = d.get("edits").map(parse_text_edits);
            if let (Some(uri), Some(edits)) = (uri, edits) {
                add(uri, edits);
            }
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
    /// The protocol's `CompletionItemKind`, 1 to 25; 0 when unknown.
    pub kind: u8,
    /// What replaces the word being typed.
    pub insert: String,
    /// What the typed prefix is matched against.
    pub filter: String,
    sort: String,
}

/// `CompletionItem[]` or a `CompletionList`, ordered the way the server asked.
pub fn parse_completion(value: &Value) -> Vec<CompletionItem> {
    let items = match value {
        Value::Array(a) => a.as_slice(),
        Value::Object(o) => o
            .get("items")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice),
        _ => &[],
    };
    let mut out: Vec<CompletionItem> = items
        .iter()
        .filter_map(parse_item)
        .take(MAX_ITEMS)
        .collect();
    out.sort_by(|a, b| a.sort.cmp(&b.sort));
    out
}

fn parse_item(v: &Value) -> Option<CompletionItem> {
    let cap = |s: &str| s.chars().take(MAX_TEXT).collect::<String>();
    let label = cap(v.get("label")?.as_str()?);
    if label.is_empty() {
        return None;
    }
    let raw = v
        .get("textEdit")
        .and_then(|e| e.get("newText"))
        .and_then(Value::as_str)
        .or_else(|| v.get("insertText").and_then(Value::as_str))
        .unwrap_or(&label);
    let insert = if v.get("insertTextFormat").and_then(Value::as_u64) == Some(2) {
        strip_snippet(&cap(raw))
    } else {
        cap(raw)
    };
    let filter = v
        .get("filterText")
        .and_then(Value::as_str)
        .map_or_else(|| label.clone(), cap);
    let sort = v
        .get("sortText")
        .and_then(Value::as_str)
        .map_or_else(|| label.clone(), cap);
    Some(CompletionItem {
        detail: v.get("detail").and_then(Value::as_str).map(cap),
        kind: v
            .get("kind")
            .and_then(Value::as_u64)
            .and_then(|k| u8::try_from(k).ok())
            .unwrap_or(0),
        label,
        insert,
        filter,
        sort,
    })
}

/// A snippet as plain text: `$0` and `$1` vanish, `${1:name}` becomes `name`, `\$` is `$`.
pub fn strip_snippet(snippet: &str) -> String {
    let chars: Vec<char> = snippet.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                out.push(chars[i + 1]);
                i += 2;
            }
            '$' if chars.get(i + 1).is_some_and(char::is_ascii_digit) => {
                i += 1;
                while chars.get(i).is_some_and(char::is_ascii_digit) {
                    i += 1;
                }
            }
            '$' if chars.get(i + 1) == Some(&'{') => {
                i += 2;
                while chars.get(i).is_some_and(char::is_ascii_digit) {
                    i += 1;
                }
                if chars.get(i) == Some(&':') {
                    i += 1;
                }
                // The placeholder runs to the matching brace; it may hold placeholders itself.
                let mut depth = 1;
                let mut inner = String::new();
                while i < chars.len() {
                    match chars[i] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                i += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    inner.push(chars[i]);
                    i += 1;
                }
                out.push_str(&strip_snippet(&inner));
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn range(l: u64) -> Value {
        json!({"start": {"line": l, "character": 1}, "end": {"line": l, "character": 4}})
    }

    #[test]
    fn locations_in_every_shape() {
        let one = json!({"uri": "file:///a.rs", "range": range(2)});
        assert_eq!(parse_locations(&one).len(), 1);
        assert_eq!(parse_locations(&json!([one.clone(), one])).len(), 2);
        let link = json!([{"targetUri": "file:///b.rs", "targetRange": range(1), "targetSelectionRange": range(7)}]);
        let got = parse_locations(&link);
        assert_eq!(got[0].uri, "file:///b.rs");
        assert_eq!(got[0].range.start.line, 7);
        assert!(parse_locations(&Value::Null).is_empty());
        assert!(parse_locations(&json!([{"uri": 5}, "x"])).is_empty());
    }

    #[test]
    fn hover_contents_in_every_shape() {
        let md = json!({"contents": {"kind": "markdown", "value": " fn x() \n"}});
        assert_eq!(parse_hover(&md).as_deref(), Some("fn x()"));
        assert_eq!(
            parse_hover(&json!({"contents": "plain"})).as_deref(),
            Some("plain")
        );
        let marked = json!({"contents": [{"language": "rust", "value": "let a"}, "docs"]});
        assert_eq!(
            parse_hover(&marked).as_deref(),
            Some("```rust\nlet a\n```\n\ndocs")
        );
        assert_eq!(parse_hover(&json!({"contents": ""})), None);
        assert_eq!(parse_hover(&Value::Null), None);
        let huge = json!({"contents": "a".repeat(100_000)});
        assert_eq!(parse_hover(&huge).map(|s| s.len()), Some(MAX_HOVER_CHARS));
    }

    #[test]
    fn completion_items_are_ordered_and_read_from_a_list_or_array() {
        let list = json!({"isIncomplete": false, "items": [
            {"label": "zeta", "sortText": "2"},
            {"label": "alpha", "kind": 3, "detail": "fn()", "sortText": "1",
             "textEdit": {"newText": "alpha()", "range": range(0)}},
            {"label": ""},
            {"nolabel": true}
        ]});
        let got = parse_completion(&list);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].label, "alpha");
        assert_eq!(got[0].insert, "alpha()");
        assert_eq!(got[0].kind, 3);
        assert_eq!(got[1].insert, "zeta");
        assert_eq!(parse_completion(&json!([{"label": "x"}])).len(), 1);
        assert!(parse_completion(&json!(7)).is_empty());
    }

    #[test]
    fn snippets_become_plain_text() {
        assert_eq!(
            strip_snippet("println!(\"${1:msg}\")$0"),
            "println!(\"msg\")"
        );
        assert_eq!(strip_snippet("a\\$b ${2} $3"), "a$b  ");
        assert_eq!(strip_snippet("${1:a ${2:b}} end"), "a b end");
        assert_eq!(strip_snippet("${1:unclosed"), "unclosed");
        let item = json!({"label": "f", "insertText": "f(${1:x})", "insertTextFormat": 2});
        assert_eq!(parse_completion(&json!([item]))[0].insert, "f(x)");
    }

    #[test]
    fn edits_are_read_in_both_workspace_shapes() {
        let edit = json!({"range": range(1), "newText": "x"});
        assert_eq!(
            parse_text_edits(&json!([edit.clone(), {"newText": 1}])).len(),
            1
        );
        assert!(parse_text_edits(&Value::Null).is_empty());
        let a = json!({"changes": {"file:///a.rs": [edit.clone()], "file:///b.rs": []}});
        let got = parse_workspace_edit(&a);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].uri, "file:///a.rs");
        let b = json!({"documentChanges": [
            {"textDocument": {"uri": "file:///c.rs", "version": 1}, "edits": [edit.clone()]},
            {"kind": "rename", "oldUri": "file:///c.rs", "newUri": "file:///d.rs"},
            {"textDocument": {"uri": "file:///c.rs"}, "edits": [edit]}
        ]});
        let got = parse_workspace_edit(&b);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].edits.len(), 2);
        assert!(parse_workspace_edit(&json!(3)).is_empty());
    }
}
