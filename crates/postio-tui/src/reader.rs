//! A message, as lines a terminal can draw.
//!
//! The body arrives from the host as the store holds it. The reader's own
//! rules -- reader view or original, the sanitiser, quote folding -- are
//! `postio_ui::reader::document`'s and `postio_body`'s, the same ones the
//! desktop and macOS readers apply; then `postio_body::markdown::from_html`,
//! and `tui-markdown` to style it (research R5).
//!
//! Everything from the message is made [`SafeText`] before it is styled, so
//! no line here can carry a control sequence to the terminal.

use postio_body::markdown::{END_FOLD, FOLD, IMAGE_SCHEME};
use postio_ui::terminal::SafeText;
use ratatui::text::Line;

/// One block of a rendered message.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Lines that are always shown.
    Lines(Vec<Line<'static>>),
    /// Quoted history: folded unless expanded.
    Fold {
        /// Whether it is folded.
        folded: bool,
        /// Its lines, when expanded.
        lines: Vec<Line<'static>>,
    },
}

/// A message's body as blocks of lines.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rendered {
    /// The blocks, top to bottom.
    pub blocks: Vec<Block>,
    /// Every link's destination, in the order they appear.
    ///
    /// `tui-markdown` already draws a link as `text (destination)`, so what a
    /// link leads to is on screen, in full, before anything follows it
    /// (FR-014). This list is what the mouse opens from; nothing here opens
    /// anything by itself.
    pub links: Vec<SafeText>,
}

impl Rendered {
    /// The lines to draw, with folds as they stand.
    pub fn lines(&self) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        for block in &self.blocks {
            match block {
                Block::Lines(lines) => out.extend(lines.iter().cloned()),
                Block::Fold {
                    folded: true,
                    lines,
                } => out.push(Line::raw(format!("▸ quoted text ({} lines)", lines.len()))),
                Block::Fold {
                    folded: false,
                    lines,
                } => {
                    out.push(Line::raw("▾ quoted text"));
                    out.extend(lines.iter().cloned());
                }
            }
        }
        out
    }
}

/// What one drawn line of a message stands for, for a click on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineTarget {
    /// Nothing to act on.
    Text,
    /// A fold marker: which block it folds.
    Fold(usize),
    /// A link: which of [`Rendered::links`].
    Link(usize),
    /// An image's placeholder.
    Placeholder,
}

impl Rendered {
    /// What each line of [`Rendered::lines`] stands for, line for line.
    pub fn targets(&self) -> Vec<LineTarget> {
        let of = |line: &Line<'static>| {
            let text = line.to_string();
            if let Some(index) = self
                .links
                .iter()
                .position(|link| text.contains(link.as_str()))
            {
                LineTarget::Link(index)
            } else if text.contains("[image:") {
                LineTarget::Placeholder
            } else {
                LineTarget::Text
            }
        };
        let mut out = Vec::new();
        for (index, block) in self.blocks.iter().enumerate() {
            match block {
                Block::Lines(lines) => out.extend(lines.iter().map(of)),
                Block::Fold { folded: true, .. } => out.push(LineTarget::Fold(index)),
                Block::Fold {
                    folded: false,
                    lines,
                } => {
                    out.push(LineTarget::Fold(index));
                    out.extend(lines.iter().map(of));
                }
            }
        }
        out
    }

    /// Fold or unfold block `index`.
    pub fn toggle_fold(&mut self, index: usize) {
        if let Some(Block::Fold { folded, .. }) = self.blocks.get_mut(index) {
            *folded = !*folded;
        }
    }
}

/// Sanitised, folded HTML as a rendered message.
pub fn from_html(html: &str) -> Rendered {
    let (markdown, links) = links_in(&placeholders(&postio_body::markdown::from_html(html)));
    // Everything from the message, made safe before it is styled: every span
    // `tui-markdown` makes is a slice of this.
    let safe = SafeText::new(&markdown);

    let mut blocks = Vec::new();
    let mut open = String::new();
    let mut quoted = String::new();
    // Folds inside a fold join it: one level of "show quoted text" is what a
    // person can act on, and the inner markers carry nothing of their own.
    let mut depth = 0usize;
    for line in safe.as_str().split('\n') {
        match line.trim() {
            FOLD => {
                if depth == 0 && !open.trim().is_empty() {
                    blocks.push(Block::Lines(styled(&open)));
                    open.clear();
                }
                depth += 1;
            }
            END_FOLD if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    blocks.push(Block::Fold {
                        folded: true,
                        lines: styled(&quoted),
                    });
                    quoted.clear();
                }
            }
            _ => {
                let into = if depth > 0 { &mut quoted } else { &mut open };
                into.push_str(line);
                into.push('\n');
            }
        }
    }
    if !quoted.trim().is_empty() {
        blocks.push(Block::Fold {
            folded: true,
            lines: styled(&quoted),
        });
    }
    if !open.trim().is_empty() {
        blocks.push(Block::Lines(styled(&open)));
    }
    Rendered {
        blocks,
        links: links.iter().map(|link| SafeText::new(link)).collect(),
    }
}

