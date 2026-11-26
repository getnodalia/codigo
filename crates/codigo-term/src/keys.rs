// SPDX-License-Identifier: MIT OR Apache-2.0
//! Turning keys into the bytes a terminal program expects.

/// A key the terminal cares about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1 to F12.
    F(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// The bytes to send for `key`. `app_cursor` is the screen's application cursor mode, which
/// changes what the arrows send. Returns nothing for combinations with no encoding.
pub fn encode(key: Key, mods: Mods, app_cursor: bool) -> Vec<u8> {
    let modifier = 1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.ctrl);
    let csi = |final_byte: char| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{final_byte}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[{n};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    match key {
        Key::Char(c) => {
            let mut out = Vec::new();
            if mods.alt {
                out.push(0x1b);
            }
            if mods.ctrl {
                if let Some(code) = control_code(c) {
                    out.push(code);
                    return out;
                }
            }
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            out
        }
        Key::Enter => vec![b'\r'],
        Key::Backspace => vec![0x7f],
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => vec![b'\t'],
        Key::Escape => vec![0x1b],
        Key::Up => csi('A'),
        Key::Down => csi('B'),
        Key::Right => csi('C'),
        Key::Left => csi('D'),
        Key::Home => csi('H'),
        Key::End => csi('F'),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::F(n) => match n {
            1..=4 => {
                let letter = char::from(b'P' + (n - 1));
                if modifier > 1 {
                    format!("\x1b[1;{modifier}{letter}").into_bytes()
                } else {
                    format!("\x1bO{letter}").into_bytes()
                }
            }
            5 => tilde(15),
            6 => tilde(17),
            7 => tilde(18),
            8 => tilde(19),
            9 => tilde(20),
            10 => tilde(21),
            11 => tilde(23),
            12 => tilde(24),
            _ => Vec::new(),
        },
    }
}

/// Ctrl+letter and the few punctuation chords, as control bytes.
fn control_code(c: char) -> Option<u8> {
    match c.to_ascii_lowercase() {
        l @ 'a'..='z' => Some(l as u8 - b'a' + 1),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '7' | '/' => Some(0x1f),
        _ => None,
    }
}

/// Text pasted into the terminal, wrapped when the program asked for bracketed paste.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    // A paste must never carry the end marker itself, or it could escape the brackets.
    let clean = text.replace("\x1b[201~", "");
    if bracketed {
        format!("\x1b[200~{clean}\x1b[201~").into_bytes()
    } else {
        clean.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
    };

    #[test]
    fn plain_text_and_control_chords() {
        assert_eq!(encode(Key::Char('a'), NONE, false), b"a");
        assert_eq!(encode(Key::Char('é'), NONE, false), "é".as_bytes());
        let ctrl = Mods { ctrl: true, ..NONE };
        assert_eq!(encode(Key::Char('c'), ctrl, false), vec![3]);
        assert_eq!(encode(Key::Char('d'), ctrl, false), vec![4]);
        let alt = Mods { alt: true, ..NONE };
        assert_eq!(encode(Key::Char('b'), alt, false), b"\x1bb");
    }

    #[test]
    fn arrows_follow_the_cursor_mode_and_modifiers() {
        assert_eq!(encode(Key::Up, NONE, false), b"\x1b[A");
        assert_eq!(encode(Key::Up, NONE, true), b"\x1bOA");
        let ctrl = Mods { ctrl: true, ..NONE };
        assert_eq!(encode(Key::Right, ctrl, false), b"\x1b[1;5C");
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(encode(Key::Enter, NONE, false), b"\r");
        assert_eq!(encode(Key::Backspace, NONE, false), vec![0x7f]);
        assert_eq!(encode(Key::Delete, NONE, false), b"\x1b[3~");
        assert_eq!(encode(Key::F(1), NONE, false), b"\x1bOP");
        assert_eq!(encode(Key::F(5), NONE, false), b"\x1b[15~");
        assert!(encode(Key::F(40), NONE, false).is_empty());
        let shift = Mods {
            shift: true,
            ..NONE
        };
        assert_eq!(encode(Key::Tab, shift, false), b"\x1b[Z");
    }

    #[test]
    fn pasting_cannot_close_its_own_brackets() {
        assert_eq!(paste("a\nb", false), b"a\rb");
        assert_eq!(paste("x", true), b"\x1b[200~x\x1b[201~");
        assert_eq!(paste("a\x1b[201~b", true), b"\x1b[200~ab\x1b[201~");
    }
}
