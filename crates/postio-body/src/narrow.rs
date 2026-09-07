//! What was lost when pasted markup was narrowed to the [`Document`].
//!
//! [`crate::parse`] is total and silent: anything outside the subset is
//! unwrapped to its text or dropped, and the caller gets a clean document
//! with no record of the difference. That is the right contract for the
//! quoting path, where the input is a message somebody else wrote and there
//! is nobody to tell.
//!
//! It is the wrong contract for a **paste**. Someone who pastes a styled
//! table out of a browser and gets four lines of plain text has had work
//! taken away from them, and a composer that says nothing looks broken
//! rather than principled. So this walks the same markup a second time and
//! counts what the dialect could not hold, in categories a person would
//! recognise.
//!
//! # It asks the parser rather than keeping its own list
//!
//! The obvious implementation is a second table of "supported elements",
//! and it would be wrong within a release: the dialect would grow a tag and
//! this would go on reporting it as lost. So every question here is put to
//! [`crate::parse`]'s own [`block_for`] and [`inline_for`]. An element the
//! parser can build something from is kept, by definition, and the two
//! cannot disagree about which those are.
//!
//! [`Document`]: crate::Document
//! [`block_for`]: crate::parse::block_for
//! [`inline_for`]: crate::parse::inline_for

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::{LocalName, QualName, ns, parse_fragment};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::document::Document;
use crate::parse::{DROPPED, block_for, inline_for, name_of};

/// A narrowed document, and what narrowing it cost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Narrowed {
    /// The document, exactly as [`crate::parse`] would have produced it.
    pub document: Document,
    /// What the dialect could not hold.
    pub lost: Lost,
}

/// What a paste lost, by category.
///
/// Counts rather than a list of elements: "3 images" is what a person needs
/// to decide whether to care, and a list of tag names is what a developer
/// needs, which is not who is reading the composer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lost {
    /// `<img>` that named something other than a part of this message.
    ///
    /// Its own category because it is the loss people notice: a remote image
    /// cannot be carried into a message being written here without fetching
    /// it, which is the one thing the reader spends its whole design
    /// refusing.
    pub images: usize,
    /// `<table>` — kept as its text, laid out as nothing.
    pub tables: usize,
    /// Colours, fonts and sizes: `style=`, `<font>`, `bgcolor`.
    pub styling: usize,
    /// `<a>` pointing somewhere a message may not link to.
    pub links: usize,
    /// Elements removed with their contents — script, style, iframe and the
    /// rest of [`DROPPED`].
    pub removed: usize,
}

impl Lost {
    /// Whether the paste survived whole.
    pub fn is_empty(&self) -> bool {
        *self == Lost::default()
    }

    /// One sentence a composer can put on screen, or `None` when nothing
    /// was lost.
    ///
    /// Written here rather than in a frontend so both composers say the same
    /// thing about the same paste — the reason every other user-facing
    /// string in this workspace is shared.
    pub fn summary(&self) -> Option<String> {
        let mut parts = Vec::new();
        let mut say = |count: usize, one: &str, many: &str| {
            if count == 1 {
                parts.push(format!("1 {one}"));
            } else if count > 1 {
                parts.push(format!("{count} {many}"));
            }
        };
        say(self.images, "image", "images");
        say(self.tables, "table", "tables");
        say(self.links, "link", "links");
        say(self.removed, "embedded element", "embedded elements");
        // Last, and phrased as a mass noun: nobody counts style attributes,
        // they notice that the colours went.
        if self.styling > 0 {
            parts.push("formatting Postio does not send".to_owned());
        }
        if parts.is_empty() {
            return None;
        }
        Some(format!("Pasted without {}.", join(&parts)))
    }
}

