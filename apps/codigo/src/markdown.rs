// SPDX-License-Identifier: MIT OR Apache-2.0
//! GitHub-flavoured Markdown, drawn with the Nodalia palette: the descriptions and comments of
//! pull requests, merge requests and issues.
//!
//! The text comes from other people, so it is hostile. It is parsed into a small tree of blocks
//! (`parse`), and only the tree is drawn (`show`): headings, paragraphs with bold, italic,
//! strikethrough, code and links, lists with task boxes, quotes, code blocks, rules and tables.
//! HTML is never run: comments are dropped, `<br>`, `<a>`, `<img>`, `<b>`, `<i>` and `<code>` are
//! understood the way GitHub understands them, and every other tag is removed with its text kept.
//! Images are not downloaded; one is drawn as a link named after its alt text. Only `http`,
//! `https` and `mailto` links can be opened.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::iter::Peekable;
use std::sync::Arc;

use egui::{vec2, Id, RichText, Sense, Stroke, Ui};
use nodalia_theme::creator::Look;
use nodalia_theme::font_size;
use nodalia_theme::for_egui::color;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// The most source text that is parsed; the rest is cut at a character boundary.
const MAX_SOURCE: usize = 100_000;
/// Quotes and lists nested deeper than this are flattened instead of nested.
const MAX_DEPTH: usize = 12;
/// A tag that runs over several lines is given up on past this many bytes.
const MAX_TAG: usize = 2_000;

/// A run of text that shares one style.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    /// Where a click goes. Always an `http`, `https` or `mailto` address.
    pub link: Option<String>,
}

/// A list entry: its blocks, and whether it is a task and done.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading(u8, Vec<Span>),
    Para(Vec<Span>),
    Code(String),
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Rule,
    Table {
        head: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
}

/// Which end tag closes the blocks being read.
#[derive(Clone, Copy, PartialEq)]
enum Stop {
    Root,
    Quote,
    Item,
}

#[derive(Default)]
struct Style {
    bold: u32,
    italic: u32,
    strike: u32,
    code: u32,
    links: Vec<Option<String>>,
}

struct Reader<'a> {
    events: Peekable<Parser<'a>>,
    style: Style,
    in_comment: bool,
    pending_tag: String,
    task: Option<bool>,
    depth: usize,
}

/// Read Markdown into blocks.
pub fn parse(source: &str) -> Vec<Block> {
    let mut end = source.len().min(MAX_SOURCE);
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM;
    let mut reader = Reader {
        events: Parser::new_ext(&source[..end], options).peekable(),
        style: Style::default(),
        in_comment: false,
        pending_tag: String::new(),
        task: None,
        depth: 0,
    };
    reader.blocks(Stop::Root)
}

/// `http`, `https` and `mailto` only: a link from a stranger never opens anything else.
pub fn safe_url(url: &str) -> Option<String> {
    let url = url.trim();
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    let ok = lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:");
    (ok && url.len() <= 2_000 && !url.chars().any(char::is_control)).then(|| url.to_owned())
}

/// The value of an attribute inside the text of a tag.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower.get(from..)?.find(name) {
        let start = from + at;
        let before_ok = start == 0
            || lower
                .as_bytes()
                .get(start - 1)
                .is_some_and(|b| b.is_ascii_whitespace());
        let after = start + name.len();
        let rest = tag.get(after..)?.trim_start();
        if before_ok && rest.starts_with('=') {
            let value = rest.get(1..)?.trim_start();
            let mut chars = value.chars();
            return match chars.next() {
                Some(q @ ('"' | '\'')) => {
                    let inner = value.get(1..)?;
                    inner.find(q).and_then(|e| inner.get(..e)).map(decode)
                }
                Some(_) => Some(decode(
                    value
                        .split(|c: char| c.is_whitespace() || c == '>')
                        .next()?,
                )),
                None => None,
            };
        }
        from = after;
    }
    None
}

