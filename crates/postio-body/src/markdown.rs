//! Markdown, in both directions, for a frontend that draws text.
//!
//! [`from_html`] is the terminal reader's: the reader sanitiser's output, as
//! CommonMark a terminal can style. Only ever sanitised HTML -- converting
//! *after* the sanitiser is what keeps every guarantee it makes
//! (`specs/005-tui-frontend` research R5, `contracts/markdown.md`).
//!
//! Two things are Postio's own on the way out, because Markdown has no word
//! for them:
//!
//! * an image is `![alt](postio-image:N)`, a placeholder the terminal draws
//!   and never follows -- including an image whose remote `src` the sanitiser
//!   removed, which would otherwise vanish without a trace;
//! * a folded quote (`<details>`, from [`crate::fold_html_quotes`]) is its
//!   content between [`FOLD`] and [`END_FOLD`] lines, which the terminal
//!   turns into a block that expands.

/// The line that opens a folded quote in [`from_html`]'s output.
///
/// Private-use characters, so ordinary text does not look like one. A sender
/// who writes them can make text fold, which is all they can do with it.
pub const FOLD: &str = "\u{E000}fold\u{E000}";

/// The line that closes a folded quote.
pub const END_FOLD: &str = "\u{E000}end\u{E000}";

/// The scheme an image placeholder carries in [`from_html`]'s output.
pub const IMAGE_SCHEME: &str = "postio-image";

/// Sanitised HTML as Markdown for a terminal.
pub fn from_html(sanitized: &str) -> String {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use htmd::element_handler::Handlers;
    use htmd::{Element, HtmlToMarkdown};

    let images = Arc::new(AtomicUsize::new(0));
    let converter = HtmlToMarkdown::builder()
        .add_handler(vec!["img"], move |_: &dyn Handlers, element: Element| {
            let alt = element
                .attrs
                .iter()
                .find(|attribute| &*attribute.name.local == "alt")
                .map(|attribute| attribute.value.trim().to_owned())
                .filter(|alt| !alt.is_empty())
                .unwrap_or_else(|| "image".to_owned());
            let number = images.fetch_add(1, Ordering::Relaxed);
            // Brackets would end the alt text early; the placeholder's words
            // are the sender's, and only ever drawn, never followed.
            let alt = alt.replace(['[', ']'], "");
            Some(format!("![{alt}]({IMAGE_SCHEME}:{number})").into())
        })
        .add_handler(
            vec!["details"],
            |handlers: &dyn Handlers, element: Element| {
                let inner = handlers.walk_children(element.node);
                Some(format!("\n\n{FOLD}\n\n{}\n\n{END_FOLD}\n\n", inner.content.trim()).into())
            },
        )
        // The terminal draws its own line for a fold; the sanitiser's summary
        // words would only repeat it.
        .add_handler(vec!["summary"], |_: &dyn Handlers, _: Element| {
            Some("".into())
        })
        .build();
    // A converter that cannot read what the sanitiser wrote has nothing to
    // show; the plain text is what the reader falls back to then.
    converter.convert(sanitized).unwrap_or_default()
}