/// A plain-text message as it was written.
///
/// Never parsed as Markdown: a line that begins `#` is the sender's `#`, not a
/// heading (US2 scenario 3). Each run of `>` lines is quoted history, folded
/// as the HTML reader folds a blockquote.
pub fn from_text(text: &str) -> Rendered {
    let safe = SafeText::new(text);
    let mut blocks = Vec::new();
    let mut open: Vec<Line<'static>> = Vec::new();
    let mut quoted: Vec<Line<'static>> = Vec::new();
    for line in safe.as_str().lines() {
        if line.trim_start().starts_with('>') {
            if !open.is_empty() {
                blocks.push(Block::Lines(std::mem::take(&mut open)));
            }
            quoted.push(Line::raw(line.to_owned()));
        } else {
            if !quoted.is_empty() {
                blocks.push(Block::Fold {
                    folded: true,
                    lines: std::mem::take(&mut quoted),
                });
            }
            open.push(Line::raw(line.to_owned()));
        }
    }
    if !quoted.is_empty() {
        blocks.push(Block::Fold {
            folded: true,
            lines: quoted,
        });
    }
    if !open.is_empty() {
        blocks.push(Block::Lines(open));
    }
    Rendered {
        blocks,
        links: Vec::new(),
    }
}

/// The Markdown unchanged, and every link's destination in order.
fn links_in(markdown: &str) -> (String, Vec<String>) {
    use pulldown_cmark::{Event, Parser, Tag};
    let links = Parser::new(markdown)
        .filter_map(|event| match event {
            Event::Start(Tag::Link { dest_url, .. }) => Some(dest_url.into_string()),
            _ => None,
        })
        .collect();
    (markdown.to_owned(), links)
}

/// Markdown as styled lines that own their text.
fn styled(markdown: &str) -> Vec<Line<'static>> {
    tui_markdown::from_str(markdown)
        .lines
        .into_iter()
        .map(|line| {
            let style = line.style;
            Line::from(
                line.spans
                    .into_iter()
                    .map(|span| ratatui::text::Span::styled(span.content.into_owned(), span.style))
                    .collect::<Vec<_>>(),
            )
            .style(style)
        })
        .collect()
}

