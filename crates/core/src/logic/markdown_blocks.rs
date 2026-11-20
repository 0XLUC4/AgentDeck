//! Markdown as a list of editable blocks, for the visual editor of the Files page.
//!
//! [`Doc::parse`] cuts the text into blocks (headings, paragraphs, list items, quotes, fenced code,
//! rules, images) and keeps everything else (tables, HTML, front matter, indented code) as raw
//! blocks. Each block keeps the exact source of its marker and of the blank lines after it, so
//! [`Doc::serialize`] gives the original text back byte for byte until a block is edited.
//! [`inline_spans`] and [`toggle_wrap`] cover the inline syntax (bold, italic, strike, code, links).

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    /// Level 1 to 6.
    Heading(u8),
    Bullet,
    Numbered,
    /// A `- [ ]` item; true when checked.
    Task(bool),
    /// One `>` line (and its lazy continuation lines).
    Quote,
    /// Fenced code; the text includes both fences.
    Code,
    Rule,
    /// A line holding only `![alt](src)`.
    Image,
    /// Kept as typed: tables, HTML, front matter, indented code.
    Raw,
}

impl BlockKind {
    pub fn is_list(self) -> bool {
        matches!(self, BlockKind::Bullet | BlockKind::Numbered | BlockKind::Task(_))
    }

    /// Edited as lines of source: Enter adds a line, not a block.
    pub fn is_multiline(self) -> bool {
        matches!(self, BlockKind::Code | BlockKind::Raw)
    }

    /// Holds inline Markdown that can be merged into or split.
    pub fn is_text(self) -> bool {
        !matches!(self, BlockKind::Code | BlockKind::Raw | BlockKind::Rule | BlockKind::Image)
    }

