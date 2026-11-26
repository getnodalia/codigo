// SPDX-License-Identifier: MIT OR Apache-2.0
//! What a server reports about a file.

use serde_json::Value;

use crate::position::{Position, Range};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
}

/// More than this per file is noise; keeping a bounded list keeps the editor fast.
pub const MAX_PER_FILE: usize = 2_000;
const MAX_MESSAGE_CHARS: usize = 4_000;

/// Read the `diagnostics` array of a `publishDiagnostics` notification. Entries that do not parse
/// are dropped; the rest are sorted by position, errors first within the same place.
pub fn parse_list(value: &Value) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = value
        .as_array()
        .map(|a| a.iter().filter_map(parse_one).take(MAX_PER_FILE).collect())
        .unwrap_or_default();
    out.sort_by_key(|d| (d.range.start.line, d.range.start.character, d.severity));
    out
}

fn parse_one(v: &Value) -> Option<Diagnostic> {
    let range = parse_range(v.get("range")?)?;
    let severity = match v.get("severity").and_then(Value::as_u64) {
        Some(1) | None => Severity::Error,
        Some(2) => Severity::Warning,
        Some(3) => Severity::Information,
        Some(_) => Severity::Hint,
    };
    let message: String = v
        .get("message")?
        .as_str()?
        .chars()
        .take(MAX_MESSAGE_CHARS)
        .collect();
    let source = v.get("source").and_then(Value::as_str).map(str::to_owned);
    let code = v.get("code").and_then(|c| match c {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    });
    Some(Diagnostic {
        range,
        severity,
        message,
        source,
        code,
    })
}

pub fn parse_range(v: &Value) -> Option<Range> {
    Some(Range {
        start: parse_position(v.get("start")?)?,
        end: parse_position(v.get("end")?)?,
    })
}

pub fn parse_position(v: &Value) -> Option<Position> {
    Some(Position {
        line: u32::try_from(v.get("line")?.as_u64()?).ok()?,
        character: u32::try_from(v.get("character")?.as_u64()?).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diagnostics_parse_and_sort_and_junk_is_dropped() {
        let v = json!([
            {"range": {"start": {"line": 5, "character": 1}, "end": {"line": 5, "character": 4}},
             "severity": 2, "message": "unused", "source": "rustc", "code": 12},
            {"range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 2}},
             "message": "boom", "code": "E0308"},
            {"range": "nonsense", "message": "x"},
            {"message": "no range"},
            {"range": {"start": {"line": -1, "character": 0}, "end": {"line": 0, "character": 0}}, "message": "neg"}
        ]);
        let d = parse_list(&v);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].message, "boom");
        assert_eq!(d[0].severity, Severity::Error);
        assert_eq!(d[0].code.as_deref(), Some("E0308"));
        assert_eq!(d[1].severity, Severity::Warning);
        assert_eq!(d[1].code.as_deref(), Some("12"));
    }

    #[test]
    fn not_an_array_is_empty() {
        assert!(parse_list(&json!({"a": 1})).is_empty());
    }
}
