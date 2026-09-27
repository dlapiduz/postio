//! The text index (research R7): the document's text in reading order,
//! with the geometry of every drawn cluster -- the one serialisation copy,
//! find and the screen reader share, so they cannot disagree.
//!
//! Built once per render, on the render thread, from the laid-out tree:
//! tabs between table cells, newlines at rows and blocks, image `alt` in
//! its place, `display: none` and `visibility: hidden` left out. Blitz's
//! own `get_selected_text` joins inline roots with a space and flattens
//! tables, so it is not used.

use std::ops::Range;

use blitz_dom::{BaseDocument, LocalName, Node, NodeId, local_name};
use parley::layout::PositionedLayoutItem;

use crate::{Cluster, Point, Rect, Rgb, Scope, TextIndex};

/// Elements whose end is a line break in reading order.
const BLOCKS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "dd",
    "details",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "ul",
];

/// Elements whose content is never read.
const UNREAD: &[&str] = &["head", "script", "style", "template", "title"];

/// Build the index of a laid-out document.
pub(crate) fn build(doc: &BaseDocument) -> TextIndex {
    let mut builder = Builder {
        doc,
        text: String::new(),
        chars: 0,
        clusters: Vec::new(),
        line: 0,
        cell: 0,
        inline: 0,
    };
    builder.walk(doc.root_node().id);
    let trimmed = builder.text.trim_end().len();
    builder.text.truncate(trimmed);
    TextIndex {
        text: builder.text,
        clusters: builder.clusters,
    }
}

struct Builder<'a> {
    doc: &'a BaseDocument,
    text: String,
    /// `text`'s length in chars: every range is in chars.
    chars: usize,
    clusters: Vec<Cluster>,
    line: u32,
    /// Cells written in the current table row.
    cell: usize,
    /// Inside an inline box (an inline-block within a line): no breaks.
    inline: usize,
}

