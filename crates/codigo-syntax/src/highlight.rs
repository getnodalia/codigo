// SPDX-License-Identifier: MIT OR Apache-2.0
//! A syntax tree that follows a buffer as it is edited, and the coloured spans it yields.

use std::ops::Range;

use codigo_text::Change;
use ropey::Rope;
use streaming_iterator::StreamingIterator;
use tree_sitter::{InputEdit, Parser, Point, QueryCursor, Tree};

use crate::language::{Grammar, LanguageDef};
use crate::scope::Scope;

/// Lines longer than this are not highlighted: a minified bundle would otherwise cost more to
/// colour than to read.
pub const MAX_HIGHLIGHT_LINE_BYTES: usize = 20_000;

/// A coloured run inside one line. `start` and `end` are byte offsets from the line's first byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub scope: Scope,
}

/// The parse of one document in one language.
pub struct Syntax {
    def: &'static LanguageDef,
    grammar: &'static Grammar,
    parser: Parser,
    tree: Option<Tree>,
    cursor: QueryCursor,
    /// Edits applied to `tree` since it was last parsed against the text.
    stale: bool,
}

impl Syntax {
    /// A parser for `def`, or `None` when it has no grammar.
    pub fn new(def: &'static LanguageDef) -> Option<Self> {
        let grammar = def.grammar()?;
        let mut parser = Parser::new();
        parser.set_language(&grammar.language).ok()?;
        Some(Self {
            def,
            grammar,
            parser,
            tree: None,
            cursor: QueryCursor::new(),
            stale: true,
        })
    }