    /// Marker of a new block of this kind.
    fn marker(self) -> String {
        match self {
            BlockKind::Heading(level) => format!("{} ", "#".repeat(level.clamp(1, 6) as usize)),
            BlockKind::Bullet => "- ".into(),
            BlockKind::Numbered => "1. ".into(),
            BlockKind::Task(false) => "- [ ] ".into(),
            BlockKind::Task(true) => "- [x] ".into(),
            BlockKind::Quote => "> ".into(),
            _ => String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// Source before the text on the first line: indentation and marker (`"  - [x] "`, `"## "`).
    pub prefix: String,
    /// Inline Markdown; several lines for paragraphs with soft breaks, code and raw blocks.
    pub text: String,
    /// Blank (whitespace-only) lines after the block, as written.
    pub blank_after: Vec<String>,
}

impl Block {
    pub fn new(kind: BlockKind, text: impl Into<String>) -> Self {
        Self { kind, prefix: kind.marker(), text: text.into(), blank_after: Vec::new() }
    }

    fn parsed(kind: BlockKind, prefix: &str, text: impl Into<String>) -> Self {
        Self { kind, prefix: prefix.to_owned(), text: text.into(), blank_after: Vec::new() }
    }

    /// Leading whitespace of the marker (nesting of list items).
    pub fn indent(&self) -> &str {
        let rest = self.prefix.trim_start_matches([' ', '\t']);
        &self.prefix[..self.prefix.len() - rest.len()]
    }

    /// Nesting depth of a list item: 2 columns (or a tab) per level.
    pub fn depth(&self) -> usize {
        let width: usize = self.indent().chars().map(|c| if c == '\t' { 4 } else { 1 }).sum();
        width / 2
    }

    /// Number and delimiter (`.` or `)`) of a numbered item.
    pub fn number(&self) -> Option<(u64, char)> {
        let rest = self.prefix.trim_start_matches([' ', '\t']);
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let delimiter = rest[digits..].chars().next().filter(|c| matches!(c, '.' | ')'))?;
        Some((rest[..digits].parse().ok()?, delimiter))
    }

    fn set_number(&mut self, number: u64) {
        if let Some((_, delimiter)) = self.number() {
            let rest = self.prefix.trim_start_matches([' ', '\t']);
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            self.prefix = format!("{}{number}{delimiter}{}", self.indent(), &rest[digits + 1..]);
        }
    }

    /// Changes the kind, rewriting the marker (list items keep their indentation).
    pub fn set_kind(&mut self, kind: BlockKind) {
        if kind == self.kind {
            return;
        }
        let indent = if self.kind.is_list() && kind.is_list() { self.indent().to_owned() } else { String::new() };
        self.prefix = indent + &kind.marker();
        self.kind = kind;
    }

    /// The block's lines as written, without line endings.
    fn source(&self) -> String {
        format!("{}{}", self.prefix, self.text)
    }
}

/// A Markdown document cut into blocks; see the module documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    pub blocks: Vec<Block>,
    /// Blank lines before the first block.
    pub leading: Vec<String>,
    eol: &'static str,
    final_newline: bool,
}

impl Default for Doc {
    fn default() -> Self {
        Self { blocks: Vec::new(), leading: Vec::new(), eol: "\n", final_newline: true }
    }
}

/// What a line starts, see [`line_start`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Start {
    Text,
    /// Prefix length and level.
    Heading(usize, u8),
    Rule,
    Quote(usize),
    /// Kind and prefix length.
    List(BlockKind, usize),
    /// Fence character and length.
    Fence(u8, usize),
    Table,
    Html,
    Indented,
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn leading_ws(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn run_len(bytes: &[u8], from: usize, byte: u8) -> usize {
    bytes[from..].iter().take_while(|&&b| b == byte).count()
}

/// List marker of `line` (any indentation): the kind and the prefix length.
fn list_marker(line: &str) -> Option<(BlockKind, usize)> {
    let ws = leading_ws(line);
    let rest = &line[ws..];
    let bytes = rest.as_bytes();
    let (marker_len, numbered) = match bytes.first()? {
        b'-' | b'*' | b'+' => (1, false),
        b'0'..=b'9' => {
            let digits = run_digits(bytes);
            if digits > 9 || !matches!(bytes.get(digits), Some(b'.' | b')')) {
                return None;
            }
            (digits + 1, true)
        }
        _ => return None,
    };
    let after = &rest[marker_len..];
    if !(after.is_empty() || after.starts_with([' ', '\t'])) {
        return None;
    }
    let len = ws + marker_len + leading_ws(after);
    if numbered {
        return Some((BlockKind::Numbered, len));
    }
    let content = &line[len..];
    for (checkbox, checked) in [("[ ]", false), ("[x]", true), ("[X]", true)] {
        if let Some(tail) = content.strip_prefix(checkbox)
            && (tail.is_empty() || tail.starts_with([' ', '\t']))
        {
            return Some((BlockKind::Task(checked), len + checkbox.len() + leading_ws(tail)));
        }
    }
    Some((BlockKind::Bullet, len))
}

fn run_digits(bytes: &[u8]) -> usize {
    bytes.iter().take_while(|b| b.is_ascii_digit()).count()
}

/// `` ``` `` or `~~~` (3 or more) opening a fenced code block.
fn fence(rest: &str) -> Option<(u8, usize)> {
    let bytes = rest.as_bytes();
    let ch = *bytes.first().filter(|&&b| b == b'`' || b == b'~')?;
    let n = run_len(bytes, 0, ch);
    (n >= 3 && !(ch == b'`' && rest[n..].contains('`'))).then_some((ch, n))
}

fn closes_fence(line: &str, ch: u8, n: usize) -> bool {
    let ws = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[ws..];
    ws <= 3 && run_len(rest.as_bytes(), 0, ch) >= n && rest.trim_start_matches(ch as char).trim().is_empty()
}

fn is_rule(rest: &str) -> bool {
    let mut marks = rest.chars().filter(|c| !c.is_whitespace());
    let Some(first) = marks.next().filter(|c| matches!(c, '-' | '*' | '_')) else { return false };
    let mut count = 1;
    for c in marks {
        if c != first {
            return false;
        }
        count += 1;
    }
    count >= 3
}

fn is_table_delimiter(line: &str) -> bool {
    line.contains('|') && line.contains('-') && line.chars().all(|c| matches!(c, '|' | ':' | '-' | ' ' | '\t'))
}

fn line_start(line: &str) -> Start {
    let ws = leading_ws(line);
    if line.starts_with('\t') || ws >= 4 {
        return match list_marker(line) {
            Some((kind, len)) => Start::List(kind, len),
            None => Start::Indented,
        };
    }
    let rest = &line[ws..];
    if let Some((ch, n)) = fence(rest) {
        return Start::Fence(ch, n);
    }
    let hashes = run_len(rest.as_bytes(), 0, b'#');
    if (1..=6).contains(&hashes) {
        let after = &rest[hashes..];
        if after.is_empty() || after.starts_with([' ', '\t']) {
            return Start::Heading(ws + hashes + leading_ws(after), hashes as u8);
        }
    }
    if is_rule(rest) {
        return Start::Rule;
    }
    if rest.starts_with('>') {
        return Start::Quote(ws + 1 + usize::from(rest[1..].starts_with(' ')));
    }
    if let Some((kind, len)) = list_marker(line) {
        return Start::List(kind, len);
    }
    if rest.starts_with('|') {
        return Start::Table;
    }
    let mut chars = rest.chars();
    if chars.next() == Some('<') && chars.next().is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?')) {
        return Start::Html;
    }
    Start::Text
}

/// Index after the lines continuing the block that ends before `from` (lazy continuation).
fn continuation(lines: &[&str], mut from: usize) -> usize {
    while from < lines.len() && !is_blank(lines[from]) && matches!(line_start(lines[from]), Start::Text | Start::Indented) {
        from += 1;
    }
    from
}

/// Index after the non-blank lines from `from`.
fn until_blank(lines: &[&str], mut from: usize) -> usize {
    while from < lines.len() && !is_blank(lines[from]) {
        from += 1;
    }
    from
}

/// Last line of a front matter block (`---` on the first line, closed by `---` or `...`).
fn front_matter_end(lines: &[&str]) -> Option<usize> {
    if lines.first() != Some(&"---") {
        return None;
    }
    lines.iter().skip(1).position(|l| *l == "---" || *l == "...").map(|i| i + 1)
}

/// The block starting at line `i`, and the index of the line after it.
fn read_block(lines: &[&str], i: usize) -> (Block, usize) {
    let line = lines[i];
    let joined = |end: usize| lines[i..end].join("\n");
    match line_start(line) {
        Start::Fence(ch, n) => {
            let close = (i + 1..lines.len()).find(|&j| closes_fence(lines[j], ch, n));
            let end = close.map_or(lines.len(), |j| j + 1);
            (Block::parsed(BlockKind::Code, "", joined(end)), end)
        }
        Start::Heading(len, level) => (Block::parsed(BlockKind::Heading(level), &line[..len], &line[len..]), i + 1),
        Start::Rule => (Block::parsed(BlockKind::Rule, "", line), i + 1),
        Start::Table => {
            let mut end = i + 1;
            while end < lines.len() && lines[end].trim_start().starts_with('|') {
                end += 1;
            }
            (Block::parsed(BlockKind::Raw, "", joined(end)), end)
        }
        Start::Html | Start::Indented => {
            let end = until_blank(lines, i + 1);
            (Block::parsed(BlockKind::Raw, "", joined(end)), end)
        }
        Start::Quote(len) => {
            let end = continuation(lines, i + 1);
            let text = std::iter::once(&line[len..]).chain(lines[i + 1..end].iter().copied()).collect::<Vec<_>>().join("\n");
            (Block::parsed(BlockKind::Quote, &line[..len], text), end)
        }
        Start::List(kind, len) => {
            let end = continuation(lines, i + 1);
            let text = std::iter::once(&line[len..]).chain(lines[i + 1..end].iter().copied()).collect::<Vec<_>>().join("\n");
            (Block::parsed(kind, &line[..len], text), end)
        }
        Start::Text => {
            let end = continuation(lines, i + 1);
            if end - i >= 2 && line.contains('|') && is_table_delimiter(lines[i + 1]) {
                let end = until_blank(lines, i + 2);
                return (Block::parsed(BlockKind::Raw, "", joined(end)), end);
            }
            let ws = leading_ws(line);
            let kind = if end == i + 1 && image_source(line).is_some() { BlockKind::Image } else { BlockKind::Paragraph };
            let text = std::iter::once(&line[ws..]).chain(lines[i + 1..end].iter().copied()).collect::<Vec<_>>().join("\n");
            (Block::parsed(kind, &line[..ws], text), end)
        }
    }
}

impl Doc {
    pub fn parse(text: &str) -> Self {
        if text.is_empty() {
            return Self { final_newline: false, ..Self::default() };
        }
        // CRLF only when every line ends so; otherwise the '\r' stay in the lines (exact round trip).
        let crlf = text.contains("\r\n") && text.matches('\n').count() == text.matches("\r\n").count();
        let eol = if crlf { "\r\n" } else { "\n" };
        let final_newline = text.ends_with(eol);
        let body = text.strip_suffix(eol).unwrap_or(text);
        let lines: Vec<&str> = body.split(eol).collect();
        let mut doc = Self { blocks: Vec::new(), leading: Vec::new(), eol, final_newline };
        let mut i = 0;
        if let Some(end) = front_matter_end(&lines) {
            doc.blocks.push(Block::parsed(BlockKind::Raw, "", lines[..=end].join("\n")));
            i = end + 1;
        }
        while i < lines.len() {
            if is_blank(lines[i]) {
                match doc.blocks.last_mut() {
                    Some(block) => block.blank_after.push(lines[i].to_owned()),
                    None => doc.leading.push(lines[i].to_owned()),
                }
                i += 1;
                continue;
            }
            let (block, next) = read_block(&lines, i);
            doc.blocks.push(block);
            i = next;
        }
        doc
    }