impl Builder<'_> {
    fn push(&mut self, c: char) {
        self.text.push(c);
        self.chars += 1;
    }

    fn last(&self) -> Option<char> {
        self.text.chars().next_back()
    }

    /// End the current line, unless it is already ended.
    fn line_break(&mut self) {
        if self.inline == 0 && self.last().is_some_and(|c| c != '\n') {
            self.trim_blanks();
            self.push('\n');
        }
    }

    /// Drop trailing spaces and tabs: no cluster covers them.
    fn trim_blanks(&mut self) {
        while let Some(c) = self.last().filter(|c| *c == ' ' || *c == '\t') {
            self.text.pop();
            self.chars -= 1;
            let _ = c;
        }
    }

    fn walk(&mut self, id: NodeId) {
        let Some(node) = self.doc.get_node(id) else {
            return;
        };
        if hidden(node) {
            return;
        }
        let tag = node
            .element_data()
            .map(|element| element.name.local.to_string());
        let tag = tag.as_deref().unwrap_or("");
        if UNREAD.contains(&tag) {
            return;
        }
        match tag {
            "td" | "th" => {
                if self.cell > 0 {
                    // Not tabs: an empty cell before this one is a column.
                    while self.last().is_some_and(|c| c == ' ' || c == '\n') {
                        self.text.pop();
                        self.chars -= 1;
                    }
                    self.push('\t');
                }
                self.cell += 1;
            }
            "br" => {
                self.push('\n');
                return;
            }
            "img" if self.inline == 0 => {
                self.alt(node);
                return;
            }
            _ => {}
        }
        if node.flags.is_inline_root() {
            self.inline_root(node);
        } else if tag == "tr" {
            let outer = std::mem::replace(&mut self.cell, 0);
            for child in node.children.iter().copied() {
                self.walk(child);
            }
            self.cell = outer;
        } else {
            for child in node.children.iter().copied() {
                self.walk(child);
            }
        }
        if tag == "tr" || BLOCKS.contains(&tag) {
            self.line_break();
        }
    }

    fn alt(&mut self, node: &Node) {
        if let Some(alt) = node.attr(local_name!("alt")).map(str::trim)
            && !alt.is_empty()
        {
            if self.last().is_some_and(|c| !c.is_whitespace()) {
                self.push(' ');
            }
            for c in alt.chars() {
                self.push(c);
            }
        }
    }

    /// One inline root: its own text in logical order, with every inline
    /// box (an image, an inline-block) read at the place it sits, and a
    /// cluster for every glyph cluster drawn.
    fn inline_root(&mut self, node: &Node) {
        let Some(data) = node
            .element_data()
            .and_then(|element| element.inline_layout_data.as_ref())
        else {
            return;
        };
        if self.inline == 0 && self.last().is_some_and(|c| c != '\n' && c != '\t') {
            self.push('\n');
        }
        let source = data.text.as_str();
        let mut boxes: Vec<(usize, NodeId)> = data
            .layout
            .inline_boxes()
            .iter()
            .map(|b| (b.index, NodeId::from_u64(b.id)))
            .collect();
        boxes.sort_by_key(|(index, _)| *index);
        let mut boxes = boxes.into_iter().peekable();
        // Where each byte of `source` landed in `text`, in chars.
        let mut at = vec![self.chars; source.len() + 1];
        for (byte, c) in source.char_indices() {
            while let Some((_, id)) = boxes.next_if(|(index, _)| *index <= byte) {
                self.inline_box(id);
            }
            at[byte] = self.chars;
            // Collapsed already by layout; this only keeps a line from
            // starting with, or doubling, a space.
            let blank = c.is_whitespace() && self.last().is_none_or(|last| last.is_whitespace());
            if !blank {
                self.push(if c.is_whitespace() { ' ' } else { c });
            }
        }
        for (_, id) in boxes {
            self.inline_box(id);
        }
        at[source.len()] = self.chars;
        for (byte, _) in source.char_indices().rev() {
            // A byte inside a multi-byte char maps where its char did.
            for inner in byte + 1..source.len() {
                if !source.is_char_boundary(inner) {
                    at[inner] = at[byte];
                } else {
                    break;
                }
            }
        }
        self.clusters_of(node, data, &at);
    }

    fn inline_box(&mut self, id: NodeId) {
        let Some(node) = self.doc.get_node(id) else {
            return;
        };
        if node
            .element_data()
            .is_some_and(|element| element.name.local == local_name!("img"))
        {
            self.alt(node);
        } else {
            self.inline += 1;
            self.walk(id);
            self.inline -= 1;
        }
    }

    fn clusters_of(&mut self, node: &Node, data: &blitz_dom::node::TextLayout, at: &[usize]) {
        let origin = node.absolute_position(0.0, 0.0);
        let layout = node.final_layout();
        let (left, top) = (
            f64::from(origin.x + layout.padding.left + layout.border.left),
            f64::from(origin.y + layout.padding.top + layout.border.top),
        );
        let scope = scope_of(self.doc, node.id);
        for line in data.layout.lines() {
            let line_no = self.line;
            self.line += 1;
            // A line's shaped run is split into glyph runs wherever the
            // style changes, and each glyph run's `run()` is the whole shaped
            // run: its clusters are the ones whose first glyph falls in this
            // glyph run's share, taken in order.
            let mut shaped: Option<(usize, Range<usize>)> = None;
            let mut consumed = 0usize;
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let brush = run.style().brush.id;
                let Some(styled) = self.doc.get_node(brush) else {
                    continue;
                };
                if invisible(styled) {
                    continue;
                }
                let color = colour(styled).unwrap_or(Rgb::from_u8(0, 0, 0));
                let ground = ground(self.doc, brush);
                let key = (run.run().index(), run.run().cluster_range());
                if shaped.as_ref() != Some(&key) {
                    shaped = Some(key);
                    consumed = 0;
                }
                let share = consumed..consumed + run.glyphs().count();
                consumed = share.end;
                let metrics = run.run().metrics();
                // Blitz shapes text at the device scale (times the zoom);
                // the index is in CSS pixels.
                let k = f64::from(self.doc.viewport().scale());
                let y0 = top + f64::from(run.baseline() - metrics.ascent) / k;
                let y1 = y0 + f64::from(metrics.ascent + metrics.descent) / k;
                let mut x = left + f64::from(run.offset()) / k;
                let mut glyph = 0usize;
                for cluster in run.run().visual_clusters() {
                    let first = glyph;
                    glyph += cluster.glyphs().count();
                    if !share.contains(&first) || glyph == first {
                        continue;
                    }
                    let advance = f64::from(cluster.advance()) / k;
                    let bytes = cluster.text_range();
                    let range = at[bytes.start.min(at.len() - 1)]..at[bytes.end.min(at.len() - 1)];
                    // A zero-advance cluster draws nothing -- a zero-width
                    // space, a BOM; a combining mark shares its base's
                    // cluster -- so it has no box to hit or highlight.
                    if advance > 0.0
                        && !range.is_empty()
                        && data.text[bytes.clone()].chars().any(|c| !c.is_whitespace())
                    {
                        self.clusters.push(Cluster {
                            range,
                            rect: Rect::new(x, y0, x + advance, y1),
                            scope: scope.clone(),
                            color,
                            painted_ground: ground,
                            line: line_no,
                        });
                    }
                    x += advance;
                }
            }
        }
    }
}

