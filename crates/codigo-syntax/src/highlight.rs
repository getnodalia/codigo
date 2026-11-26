// SPDX-License-Identifier: MIT OR Apache-2.0
//! A syntax tree that follows a buffer as it is edited, and the coloured spans it yields.

use std::ops::Range;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use codigo_text::Change;
use ropey::Rope;
use streaming_iterator::StreamingIterator;
use tree_sitter::{InputEdit, Parser, Point, QueryCursor, Tree};

use crate::language::{Grammar, LanguageDef};
use crate::scope::Scope;

/// Lines longer than this are not highlighted: a minified bundle would otherwise cost more to
/// colour than to read.
pub const MAX_HIGHLIGHT_LINE_BYTES: usize = 20_000;

/// Texts at least this long are parsed on a worker thread, so that opening or editing a big
/// file never holds up the window. Smaller ones take a few milliseconds and are parsed in place.
pub const BACKGROUND_PARSE_BYTES: usize = 48 * 1024;

/// Texts longer than this are not parsed at all.
pub const MAX_PARSE_BYTES: usize = 24 * 1024 * 1024;

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
    /// `None` while a worker thread holds it.
    parser: Option<Parser>,
    tree: Option<Tree>,
    cursor: QueryCursor,
    /// Edits applied to `tree` since it was last parsed against the text.
    stale: bool,
    /// A parse running on a worker thread.
    job: Option<Receiver<(Parser, Option<Tree>)>>,
    /// Edits made while that parse was running; the tree it returns knows nothing of them.
    missed: Vec<InputEdit>,
    /// The running parse is of a text that has since been replaced.
    discard: bool,
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
            parser: Some(parser),
            tree: None,
            cursor: QueryCursor::new(),
            stale: true,
            job: None,
            missed: Vec::new(),
            discard: false,
        })
    }

    pub fn language(&self) -> &'static LanguageDef {
        self.def
    }

    /// Tell the tree what the text did. Call once per change, in order, then [`reparse`].
    ///
    /// [`reparse`]: Syntax::reparse
    pub fn edit(&mut self, changes: &[Change]) {
        let edits: Vec<InputEdit> = changes
            .iter()
            .map(|c| InputEdit {
                start_byte: c.start_byte,
                old_end_byte: c.old_end_byte,
                new_end_byte: c.new_end_byte,
                start_position: point(c.start_point),
                old_end_position: point(c.old_end_point),
                new_end_position: point(c.new_end_point),
            })
            .collect();
        if self.job.is_some() {
            self.missed.extend(edits.iter().copied());
        }
        if let Some(tree) = self.tree.as_mut() {
            for e in &edits {
                tree.edit(e);
            }
        }
        self.stale = true;
    }

    /// Parse `rope` now, reusing what the tree still knows. Waits for a parse that is already
    /// running on a worker thread.
    pub fn reparse(&mut self, rope: &Rope) {
        if let Some(job) = self.job.take() {
            if let Ok((parser, tree)) = job.recv() {
                self.parser = Some(parser);
                self.tree = tree;
            }
            self.missed.clear();
            self.discard = false;
        }
        let Some(parser) = self.parser.as_mut() else {
            return;
        };
        self.tree = parse(parser, rope, self.tree.as_ref());
        self.stale = false;
    }

    /// Bring the tree up to date without ever waiting: short texts are parsed in place, long
    /// ones on a worker thread whose result is picked up by a later call. Returns true while
    /// the tree is behind the text, so the caller knows to ask again soon.
    pub fn refresh(&mut self, rope: &Rope) -> bool {
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok((parser, mut tree)) => {
                    self.job = None;
                    self.parser = Some(parser);
                    if std::mem::take(&mut self.discard) {
                        // The text was replaced while this parse ran: it describes the old one.
                        self.missed.clear();
                        self.tree = None;
                        self.stale = true;
                    } else {
                        if let Some(t) = tree.as_mut() {
                            for e in self.missed.drain(..) {
                                t.edit(&e);
                            }
                        }
                        self.missed.clear();
                        self.tree = tree;
                        // Whatever was typed meanwhile (`stale` stays set by `edit`) is parsed next.
                    }
                }
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => {
                    self.job = None;
                    self.missed.clear();
                    self.stale = false;
                    return false;
                }
            }
        }
        if !self.stale {
            return false;
        }
        if rope.len_bytes() > MAX_PARSE_BYTES {
            self.tree = None;
            self.stale = false;
            return false;
        }
        if rope.len_bytes() < BACKGROUND_PARSE_BYTES {
            self.reparse(rope);
            return false;
        }
        let Some(mut parser) = self.parser.take() else {
            return false;
        };
        let old = self.tree.clone();
        let rope = rope.clone();
        let (tx, rx) = mpsc::channel();
        self.missed.clear();
        let spawned = std::thread::Builder::new()
            .name("codigo-parse".into())
            .spawn(move || {
                let tree = parse(&mut parser, &rope, old.as_ref());
                let _ = tx.send((parser, tree));
            });
        match spawned {
            Ok(_) => {
                self.job = Some(rx);
                // Edits made from now on are replayed on the result.
                self.stale = false;
                true
            }
            Err(_) => {
                self.stale = false;
                false
            }
        }
    }

    /// True while a worker thread is parsing.
    pub fn is_parsing(&self) -> bool {
        self.job.is_some()
    }

    /// Drop the tree, for a text that was replaced wholesale.
    pub fn reset(&mut self) {
        self.tree = None;
        self.stale = true;
        self.discard = self.job.is_some();
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
        let byte_of = |line: usize| {
            if line >= rope.len_lines() {
                rope.len_bytes()
            } else {
                rope.line_to_byte(line)
            }
        };

        // Collect every capture overlapping the lines, then paint them in pattern order so that
        // a pattern written later in the query wins, the way grammars' own queries assume.
        // Lines too long to colour are left out of the query altogether: asking for the
        // captures of a minified bundle costs more than everything else on screen.
        let mut found: Vec<(usize, usize, usize, Scope)> = Vec::new();
        let mut at = start_line;
        while at < end_line {
            let width = |line: usize| byte_of(line + 1).saturating_sub(byte_of(line));
            if width(at) > MAX_HIGHLIGHT_LINE_BYTES {
                at += 1;
                continue;
            }
            let mut to = at + 1;
            while to < end_line && width(to) <= MAX_HIGHLIGHT_LINE_BYTES {
                to += 1;
            }
            let (lo, hi) = (byte_of(at), byte_of(to));
            self.cursor.set_byte_range(lo..hi);
            let mut captures =
                self.cursor
                    .captures(&self.grammar.query, tree.root_node(), RopeText { rope });
            while let Some((m, index)) = captures.next() {
                let Some(capture) = m.captures().get(*index) else {
                    continue;
                };
                let Some(Some(scope)) = self.grammar.scopes.get(capture.index as usize) else {
                    continue;
                };
                let node = capture.node;
                if node.end_byte() <= lo || node.start_byte() >= hi {
                    continue;
                }
                found.push((m.pattern_index, node.start_byte(), node.end_byte(), *scope));
            }
            at = to;
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

fn parse(parser: &mut Parser, rope: &Rope, old: Option<&Tree>) -> Option<Tree> {
    let len = rope.len_bytes();
    let mut read = |byte: usize, _: Point| -> &[u8] {
        if byte >= len {
            return &[];
        }
        let (chunk, start, _, _) = rope.chunk_at_byte(byte);
        chunk.as_bytes().get(byte - start..).unwrap_or(&[])
    };
    parser.parse_with_options(&mut read, old, None)
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

    fn big_rust(lines: usize) -> String {
        (0..lines)
            .map(|i| format!("pub fn f{i}(x: u32) -> u32 {{ x + {i} }}\n"))
            .collect()
    }

    fn settle(s: &mut Syntax, rope: &Rope) {
        for _ in 0..2000 {
            if !s.refresh(rope) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the parse never finished");
    }

    #[test]
    fn a_big_text_is_parsed_on_a_worker_without_waiting() {
        let text = big_rust(6000);
        assert!(text.len() > BACKGROUND_PARSE_BYTES);
        let mut s = Syntax::new(find("rust").expect("rust")).expect("grammar");
        let rope = Rope::from_str(&text);
        assert!(s.refresh(&rope), "the first call only starts the parse");
        assert!(s.is_parsing());
        assert!(s.tree().is_none());
        settle(&mut s, &rope);
        assert!(!s.is_parsing());
        let spans = s.spans(&rope, 0..2);
        assert!(spans[0].iter().any(|sp| sp.scope == Scope::Keyword));
    }

    #[test]
    fn edits_made_while_a_parse_runs_are_not_lost() {
        let text = big_rust(6000);
        let mut buffer = Buffer::from_text(&text);
        let mut s = Syntax::new(find("rust").expect("rust")).expect("grammar");
        assert!(s.refresh(buffer.rope()));
        // Type at the top while the worker is busy.
        buffer.set_cursor(0);
        let changes = buffer.insert_text("// typed\n");
        s.edit(&changes);
        settle(&mut s, buffer.rope());
        let rope = buffer.rope().clone();
        let spans = s.spans(&rope, 0..2);
        assert!(
            spans[0].iter().any(|sp| sp.scope == Scope::Comment),
            "the line typed meanwhile is a comment: {spans:?}"
        );
        assert!(spans[1].iter().any(|sp| sp.scope == Scope::Keyword));
    }

    #[test]
    fn a_minified_line_costs_nothing_and_does_not_hide_the_others() {
        let mut text = String::from("fn a() {}\n");
        text.push_str(&"let x = 1; ".repeat(10_000));
        text.push_str("\nfn b() {}\n");
        let (mut s, rope) = syntax("rust", &text);
        let spans = s.spans(&rope, 0..3);
        assert!(spans[1].is_empty());
        assert!(spans[0].iter().any(|sp| sp.scope == Scope::Keyword));
        assert!(spans[2].iter().any(|sp| sp.scope == Scope::Keyword));
    }
}