/// Markdown, as the composer's own document.
///
/// What the terminal composer sends through: the HTML part is
/// [`crate::render`] of this, the generator the desktop composer uses, so
/// the same content makes the same HTML from either (`contracts/markdown.md`,
/// SC-006). Total -- every input is some document, and it never panics.
///
/// What the document has no node for stays the words the person typed: a
/// table, strikethrough, raw HTML. A link to anything but `http`, `https` or
/// `mailto` is its words and no link. An image is an image only when it names
/// a part attached to this draft (`cid:`); one that names the network stays
/// the text that was written and is **never fetched** -- nothing in compose
/// may reach a remote URL (ADR 0003's privacy rule, which holds here).
pub fn to_document(markdown: &str) -> crate::Document {
    use pulldown_cmark::{Event, Parser, Tag};

    let mut stack = vec![Frame::Blocks(Vec::new())];
    for event in Parser::new(markdown) {
        match event {
            Event::Start(tag) => stack.push(match tag {
                Tag::Paragraph => Frame::Inlines(Holder::Paragraph, Vec::new()),
                Tag::Heading { level, .. } => Frame::Inlines(
                    Holder::Heading(
                        crate::HeadingLevel::from_digit(level as u8)
                            .unwrap_or(crate::HeadingLevel::Three),
                    ),
                    Vec::new(),
                ),
                Tag::BlockQuote(_) => Frame::Quote(Vec::new()),
                Tag::CodeBlock(_) => Frame::Code(String::new()),
                Tag::List(first) => Frame::List(first.is_some(), Vec::new()),
                Tag::Item => Frame::Item(Vec::new(), Vec::new()),
                Tag::Emphasis => Frame::Inlines(Holder::Emphasis, Vec::new()),
                Tag::Strong => Frame::Inlines(Holder::Strong, Vec::new()),
                Tag::Link { dest_url, .. } => {
                    Frame::Inlines(Holder::Link(crate::Href::parse(&dest_url)), Vec::new())
                }
                Tag::Image { dest_url, .. } => Frame::Image(dest_url.into_string(), String::new()),
                // Anything else the parser was not asked to know is not
                // reached; what is, keeps its words below.
                _ => Frame::Inlines(Holder::Transparent, Vec::new()),
            }),
            Event::End(_) => {
                let Some(frame) = stack.pop() else { break };
                if stack.is_empty() {
                    stack.push(frame);
                    break;
                }
                close(&mut stack, frame);
            }
            Event::Text(text) => match stack.last_mut() {
                Some(Frame::Code(code)) => code.push_str(&text),
                Some(Frame::Image(_, alt)) => alt.push_str(&text),
                _ => push_inline(&mut stack, crate::Inline::Text(text.into_string())),
            },
            Event::Code(code) => push_inline(&mut stack, crate::Inline::Code(code.into_string())),
            Event::Html(html) | Event::InlineHtml(html) => {
                push_inline(&mut stack, crate::Inline::Text(html.into_string()));
            }
            Event::SoftBreak => push_inline(&mut stack, crate::Inline::Text(" ".to_owned())),
            Event::HardBreak => push_inline(&mut stack, crate::Inline::Break),
            Event::Rule => push_block(&mut stack, crate::Block::Rule),
            _ => {}
        }
    }
    // Close whatever an unfinished input left open, innermost first.
    while stack.len() > 1 {
        let frame = stack.pop().expect("more than one");
        close(&mut stack, frame);
    }
    let blocks = match stack.pop() {
        Some(Frame::Blocks(blocks)) => blocks,
        _ => Vec::new(),
    };
    crate::Document { blocks }
}

/// The composer's document, as Markdown the terminal composer can edit.
///
/// What opens a draft written in the desktop composer: its HTML, parsed to
/// the document, written back out as the Markdown that reads into that same
/// document -- `to_document(&from_document(d)) == d` for any document without
/// a carried quote. A quote carried from the message being answered
/// (`Block::Quoted`) is not Markdown and is not written here; the composer
/// keeps it whole beside the text.
pub fn from_document(document: &crate::Document) -> String {
    let mut out = String::new();
    write_blocks(&mut out, &document.blocks, "");
    out
}

/// `blocks`, each line after the first of each block prefixed with `indent`
/// (a list item's continuation, a quote's `> `), a blank line between them.
fn write_blocks(out: &mut String, blocks: &[crate::Block], indent: &str) {
    let mut first = true;
    for block in blocks {
        let mut text = String::new();
        if !write_block(&mut text, block) {
            continue;
        }
        if !first {
            out.push_str(indent.trim_end());
            out.push('\n');
            out.push_str(indent);
        }
        first = false;
        out.push_str(&text.replace('\n', &format!("\n{indent}")));
        out.push('\n');
    }
}