/// `display: none`: not laid out, not read.
fn hidden(node: &Node) -> bool {
    node.primary_styles()
        .is_some_and(|styles| styles.get_box().display.is_none())
}

/// `visibility: hidden` or `collapse`: laid out, but neither drawn nor read.
fn invisible(node: &Node) -> bool {
    use style::computed_values::visibility::T as Visibility;
    node.primary_styles()
        .is_some_and(|styles| !matches!(styles.get_inherited_box().visibility, Visibility::Visible))
}

fn srgb(colour: &style::color::AbsoluteColor) -> (Rgb, f32) {
    let [r, g, b, a] = *colour
        .to_color_space(style::color::ColorSpace::Srgb)
        .raw_components();
    (
        Rgb {
            r: f64::from(r).clamp(0.0, 1.0),
            g: f64::from(g).clamp(0.0, 1.0),
            b: f64::from(b).clamp(0.0, 1.0),
        },
        a,
    )
}

/// The colour text in `node` is drawn in.
fn colour(node: &Node) -> Option<Rgb> {
    node.primary_styles()
        .map(|styles| srgb(&styles.clone_color()).0)
}

/// The colour behind `id`: the nearest ancestor's opaque background, or
/// white. Research R10's walk composites translucent grounds and images;
/// the theme rule (T073) replaces this with it.
fn ground(doc: &BaseDocument, id: NodeId) -> Rgb {
    let mut at = doc.get_node(id);
    while let Some(node) = at {
        if let Some(styles) = node.primary_styles() {
            let current = styles.clone_color();
            let background = styles
                .get_background()
                .background_color
                .resolve_to_absolute(&current);
            let (rgb, alpha) = srgb(&background);
            if alpha >= 0.999 {
                return rgb;
            }
        }
        at = node.parent.and_then(|parent| doc.get_node(parent));
    }
    Rgb::from_u8(255, 255, 255)
}

/// The message a node belongs to.
fn scope_of(doc: &BaseDocument, id: NodeId) -> Scope {
    let attribute = LocalName::from("data-postio-message");
    let mut at = doc.get_node(id);
    while let Some(node) = at {
        if let Some(scope) = node.attr(attribute.clone()) {
            return scope.to_owned();
        }
        at = node.parent.and_then(|parent| doc.get_node(parent));
    }
    Scope::new()
}

impl TextIndex {
    /// The offset of the glyph at `point`: its start, or its end when the
    /// point is in its trailing half. A point beside a line's text snaps to
    /// the nearest cluster on that line.
    pub fn hit(&self, point: Point) -> Option<usize> {
        let on_line: Vec<&Cluster> = self
            .clusters
            .iter()
            .filter(|c| point.y >= c.rect.y0 && point.y < c.rect.y1)
            .collect();
        // Horizontal distance first; then, because a tall line's glyph boxes
        // can overlap the next line's, how far the point is from the
        // cluster's vertical centre.
        let nearest = on_line.into_iter().min_by(|a, b| {
            let distance = |c: &Cluster| {
                let across = if point.x < c.rect.x0 {
                    c.rect.x0 - point.x
                } else if point.x > c.rect.x1 {
                    point.x - c.rect.x1
                } else {
                    0.0
                };
                (across, (point.y - c.rect.center().y).abs())
            };
            let (a, b) = (distance(a), distance(b));
            a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
        })?;
        Some(if point.x <= nearest.rect.center().x {
            nearest.range.start
        } else {
            nearest.range.end
        })
    }