    pub fn language(&self) -> &'static LanguageDef {
        self.def
    }

    /// Tell the tree what the text did. Call once per change, in order, then [`reparse`].
    ///
    /// [`reparse`]: Syntax::reparse
    pub fn edit(&mut self, changes: &[Change]) {
        let Some(tree) = self.tree.as_mut() else {
            return;
        };
        for c in changes {
            tree.edit(&InputEdit {
                start_byte: c.start_byte,
                old_end_byte: c.old_end_byte,
                new_end_byte: c.new_end_byte,
                start_position: point(c.start_point),
                old_end_position: point(c.old_end_point),
                new_end_position: point(c.new_end_point),
            });
        }
        self.stale = true;
    }

    /// Parse `rope`, reusing what the tree still knows.
    pub fn reparse(&mut self, rope: &Rope) {
        let len = rope.len_bytes();
        let mut read = |byte: usize, _: Point| -> &[u8] {
            if byte >= len {
                return &[];
            }
            let (chunk, start, _, _) = rope.chunk_at_byte(byte);
            chunk.as_bytes().get(byte - start..).unwrap_or(&[])
        };
        self.tree = self
            .parser
            .parse_with_options(&mut read, self.tree.as_ref(), None);
        self.stale = false;
    }

    /// Drop the tree, for a text that was replaced wholesale.
    pub fn reset(&mut self) {
        self.tree = None;
        self.stale = true;
    }

    /// True when the text changed since the last [`reparse`](Syntax::reparse).
    pub fn is_stale(&self) -> bool {
        self.stale
    }

    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref()
    }

    /// Whether the parse contains syntax errors, for a status-bar hint.
    pub fn has_errors(&self) -> bool {
        self.tree
            .as_ref()
            .is_some_and(|t| t.root_node().has_error())
    }

    /// The coloured spans of each line in `lines`, one `Vec` per line, sorted and without
    /// overlaps. Returns empty lines until the first parse.
    pub fn spans(&mut self, rope: &Rope, lines: Range<usize>) -> Vec<Vec<Span>> {
        let end_line = lines.end.min(rope.len_lines());
        let start_line = lines.start.min(end_line);
        let mut out = vec![Vec::new(); end_line - start_line];
        let Some(tree) = self.tree.as_ref() else {
            return out;
        };
        if out.is_empty() {
            return out;
        }
        let first_byte = rope.line_to_byte(start_line);
        let last_byte = if end_line >= rope.len_lines() {
            rope.len_bytes()
        } else {
            rope.line_to_byte(end_line)
        };

        // Collect every capture overlapping the range, then paint them in pattern order so that
        // a pattern written later in the query wins, the way grammars' own queries assume.
        let mut found: Vec<(usize, usize, usize, Scope)> = Vec::new();
        self.cursor.set_byte_range(first_byte..last_byte);
        let source = RopeText { rope };
        let mut captures = self
            .cursor
            .captures(&self.grammar.query, tree.root_node(), source);
        while let Some((m, index)) = captures.next() {
            let Some(capture) = m.captures().get(*index) else {
                continue;
            };
            let Some(Some(scope)) = self.grammar.scopes.get(capture.index as usize) else {
                continue;
            };
            let node = capture.node;
            if node.end_byte() <= first_byte || node.start_byte() >= last_byte {
                continue;
            }
            found.push((m.pattern_index, node.start_byte(), node.end_byte(), *scope));
        }
        found.sort_by_key(|f| f.0);

        for (i, line_spans) in out.iter_mut().enumerate() {
            let line = start_line + i;
            let line_start = rope.line_to_byte(line);
            let line_end = if line + 1 >= rope.len_lines() {
                rope.len_bytes()
            } else {
                rope.line_to_byte(line + 1)
            };
            // The line break itself is never coloured.
            let text_end = line_end - trailing_break(rope, line_end);
            let width = text_end.saturating_sub(line_start);
            if width == 0 || width > MAX_HIGHLIGHT_LINE_BYTES {
                continue;
            }
            let mut paint: Vec<Option<Scope>> = vec![None; width];
            let mut any = false;
            for &(_, s, e, scope) in &found {
                let s = s.max(line_start);
                let e = e.min(text_end);
                if s >= e {
                    continue;
                }
                any = true;
                for cell in &mut paint[s - line_start..e - line_start] {
                    *cell = Some(scope);
                }
            }
            if any {
                *line_spans = runs(&paint);
            }
        }
        out
    }

    /// The kind of the smallest named node covering a byte range, for tests and tools.
    pub fn node_kind_at(&self, bytes: Range<usize>) -> Option<String> {
        let tree = self.tree.as_ref()?;
        tree.root_node()
            .named_descendant_for_byte_range(bytes.start, bytes.end)
            .map(|n| n.kind().to_string())
    }
}

fn point((row, column): (usize, usize)) -> Point {
    Point { row, column }
}

/// How many bytes of line break end the line ending at `line_end`.
fn trailing_break(rope: &Rope, line_end: usize) -> usize {
    if line_end == 0 {
        return 0;
    }
    let c = rope.char_at_byte_checked(line_end - 1);
    usize::from(c == Some('\n'))
}

/// Merge equal neighbouring cells into spans.
fn runs(paint: &[Option<Scope>]) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut i = 0;
    while i < paint.len() {
        let Some(scope) = paint[i] else {
            i += 1;
            continue;
        };
        let start = i;
        while i < paint.len() && paint[i] == Some(scope) {
            i += 1;
        }
        out.push(Span {
            start,
            end: i,
            scope,
        });
    }
    out
}

/// Lets a query cursor read node text out of a rope, for the predicates (`#eq?`, `#match?`).
struct RopeText<'a> {
    rope: &'a Rope,
}

impl<'a> tree_sitter::TextProvider<&'a [u8]> for RopeText<'a> {
    type I = RopeChunks<'a>;

    fn text(&mut self, node: tree_sitter::Node) -> Self::I {
        let len = self.rope.len_bytes();
        let start = node.start_byte().min(len);
        let end = node.end_byte().min(len);
        RopeChunks {
            rope: self.rope,
            at: start,
            end,
        }
    }
}