/// One block, without its trailing newline; `false` for one Markdown does
/// not carry (a quote carried from the message being answered).
fn write_block(out: &mut String, block: &crate::Block) -> bool {
    use crate::Block;
    match block {
        Block::Paragraph(inlines) => write_inlines(out, inlines),
        Block::Heading { level, inlines } => {
            out.push_str(&"#".repeat(usize::from(level.digit())));
            out.push(' ');
            write_inlines(out, inlines);
        }
        Block::List { ordered, items } => {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                let marker = if *ordered {
                    format!("{}. ", index + 1)
                } else {
                    "- ".to_owned()
                };
                out.push_str(&marker);
                let indent = " ".repeat(marker.len());
                let mut inner = String::new();
                write_item(&mut inner, item, &indent);
                out.push_str(inner.trim_end_matches('\n'));
            }
        }
        Block::Quote(blocks) => {
            let mut inner = String::new();
            write_blocks(&mut inner, blocks, "");
            let quoted: Vec<String> = inner
                .trim_end_matches('\n')
                .lines()
                .map(|line| {
                    if line.is_empty() {
                        ">".to_owned()
                    } else {
                        format!("> {line}")
                    }
                })
                .collect();
            out.push_str(&quoted.join("\n"));
        }
        Block::Pre(code) => {
            let fence = if code.contains("```") { "~~~" } else { "```" };
            out.push_str(fence);
            out.push('\n');
            out.push_str(code);
            if !code.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(fence);
        }
        Block::Rule => out.push_str("---"),
        Block::Quoted(_) => return false,
    }
    true
}

/// A list item: its first block after the marker, the rest indented under
/// it, tight -- no blank line -- which is how a person types a list.
fn write_item(out: &mut String, blocks: &[crate::Block], indent: &str) {
    for (index, block) in blocks.iter().enumerate() {
        let mut text = String::new();
        if !write_block(&mut text, block) {
            continue;
        }
        if index > 0 {
            out.push('\n');
            out.push_str(indent);
        }
        out.push_str(&text.replace('\n', &format!("\n{indent}")));
    }
}

fn write_inlines(out: &mut String, inlines: &[crate::Inline]) {
    use crate::Inline;
    for inline in inlines {
        match inline {
            Inline::Text(text) => out.push_str(&escape(text)),
            Inline::Strong(inner) => {
                out.push_str("**");
                write_inlines(out, inner);
                out.push_str("**");
            }
            Inline::Emphasis(inner) => {
                out.push('*');
                write_inlines(out, inner);
                out.push('*');
            }
            Inline::Code(code) => {
                // A fence longer than any run of backticks inside.
                let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
                let fence = "`".repeat(longest + 1);
                let pad = if code.starts_with('`') || code.ends_with('`') {
                    " "
                } else {
                    ""
                };
                out.push_str(&format!("{fence}{pad}{code}{pad}{fence}"));
            }
            Inline::Link { href, inlines } => {
                out.push('[');
                write_inlines(out, inlines);
                out.push_str(&format!("]({})", href.as_str()));
            }
            Inline::Image { content_id, alt } => {
                out.push_str(&format!("![{}](cid:{})", escape(alt), content_id.as_str()));
            }
            Inline::Break => out.push_str("\\\n"),
        }
    }
}