/// `a`, `a and b`, `a, b and c`.
fn join(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Narrow `html`, and say what that cost.
///
/// The document is [`crate::parse`]'s, unchanged — this adds a report, it
/// does not add a second parser.
pub fn narrow(html: &str) -> Narrowed {
    Narrowed {
        document: crate::parse::parse(html),
        lost: lost_in(html),
    }
}

/// Count what the dialect cannot hold.
fn lost_in(html: &str) -> Lost {
    let dom = parse_fragment(
        RcDom::default(),
        ParseOpts::default(),
        QualName::new(None, ns!(html), LocalName::from("body")),
        Vec::new(),
        false,
    )
    .one(html);

    let mut lost = Lost::default();
    count(&dom.document, &mut lost);
    lost
}

fn count(node: &Handle, lost: &mut Lost) {
    if let Some(name) = name_of(node) {
        // Removed with its contents: nothing below it survives, so nothing
        // below it is walked. Counting the subtree would report a `<script>`
        // as five losses.
        if DROPPED.contains(&name.as_str()) {
            lost.removed += 1;
            return;
        }
        classify(node, &name, lost);
    }
    for child in node.children.borrow().iter() {
        count(child, lost);
    }
}

/// Whether this element survives, and what it costs when it does not.
fn classify(node: &Handle, name: &str, lost: &mut Lost) {
    // Styling is counted whether or not the element itself survives: a
    // `<p style="color:red">` is kept *as a paragraph*, and the colour is
    // still gone.
    if carries_styling(node, name) {
        lost.styling += 1;
    }

    // The parser's own answer, so this cannot drift from the dialect.
    if block_for(node, name).is_some() || inline_for(node, name).is_some() {
        return;
    }

    match name {
        // Counted even though `<td>` and `<tr>` are unwrapped below it: one
        // table is one loss, not one per cell.
        "table" => lost.tables += 1,
        "img" => lost.images += 1,
        // An `<a>` the parser refused is one whose href is not http, https
        // or mailto -- `javascript:`, `file:`, a bare relative path. The
        // text stays; what goes is the link.
        "a" => lost.links += 1,
        // Everything else unwraps to its text, which is the author's words
        // arriving intact. A `<div>` is not a loss.
        _ => {}
    }
}

/// Whether this element carries colour, font or size the dialect drops.
fn carries_styling(node: &Handle, name: &str) -> bool {
    if name == "font" {
        return true;
    }
    let NodeData::Element { attrs, .. } = &node.data else {
        return false;
    };
    attrs.borrow().iter().any(|attribute| {
        matches!(
            attribute.name.local.as_ref(),
            "style" | "bgcolor" | "color" | "face" | "size"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paste_the_dialect_can_hold_whole_loses_nothing() {
        let narrowed = narrow("<p>Hello <strong>there</strong>, <em>friend</em>.</p>");
        assert!(narrowed.lost.is_empty());
        assert_eq!(narrowed.lost.summary(), None);
    }

    #[test]
    fn a_table_is_one_loss_not_one_per_cell() {
        // The count is what a person reads. "12 tables" for one table with
        // twelve cells would be worse than saying nothing.
        let lost =
            narrow("<table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>")
                .lost;
        assert_eq!(lost.tables, 1);
    }

    #[test]
    fn a_remote_image_is_counted_and_a_part_of_this_message_is_not() {
        // The one people notice, and the distinction that matters: an image
        // this message carries survives, a remote one cannot.
        assert_eq!(
            narrow("<img src='https://example.com/a.png'>").lost.images,
            1
        );
        assert_eq!(
            narrow("<img src='cid:a@example.com' alt=''>").lost.images,
            0
        );
    }

    #[test]
    fn a_link_the_dialect_refuses_is_counted_and_its_text_survives() {
        let narrowed = narrow("<p><a href='javascript:alert(1)'>click</a></p>");
        assert_eq!(narrowed.lost.links, 1);
        // The words are the author's and they stay; only the link goes.
        assert!(format!("{:?}", narrowed.document).contains("click"));
    }

    #[test]
    fn an_ordinary_link_is_not_a_loss() {
        assert_eq!(
            narrow("<p><a href='https://example.com'>hi</a></p>")
                .lost
                .links,
            0
        );
    }

    #[test]
    fn a_script_is_one_removal_and_its_contents_are_not_counted_again() {
        let lost = narrow("<p>hi</p><script>var a = 1; var b = 2;</script>").lost;
        assert_eq!(lost.removed, 1);
    }

    #[test]
    fn styling_is_counted_even_on_an_element_that_survives() {
        // The paragraph is kept; the colour is not. A report that only
        // counted *discarded elements* would call this a clean paste, and
        // the person would watch their formatting vanish unremarked.
        let lost = narrow("<p style='color: red'>red</p>").lost;
        assert_eq!(lost.styling, 1);
        assert!(!lost.is_empty());
    }

    #[test]
    fn a_div_is_not_a_loss_because_its_text_is_the_authors() {
        assert!(narrow("<div>just words</div>").lost.is_empty());
    }

    #[test]
    fn the_summary_reads_as_a_sentence_rather_than_a_tally() {
        let lost = Lost {
            images: 2,
            tables: 1,
            styling: 4,
            links: 0,
            removed: 0,
        };
        assert_eq!(
            lost.summary().as_deref(),
            Some("Pasted without 2 images, 1 table and formatting Postio does not send.")
        );
    }

    #[test]
    fn one_of_something_is_singular() {
        let lost = Lost {
            images: 1,
            ..Lost::default()
        };
        assert_eq!(lost.summary().as_deref(), Some("Pasted without 1 image."));
    }

    #[test]
    fn the_document_is_exactly_what_parse_would_have_produced() {
        // This module adds a report; it must not add a second parser, or the
        // thing the composer shows and the thing it saves could differ.
        let html = "<div>a<table><tr><td>b</td></tr></table><script>c</script></div>";
        assert_eq!(narrow(html).document, crate::parse::parse(html));
    }
}
