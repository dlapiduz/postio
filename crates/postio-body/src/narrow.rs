//! What was lost when pasted markup was narrowed to the `Document`, counted in categories a person
//! recognises.
//! `parse` is silent, which suits quoting but not a paste; this asks `parse`'s own `block_for` and
//! `inline_for`, so it cannot drift from the dialect.

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::{LocalName, QualName, ns, parse_fragment};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::document::Document;
use crate::parse::{DROPPED, block_for, inline_for, name_of};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Narrowed {
    pub document: Document,
    pub lost: Lost,
}

/// Counts, not element names: "3 images" is what a person needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lost {
    /// Its own category because people notice it: a remote image cannot be carried into a message
    /// without fetching it.
    pub images: usize,
    pub tables: usize,
    pub styling: usize,
    pub links: usize,
    pub removed: usize,
}

impl Lost {
    pub fn is_empty(&self) -> bool {
        *self == Lost::default()
    }

    /// One sentence for a composer to show, or `None` when nothing was lost; shared so both
    /// composers say the same.
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
        if self.styling > 0 {
            parts.push("formatting Postio does not send".to_owned());
        }
        if parts.is_empty() {
            return None;
        }
        Some(format!("Pasted without {}.", join(&parts)))
    }
}

fn join(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The document is `parse`'s, unchanged; this adds a report, not a second parser.
pub fn narrow(html: &str) -> Narrowed {
    Narrowed {
        document: crate::parse::parse(html),
        lost: lost_in(html),
    }
}

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
        // Removed with its contents, so the subtree is not walked: a `<script>` is one loss.
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

fn classify(node: &Handle, name: &str, lost: &mut Lost) {
    // Counted even when the element survives: `<p style="color:red">` keeps the paragraph, not the
    // colour.
    if carries_styling(node, name) {
        lost.styling += 1;
    }

    if block_for(node, name).is_some() || inline_for(node, name).is_some() {
        return;
    }

    match name {
        // One table is one loss, not one per cell.
        "table" => lost.tables += 1,
        "img" => lost.images += 1,
        "a" => lost.links += 1,
        _ => {}
    }
}

fn carries_styling(node: &Handle, name: &str) -> bool {
    if name == "font" {
        return true;
    }
    let NodeData::Element { attrs, .. } = &node.data else {
        return false;
    };
    attrs.borrow().iter().any(|attribute| {
        matches!(
            &*attribute.name.local,
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
        let lost =
            narrow("<table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>")
                .lost;
        assert_eq!(lost.tables, 1);
    }

    #[test]
    fn a_remote_image_is_counted_and_a_part_of_this_message_is_not() {
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
        // A report counting only discarded elements would call this clean while the colour
        // vanishes.
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
        let html = "<div>a<table><tr><td>b</td></tr></table><script>c</script></div>";
        assert_eq!(narrow(html).document, crate::parse::parse(html));
    }
}