/// Text, with everything Markdown would read as markup escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, c) in text.char_indices() {
        let at_line_start = index == 0 || text[..index].ends_with('\n');
        let special = matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '!' | '|'
        ) || (at_line_start && matches!(c, '#' | '-' | '+' | '=' | '~'))
            || (c == '.'
                && text[..index]
                    .chars()
                    .rev()
                    .take_while(char::is_ascii_digit)
                    .count()
                    > 0
                && text[..index]
                    .trim_start()
                    .chars()
                    .all(|c| c.is_ascii_digit()));
        if special {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// What holds a run of inlines.
enum Holder {
    Paragraph,
    Heading(crate::HeadingLevel),
    Emphasis,
    Strong,
    /// A link, when its target is one the document can carry.
    Link(Option<crate::Href>),
    /// Something the document has no node for: its words pass through.
    Transparent,
}

/// One open container while the Markdown is read.
enum Frame {
    Blocks(Vec<crate::Block>),
    Quote(Vec<crate::Block>),
    List(bool, Vec<Vec<crate::Block>>),
    /// A list item: its blocks, and the words of a tight item not yet
    /// wrapped in a paragraph.
    Item(Vec<crate::Block>, Vec<crate::Inline>),
    Inlines(Holder, Vec<crate::Inline>),
    Code(String),
    Image(String, String),
}

fn push_inline(stack: &mut [Frame], inline: crate::Inline) {
    match stack.last_mut() {
        Some(Frame::Inlines(_, inlines) | Frame::Item(_, inlines)) => {
            // One run of words is one text, however the parser split it -- at
            // an escape, a soft break -- so a document reads back the same.
            if let (crate::Inline::Text(more), Some(crate::Inline::Text(last))) =
                (&inline, inlines.last_mut())
            {
                last.push_str(more);
            } else {
                inlines.push(inline);
            }
        }
        Some(Frame::Code(code)) => {
            if let crate::Inline::Text(text) = inline {
                code.push_str(&text);
            }
        }
        Some(Frame::Image(_, alt)) => {
            if let crate::Inline::Text(text) = inline {
                alt.push_str(&text);
            }
        }
        _ => push_block(stack, crate::Block::Paragraph(vec![inline])),
    }
}

fn push_block(stack: &mut [Frame], block: crate::Block) {
    match stack.last_mut() {
        Some(Frame::Blocks(blocks) | Frame::Quote(blocks)) => blocks.push(block),
        Some(Frame::Item(blocks, loose)) => {
            if !loose.is_empty() {
                blocks.push(crate::Block::Paragraph(std::mem::take(loose)));
            }
            blocks.push(block);
        }
        Some(Frame::List(_, items)) => items.push(vec![block]),
        // A block inside an inline container: close it into the words.
        Some(Frame::Inlines(..) | Frame::Code(_) | Frame::Image(..)) | None => {
            if let Some(Frame::Blocks(blocks)) = stack.first_mut() {
                blocks.push(block);
            }
        }
    }
}

fn close(stack: &mut [Frame], frame: Frame) {
    use crate::{Block, Inline};
    match frame {
        Frame::Inlines(Holder::Paragraph, inlines) => {
            if !inlines.is_empty() {
                push_block(stack, Block::Paragraph(inlines));
            }
        }
        Frame::Inlines(Holder::Heading(level), inlines) => {
            push_block(stack, Block::Heading { level, inlines });
        }
        Frame::Inlines(Holder::Emphasis, inlines) => push_inline(stack, Inline::Emphasis(inlines)),
        Frame::Inlines(Holder::Strong, inlines) => push_inline(stack, Inline::Strong(inlines)),
        Frame::Inlines(Holder::Link(Some(href)), inlines) => {
            push_inline(stack, Inline::Link { href, inlines });
        }
        Frame::Inlines(Holder::Link(None) | Holder::Transparent, inlines) => {
            for inline in inlines {
                push_inline(stack, inline);
            }
        }
        Frame::Image(dest, alt) => {
            let attached = dest.strip_prefix("cid:").and_then(crate::ContentId::parse);
            match attached {
                Some(content_id) => push_inline(stack, Inline::Image { content_id, alt }),
                // Never fetched: the words that were typed, and nothing more.
                None => push_inline(stack, Inline::Text(format!("![{alt}]({dest})"))),
            }
        }
        Frame::Code(code) => push_block(stack, Block::Pre(code)),
        Frame::Quote(blocks) => push_block(stack, Block::Quote(blocks)),
        Frame::List(ordered, items) => push_block(stack, Block::List { ordered, items }),
        Frame::Item(mut blocks, loose) => {
            if !loose.is_empty() {
                blocks.push(Block::Paragraph(loose));
            }
            if let Some(Frame::List(_, items)) = stack.last_mut() {
                items.push(blocks);
            } else {
                for block in blocks {
                    push_block(stack, block);
                }
            }
        }
        Frame::Blocks(blocks) => {
            for block in blocks {
                push_block(stack, block);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RemoteImages, fold_html_quotes, sanitize_body};

    fn read(html: &str) -> String {
        from_html(&fold_html_quotes(
            &sanitize_body(html, RemoteImages::Blocked).html,
        ))
    }

    #[test]
    fn structure_survives() {
        let markdown = read(
            "<h2>Plan</h2><p>Some <b>bold</b> and <a href=\"https://example.com/x\">a link</a>.</p><ul><li>one</li><li>two</li></ul>",
        );
        assert!(markdown.contains("Plan"), "{markdown}");
        assert!(markdown.contains("**bold**"), "{markdown}");
        assert!(
            markdown.contains("[a link](https://example.com/x)"),
            "{markdown}"
        );
        assert!(
            markdown.contains("one") && markdown.contains("two"),
            "{markdown}"
        );
    }

    #[test]
    fn a_blocked_remote_image_leaves_a_placeholder_not_nothing() {
        let markdown = read(
            "<p>Before</p><img src=\"https://tracker.example.org/p.gif\" alt=\"Logo\"><p>After</p>",
        );
        assert!(markdown.contains("![Logo](postio-image:"), "{markdown}");
        assert!(!markdown.contains("tracker.example.org"), "{markdown}");
    }

    #[test]
    fn an_image_with_no_alt_is_still_named() {
        let markdown = read("<img src=\"https://example.org/a.png\">");
        assert!(markdown.contains("![image](postio-image:"), "{markdown}");
    }

    #[test]
    fn a_folded_quote_is_marked_so_it_can_expand() {
        let markdown = read("<p>Thanks!</p><blockquote><p>Earlier words</p></blockquote>");
        let fold = markdown
            .find(FOLD)
            .unwrap_or_else(|| panic!("no fold: {markdown:?}"));
        let end = markdown
            .find(END_FOLD)
            .unwrap_or_else(|| panic!("no end: {markdown:?}"));
        let inside = &markdown[fold..end];
        assert!(inside.contains("Earlier words"), "{markdown:?}");
        assert!(markdown[..fold].contains("Thanks!"), "{markdown:?}");
    }

    #[test]
    fn a_table_is_still_a_table() {
        let markdown = read(
            "<table><tr><th>Item</th><th>Qty</th></tr><tr><td>Widget</td><td>3</td></tr></table>",
        );
        assert!(
            markdown.contains("| Item") && markdown.contains("| Widget"),
            "{markdown}"
        );
    }

    mod writing {
        use super::super::to_document;
        use crate::{Block, HeadingLevel, Inline, render};

        fn text(s: &str) -> Inline {
            Inline::Text(s.to_owned())
        }

        #[test]
        fn words_alone_are_plain_text() {
            let document = to_document("Hello Ada,\n\nThanks for the notes.");
            assert!(document.is_plain_text(), "{document:?}");
            assert_eq!(document.blocks.len(), 2);
        }

        #[test]
        fn emphasis_code_and_a_hard_break() {
            let document = to_document("Some **bold**, *italic* and `code`  \nnext line");
            let Block::Paragraph(inlines) = &document.blocks[0] else {
                panic!("{document:?}")
            };
            assert!(
                inlines.contains(&Inline::Strong(vec![text("bold")])),
                "{inlines:?}"
            );
            assert!(
                inlines.contains(&Inline::Emphasis(vec![text("italic")])),
                "{inlines:?}"
            );
            assert!(
                inlines.contains(&Inline::Code("code".into())),
                "{inlines:?}"
            );
            assert!(inlines.contains(&Inline::Break), "{inlines:?}");
            assert!(!document.is_plain_text());
        }

        #[test]
        fn headings_narrow_to_three_levels() {
            let document = to_document("# One\n\n#### Four");
            assert!(matches!(
                &document.blocks[0],
                Block::Heading {
                    level: HeadingLevel::One,
                    ..
                }
            ));
            assert!(matches!(
                &document.blocks[1],
                Block::Heading {
                    level: HeadingLevel::Three,
                    ..
                }
            ));
        }

        #[test]
        fn lists_nest_and_quotes_and_code_and_rules_map() {
            let document = to_document(
                "- one\n  - inner\n- two\n\n> quoted\n\n```rust\nfn x() {}\n```\n\n---",
            );
            let Block::List {
                ordered: false,
                items,
            } = &document.blocks[0]
            else {
                panic!("{document:?}")
            };
            assert_eq!(items.len(), 2);
            assert!(
                items[0]
                    .iter()
                    .any(|block| matches!(block, Block::List { .. })),
                "{items:?}"
            );
            assert!(
                matches!(&document.blocks[1], Block::Quote(_)),
                "{document:?}"
            );
            assert_eq!(document.blocks[2], Block::Pre("fn x() {}\n".into()));
            assert_eq!(document.blocks[3], Block::Rule);
        }

        #[test]
        fn an_ordered_list_is_ordered() {
            let document = to_document("1. first\n2. second");
            assert!(
                matches!(&document.blocks[0], Block::List { ordered: true, items } if items.len() == 2)
            );
        }

        #[test]
        fn a_web_link_is_a_link_and_a_script_link_is_only_words() {
            let document =
                to_document("[site](https://example.com) and [trap](javascript:alert(1))");
            let Block::Paragraph(inlines) = &document.blocks[0] else {
                panic!("{document:?}")
            };
            assert!(inlines.iter().any(|inline| matches!(inline, Inline::Link { href, .. } if href.as_str() == "https://example.com")), "{inlines:?}");
            let html = render(&document).1;
            assert!(!html.contains("javascript:"), "{html}");
            assert!(html.contains("trap"), "{html}");
        }

        #[test]
        fn an_attached_image_is_an_image_and_a_remote_one_is_never_fetched() {
            let document = to_document(
                "![chart](cid:chart.1@postio.invalid) ![pixel](https://tracker.example.org/p.gif)",
            );
            let Block::Paragraph(inlines) = &document.blocks[0] else {
                panic!("{document:?}")
            };
            assert!(inlines.iter().any(|inline| matches!(inline, Inline::Image { content_id, .. } if content_id.as_str() == "chart.1@postio.invalid")), "{inlines:?}");
            let html = render(&document).1;
            assert!(!html.contains("<img src=\"https"), "{html}");
            assert!(
                html.contains("tracker.example.org/p.gif"),
                "kept as the words written: {html}"
            );
        }

        #[test]
        fn a_table_stays_the_text_that_was_typed() {
            let document = to_document("| a | b |\n|---|---|\n| 1 | 2 |");
            let html = render(&document).1;
            assert!(html.contains("| a | b |"), "{html}");
        }

        #[test]
        fn any_input_is_some_document() {
            for input in [
                "",
                "*",
                "**unclosed",
                "[",
                "![",
                "> > >",
                "- \n-",
                "```",
                "<script>x</script>",
                "\u{0}",
            ] {
                let _ = to_document(input);
            }
        }
    }

    mod reopening {
        use super::super::{from_document, to_document};

        const SAMPLES: &[&str] = &[
            "Hello Ada,\n\nThanks for the notes.",
            "Some **bold**, *italic* and `code`\\\nnext line",
            "# One\n\n## Two\n\n### Three",
            "- one\n  - inner\n- two",
            "1. first\n2. second",
            "> quoted\n>\n> - with a list",
            "```\nfn x() {}\n```",
            "---",
            "[site](https://example.com) and ![chart](cid:chart.1@postio.invalid)",
            "Stars * and _underscores_ and a # and 1. not a list",
            "a `tick`` pair and \\*escaped\\*",
            "**bold *and italic* inside**",
            "| a | b |\n|---|---|",
        ];

        #[test]
        fn a_document_written_as_markdown_reads_back_the_same() {
            for sample in SAMPLES {
                let document = to_document(sample);
                let written = from_document(&document);
                assert_eq!(
                    to_document(&written),
                    document,
                    "{sample:?} wrote {written:?}"
                );
            }
        }

        #[test]
        fn markdown_it_writes_is_what_a_person_would_type() {
            let written = from_document(&to_document(
                "Some **bold** and a [link](https://example.com).",
            ));
            assert_eq!(
                written.trim_end(),
                "Some **bold** and a [link](https://example.com)."
            );
        }
    }
}
