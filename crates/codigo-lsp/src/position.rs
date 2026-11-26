// SPDX-License-Identifier: MIT OR Apache-2.0
//! Positions as the protocol counts them: zero-based lines, and columns in UTF-16 code units.
//! The editor counts chars, so every position crossing the boundary is converted.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// The char column that sits at `utf16` code units into `line`. A column past the end clamps to
/// the line's length; one that lands inside a surrogate pair rounds down to the char before it.
pub fn utf16_to_char(line: &str, utf16: u32) -> usize {
    let mut units = 0u32;
    for (i, ch) in line.chars().enumerate() {
        let next = units.saturating_add(ch.len_utf16() as u32);
        if next > utf16 {
            return i;
        }
        units = next;
    }
    line.chars().count()
}

/// The UTF-16 column of the char at `col` in `line`, clamped to the line's length.
pub fn char_to_utf16(line: &str, col: usize) -> u32 {
    line.chars()
        .take(col)
        .map(|c| c.len_utf16() as u32)
        .fold(0u32, u32::saturating_add)
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// `file:///home/me/a%20b.rs` for an absolute path. Returns `None` for a relative one.
pub fn path_to_uri(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let text = path.to_str()?;
    let mut out = String::from("file://");
    #[cfg(windows)]
    {
        out.push('/');
    }
    for b in text.replace('\\', "/").bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 15) as usize] as char);
            }
        }
    }
    Some(out)
}

/// The path of a `file://` URI, or `None` for any other scheme or a malformed escape.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = hex(*bytes.get(i + 1)?)?;
            let lo = hex(*bytes.get(i + 2)?)?;
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let text = String::from_utf8(out).ok()?;
    #[cfg(windows)]
    let text = text.strip_prefix('/').unwrap_or(&text).to_string();
    if text.is_empty() {
        return None;
    }
    Some(PathBuf::from(text))
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_columns_count_astral_chars_twice() {
        let line = "a😀b";
        assert_eq!(char_to_utf16(line, 2), 3);
        assert_eq!(utf16_to_char(line, 3), 2);
        assert_eq!(
            utf16_to_char(line, 2),
            1,
            "inside a surrogate pair rounds down"
        );
        assert_eq!(utf16_to_char(line, 99), 3);
        assert_eq!(char_to_utf16(line, 99), 4);
    }

    #[test]
    fn accents_are_one_unit() {
        assert_eq!(char_to_utf16("año", 2), 2);
        assert_eq!(utf16_to_char("año", 2), 2);
    }

    #[test]
    #[cfg(unix)]
    fn uris_round_trip_with_spaces_and_unicode() {
        let p = Path::new("/home/me/mi carpeta/año.rs");
        let uri = path_to_uri(p).expect("uri");
        assert_eq!(uri, "file:///home/me/mi%20carpeta/a%C3%B1o.rs");
        assert_eq!(uri_to_path(&uri).as_deref(), Some(p));
    }

    #[test]
    fn bad_uris_are_none() {
        assert_eq!(uri_to_path("https://x/y"), None);
        assert_eq!(uri_to_path("file:///a%2"), None);
        assert_eq!(uri_to_path("file:///a%zz"), None);
        assert_eq!(path_to_uri(Path::new("relative.rs")), None);
    }
}
