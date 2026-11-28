use std::ops::Range;

use gtk::prelude::*;
use postio_body::projection::{AtomicKind, PrefixKind};
use postio_body::{LineKind, Run};

use super::Inner;

const INDENT: i32 = 24;
const HEADING_SCALE: [f64; 6] = [1.6, 1.35, 1.2, 1.1, 1.0, 0.9];

pub(super) fn tags() -> gtk::TextTagTable {
    let table = gtk::TextTagTable::new();
    let add = |tag: gtk::TextTag| {
        table.add(&tag);
    };
    add(gtk::TextTag::builder().name("strong").weight(700).build());
    add(gtk::TextTag::builder()
        .name("emphasis")
        .style(gtk::pango::Style::Italic)
        .build());
    add(gtk::TextTag::builder()
        .name("strikethrough")
        .strikethrough(true)
        .build());
    add(gtk::TextTag::builder()
        .name("code")
        .family("monospace")
        .build());
    add(gtk::TextTag::builder()
        .name("link")
        .underline(gtk::pango::Underline::Single)
        .build());
    for (index, scale) in HEADING_SCALE.iter().enumerate() {
        add(gtk::TextTag::builder()
            .name(format!("heading-{}", index + 1))
            .scale(*scale)
            .weight(700)
            .pixels_above_lines(8)
            .build());
    }
    add(gtk::TextTag::builder()
        .name("code-block")
        .family("monospace")
        .build());
    add(gtk::TextTag::builder()
        .name("prefix")
        .editable(false)
        .build());
    let tint = gtk::gdk::RGBA::new(0.5, 0.5, 0.5, 0.12);
    add(gtk::TextTag::builder()
        .name("quoted")
        .editable(false)
        .paragraph_background_rgba(&tint)
        .left_margin(INDENT / 2)
        .build());
    add(gtk::TextTag::builder()
        .name("quote-folded")
        .invisible(true)
        .build());
    add(gtk::TextTag::builder().name("rule").editable(false).build());
    for depth in 1..=6 {
        add(gtk::TextTag::builder()
            .name(format!("list-{depth}"))
            .left_margin(INDENT * depth)
            .indent(-INDENT)
            .build());
        add(gtk::TextTag::builder()
            .name(format!("quote-{depth}"))
            .left_margin(INDENT * depth)
            .build());
    }
    table
}

pub(super) fn retag(inner: &Inner, range: Range<u32>) -> u32 {
    let buffer = &inner.buffer;
    let projection = inner.projection.borrow();
    let span = |from: u32, to: u32| {
        (
            buffer.iter_at_offset(from as i32),
            buffer.iter_at_offset(to as i32),
        )
    };
    let (start, end) = span(range.start, range.end);
    buffer.remove_all_tags(&start, &end);
    let mut applied = 0;
    let mut tag = |name: &str, from: u32, to: u32| {
        if from < to {
            let (a, b) = span(from, to);
            buffer.apply_tag_by_name(name, &a, &b);
            applied += 1;
        }
    };
    for (at, run) in projection.runs_in(range.clone()) {
        let to = at + run.len();
        match run {
            Run::Text { marks, .. } => {
                for (on, name) in [
                    (marks.strong, "strong"),
                    (marks.emphasis, "emphasis"),
                    (marks.strikethrough, "strikethrough"),
                    (marks.code, "code"),
                    (marks.link.is_some(), "link"),
                ] {
                    if on {
                        tag(name, at, to);
                    }
                }
            }
            Run::Prefix { kind, .. } => {
                let _: &PrefixKind = kind;
                tag("prefix", at, to);
            }
            Run::Atomic { kind, .. } => match kind {
                AtomicKind::Quoted => {
                    tag("quoted", at, to);
                    if inner.quote_folded.get() {
                        let text: String = buffer
                            .slice(
                                &buffer.iter_at_offset(at as i32),
                                &buffer.iter_at_offset(to as i32),
                                true,
                            )
                            .into();
                        if let Some(from) = super::quoted::hidden(&text) {
                            tag("quote-folded", at + from, to);
                        }
                    }
                }
                AtomicKind::Table => tag("quoted", at, to),
                AtomicKind::Rule => tag("rule", at, to),
            },
            _ => {}
        }
    }
    for line in projection.lines_in(range) {
        let end = line.text_start + line.len;
        match line.style.kind {
            LineKind::Heading(level) => tag(&format!("heading-{}", level.digit()), line.start, end),
            LineKind::Code => tag("code-block", line.start, end),
            LineKind::Paragraph => {}
        }
        if line.style.list_depth > 0 {
            tag(
                &format!("list-{}", line.style.list_depth.min(6)),
                line.start,
                end.max(line.start + 1),
            );
        } else if line.style.quote_depth > 0 {
            tag(
                &format!("quote-{}", line.style.quote_depth.min(6)),
                line.start,
                end.max(line.start + 1),
            );
        }
    }
    applied
}