    pub fn serialize(&self) -> String {
        let mut lines: Vec<String> = self.leading.clone();
        for block in &self.blocks {
            lines.extend(block.source().split('\n').map(str::to_owned));
            lines.extend(block.blank_after.iter().cloned());
        }
        if lines.is_empty() {
            return String::new();
        }
        let mut text = lines.join(self.eol);
        if self.final_newline {
            text.push_str(self.eol);
        }
        text
    }

    /// 0-based line where block `index` starts.
    pub fn line_of(&self, index: usize) -> usize {
        self.leading.len()
            + self.blocks[..index.min(self.blocks.len())]
                .iter()
                .map(|b| b.source().matches('\n').count() + 1 + b.blank_after.len())
                .sum::<usize>()
    }

    /// Headings as (block index, level, text), for the outline.
    pub fn headings(&self) -> Vec<(usize, u8, &str)> {
        self.blocks
            .iter()
            .enumerate()
            .filter_map(|(i, b)| match b.kind {
                BlockKind::Heading(level) => Some((i, level, heading_title(&b.text))),
                _ => None,
            })
            .collect()
    }

    /// Inserts `block` after block `index` (at the start when the document is empty) and returns
    /// its index. List items and quotes stay tight; other blocks get a blank line between them.
    pub fn insert_after(&mut self, index: usize, mut block: Block) -> usize {
        if self.blocks.is_empty() {
            self.blocks.push(block);
            return 0;
        }
        let index = index.min(self.blocks.len() - 1);
        let previous = &mut self.blocks[index];
        block.blank_after = std::mem::take(&mut previous.blank_after);
        let tight = (previous.kind.is_list() && block.kind.is_list()) || (previous.kind == BlockKind::Quote && block.kind == BlockKind::Quote);
        if !tight {
            previous.blank_after.push(String::new());
        }
        if block.blank_after.is_empty() && index + 1 < self.blocks.len() {
            // Without a blank line, the next block could join this one (two paragraphs).
            let next = self.blocks[index + 1].kind;
            let tight = (block.kind.is_list() && next.is_list()) || (block.kind == BlockKind::Quote && next == BlockKind::Quote);
            if !tight {
                block.blank_after.push(String::new());
            }
        }
        self.blocks.insert(index + 1, block);
        index + 1
    }