/// The few character references that turn up in raw HTML.
fn decode(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn ends_with_break(spans: &[Span]) -> bool {
    spans.last().is_none_or(|s| s.text.ends_with('\n'))
}

impl Reader<'_> {
    fn span(&self, text: &str) -> Span {
        Span {
            text: text.to_owned(),
            bold: self.style.bold > 0,
            italic: self.style.italic > 0,
            strike: self.style.strike > 0,
            code: self.style.code > 0,
            link: self.style.links.iter().rev().find_map(Clone::clone),
        }
    }

    fn newline(&self, out: &mut Vec<Span>) {
        if !ends_with_break(out) {
            out.push(Span {
                text: "\n".into(),
                ..Span::default()
            });
        }
    }

    /// Read blocks until the end tag that closes `stop` (or the end of the text).
    fn blocks(&mut self, stop: Stop) -> Vec<Block> {
        let mut out = Vec::new();
        while let Some(event) = self.events.peek() {
            match event {
                Event::End(end) => {
                    let closes = matches!(
                        (stop, end),
                        (Stop::Quote, TagEnd::BlockQuote(_)) | (Stop::Item, TagEnd::Item)
                    );
                    self.events.next();
                    if closes {
                        break;
                    }
                }
                Event::Start(Tag::Heading { .. }) => {
                    let Some(Event::Start(Tag::Heading { level, .. })) = self.events.next() else {
                        continue;
                    };
                    let spans = self.inline(Some(Closer::Heading));
                    out.push(Block::Heading(heading_level(level), spans));
                }
                Event::Start(Tag::Paragraph | Tag::HtmlBlock) => {
                    self.events.next();
                    let spans = self.inline(Some(Closer::Block));
                    push_para(&mut out, spans);
                }
                Event::Start(Tag::BlockQuote(_)) => {
                    self.events.next();
                    if self.depth >= MAX_DEPTH {
                        continue;
                    }
                    self.depth += 1;
                    let inner = self.blocks(Stop::Quote);
                    self.depth -= 1;
                    out.push(Block::Quote(inner));
                }
                Event::Start(Tag::CodeBlock(_)) => {
                    self.events.next();
                    let mut code = String::new();
                    for event in self.events.by_ref() {
                        match event {
                            Event::End(TagEnd::CodeBlock) => break,
                            Event::Text(t) => code.push_str(&t),
                            _ => {}
                        }
                    }
                    out.push(Block::Code(code.trim_end_matches('\n').to_owned()));
                }
                Event::Start(Tag::List(_)) => {
                    let Some(Event::Start(Tag::List(start))) = self.events.next() else {
                        continue;
                    };
                    out.push(self.list(start));
                }
                Event::Start(Tag::Table(_)) => {
                    self.events.next();
                    out.push(self.table());
                }
                Event::Rule => {
                    self.events.next();
                    out.push(Block::Rule);
                }
                Event::Start(Tag::Item) => {
                    // An item outside a list: read it as plain blocks.
                    self.events.next();
                }
                Event::Start(
                    Tag::FootnoteDefinition(_)
                    | Tag::MetadataBlock(_)
                    | Tag::DefinitionList
                    | Tag::DefinitionListTitle
                    | Tag::DefinitionListDefinition,
                ) => {
                    // A tag this view does not draw (footnotes, metadata): read what is inside.
                    self.events.next();
                }
                _ => {
                    let spans = self.inline(None);
                    push_para(&mut out, spans);
                }
            }
        }
        out
    }

    fn list(&mut self, start: Option<u64>) -> Block {
        let mut items = Vec::new();
        while let Some(event) = self.events.next() {
            match event {
                Event::End(TagEnd::List(_)) => break,
                Event::Start(Tag::Item) => {
                    self.task = None;
                    let blocks = if self.depth >= MAX_DEPTH {
                        // Too deep to nest: keep the text, drop the structure.
                        let mut spans = self.inline(Some(Closer::Item));
                        let mut blocks = Vec::new();
                        push_para(&mut blocks, std::mem::take(&mut spans));
                        blocks
                    } else {
                        self.depth += 1;
                        let blocks = self.blocks(Stop::Item);
                        self.depth -= 1;
                        blocks
                    };
                    items.push(Item {
                        task: self.task.take(),
                        blocks,
                    });
                }
                _ => {}
            }
        }
        Block::List { start, items }
    }

    fn table(&mut self) -> Block {
        let mut head = Vec::new();
        let mut rows: Vec<Vec<Vec<Span>>> = Vec::new();
        let mut row: Vec<Vec<Span>> = Vec::new();
        while let Some(event) = self.events.next() {
            match event {
                Event::End(TagEnd::Table) => break,
                Event::End(TagEnd::TableHead) => {
                    head = std::mem::take(&mut row);
                }
                Event::Start(Tag::TableRow) => row = Vec::new(),
                Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
                Event::Start(Tag::TableCell) => {
                    let cell = self.inline(Some(Closer::Cell));
                    row.push(cell);
                }
                _ => {}
            }
        }
        Block::Table { head, rows }
    }

    /// Read a run of inline events. With `closer` it ends at that end tag (consumed); without,
    /// at the first event that belongs to a block.
    fn inline(&mut self, closer: Option<Closer>) -> Vec<Span> {
        let mut out: Vec<Span> = Vec::new();
        while let Some(event) = self.events.peek() {
            match event {
                Event::End(end) => {
                    let end = *end;
                    if let Some(kind) = inline_end(&end) {
                        self.events.next();
                        self.close(kind);
                    } else if closer.is_some_and(|c| c.matches(&end)) {
                        self.events.next();
                        break;
                    } else if closer.is_none() {
                        break;
                    } else {
                        // An end tag of something this view skips.
                        self.events.next();
                    }
                }
                Event::Start(tag) => {
                    let tag = tag.clone();
                    match tag {
                        Tag::Emphasis => {
                            self.events.next();
                            self.style.italic += 1;
                        }
                        Tag::Strong => {
                            self.events.next();
                            self.style.bold += 1;
                        }
                        Tag::Strikethrough => {
                            self.events.next();
                            self.style.strike += 1;
                        }
                        Tag::Link { dest_url, .. } => {
                            self.events.next();
                            self.style.links.push(safe_url(&dest_url));
                        }
                        Tag::Image { dest_url, .. } => {
                            self.events.next();
                            let mut alt = String::new();
                            for e in self.events.by_ref() {
                                match e {
                                    Event::End(TagEnd::Image) => break,
                                    Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                                    _ => {}
                                }
                            }
                            self.image(&mut out, &alt, &dest_url);
                        }
                        _ if closer.is_none() => break,
                        _ => {
                            self.events.next();
                        }
                    }
                }
                Event::Text(_)
                | Event::Code(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::SoftBreak
                | Event::HardBreak
                | Event::TaskListMarker(_)
                | Event::FootnoteReference(_)
                | Event::InlineMath(_)
                | Event::DisplayMath(_) => {
                    let Some(event) = self.events.next() else {
                        break;
                    };
                    self.leaf(event, &mut out);
                }
                Event::Rule => break,
            }
        }
        out
    }

    fn leaf(&mut self, event: Event<'_>, out: &mut Vec<Span>) {
        match event {
            Event::Text(t) => out.push(self.span(&t)),
            Event::Code(t) => {
                let mut span = self.span(&t);
                span.code = true;
                out.push(span);
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                let mut span = self.span(&t);
                span.code = true;
                out.push(span);
            }
            Event::FootnoteReference(t) => out.push(self.span(&format!("[{t}]"))),
            Event::SoftBreak | Event::HardBreak => self.newline(out),
            Event::TaskListMarker(done) => self.task = Some(done),
            Event::Html(h) | Event::InlineHtml(h) => self.html(&h, out),
            _ => {}
        }
    }

    fn close(&mut self, kind: InlineEnd) {
        match kind {
            InlineEnd::Emphasis => self.style.italic = self.style.italic.saturating_sub(1),
            InlineEnd::Strong => self.style.bold = self.style.bold.saturating_sub(1),
            InlineEnd::Strike => self.style.strike = self.style.strike.saturating_sub(1),
            InlineEnd::Link => {
                self.style.links.pop();
            }
        }
    }

    fn image(&mut self, out: &mut Vec<Span>, alt: &str, src: &str) {
        let name = if alt.trim().is_empty() {
            "image"
        } else {
            alt.trim()
        };
        let mut span = self.span(name);
        if let Some(url) = safe_url(src) {
            span.link = Some(url);
        }
        out.push(span);
    }

    /// Raw HTML: comments vanish, a few tags are understood, the rest is removed.
    fn html(&mut self, raw: &str, out: &mut Vec<Span>) {
        let joined;
        let mut rest = if self.pending_tag.is_empty() {
            raw
        } else {
            joined = format!("{}{raw}", std::mem::take(&mut self.pending_tag));
            joined.as_str()
        };
        loop {
            if self.in_comment {
                match rest.find("-->") {
                    Some(i) => {
                        self.in_comment = false;
                        rest = rest.get(i + 3..).unwrap_or("");
                    }
                    None => return,
                }
                continue;
            }
            let Some(lt) = rest.find('<') else {
                self.html_text(rest, out);
                return;
            };
            self.html_text(rest.get(..lt).unwrap_or(""), out);
            rest = rest.get(lt..).unwrap_or("");
            if rest.starts_with("<!--") {
                self.in_comment = true;
                rest = rest.get(4..).unwrap_or("");
                continue;
            }
            match rest.find('>') {
                Some(gt) => {
                    self.tag(rest.get(1..gt).unwrap_or(""), out);
                    rest = rest.get(gt + 1..).unwrap_or("");
                }
                None => {
                    if rest.len() <= MAX_TAG {
                        self.pending_tag = rest.to_owned();
                    }
                    return;
                }
            }
        }
    }

    fn html_text(&mut self, text: &str, out: &mut Vec<Span>) {
        let text = decode(text);
        let mut lines = text.split('\n').peekable();
        while let Some(line) = lines.next() {
            if !line.trim().is_empty() {
                out.push(self.span(line.trim_start_matches(['\r'])));
            }
            if lines.peek().is_some() && !line.trim().is_empty() {
                out.push(self.span(" "));
            }
        }
    }

    fn tag(&mut self, inner: &str, out: &mut Vec<Span>) {
        let closing = inner.starts_with('/');
        let body = inner.trim_start_matches('/');
        let name: String = body
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase();
        match name.as_str() {
            "br" | "p" | "div" | "tr" | "li" | "ul" | "ol" | "details" | "summary" | "hr"
            | "blockquote" | "pre" | "table" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.newline(out);
            }
            "img" => {
                let alt = attr(body, "alt").unwrap_or_default();
                let src = attr(body, "src").unwrap_or_default();
                self.image(out, &alt, &src);
            }
            "a" if closing => {
                self.style.links.pop();
            }
            "a" => {
                let href = attr(body, "href").and_then(|h| safe_url(&h));
                self.style.links.push(href);
            }
            "b" | "strong" => {
                if closing {
                    self.close(InlineEnd::Strong);
                } else {
                    self.style.bold += 1;
                }
            }
            "i" | "em" => {
                if closing {
                    self.close(InlineEnd::Emphasis);
                } else {
                    self.style.italic += 1;
                }
            }
            "del" | "s" | "strike" => {
                if closing {
                    self.close(InlineEnd::Strike);
                } else {
                    self.style.strike += 1;
                }
            }
            "code" | "kbd" => {
                if closing {
                    self.style.code = self.style.code.saturating_sub(1);
                } else {
                    self.style.code += 1;
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
enum Closer {
    Heading,
    Block,
    Cell,
    Item,
}

impl Closer {
    fn matches(self, end: &TagEnd) -> bool {
        match self {
            Closer::Heading => matches!(end, TagEnd::Heading(_)),
            Closer::Block => matches!(end, TagEnd::Paragraph | TagEnd::HtmlBlock),
            Closer::Cell => matches!(end, TagEnd::TableCell),
            Closer::Item => matches!(end, TagEnd::Item),
        }
    }
}

#[derive(Clone, Copy)]
enum InlineEnd {
    Emphasis,
    Strong,
    Strike,
    Link,
}

fn inline_end(end: &TagEnd) -> Option<InlineEnd> {
    match end {
        TagEnd::Emphasis => Some(InlineEnd::Emphasis),
        TagEnd::Strong => Some(InlineEnd::Strong),
        TagEnd::Strikethrough => Some(InlineEnd::Strike),
        TagEnd::Link => Some(InlineEnd::Link),
        _ => None,
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Add a paragraph unless it holds nothing but blanks.
fn push_para(out: &mut Vec<Block>, mut spans: Vec<Span>) {
    while spans
        .last()
        .is_some_and(|s| s.text.trim().is_empty() && s.link.is_none())
    {
        spans.pop();
    }
    while spans
        .first()
        .is_some_and(|s| s.text.trim().is_empty() && s.link.is_none())
    {
        spans.remove(0);
    }
    if !spans.is_empty() {
        out.push(Block::Para(spans));
    }
}

// --- Drawing ----------------------------------------------------------------------------------

struct Paint<'a> {
    look: &'a Look,
    tables: usize,
    base: Id,
}

/// Draw Markdown in `ui`. The parse is kept for as long as the text does not change.
pub fn show(ui: &mut Ui, look: &Look, source: &str) {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    let key = ui.scope_id().with(("codigo-markdown", hasher.finish()));
    let cached: Option<Arc<Vec<Block>>> = ui.data(|d| d.get_temp(key));
    let blocks = cached.unwrap_or_else(|| {
        let parsed = Arc::new(parse(source));
        ui.data_mut(|d| d.insert_temp(key, parsed.clone()));
        parsed
    });
    let mut paint = Paint {
        look,
        tables: 0,
        base: key,
    };
    paint.blocks(ui, &blocks, false);
}

impl Paint<'_> {
    fn blocks(&mut self, ui: &mut Ui, blocks: &[Block], quiet: bool) {
        let p = &self.look.palette;
        // Body text a little softer than bold text and headings, so that bold shows.
        let ink = if quiet {
            color(p.text_secondary)
        } else {
            color(p.text_primary).lerp_to_gamma(color(p.text_secondary), 0.3)
        };
        let last = blocks.len().saturating_sub(1);
        for (i, block) in blocks.iter().enumerate() {
            match block {
                Block::Heading(level, spans) => {
                    let size = match level {
                        1 => font_size::TITLE + 2.0,
                        2 => font_size::TITLE - 1.0,
                        3 => font_size::LARGE,
                        _ => font_size::BODY,
                    };
                    if i > 0 {
                        ui.add_space(4.0);
                    }
                    self.spans(ui, spans, ink, Some(size), true);
                }
                Block::Para(spans) => self.spans(ui, spans, ink, None, false),
                Block::Code(code) => {
                    egui::Frame::new()
                        .fill(color(p.surface_sunken))
                        .corner_radius(6)
                        .inner_margin(8)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(code)
                                        .monospace()
                                        .size(font_size::SMALL)
                                        .color(color(p.text_primary)),
                                )
                                .wrap(),
                            );
                        });
                }
                Block::Quote(inner) => {
                    ui.indent(("md-quote", self.base, i), |ui| {
                        self.blocks(ui, inner, true);
                    });
                }
                Block::List { start, items } => self.list(ui, *start, items),
                Block::Rule => {
                    ui.add_space(2.0);
                    ui.separator();
                }
                Block::Table { head, rows } => self.table(ui, head, rows, ink),
            }
            if i != last {
                ui.add_space(6.0);
            }
        }
    }

    fn list(&mut self, ui: &mut Ui, start: Option<u64>, items: &[Item]) {
        let p = &self.look.palette;
        for (n, item) in items.iter().enumerate() {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.spacing_mut().interact_size.y = font_size::BODY + 4.0;
                ui.add_space(4.0);
                match (item.task, start) {
                    (Some(done), _) => task_box(ui, self.look, done),
                    (None, Some(first)) => {
                        let number = first.saturating_add(n as u64);
                        ui.add_sized(
                            vec2(22.0, 18.0),
                            egui::Label::new(
                                RichText::new(format!("{number}.")).color(color(p.text_secondary)),
                            ),
                        );
                    }
                    (None, None) => {
                        let (rect, _) = ui.allocate_exact_size(vec2(10.0, 18.0), Sense::hover());
                        ui.painter()
                            .circle_filled(rect.center(), 2.0, color(p.text_secondary));
                    }
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    self.blocks(ui, &item.blocks, item.task == Some(true));
                });
            });
        }
    }

    fn table(
        &mut self,
        ui: &mut Ui,
        head: &[Vec<Span>],
        rows: &[Vec<Vec<Span>>],
        ink: egui::Color32,
    ) {
        self.tables += 1;
        egui::Grid::new(("md-table", self.base, self.tables))
            .spacing(vec2(14.0, 4.0))
            .min_row_height(font_size::BODY + 4.0)
            .show(ui, |ui| {
                for cell in head {
                    self.spans(ui, cell, ink, None, true);
                }
                ui.end_row();
                for row in rows {
                    for cell in row {
                        self.spans(ui, cell, ink, None, false);
                    }
                    ui.end_row();
                }
            });
    }

    /// Spans flowing and wrapping like one paragraph.
    fn spans(
        &self,
        ui: &mut Ui,
        spans: &[Span],
        ink: egui::Color32,
        size: Option<f32>,
        strong: bool,
    ) {
        let p = &self.look.palette;
        // NDL's controls are tall; a line of prose is not. Set before the row is laid out.
        ui.scope(|ui| {
            ui.spacing_mut().interact_size.y = font_size::BODY + 4.0;
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                for span in spans {
                    if span.text == "\n" {
                        ui.end_row();
                        continue;
                    }
                    let mut text = RichText::new(&span.text).color(ink);
                    if let Some(size) = size {
                        text = text.size(size);
                    }
                    if span.bold || strong {
                        text = text.color(color(p.text_primary)).strong();
                    }
                    if span.italic {
                        text = text.italics();
                    }
                    if span.strike {
                        text = text.strikethrough();
                    }
                    if span.code {
                        text = text.monospace().background_color(color(p.surface_sunken));
                        if size.is_none() {
                            text = text.size(font_size::SMALL + 0.5);
                        }
                    }
                    match &span.link {
                        Some(url) => {
                            let response = ui
                                .add(egui::Link::new(text.color(color(p.text_link))))
                                .on_hover_text(url);
                            if response.clicked() {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(url.clone()));
                            }
                        }
                        None => {
                            ui.add(egui::Label::new(text));
                        }
                    }
                }
            });
        });
    }
}

