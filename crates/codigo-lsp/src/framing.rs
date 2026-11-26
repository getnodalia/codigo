// SPDX-License-Identifier: MIT OR Apache-2.0
//! The base protocol: `Content-Length: N\r\n\r\n` followed by N bytes of JSON.

use std::io::{self, BufRead, Read, Write};

use serde_json::Value;

/// The largest message accepted. Servers send whole-file symbol lists and completion sets, but a
/// header claiming more than this is a bug or an attack.
pub const MAX_MESSAGE: usize = 64 * 1024 * 1024;
const MAX_HEADER_LINES: usize = 32;
const MAX_HEADER_LINE: u64 = 8 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("the server closed the connection")]
    Closed,
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
    #[error("bad header: {0}")]
    Header(String),
    #[error("message of {0} bytes is over the limit")]
    TooLarge(usize),
}

/// Read one message. `Ok(None)` means the body was not valid JSON: the caller skips it and reads
/// on, since the framing itself is still intact.
pub fn read_message<R: BufRead>(r: &mut R) -> Result<Option<Value>, FrameError> {
    let mut length: Option<usize> = None;
    let mut lines = 0;
    loop {
        let mut line = Vec::new();
        let mut limited = Read::take(&mut *r, MAX_HEADER_LINE);
        let n = limited.read_until(b'\n', &mut line)?;
        if n == 0 {
            return Err(FrameError::Closed);
        }
        if !line.ends_with(b"\n") {
            return Err(FrameError::Header("header line too long".into()));
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            break;
        }
        lines += 1;
        if lines > MAX_HEADER_LINES {
            return Err(FrameError::Header("too many headers".into()));
        }
        if let Some((name, value)) = text.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| FrameError::Header(format!("bad length {value:?}")))?,
                );
            }
        }
    }
    let length = length.ok_or_else(|| FrameError::Header("no Content-Length".into()))?;
    if length > MAX_MESSAGE {
        return Err(FrameError::TooLarge(length));
    }
    let mut body = vec![0u8; length];
    r.read_exact(&mut body).map_err(|e| {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            FrameError::Closed
        } else {
            FrameError::Io(e)
        }
    })?;
    Ok(serde_json::from_slice(&body).ok())
}

pub fn write_message<W: Write + ?Sized>(w: &mut W, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn a_message_round_trips() {
        let mut buf = Vec::new();
        write_message(&mut buf, &json!({"jsonrpc": "2.0", "id": 1, "method": "x"})).expect("w");
        let got = read_message(&mut Cursor::new(buf))
            .expect("r")
            .expect("json");
        assert_eq!(got["method"], "x");
    }

    #[test]
    fn two_messages_in_a_row_and_other_headers_are_fine() {
        let a = r#"{"a":1}"#;
        let b = r#"{"b":2}"#;
        let raw = format!(
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-length: {}\r\n\r\n{}Content-Length: {}\r\n\r\n{}",
            a.len(),
            a,
            b.len(),
            b
        );
        let mut c = Cursor::new(raw.into_bytes());
        assert_eq!(read_message(&mut c).expect("1").expect("j")["a"], 1);
        assert_eq!(read_message(&mut c).expect("2").expect("j")["b"], 2);
        assert!(matches!(read_message(&mut c), Err(FrameError::Closed)));
    }

    #[test]
    fn invalid_json_is_skipped_not_fatal() {
        let raw = b"Content-Length: 3\r\n\r\n{{{Content-Length: 2\r\n\r\n{}".to_vec();
        let mut c = Cursor::new(raw);
        assert!(read_message(&mut c).expect("frame").is_none());
        assert!(read_message(&mut c).expect("frame").is_some());
    }

    #[test]
    fn hostile_headers_are_errors() {
        for raw in [
            "Content-Length: 99999999999\r\n\r\n".to_string(),
            "Content-Length: abc\r\n\r\n".to_string(),
            "X: y\r\n\r\n{}".to_string(),
            format!("Content-Length: {}\r\n\r\n", MAX_MESSAGE + 1),
            "A".repeat(10_000),
        ] {
            assert!(read_message(&mut Cursor::new(raw.into_bytes())).is_err());
        }
    }

    #[test]
    fn a_truncated_body_is_a_closed_connection() {
        let raw = b"Content-Length: 10\r\n\r\n{}".to_vec();
        assert!(matches!(
            read_message(&mut Cursor::new(raw)),
            Err(FrameError::Closed)
        ));
    }
}