    /// Enter at byte `at` of block `index`: the text after `at` moves to a new block below (same list
    /// kind, an unchecked task, the next number; a paragraph after other blocks). Returns its index.
    pub fn split(&mut self, index: usize, at: usize) -> usize {
        let block = &mut self.blocks[index];
        let at = floor_char_boundary(&block.text, at);
        let rest = block.text.split_off(at);
        let kind = match block.kind {
            BlockKind::Task(_) => BlockKind::Task(false),
            kind @ (BlockKind::Bullet | BlockKind::Numbered | BlockKind::Quote) => kind,
            _ => BlockKind::Paragraph,
        };
        let mut new = Block::new(kind, rest);
        if kind.is_list() {
            new.prefix = format!("{}{}", block.indent(), new.prefix);
        }
        if let (BlockKind::Numbered, Some((number, delimiter))) = (kind, block.number()) {
            new.prefix = format!("{}{}{delimiter} ", block.indent(), number + 1);
        }
        let new_index = self.insert_after(index, new);
        if kind == BlockKind::Numbered {
            self.renumber(new_index);
        }
        new_index
    }

    /// Turns block `index` into a paragraph, with blank lines around it so it stays one.
    pub fn to_paragraph(&mut self, index: usize) {
        self.blocks[index].set_kind(BlockKind::Paragraph);
        if index > 0 && self.blocks[index - 1].blank_after.is_empty() {
            self.blocks[index - 1].blank_after.push(String::new());
        }
        if index + 1 < self.blocks.len() && self.blocks[index].blank_after.is_empty() {
            self.blocks[index].blank_after.push(String::new());
        }
    }

    /// Backspace at the start of block `index`: appends its text to the previous text block and
    /// returns where the join is in that block's text, or None when there is no such block.
    pub fn merge_up(&mut self, index: usize) -> Option<usize> {
        if index == 0 || index >= self.blocks.len() || !self.blocks[index - 1].kind.is_text() {
            return None;
        }
        let block = self.blocks.remove(index);
        let previous = &mut self.blocks[index - 1];
        let join = previous.text.len();
        previous.text.push_str(&block.text);
        previous.blank_after = block.blank_after;
        Some(join)
    }

    /// Removes block `index`; the block above takes its blank lines (what followed it follows that).
    pub fn remove(&mut self, index: usize) {
        let block = self.blocks.remove(index);
        if index > 0 {
            self.blocks[index - 1].blank_after = block.blank_after;
        }
    }

    /// Numbers the numbered items following `index` at its indentation one after another.
    pub fn renumber(&mut self, index: usize) {
        let mut start = index;
        while start > 0 && self.same_list(start - 1, index) {
            start -= 1;
        }
        let Some((mut number, _)) = self.blocks.get(start).and_then(Block::number) else { return };
        let mut i = start + 1;
        while i < self.blocks.len() && self.same_list(i, index) {
            number += 1;
            self.blocks[i].set_number(number);
            i += 1;
        }
    }

    fn same_list(&self, i: usize, of: usize) -> bool {
        let (a, b) = (&self.blocks[i], &self.blocks[of]);
        a.kind == BlockKind::Numbered && b.kind == BlockKind::Numbered && a.indent() == b.indent()
    }

    /// Checks or unchecks the task item `index`.
    pub fn toggle_task(&mut self, index: usize) {
        let block = &mut self.blocks[index];
        if let BlockKind::Task(checked) = block.kind {
            let (from, to) = if checked { (["[x]", "[X]"], "[ ]") } else { (["[ ]", "[ ]"], "[x]") };
            for from in from {
                if let Some(at) = block.prefix.rfind(from) {
                    block.prefix.replace_range(at..at + 3, to);
                    break;
                }
            }
            block.kind = BlockKind::Task(!checked);
        }
    }
}

/// Heading text without its optional closing `#`s (`"Title ##"`), keeping `"C#"`.
pub fn heading_title(text: &str) -> &str {
    let text = text.trim_end();
    let open = text.trim_end_matches('#');
    if open.len() < text.len() && (open.is_empty() || open.ends_with([' ', '\t'])) { open.trim_end() } else { text }
}

fn floor_char_boundary(text: &str, mut at: usize) -> usize {
    at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// A Markdown marker typed at the start of a block's text (`"## "`, `"- "`, `"1. "`, `"[] "`,
/// `"> "`): the kind it asks for, the prefix to use, and how many bytes of the text it was.
pub fn typed_marker(current: BlockKind, text: &str) -> Option<(BlockKind, String, usize)> {
    let task = |checked, len| Some((BlockKind::Task(checked), BlockKind::Task(checked).marker(), len));
    for (typed, checked) in [("[] ", false), ("[ ] ", false), ("[x] ", true)] {
        if text.starts_with(typed) && matches!(current, BlockKind::Paragraph | BlockKind::Bullet) {
            return task(checked, typed.len());
        }
    }
    if current != BlockKind::Paragraph || text.starts_with([' ', '\t']) {
        return None;
    }
    let (kind, len) = match line_start(text) {
        Start::Heading(len, level) => (BlockKind::Heading(level), len),
        Start::Quote(len) => (BlockKind::Quote, len),
        Start::List(kind, len) => (kind, len),
        _ => return None,
    };
    let marker = &text[..len];
    if !marker.ends_with(' ') {
        return None;
    }
    let prefix = if matches!(kind, BlockKind::Task(_)) { kind.marker() } else { marker.to_owned() };
    Some((kind, prefix, len))
}

/// Alt text and source of a line holding only an image (`![alt](src "title")`).
pub fn image_source(line: &str) -> Option<(String, String)> {
    let inner = line.trim().strip_prefix("![")?.strip_suffix(')')?;
    let (alt, target) = inner.split_once("](")?;
    if alt.contains(']') || target.contains(['(', ')']) {
        return None;
    }
    let src = match target.strip_prefix('<') {
        Some(bracketed) => bracketed.split_once('>')?.0,
        None => target.split_whitespace().next()?,
    };
    Some((alt.to_owned(), src.to_owned()))
}

/// Info string (language) and body of a fenced code block's text.
pub fn code_parts(text: &str) -> (&str, String) {
    let mut lines: Vec<&str> = text.split('\n').collect();
    let first = lines.remove(0);
    let trimmed = first.trim_start();
    let Some((ch, n)) = fence(trimmed) else { return ("", text.to_owned()) };
    if lines.last().is_some_and(|last| closes_fence(last, ch, n)) {
        lines.pop();
    }
    (trimmed[n..].trim(), lines.join("\n"))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
}

/// A run of text shown with one style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// As shown: markers and escapes removed.
    pub text: String,
    pub style: InlineStyle,
    /// Bytes of the source the text comes from (inside the markers).
    pub source: Range<usize>,
}

/// The inline Markdown of a block as styled runs (bold, italic, strike, code, link labels).
pub fn inline_spans(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    parse_inline(text, 0, InlineStyle::default(), &mut spans);
    spans
}

/// Style of the source at byte `at` (the caret), for the toolbar's pressed buttons.
pub fn style_at(text: &str, at: usize) -> InlineStyle {
    inline_spans(text)
        .into_iter()
        .find(|s| s.style != InlineStyle::default() && s.source.start <= at && at <= s.source.end)
        .map_or_else(InlineStyle::default, |s| s.style)
}

fn push_span(spans: &mut Vec<Span>, text: &str, style: InlineStyle, source: Range<usize>) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.style == style
        && last.source.end == source.start
    {
        last.text.push_str(text);
        last.source.end = source.end;
        return;
    }
    spans.push(Span { text: text.to_owned(), style, source });
}