struct RopeChunks<'a> {
    rope: &'a Rope,
    at: usize,
    end: usize,
}

impl<'a> Iterator for RopeChunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.at >= self.end {
            return None;
        }
        let (chunk, start, _, _) = self.rope.chunk_at_byte(self.at);
        let from = self.at - start;
        let bytes = chunk.as_bytes();
        let to = bytes.len().min(self.end - start);
        let piece = bytes.get(from..to)?;
        self.at += piece.len().max(1);
        Some(piece)
    }
}

trait ByteChar {
    fn char_at_byte_checked(&self, byte: usize) -> Option<char>;
}

impl ByteChar for Rope {
    fn char_at_byte_checked(&self, byte: usize) -> Option<char> {
        if byte >= self.len_bytes() {
            return None;
        }
        let (chunk, start, _, _) = self.chunk_at_byte(byte);
        chunk.as_bytes().get(byte - start).map(|b| char::from(*b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::{detect, find};
    use codigo_text::{Buffer, Edit};

    fn syntax(id: &str, text: &str) -> (Syntax, Rope) {
        let def = find(id).expect("language");
        let mut s = Syntax::new(def).expect("grammar");
        let rope = Rope::from_str(text);
        s.reparse(&rope);
        (s, rope)
    }

    fn scopes_of(spans: &[Span], line: &str) -> Vec<(String, Scope)> {
        spans
            .iter()
            .map(|s| (line[s.start..s.end].to_string(), s.scope))
            .collect()
    }

    #[test]
    fn rust_keywords_strings_and_comments_are_found() {
        let text = "fn main() {\n    let s = \"hi\"; // note\n}\n";
        let (mut s, rope) = syntax("rust", text);
        let spans = s.spans(&rope, 0..3);
        let l0 = scopes_of(&spans[0], "fn main() {");
        assert!(l0.iter().any(|(t, sc)| t == "fn" && *sc == Scope::Keyword));
        let l1 = scopes_of(&spans[1], "    let s = \"hi\"; // note");
        assert!(l1.iter().any(|(t, sc)| t == "let" && *sc == Scope::Keyword));
        assert!(l1
            .iter()
            .any(|(t, sc)| t == "\"hi\"" && *sc == Scope::String));
        assert!(l1
            .iter()
            .any(|(t, sc)| t == "// note" && *sc == Scope::Comment));
        assert!(spans[2].iter().all(|sp| sp.end <= 1));
    }

    #[test]
    fn spans_are_sorted_and_never_overlap() {
        let text = "fn f<'a>(x: &'a str) -> Option<&'a str> { Some(x) }\n";
        let (mut s, rope) = syntax("rust", text);
        let spans = &s.spans(&rope, 0..1)[0];
        assert!(!spans.is_empty());
        for w in spans.windows(2) {
            assert!(w[0].end <= w[1].start);
        }
        for sp in spans {
            assert!(sp.start < sp.end && sp.end < text.len());
        }
    }

    #[test]
    fn a_block_comment_is_clipped_to_every_line_it_covers() {
        let text = "/* a\n b\n c */ fn x() {}\n";
        let (mut s, rope) = syntax("rust", text);
        let spans = s.spans(&rope, 0..3);
        assert_eq!(spans[0][0].scope, Scope::Comment);
        assert_eq!((spans[0][0].start, spans[0][0].end), (0, 4));
        assert_eq!(spans[1][0].scope, Scope::Comment);
        assert_eq!((spans[1][0].start, spans[1][0].end), (0, 2));
        assert_eq!((spans[2][0].start, spans[2][0].end), (0, 5));
    }

    #[test]
    fn edits_reparse_incrementally_and_match_a_fresh_parse() {
        let mut buf = Buffer::from_text("fn a() {}\nfn b() {}\n");
        let def = find("rust").expect("rust");
        let mut s = Syntax::new(def).expect("grammar");
        s.reparse(buf.rope());
        assert!(!s.is_stale());

        buf.set_cursor(0);
        let changes = buf.insert_text("// hello\n");
        s.edit(&changes);
        assert!(s.is_stale());
        s.reparse(buf.rope());

        let mut fresh = Syntax::new(def).expect("grammar");
        fresh.reparse(buf.rope());
        let n = buf.line_count();
        assert_eq!(s.spans(buf.rope(), 0..n), fresh.spans(buf.rope(), 0..n));
        assert_eq!(
            s.tree().map(|t| t.root_node().to_sexp()),
            fresh.tree().map(|t| t.root_node().to_sexp())
        );
    }

    #[test]
    fn multibyte_text_keeps_byte_columns_consistent() {
        let text = "let s = \"añá😀\"; // ñ\n";
        let (mut s, rope) = syntax(
            "rust",
            &text
                .replace("let s", "fn f() { let s")
                .replace("// ñ", "// ñ }"),
        );
        let line = rope.line(0).to_string();
        let spans = &s.spans(&rope, 0..1)[0];
        let string = spans
            .iter()
            .find(|sp| sp.scope == Scope::String)
            .expect("string");
        assert_eq!(&line[string.start..string.end], "\"añá😀\"");
    }

    #[test]
    fn highlights_other_languages() {
        for (id, text) in [
            ("python", "def f(x):\n    return 'a'  # c\n"),
            ("javascript", "const a = () => `x`; // c\n"),
            (
                "typescript",
                "interface A { b: string }\nconst x: number = 1;\n",
            ),
            ("json", "{\"a\": [1, true, null]}\n"),
            ("toml", "[package]\nname = \"x\"\n"),
            ("yaml", "a: 1\nb: [x, y]\n"),
            ("markdown", "# Title\n\ntext\n"),
            ("bash", "if [ -f x ]; then echo \"$a\"; fi\n"),
            ("c", "int main(void) { return 0; }\n"),
            ("cpp", "class A { public: int x; };\n"),
            ("go", "package main\nfunc main() {}\n"),
            ("java", "class A { void f() {} }\n"),
            ("html", "<div class=\"a\">x</div>\n"),
            ("css", "a { color: red; }\n"),
        ] {
            let (mut s, rope) = syntax(id, text);
            let all = s.spans(&rope, 0..rope.len_lines());
            assert!(all.iter().any(|l| !l.is_empty()), "{id} coloured nothing");
        }
    }

    #[test]
    fn an_unparsed_or_out_of_range_request_is_empty_not_a_panic() {
        let def = detect("a.rs");
        let mut s = Syntax::new(def).expect("grammar");
        let rope = Rope::from_str("fn a() {}");
        assert_eq!(s.spans(&rope, 0..5), vec![Vec::new(); 1]);
        s.reparse(&rope);
        assert_eq!(s.spans(&rope, 9..9), Vec::<Vec<Span>>::new());
        let reversed = Range { start: 3, end: 1 };
        assert_eq!(s.spans(&rope, reversed).len(), 0);
        let empty = Rope::new();
        s.reparse(&empty);
        assert_eq!(s.spans(&empty, 0..1).len(), 1);
    }

    #[test]
    fn grammarless_languages_have_no_syntax() {
        assert!(Syntax::new(detect("a.txt")).is_none());
        assert!(Syntax::new(detect("a.sql")).is_none());
    }

    #[test]
    fn crlf_free_text_with_edits_in_the_middle_of_a_token() {
        let mut buf = Buffer::from_text("fn main() { let x = 1; }\n");
        let def = find("rust").expect("rust");
        let mut s = Syntax::new(def).expect("grammar");
        s.reparse(buf.rope());
        let changes = buf
            .apply_edits(vec![Edit::new(3..7, "other")])
            .unwrap_or_default();
        s.edit(&changes);
        s.reparse(buf.rope());
        assert_eq!(s.node_kind_at(3..8), Some("identifier".to_string()));
    }
}
