use std::ops::Range;

use crate::document::{Block, ContentId, Document, HeadingLevel, ListKind, Task, WebHref};
use crate::edit_ops::flat::{self, Atom, MarkSet};
use crate::position::DocPosition;

/// A list break inside a paragraph, which a text view lays out as a new line of the same paragraph.
pub const LINE_BREAK: char = '\u{2028}';
pub const OBJECT: char = '\u{FFFC}';

static SEPARATOR: Run = Run::Separator;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    Text { len: u32, marks: MarkSet },
    Break,
    Image { content_id: ContentId, alt: String },
    RemoteImage { href: WebHref, alt: String },
    Prefix { kind: PrefixKind, len: u32 },
    Atomic { kind: AtomicKind, len: u32 },
    Separator,
    Preedit { len: u32 },
}

impl Run {
    pub fn len(&self) -> u32 {
        match self {
            Run::Text { len, .. }
            | Run::Prefix { len, .. }
            | Run::Atomic { len, .. }
            | Run::Preedit { len } => *len,
            Run::Break | Run::Image { .. } | Run::RemoteImage { .. } | Run::Separator => 1,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefixKind {
    Bullet { depth: u32 },
    Number(u32),
    Task(Task),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomicKind {
    Quoted,
    Rule,
    Table,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Paragraph,
    Heading(HeadingLevel),
    Code,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineStyle {
    pub kind: LineKind,
    pub list_depth: u32,
    pub quote_depth: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub path: Vec<u32>,
    pub start: u32,
    pub text_start: u32,
    pub len: u32,
    pub style: LineStyle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preedit {
    pub at: DocPosition,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Projection {
    units: Vec<Unit>,
    starts: Vec<u32>,
    preedit: Option<Preedit>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Unit {
    text: String,
    len: u32,
    runs: Vec<(u32, Run)>,
    lines: Vec<UnitLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UnitLine {
    tail: Vec<u32>,
    start: u32,
    text_start: u32,
    len: u32,
    style: LineStyle,
    hole: Option<(u32, u32)>,
}

impl UnitLine {
    fn buffer_len(&self) -> u32 {
        self.len + self.hole.map_or(0, |(_, len)| len)
    }

    fn to_buffer(&self, offset: u32) -> u32 {
        match self.hole {
            Some((at, len)) if offset > at => offset + len,
            _ => offset,
        }
    }

    fn to_document(&self, relative: u32) -> u32 {
        match self.hole {
            Some((at, len)) if relative > at => relative.saturating_sub(len).max(at),
            _ => relative,
        }
    }
}

pub fn project(document: &Document, preedit: Option<&Preedit>) -> Projection {
    let units = document
        .blocks
        .iter()
        .enumerate()
        .map(|(top, block)| unit_of(block, top, preedit))
        .collect();
    let mut projection = Projection {
        units,
        starts: Vec::new(),
        preedit: preedit.cloned(),
    };
    projection.restart(0);
    projection
}

impl Projection {
    pub fn len(&self) -> u32 {
        match (self.starts.last(), self.units.last()) {
            (Some(start), Some(unit)) => start + unit.len,
            _ => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        for (index, unit) in self.units.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            out.push_str(&unit.text);
        }
        out
    }

    pub fn runs(&self) -> impl Iterator<Item = (u32, &Run)> + '_ {
        self.runs_of(0..self.units.len())
    }

    pub fn lines(&self) -> impl Iterator<Item = Line> + '_ {
        self.lines_of(0..self.units.len())
    }

    pub fn runs_in(&self, range: Range<u32>) -> impl Iterator<Item = (u32, &Run)> + '_ {
        self.runs_of(self.units_over(range))
    }

    pub fn lines_in(&self, range: Range<u32>) -> impl Iterator<Item = Line> + '_ {
        self.lines_of(self.units_over(range))
    }

    pub fn position_of(&self, offset: u32) -> DocPosition {
        if self.units.is_empty() {
            return DocPosition::default();
        }
        let unit = self.starts.partition_point(|&start| start <= offset).max(1) - 1;
        let relative = offset.saturating_sub(self.starts[unit]);
        if let Some(line) = self.units[unit]
            .lines
            .iter()
            .find(|line| line.text_start + line.buffer_len() >= relative)
        {
            let into = line.to_document(relative.saturating_sub(line.text_start));
            return self.position(unit, line, into);
        }
        let after = (unit + 1..self.units.len()).find_map(|u| {
            self.units[u]
                .lines
                .first()
                .map(|line| self.position(u, line, 0))
        });
        let before = || {
            (0..=unit).rev().find_map(|u| {
                self.units[u]
                    .lines
                    .last()
                    .map(|line| self.position(u, line, line.len))
            })
        };
        after.or_else(before).unwrap_or_default()
    }

    pub fn offset_of(&self, position: &DocPosition) -> u32 {
        let Some(unit) = self.units.get(position.top()) else {
            return self.len();
        };
        let start = self.starts[position.top()];
        let tail = position.path.get(1..).unwrap_or_default();
        match unit
            .lines
            .binary_search_by(|line| line.tail.as_slice().cmp(tail))
        {
            Ok(found) => {
                let line = &unit.lines[found];
                start + line.text_start + line.to_buffer(position.offset.min(line.len))
            }
            Err(_) => start,
        }
    }

    pub fn utf16_of(&self, offset: u32) -> u32 {
        let mut left = offset as usize;
        let mut units = 0;
        for (index, unit) in self.units.iter().enumerate() {
            if index > 0 {
                if left == 0 {
                    break;
                }
                units += 1;
                left -= 1;
            }
            for c in unit.text.chars().take(left) {
                units += c.len_utf16() as u32;
            }
            left = left.saturating_sub(unit.len as usize);
            if left == 0 {
                break;
            }
        }
        units
    }

    pub fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_ref()
    }

    fn position(&self, unit: usize, line: &UnitLine, offset: u32) -> DocPosition {
        let mut path = Vec::with_capacity(line.tail.len() + 1);
        path.push(unit as u32);
        path.extend_from_slice(&line.tail);
        DocPosition::new(path, offset.min(line.len))
    }

    fn units_over(&self, range: Range<u32>) -> Range<usize> {
        if self.units.is_empty() {
            return 0..0;
        }
        let first = self
            .starts
            .partition_point(|&start| start <= range.start)
            .max(1)
            - 1;
        let last = self
            .starts
            .partition_point(|&start| start < range.end.max(range.start + 1));
        first..last.max(first + 1).min(self.units.len())
    }

    fn runs_of(&self, units: Range<usize>) -> impl Iterator<Item = (u32, &Run)> + '_ {
        let first = units.start;
        units.flat_map(move |u| {
            let start = self.starts[u];
            let separator = (u > first).then(|| (start - 1, &SEPARATOR));
            separator.into_iter().chain(
                self.units[u]
                    .runs
                    .iter()
                    .map(move |(at, run)| (start + at, run)),
            )
        })
    }

    fn lines_of(&self, units: Range<usize>) -> impl Iterator<Item = Line> + '_ {
        units.flat_map(move |u| {
            let start = self.starts[u];
            self.units[u].lines.iter().map(move |line| {
                let mut path = vec![u as u32];
                path.extend_from_slice(&line.tail);
                Line {
                    path,
                    start: start + line.start,
                    text_start: start + line.text_start,
                    len: line.buffer_len(),
                    style: line.style,
                }
            })
        })
    }

    fn restart(&mut self, from: usize) {
        self.starts.truncate(from);
        let mut at = match (from.checked_sub(1), self.starts.last()) {
            (Some(previous), Some(&start)) => start + self.units[previous].len + 1,
            _ => 0,
        };
        for unit in &self.units[from..] {
            self.starts.push(at);
            at += unit.len + 1;
        }
    }
}

#[derive(Default)]
struct UnitBuilder {
    unit: Unit,
    tail: Vec<u32>,
    preedit: Option<(Vec<u32>, u32, String)>,
}

impl UnitBuilder {
    fn push(&mut self, text: &str, len: u32, run: Run) {
        self.unit.runs.push((self.unit.len, run));
        self.unit.text.push_str(text);
        self.unit.len += len;
    }

    fn separate(&mut self) {
        if !self.unit.runs.is_empty() {
            self.push("\n", 1, Run::Separator);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Context {
    list_depth: u32,
    quote_depth: u32,
}

fn unit_of(block: &Block, top: usize, preedit: Option<&Preedit>) -> Unit {
    let mut builder = UnitBuilder {
        preedit: preedit
            .filter(|p| p.at.top() == top && !p.text.is_empty())
            .map(|p| (p.at.path[1..].to_vec(), p.at.offset, p.text.clone())),
        ..UnitBuilder::default()
    };
    block_into(&mut builder, block, Context::default(), None);
    builder.unit
}

fn block_into(out: &mut UnitBuilder, block: &Block, context: Context, prefix: Option<PrefixKind>) {
    match block {
        Block::Paragraph(_) | Block::Heading { .. } | Block::Code { .. } => {
            leaf_into(out, block, context, prefix)
        }
        Block::List { kind, items } => {
            let inner = Context {
                list_depth: context.list_depth + 1,
                ..context
            };
            for (index, item) in items.iter().enumerate() {
                let mark = match (item.task, kind) {
                    (Some(task), _) => PrefixKind::Task(task),
                    (None, ListKind::Ordered { start }) => {
                        PrefixKind::Number(start.saturating_add(index as u32))
                    }
                    (None, ListKind::Bullet) => PrefixKind::Bullet {
                        depth: inner.list_depth,
                    },
                };
                for (child, block) in item.blocks.iter().enumerate() {
                    out.tail.extend([index as u32, child as u32]);
                    block_into(out, block, inner, (child == 0).then_some(mark));
                    out.tail.truncate(out.tail.len() - 2);
                }
            }
        }
        Block::Quote(children) => {
            let inner = Context {
                quote_depth: context.quote_depth + 1,
                ..context
            };
            for (index, child) in children.iter().enumerate() {
                out.tail.push(index as u32);
                block_into(out, child, inner, None);
                out.tail.pop();
            }
        }
        Block::Quoted(quoted) => {
            let text = quoted.text().trim_end_matches('\n');
            let text = if text.is_empty() {
                OBJECT.to_string()
            } else {
                text.to_owned()
            };
            atomic_into(out, &text, AtomicKind::Quoted);
        }
        Block::Rule => atomic_into(out, &OBJECT.to_string(), AtomicKind::Rule),
        Block::Table(_) => {
            let text = Document {
                blocks: vec![block.clone()],
            }
            .to_text();
            atomic_into(out, text.trim_end_matches('\n'), AtomicKind::Table);
        }
    }
}

fn atomic_into(out: &mut UnitBuilder, text: &str, kind: AtomicKind) {
    out.separate();
    let len = text.chars().count() as u32;
    out.push(text, len, Run::Atomic { kind, len });
}

fn leaf_into(out: &mut UnitBuilder, block: &Block, context: Context, prefix: Option<PrefixKind>) {
    out.separate();
    let start = out.unit.len;
    if let Some(kind) = prefix {
        let text = prefix_text(kind);
        let len = text.chars().count() as u32;
        out.push(&text, len, Run::Prefix { kind, len });
    }
    let text_start = out.unit.len;
    let kind = match block {
        Block::Heading { level, .. } => LineKind::Heading(*level),
        Block::Code { .. } => LineKind::Code,
        _ => LineKind::Paragraph,
    };
    let cells = flat::of_block(block).unwrap_or_default();
    let hole = match &out.preedit {
        Some((tail, at, text)) if *tail == out.tail => {
            Some(((*at as usize).min(cells.len()), text.clone()))
        }
        _ => None,
    };
    let mut index = 0;
    while index <= cells.len() {
        if let Some((at, text)) = &hole
            && *at == index
        {
            let len = text.chars().count() as u32;
            out.push(text, len, Run::Preedit { len });
        }
        let Some(cell) = cells.get(index) else {
            break;
        };
        let stop = match &hole {
            Some((at, _)) if *at > index => *at,
            _ => cells.len(),
        };
        match &cell.atom {
            Atom::Char(_) => {
                let run = cells[index..stop]
                    .iter()
                    .take_while(|next| {
                        matches!(next.atom, Atom::Char(_)) && next.marks == cell.marks
                    })
                    .count();
                let text: String = cells[index..index + run]
                    .iter()
                    .filter_map(|cell| match cell.atom {
                        Atom::Char(c) => Some(c),
                        _ => None,
                    })
                    .collect();
                out.push(
                    &text,
                    run as u32,
                    Run::Text {
                        len: run as u32,
                        marks: MarkSet::clone(&cell.marks),
                    },
                );
                index += run;
                continue;
            }
            Atom::Break => out.push(&LINE_BREAK.to_string(), 1, Run::Break),
            Atom::Image { content_id, alt } => out.push(
                &OBJECT.to_string(),
                1,
                Run::Image {
                    content_id: content_id.clone(),
                    alt: alt.clone(),
                },
            ),
            Atom::Remote { href, alt } => out.push(
                &OBJECT.to_string(),
                1,
                Run::RemoteImage {
                    href: href.clone(),
                    alt: alt.clone(),
                },
            ),
        }
        index += 1;
    }
    out.unit.lines.push(UnitLine {
        tail: out.tail.clone(),
        start,
        text_start,
        len: cells.len() as u32,
        style: LineStyle {
            kind,
            list_depth: context.list_depth,
            quote_depth: context.quote_depth,
        },
        hole: hole.map(|(at, text)| (at as u32, text.chars().count() as u32)),
    });
}

fn prefix_text(kind: PrefixKind) -> String {
    match kind {
        PrefixKind::Bullet { depth } => match depth {
            0 | 1 => "\u{2022} ".to_owned(),
            2 => "\u{25E6} ".to_owned(),
            _ => "\u{25AA} ".to_owned(),
        },
        PrefixKind::Number(n) => format!("{n}. "),
        PrefixKind::Task(Task::Open) => "\u{2610} ".to_owned(),
        PrefixKind::Task(Task::Done) => "\u{2611} ".to_owned(),
    }
}