/// End of a code span opened by `n` backticks before `from`: index of the closing run.
fn code_close(s: &str, from: usize, n: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let run = run_len(bytes, i, b'`');
            if run == n {
                return Some(i);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

/// `[label](url)` starting at `open` (the `[`): index of `]` and of `)`.
fn link_at(s: &str, open: usize) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut depth = 0;
    let mut i = open;
    let label_end = loop {
        match bytes.get(i)? {
            b'\\' => i += 1,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break i;
                }
            }
            _ => {}
        }
        i += 1;
    };
    if bytes.get(label_end + 1) != Some(&b'(') {
        return None;
    }
    let mut depth = 0;
    for (j, &b) in bytes.iter().enumerate().skip(label_end + 1) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((label_end, j));
                }
            }
            _ => {}
        }
    }
    None
}

/// Closing run of exactly `n` `ch` after `from`, not preceded by whitespace.
fn emphasis_close(s: &str, from: usize, ch: u8, n: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => {
                let run = run_len(bytes, i, b'`');
                i = code_close(s, i + run, run).map_or(i + run, |close| close + run);
            }
            b if b == ch => {
                let run = run_len(bytes, i, ch);
                let after_ok = ch != b'_' || !s[i + run..].chars().next().is_some_and(char::is_alphanumeric);
                if run == n && i > from && !bytes[i - 1].is_ascii_whitespace() && after_ok {
                    return Some(i);
                }
                i += run;
            }
            _ => i += 1,
        }
    }
    None
}

fn parse_inline(s: &str, base: usize, style: InlineStyle, spans: &mut Vec<Span>) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' && bytes.get(i + 1).is_some_and(u8::is_ascii_punctuation) {
            push_span(spans, &s[i + 1..i + 2], style, base + i..base + i + 2);
            i += 2;
            continue;
        }
        if c == b'`' {
            let n = run_len(bytes, i, b'`');
            if let Some(close) = code_close(s, i + n, n) {
                let inner = &s[i + n..close];
                let inner = match inner.strip_prefix(' ').and_then(|t| t.strip_suffix(' ')) {
                    Some(trimmed) if !trimmed.is_empty() => trimmed,
                    _ => inner,
                };
                push_span(spans, inner, InlineStyle { code: true, ..style }, base + i + n..base + close);
                i = close + n;
            } else {
                push_span(spans, &s[i..i + n], style, base + i..base + i + n);
                i += n;
            }
            continue;
        }
        let open = if c == b'!' && bytes.get(i + 1) == Some(&b'[') { i + 1 } else { i };
        if bytes[open] == b'['
            && let Some((label_end, close)) = link_at(s, open)
        {
            parse_inline(&s[open + 1..label_end], base + open + 1, InlineStyle { link: true, ..style }, spans);
            i = close + 1;
            continue;
        }
        if matches!(c, b'*' | b'_' | b'~') {
            let n = run_len(bytes, i, c);
            let opens = bytes.get(i + n).is_some_and(|b| !b.is_ascii_whitespace())
                && (c != b'_' || !s[..i].chars().next_back().is_some_and(char::is_alphanumeric));
            let flags = match (c, n) {
                (b'~', 2) => Some(InlineStyle { strike: true, ..style }),
                (b'~', _) => None,
                (_, 1) => Some(InlineStyle { italic: true, ..style }),
                (_, 2) => Some(InlineStyle { bold: true, ..style }),
                (_, 3) => Some(InlineStyle { bold: true, italic: true, ..style }),
                _ => None,
            };
            if let Some(inner_style) = flags.filter(|_| opens)
                && let Some(close) = emphasis_close(s, i + n, c, n)
            {
                parse_inline(&s[i + n..close], base + i + n, inner_style, spans);
                i = close + n;
            } else {
                push_span(spans, &s[i..i + n], style, base + i..base + i + n);
                i += n;
            }
            continue;
        }
        let len = s[i..].chars().next().map_or(1, char::len_utf8);
        push_span(spans, &s[i..i + len], style, base + i..base + i + len);
        i += len;
    }
}