    /// The word around `offset`: letters and digits either side of it.
    pub fn word_at(&self, offset: usize) -> Range<usize> {
        let chars: Vec<char> = self.text.chars().collect();
        let word = |i: usize| chars.get(i).is_some_and(|c| c.is_alphanumeric());
        if !word(offset) {
            return offset..offset;
        }
        let mut start = offset;
        while start > 0 && word(start - 1) {
            start -= 1;
        }
        let mut end = offset;
        while word(end) {
            end += 1;
        }
        start..end
    }

    /// The laid-out line around `offset`, as a range of the text.
    pub fn line_at(&self, offset: usize) -> Range<usize> {
        let Some(line) = self
            .clusters
            .iter()
            .find(|c| c.range.contains(&offset) || c.range.start >= offset)
            .map(|c| c.line)
        else {
            return offset..offset;
        };
        let on_line = self.clusters.iter().filter(|c| c.line == line);
        let start = on_line
            .clone()
            .map(|c| c.range.start)
            .min()
            .unwrap_or(offset);
        let end = on_line.map(|c| c.range.end).max().unwrap_or(offset);
        start..end
    }

    /// The boxes `range` is drawn in: one per line it touches.
    pub fn rects(&self, range: Range<usize>) -> Vec<Rect> {
        let mut out: Vec<(u32, Rect)> = Vec::new();
        for cluster in &self.clusters {
            if cluster.range.start >= range.end || cluster.range.end <= range.start {
                continue;
            }
            match out.iter_mut().find(|(line, _)| *line == cluster.line) {
                Some((_, rect)) => *rect = rect.union(cluster.rect),
                None => out.push((cluster.line, cluster.rect)),
            }
        }
        out.into_iter().map(|(_, rect)| rect).collect()
    }

    /// The text of `range`, in chars; clamped to the text.
    pub fn slice(&self, range: Range<usize>) -> &str {
        let byte = |char_offset: usize| {
            self.text
                .char_indices()
                .nth(char_offset)
                .map_or(self.text.len(), |(byte, _)| byte)
        };
        let start = byte(range.start);
        let end = byte(range.end.max(range.start));
        &self.text[start..end]
    }

    /// Every match of `query`, in order, ignoring case and diacritics:
    /// `TOTAL` finds `Total` and `tötal`.
    pub fn find(&self, query: &str) -> Vec<Range<usize>> {
        let (needle, _) = fold(query);
        if needle.is_empty() {
            return Vec::new();
        }
        let (haystack, origin) = fold(&self.text);
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(found) = haystack[from..]
            .windows(needle.len())
            .position(|window| window == needle.as_slice())
        {
            let start = from + found;
            let end = start + needle.len();
            out.push(origin[start]..origin[end - 1] + 1);
            from = end;
        }
        out
    }

    /// The first offset drawn at or below `y`: what a zoom keeps in place.
    ///
    /// The topmost cluster below `y`, leftmost on a tie -- not the first in
    /// reading order, which in a table can be the top of the next column.
    pub fn char_at_top(&self, y: f64) -> usize {
        self.clusters
            .iter()
            .filter(|c| c.rect.y1 > y)
            .min_by(|a, b| {
                a.rect
                    .y0
                    .total_cmp(&b.rect.y0)
                    .then(a.rect.x0.total_cmp(&b.rect.x0))
            })
            .map_or(self.text.chars().count(), |c| c.range.start)
    }
}

/// `text` folded for matching, and for each folded char the offset of the
/// char it came from.
fn fold(text: &str) -> (Vec<char>, Vec<usize>) {
    use unicode_normalization::UnicodeNormalization as _;
    use unicode_normalization::char::is_combining_mark;
    let mut folded = Vec::with_capacity(text.len());
    let mut origin = Vec::with_capacity(text.len());
    for (offset, c) in text.chars().enumerate() {
        for d in std::iter::once(c).nfd().filter(|d| !is_combining_mark(*d)) {
            for lower in d.to_lowercase() {
                folded.push(lower);
                origin.push(offset);
            }
        }
    }
    (folded, origin)
}