/// A small check box, drawn rather than typed so that it needs no particular font.
fn task_box(ui: &mut Ui, look: &Look, done: bool) {
    let p = &look.palette;
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 18.0), Sense::hover());
    let r = egui::Rect::from_center_size(rect.center(), vec2(12.0, 12.0));
    let painter = ui.painter();
    if done {
        painter.rect_filled(r, 3, color(p.accent_default));
        let ink = egui::Color32::WHITE;
        let a = r.left_top() + vec2(2.8, 6.2);
        let b = r.left_top() + vec2(5.0, 8.6);
        let c = r.left_top() + vec2(9.4, 3.6);
        painter.line_segment([a, b], Stroke::new(1.6, ink));
        painter.line_segment([b, c], Stroke::new(1.6, ink));
    } else {
        painter.rect_stroke(
            r,
            3,
            Stroke::new(1.3, color(p.text_tertiary)),
            egui::StrokeKind::Inside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodalia_theme::{Accent, Mode};

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    /// The pull request description that was drawn as raw text: headings, a list, code, a rule
    /// and a footer made of HTML comments, a picture and links.
    const PR: &str = "## What\n\nThe panel could not see the file. Now:\n\n- Each question carries the **open file** and `@name`\n- The model replies with blocks\n\n## Tested\n\n- [x] fmt\n- [ ] macOS\n\n---\n_Generated by [Claude Code](https://claude.ai/code)_\n\n<!-- codesmith:footer -->\n---\n<a href=\"https://example.com/p?a=1&amp;b=2\"><picture><source media=\"(prefers-color-scheme: dark)\" srcset=\"x.svg\"><img alt=\"Proof this PR\" src=\"https://example.com/p.png\" width=\"121\"></picture></a>\n<sup><a href=\"https://example.com/open\">Open in [code]smith</a> · Need help?</sup>\n\n<!-- codesmith:autofix:disabled -->\n<!-- /codesmith:footer -->";

    #[test]
    fn a_pull_request_description_becomes_blocks_without_markup() {
        let blocks = parse(PR);
        assert!(matches!(&blocks[0], Block::Heading(2, s) if text(s) == "What"));
        let Block::List { items, .. } = &blocks[2] else {
            panic!("expected a list, got {:?}", blocks[2]);
        };
        assert_eq!(items.len(), 2);
        let Block::Para(first) = &items[0].blocks[0] else {
            panic!("expected a paragraph");
        };
        assert!(first.iter().any(|s| s.bold && s.text == "open file"));
        assert!(first.iter().any(|s| s.code && s.text == "@name"));
        let all = format!("{blocks:?}");
        for raw in [
            "<!--", "-->", "<picture", "<source", "srcset", "&amp;", "<a href", "<sup",
        ] {
            assert!(!all.contains(raw), "{raw} leaked into {all}");
        }
        // The picture became a link named after its alt text, and the anchor kept its address.
        let spans: Vec<&Span> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Para(s) => Some(s),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(spans
            .iter()
            .any(|s| s.text == "Proof this PR"
                && s.link.as_deref() == Some("https://example.com/p.png")));
        let opened: String = spans
            .iter()
            .filter(|s| s.link.as_deref() == Some("https://example.com/open"))
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(opened, "Open in [code]smith");
        assert!(blocks.iter().any(|b| matches!(b, Block::Rule)));
    }

    #[test]
    fn task_lists_code_quotes_and_tables_are_read() {
        let blocks = parse(
            "- [x] done\n- [ ] todo\n\n```rust\nfn main() {}\n```\n\n> quoted\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n1. one\n2. two\n",
        );
        let Block::List { items, .. } = &blocks[0] else {
            panic!("list");
        };
        assert_eq!(items[0].task, Some(true));
        assert_eq!(items[1].task, Some(false));
        assert_eq!(blocks[1], Block::Code("fn main() {}".into()));
        assert!(matches!(&blocks[2], Block::Quote(_)));
        let Block::Table { head, rows } = &blocks[3] else {
            panic!("table");
        };
        assert_eq!(text(&head[1]), "b");
        assert_eq!(text(&rows[0][0]), "1");
        assert!(matches!(&blocks[4], Block::List { start: Some(1), items } if items.len() == 2));
    }

    #[test]
    fn only_web_and_mail_links_can_be_opened() {
        for bad in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,x",
            "ssh://host",
            "/relative",
            "",
        ] {
            assert_eq!(safe_url(bad), None, "{bad}");
        }
        assert!(safe_url("https://github.com/a/b").is_some());
        assert!(safe_url("HTTP://example.com").is_some());
        assert!(safe_url("mailto:a@b.c").is_some());
        let blocks = parse("[x](javascript:alert(1)) and <a href=\"file:///etc/passwd\">y</a>");
        let Block::Para(spans) = &blocks[0] else {
            panic!("paragraph");
        };
        assert!(spans.iter().all(|s| s.link.is_none()));
        assert_eq!(text(spans), "x and y");
    }

    #[test]
    fn a_comment_or_a_tag_split_over_lines_is_still_removed() {
        let blocks = parse("<!--\nhidden\n-->\n\nshown <b\n class=\"x\">bold</b>");
        let all = format!("{blocks:?}");
        assert!(!all.contains("hidden"), "{all}");
        assert!(all.contains("shown"), "{all}");
        assert!(!all.contains("class"), "{all}");
    }

    #[test]
    fn hostile_text_never_panics() {
        let mut nasty = vec![
            "<".to_string(),
            "<!--".to_string(),
            "<a href=".to_string(),
            "<img alt=\"".to_string(),
            "</".repeat(5000),
            "<a ".repeat(3000),
            "> ".repeat(500) + "deep",
            "- ".repeat(500) + "deep",
            "*".repeat(10_000),
            "[".repeat(10_000),
            "é<é>é".repeat(3000),
            "|a|\n|-|\n".to_string(),
            "```".to_string(),
            "x".repeat(MAX_SOURCE * 2),
        ];
        nasty.push("日本語<!--日本語-->日本語".repeat(40_000));
        for text in &nasty {
            let _ = parse(text);
        }
    }

    #[test]
    fn it_draws_inside_a_frame_without_panicking() {
        let look = Look::new(Mode::Dark.palette_with(Accent::Codigo));
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_max_width(320.0);
            show(ui, &look, PR);
            show(
                ui,
                &look,
                "| a | b |\n|---|---|\n| 1 | 2 |\n\n> q\n\n1. x\n2. y",
            );
        });
        out.textures_delta.clear();
    }
}