/// Word (letters, digits, `_`) around byte `at`; empty when there is none.
fn word_at(text: &str, at: usize) -> Range<usize> {
    let at = floor_char_boundary(text, at);
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let start = text[..at].char_indices().rev().take_while(|&(_, c)| is_word(c)).last().map_or(at, |(i, _)| i);
    let end = at + text[at..].chars().take_while(|&c| is_word(c)).map(char::len_utf8).sum::<usize>();
    start..end
}

/// Toggles `marker` (`**`, `*`, `~~`) around the selection, or around the word at the caret when
/// nothing is selected. Returns the new text and the selection to restore.
pub fn toggle_wrap(text: &str, selection: Range<usize>, marker: &str) -> (String, Range<usize>) {
    let mut sel = floor_char_boundary(text, selection.start)..floor_char_boundary(text, selection.end);
    if sel.is_empty() {
        let word = word_at(text, sel.start);
        if !word.is_empty() {
            sel = word;
        }
    }
    let m = marker.len();
    let ch = marker.as_bytes()[0];
    let bytes = text.as_bytes();
    let before = bytes[..sel.start].iter().rev().take_while(|&&b| b == ch).count();
    let after = run_len(bytes, sel.end, ch);
    let fits = |run: usize| run == m || (ch != b'~' && run == 3);
    if before >= m && after >= m && fits(before) && fits(after) {
        let unwrapped = format!("{}{}{}", &text[..sel.start - m], &text[sel.clone()], &text[sel.end + m..]);
        return (unwrapped, sel.start - m..sel.end - m);
    }
    let wrapped = format!("{}{marker}{}{marker}{}", &text[..sel.start], &text[sel.clone()], &text[sel.end..]);
    (wrapped, sel.start + m..sel.end + m)
}

