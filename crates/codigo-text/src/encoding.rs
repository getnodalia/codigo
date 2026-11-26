// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading and writing files: UTF-8 (with or without a byte order mark), the file's own line
//! ending kept, and binary files refused.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// A NUL byte in the first 8 KiB: not text.
    Binary,
    /// Not valid UTF-8 (UTF-16 files, legacy code pages).
    NotUtf8,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Binary => f.write_str("the file is binary"),
            DecodeError::NotUtf8 => f.write_str("the file is not UTF-8"),
        }
    }
}

impl std::error::Error for DecodeError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// The text with `\n` line endings.
    pub text: String,
    pub line_ending: LineEnding,
    pub bom: bool,
}

/// Decode a file. A file with mixed endings is read as CRLF only when most of its lines end so.
pub fn decode(bytes: &[u8]) -> Result<Decoded, DecodeError> {
    let (bom, bytes) = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => (true, rest),
        None => (false, bytes),
    };
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Err(DecodeError::Binary);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| DecodeError::NotUtf8)?;
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    if crlf > lf {
        Ok(Decoded {
            text: text.replace("\r\n", "\n"),
            line_ending: LineEnding::CrLf,
            bom,
        })
    } else {
        Ok(Decoded {
            text: text.to_owned(),
            line_ending: LineEnding::Lf,
            bom,
        })
    }
}

/// The bytes to write for a buffer's text.
pub fn encode(text: &str, line_ending: LineEnding, bom: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 3);
    if bom {
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    match line_ending {
        LineEnding::Lf => out.extend_from_slice(text.as_bytes()),
        LineEnding::CrLf => out.extend_from_slice(text.replace('\n', "\r\n").as_bytes()),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_is_kept_for_saving_and_not_in_the_text() {
        let d = decode(b"a\r\nb\r\nc").unwrap();
        assert_eq!(d.text, "a\nb\nc");
        assert_eq!(d.line_ending, LineEnding::CrLf);
        assert_eq!(encode(&d.text, d.line_ending, d.bom), b"a\r\nb\r\nc");
    }

    #[test]
    fn a_mostly_lf_file_stays_lf() {
        let d = decode(b"a\nb\nc\r\n").unwrap();
        assert_eq!(d.line_ending, LineEnding::Lf);
    }

    #[test]
    fn the_byte_order_mark_round_trips() {
        let d = decode(b"\xEF\xBB\xBFhi").unwrap();
        assert!(d.bom);
        assert_eq!(d.text, "hi");
        assert_eq!(encode("hi", LineEnding::Lf, true), b"\xEF\xBB\xBFhi");
    }

    #[test]
    fn binary_and_foreign_encodings_are_refused() {
        assert_eq!(decode(b"ab\0cd"), Err(DecodeError::Binary));
        assert_eq!(decode(&[0xFF, 0xFE, b'a']), Err(DecodeError::NotUtf8));
        assert!(decode(b"").is_ok());
    }
}