/// `![alt](postio-image:N)` as the words `[image: alt]`, escaped so Markdown
/// draws the brackets rather than reading a link.
fn placeholders(markdown: &str) -> String {
    let target = format!("]({IMAGE_SCHEME}:");
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(start) = rest.find("![") {
        // A placeholder never spans lines; an ordinary `![` must not pair with
        // a later image's target and swallow the words between them.
        let line_end = rest[start..].find('\n').map_or(rest.len(), |at| start + at);
        let (Some(close), Some(end)) = (
            rest[start..line_end].find(&target),
            rest[start..line_end].rfind(')'),
        ) else {
            out.push_str(&rest[..start + 2]);
            rest = &rest[start + 2..];
            continue;
        };
        let end = end - close;
        out.push_str(&rest[..start]);
        let alt = &rest[start + 2..start + close];
        out.push_str(&format!("\\[image: {alt}\\]"));
        rest = &rest[start + close + end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_body::{RemoteImages, fold_html_quotes, sanitize_body};

    fn text(rendered: &Rendered) -> String {
        rendered
            .lines()
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn markdown_structure_is_drawn_without_its_markup() {
        let rendered =
            from_html("<h2>Plan</h2><p>Some <b>bold</b> words.</p><ul><li>one</li></ul>");
        let drawn = text(&rendered);
        assert!(
            drawn.contains("Plan") && drawn.contains("bold") && drawn.contains("one"),
            "{drawn}"
        );
        assert!(
            !drawn.contains("**"),
            "the markup is styling, not text: {drawn}"
        );
    }

    #[test]
    fn an_image_is_a_labelled_placeholder() {
        let html = sanitize_body(
            "<p>Hi</p><img src=\"https://example.org/x.png\" alt=\"Logo\">",
            RemoteImages::Blocked,
        )
        .html;
        let drawn = text(&from_html(&html));
        assert!(drawn.contains("[image: Logo]"), "{drawn}");
        assert!(!drawn.contains("postio-image"), "{drawn}");
    }

    #[test]
    fn a_literal_exclamation_bracket_does_not_swallow_the_text_after_it() {
        let rendered = placeholders("Wow![sic]\nlater ![Logo](postio-image:0) end");
        assert!(rendered.starts_with("Wow![sic]\n"), "{rendered}");
        assert!(
            rendered.contains(r"\[image: Logo\]"),
            "escaped for Markdown: {rendered}"
        );
        assert!(rendered.ends_with(" end"), "{rendered}");
    }

    #[test]
    fn plain_text_is_shown_as_written_not_read_as_markdown() {
        let drawn = text(&from_text("# not a heading\n**not bold**\nplain"));
        assert_eq!(drawn, "# not a heading\n**not bold**\nplain");
    }

    #[test]
    fn a_run_of_quoted_lines_folds() {
        let rendered =
            from_text("Sounds good.\n\n> On Monday you wrote:\n> the old words\n\nThanks");
        let drawn = text(&rendered);
        assert!(
            drawn.contains("Sounds good.") && drawn.contains("Thanks"),
            "{drawn}"
        );
        assert!(drawn.contains("▸ quoted text (2 lines)"), "{drawn}");
        assert!(!drawn.contains("the old words"), "{drawn}");
    }

    #[test]
    fn plain_text_is_made_safe_too() {
        let drawn = text(&from_text("title\u{1b}]0;pwned\u{7}"));
        assert!(
            !drawn.contains('\u{1b}') && !drawn.contains('\u{7}'),
            "{drawn:?}"
        );
    }

    #[test]
    fn a_link_shows_where_it_goes_before_anything_follows_it() {
        let html = sanitize_body(
            "<p>See <a href=\"https://example.com/invoice/42\">your invoice</a> and <a href=\"https://example.org/\">this</a>.</p>",
            RemoteImages::Blocked,
        )
        .html;
        let rendered = from_html(&html);
        let drawn = text(&rendered);
        assert!(
            drawn.contains("your invoice (https://example.com/invoice/42)"),
            "{drawn}"
        );
        assert!(drawn.contains("this (https://example.org/)"), "{drawn}");
        let links: Vec<&str> = rendered.links.iter().map(SafeText::as_str).collect();
        assert_eq!(
            links,
            ["https://example.com/invoice/42", "https://example.org/"]
        );
    }

    #[test]
    fn quoted_history_is_folded_until_expanded() {
        let html = fold_html_quotes(
            &sanitize_body(
                "<p>Thanks!</p><blockquote><p>Earlier words</p></blockquote>",
                RemoteImages::Blocked,
            )
            .html,
        );
        let rendered = from_html(&html);
        let drawn = text(&rendered);
        assert!(drawn.contains("Thanks!"), "{drawn}");
        assert!(drawn.contains("▸ quoted text"), "{drawn}");
        assert!(!drawn.contains("Earlier words"), "folded: {drawn}");
    }

    #[test]
    fn no_corpus_message_reaches_the_terminal_with_markup_or_a_control_character() {
        // SC-005, on what the terminal would actually draw.
        let mut checked = 0;
        for fixture in postio_model::test_corpus::all() {
            let message = fixture.parse();
            let Some(html) = message.body.html.as_deref() else {
                continue;
            };
            let folded = fold_html_quotes(&sanitize_body(html, RemoteImages::Blocked).html);
            let mut rendered = from_html(&folded);
            for block in &mut rendered.blocks {
                if let Block::Fold { folded, .. } = block {
                    *folded = false;
                }
            }
            let drawn = text(&rendered);
            for forbidden in [
                "<script",
                "<div",
                "<img",
                "<table",
                "<style",
                "javascript:",
                "postio-image",
            ] {
                assert!(
                    !drawn.to_ascii_lowercase().contains(forbidden),
                    "{forbidden} in {}",
                    fixture.name()
                );
            }
            if let Some(c) = drawn
                .chars()
                .find(|c| (c.is_control() && *c != '\n') || ('\u{202a}'..='\u{202e}').contains(c))
            {
                panic!("{c:?} from {} reached the terminal", fixture.name());
            }
            checked += 1;
        }
        assert!(checked >= 5, "only {checked} HTML messages were drawn");
    }
}