/// Makes the selection (or the word at the caret) a link and selects the `url` placeholder.
pub fn insert_link(text: &str, selection: Range<usize>) -> (String, Range<usize>) {
    let mut sel = floor_char_boundary(text, selection.start)..floor_char_boundary(text, selection.end);
    if sel.is_empty() {
        sel = word_at(text, sel.start);
    }
    let label = if sel.is_empty() { "link" } else { &text[sel.clone()] };
    let url_at = sel.start + label.len() + 3;
    let linked = format!("{}[{label}](url){}", &text[..sel.start], &text[sel.end..]);
    (linked, url_at..url_at + 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(text: &str) {
        assert_eq!(Doc::parse(text).serialize(), text, "round trip of {text:?}");
    }

    fn kinds(text: &str) -> Vec<BlockKind> {
        Doc::parse(text).blocks.iter().map(|b| b.kind).collect()
    }

    #[test]
    fn round_trips_unedited_documents() {
        for text in [
            "",
            "\n",
            "\n\n",
            "plain",
            "plain\n",
            "# Title\n\nSome *text* with **bold** and `code`.\n",
            "# Title\r\n\r\n- a\r\n- b\r\n",
            "mixed\r\nendings\nhere\n",
            "\n\n  leading blanks\n\n\n",
            "line one\nline two\n   \n\ttabbed blank\n",
            "- a\n- b\n  - nested\n    continuation\n* star\n+ plus\n",
            "1. one\n2) two\n10. ten\n",
            "- [ ] todo\n- [x] done\n  - [X] nested done\n- [ ]\n",
            "> quote\n> second\nlazy\n\n>no space\n",
            "```rust\nfn main() {}\n\n```\ntext\n~~~\nunterminated\n",
            "````\n```\nnested fence\n```\n````\n",
            "---\ntitle: x\ntags: [a]\n---\n# Body\n",
            "---\n\n***\n_ _ _\n",
            "| a | b |\n|---|---|\n| 1 | 2 |\n\nx | y\n--|--\n1 | 2\n",
            "<div>\n  <b>html</b>\n</div>\n\nafter\n",
            "    indented code\n    more\n\npara\n",
            "![alt](img/a.png)\n\n![b](<b c.png> \"title\")\n",
            "#\n## \n###### six\n####### seven\n#hashtag\n",
            "trailing spaces   \nand no final newline",
            "- \n-\n1.\n",
            "unicode é ✓ 日本\n- ☐ item\n",
        ] {
            round_trip(text);
        }
    }

    #[test]
    fn recognizes_block_kinds() {
        use BlockKind::*;
        assert_eq!(
            kinds("# H\n\npara\nmore\n\n- a\n1. b\n- [ ] c\n- [x] d\n> q\n\n---\n\n```\nx\n```\n![i](i.png)\n"),
            [Heading(1), Paragraph, Bullet, Numbered, Task(false), Task(true), Quote, Rule, Code, Image]
        );
        assert_eq!(kinds("---\na: 1\n---\ntext\n"), [Raw, Paragraph]);
        assert_eq!(kinds("| a |\n|---|\n"), [Raw]);
        assert_eq!(kinds("a | b\n--|--\n"), [Raw]);
        assert_eq!(kinds("<p>x</p>\n"), [Raw]);
        assert_eq!(kinds("    code\n"), [Raw]);
        assert_eq!(kinds("**bold** start\n-5 degrees\n"), [Paragraph]);
        assert_eq!(kinds("![a](b) and text\n"), [Paragraph]);
    }

    #[test]
    fn keeps_markers_and_text_apart() {
        let doc = Doc::parse("## Title\n  - [x] done item\n3) third\n> quoted\n");
        let parts: Vec<(&str, &str)> = doc.blocks.iter().map(|b| (b.prefix.as_str(), b.text.as_str())).collect();
        assert_eq!(parts, [("## ", "Title"), ("  - [x] ", "done item"), ("3) ", "third"), ("> ", "quoted")]);
        assert_eq!(doc.blocks[1].depth(), 1);
        assert_eq!(doc.blocks[2].number(), Some((3, ')')));
    }

    #[test]
    fn edits_serialize_as_markdown() {
        let mut doc = Doc::parse("# Title\n\nHello\n");
        doc.blocks[1].text = "Hello **world**".into();
        assert_eq!(doc.serialize(), "# Title\n\nHello **world**\n");
        doc.blocks[1].set_kind(BlockKind::Heading(2));
        assert_eq!(doc.serialize(), "# Title\n\n## Hello **world**\n");
        doc.blocks[1].set_kind(BlockKind::Task(false));
        assert_eq!(doc.serialize(), "# Title\n\n- [ ] Hello **world**\n");
        let mut crlf = Doc::parse("a\r\n");
        crlf.blocks[0].text = "a\nb".into();
        assert_eq!(crlf.serialize(), "a\r\nb\r\n");
    }

    #[test]
    fn splits_blocks_like_enter() {
        let mut doc = Doc::parse("- one two\n- three\n");
        let new = doc.split(0, 3);
        assert_eq!(new, 1);
        assert_eq!(doc.serialize(), "- one\n-  two\n- three\n");

        let mut doc = Doc::parse("1. a\n2. b\n");
        doc.split(0, 1);
        assert_eq!(doc.serialize(), "1. a\n2. \n3. b\n");

        let mut doc = Doc::parse("  - [x] done\n");
        doc.split(0, 4);
        assert_eq!(doc.serialize(), "  - [x] done\n  - [ ] \n");

        let mut doc = Doc::parse("first second\nnext\n");
        doc.split(0, 5);
        assert_eq!(doc.serialize(), "first\n\n second\nnext\n");

        // A new paragraph keeps a blank line before the next one, so they stay two.
        let mut doc = Doc::parse("# Head\npara\n");
        doc.split(0, 6);
        doc.blocks[1].text = "x".into();
        assert_eq!(doc.serialize(), "# Head\n\nx\n\npara\n");
        assert_eq!(kinds(&doc.serialize()), [BlockKind::Heading(1), BlockKind::Paragraph, BlockKind::Paragraph]);
    }

    #[test]
    fn exits_lists_and_merges_blocks_like_backspace() {
        let mut doc = Doc::parse("- a\n- b\n- c\n");
        doc.to_paragraph(1);
        assert_eq!(doc.serialize(), "- a\n\nb\n\n- c\n");
        assert_eq!(kinds(&doc.serialize()), [BlockKind::Bullet, BlockKind::Paragraph, BlockKind::Bullet]);

        let mut doc = Doc::parse("# Title\n\nbody\n");
        assert_eq!(doc.merge_up(1), Some(5));
        assert_eq!(doc.serialize(), "# Titlebody\n");

        let mut doc = Doc::parse("```\nx\n```\n\nafter\n");
        assert_eq!(doc.merge_up(1), None);
        doc.remove(1);
        assert_eq!(doc.serialize(), "```\nx\n```\n");
    }

    #[test]
    fn renumbers_and_toggles_tasks() {
        let mut doc = Doc::parse("1. a\n1. b\n  1. nested\n7. c\n");
        doc.renumber(0);
        assert_eq!(doc.serialize(), "1. a\n2. b\n  1. nested\n7. c\n");
        let mut doc = Doc::parse("- [ ] a\n- [X] b\n");
        doc.toggle_task(0);
        doc.toggle_task(1);
        assert_eq!(doc.serialize(), "- [x] a\n- [ ] b\n");
        assert_eq!(doc.blocks[0].kind, BlockKind::Task(true));
    }

    #[test]
    fn inserts_with_blank_lines_where_needed() {
        let mut doc = Doc::default();
        assert_eq!(doc.insert_after(0, Block::new(BlockKind::Paragraph, "x")), 0);
        doc.insert_after(0, Block::new(BlockKind::Rule, "---"));
        assert_eq!(doc.serialize(), "x\n\n---\n");
        let mut doc = Doc::parse("- a\n\ntext\n");
        doc.insert_after(0, Block::new(BlockKind::Bullet, "b"));
        assert_eq!(doc.serialize(), "- a\n- b\n\ntext\n");
    }

    #[test]
    fn finds_lines_and_headings() {
        let doc = Doc::parse("\n# A\n\ntext\nmore\n## B ##\n");
        assert_eq!(doc.line_of(0), 1);
        assert_eq!(doc.line_of(2), 5);
        assert_eq!(doc.headings(), [(0, 1, "A"), (2, 2, "B")]);
        assert_eq!(heading_title("Learn C#"), "Learn C#");
        assert_eq!(heading_title("Done ## "), "Done");
    }

    #[test]
    fn reads_typed_markers() {
        use BlockKind::*;
        assert_eq!(typed_marker(Paragraph, "## x"), Some((Heading(2), "## ".into(), 3)));
        assert_eq!(typed_marker(Paragraph, "- "), Some((Bullet, "- ".into(), 2)));
        assert_eq!(typed_marker(Paragraph, "3. x"), Some((Numbered, "3. ".into(), 3)));
        assert_eq!(typed_marker(Paragraph, "[] x"), Some((Task(false), "- [ ] ".into(), 3)));
        assert_eq!(typed_marker(Bullet, "[ ] "), Some((Task(false), "- [ ] ".into(), 4)));
        assert_eq!(typed_marker(Paragraph, "- [x] y"), Some((Task(true), "- [x] ".into(), 6)));
        assert_eq!(typed_marker(Paragraph, "> "), Some((Quote, "> ".into(), 2)));
        assert_eq!(typed_marker(Paragraph, "#"), None);
        assert_eq!(typed_marker(Paragraph, "#tag"), None);
        assert_eq!(typed_marker(Paragraph, "-"), None);
        assert_eq!(typed_marker(Heading(1), "- x"), None);
    }

    #[test]
    fn reads_images_and_code() {
        assert_eq!(image_source(" ![A cat](cat.png \"t\") "), Some(("A cat".into(), "cat.png".into())));
        assert_eq!(image_source("![x](<a b.png>)"), Some(("x".into(), "a b.png".into())));
        assert_eq!(image_source("![x](a.png) more"), None);
        assert_eq!(code_parts("```rust\nfn a()\n```"), ("rust", "fn a()".into()));
        assert_eq!(code_parts("~~~\nopen"), ("", "open".into()));
    }

    fn shown(text: &str) -> Vec<(String, InlineStyle)> {
        inline_spans(text).into_iter().map(|s| (s.text, s.style)).collect()
    }

    #[test]
    fn styles_inline_markdown() {
        let plain = InlineStyle::default();
        let bold = InlineStyle { bold: true, ..plain };
        let italic = InlineStyle { italic: true, ..plain };
        assert_eq!(shown("a **b** c"), [("a ".into(), plain), ("b".into(), bold), (" c".into(), plain)]);
        assert_eq!(shown("*i* _j_"), [("i".into(), italic), (" ".into(), plain), ("j".into(), italic)]);
        assert_eq!(shown("***x***"), [("x".into(), InlineStyle { bold: true, italic: true, ..plain })]);
        assert_eq!(shown("**a *b* c**")[1], ("b".into(), InlineStyle { bold: true, italic: true, ..plain }));
        assert_eq!(shown("~~gone~~"), [("gone".into(), InlineStyle { strike: true, ..plain })]);
        assert_eq!(shown("`a*b*` x"), [("a*b*".into(), InlineStyle { code: true, ..plain }), (" x".into(), plain)]);
        assert_eq!(shown("see [the **docs**](http://x)"), [
            ("see ".into(), plain),
            ("the ".into(), InlineStyle { link: true, ..plain }),
            ("docs".into(), InlineStyle { link: true, bold: true, ..plain }),
        ]);
        assert_eq!(shown("snake_case_name 2 * 3 * 4"), [("snake_case_name 2 * 3 * 4".into(), plain)]);
        assert_eq!(shown(r"\*not\*"), [("*not*".into(), plain)]);
        assert_eq!(shown("**open"), [("**open".into(), plain)]);
        assert_eq!(shown("é **ü**"), [("é ".into(), plain), ("ü".into(), bold)]);
    }

    #[test]
    fn finds_the_style_at_the_caret() {
        assert!(style_at("a **bold** c", 5).bold);
        assert!(!style_at("a **bold** c", 11).bold);
        assert!(style_at("~~x~~", 2).strike);
    }

    #[test]
    fn toggles_wrapping_markers() {
        assert_eq!(toggle_wrap("make bold", 5..9, "**"), ("make **bold**".into(), 7..11));
        assert_eq!(toggle_wrap("make **bold**", 7..11, "**"), ("make bold".into(), 5..9));
        // The word at the caret.
        assert_eq!(toggle_wrap("a word here", 4..4, "*"), ("a *word* here".into(), 3..7));
        assert_eq!(toggle_wrap("a *word* here", 5..5, "*"), ("a word here".into(), 2..6));
        // Italic on bold adds a star rather than removing one.
        assert_eq!(toggle_wrap("**x**", 2..3, "*"), ("***x***".into(), 3..4));
        assert_eq!(toggle_wrap("***x***", 3..4, "**"), ("*x*".into(), 1..2));
        assert_eq!(toggle_wrap("", 0..0, "~~"), ("~~~~".into(), 2..2));
    }

    #[test]
    fn inserts_links() {
        assert_eq!(insert_link("see docs now", 4..8), ("see [docs](url) now".into(), 11..14));
        assert_eq!(insert_link("", 0..0), ("[link](url)".into(), 7..10));
    }
}
