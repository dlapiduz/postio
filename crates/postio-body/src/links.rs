//! The links a reader document offers to `A`, *Open attachment or link*.
//!
//! What the person sees is what is offered: the document handed to the
//! reader, walked for its anchors, each target once, in reading order, with
//! the words it was written behind -- so a link whose words name one place
//! and whose target is another shows both before it opens. GTK reads the
//! same from its laid-out document; a web view's reader asks this.

use html5ever::driver::ParseOpts;
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// One link a reader document offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The words it was written behind, its whitespace collapsed; the
    /// target itself when it was written behind none (an image, say).
    pub words: String,
    /// Where it goes, in full.
    pub target: String,
}

/// The schemes a link may open outside the reader: the same three the
/// reader's own renderer opens (`postio_render::LinkTarget::External`).
const SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// Every `http`, `https` and `mailto` link in `html`, in reading order, each
/// target once (the first words it was written behind are the ones kept).
/// Anything else -- the reader's own verbs, a fragment, a script -- is not a
/// place to open.
pub fn links(html: &str) -> Vec<Link> {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let mut found = Vec::new();
    walk(&dom.document, &mut found);
    found
}

fn walk(node: &Handle, found: &mut Vec<Link>) {
    if let NodeData::Element { name, attrs, .. } = &node.data {
        match &*name.local {
            // Not part of what is read.
            "head" | "script" | "style" | "template" => return,
            "a" => {
                let href = attrs
                    .borrow()
                    .iter()
                    .find(|attr| &*attr.name.local == "href")
                    .map(|attr| attr.value.to_string());
                if let Some(target) = href.as_deref().and_then(target) {
                    if !found.iter().any(|link| link.target == target) {
                        let words = words(node);
                        found.push(Link {
                            words: if words.is_empty() {
                                target.clone()
                            } else {
                                words
                            },
                            target,
                        });
                    }
                    // A link inside a link is not one a person can reach.
                    return;
                }
            }
            _ => {}
        }
    }
    for child in node.children.borrow().iter() {
        walk(child, found);
    }
}

/// `href` as the place it opens, when it is one of [`SCHEMES`].
fn target(href: &str) -> Option<String> {
    let url = ammonia::Url::parse(href.trim()).ok()?;
    SCHEMES.contains(&url.scheme()).then(|| url.to_string())
}

/// The text under `node`, its whitespace collapsed to single spaces.
fn words(node: &Handle) -> String {
    let mut text = String::new();
    collect(node, &mut text);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Elements whose text is words apart from what is around them.
const BLOCKS: [&str; 15] = [
    "br",
    "p",
    "div",
    "li",
    "td",
    "th",
    "tr",
    "table",
    "blockquote",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
];

fn collect(node: &Handle, text: &mut String) {
    match &node.data {
        NodeData::Text { contents } => text.push_str(&contents.borrow()),
        NodeData::Element { name, .. } => {
            let local = &*name.local;
            if matches!(local, "script" | "style") {
                return;
            }
            let block = BLOCKS.contains(&local);
            if block {
                text.push(' ');
            }
            for child in node.children.borrow().iter() {
                collect(child, text);
            }
            if block {
                text.push(' ');
            }
        }
        _ => {
            for child in node.children.borrow().iter() {
                collect(child, text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(html: &str) -> Vec<(String, String)> {
        links(html)
            .into_iter()
            .map(|link| (link.words, link.target))
            .collect()
    }

    #[test]
    fn a_link_is_offered_with_its_words_and_its_whole_target() {
        assert_eq!(
            said(r#"<p>See <a href="https://example.com/report?q=1">the  <b>re</b>port</a>.</p>"#),
            [(
                "the report".to_owned(),
                "https://example.com/report?q=1".to_owned()
            )]
        );
    }

    #[test]
    fn words_that_name_another_place_are_shown_beside_where_it_goes() {
        let offered = said(r#"<a href="https://example.net/login">https://example.com</a>"#);
        assert_eq!(offered[0].0, "https://example.com");
        assert_eq!(offered[0].1, "https://example.net/login");
    }

    #[test]
    fn each_target_once_in_reading_order() {
        let offered = said(concat!(
            r#"<p><a href="mailto:ada@example.com">Ada</a> "#,
            r#"<a href="https://example.com/b">b</a> "#,
            r#"<a href="mailto:ada@example.com">write to Ada</a></p>"#,
        ));
        let targets: Vec<&str> = offered.iter().map(|(_, target)| target.as_str()).collect();
        assert_eq!(targets, ["mailto:ada@example.com", "https://example.com/b"]);
        assert_eq!(offered[0].0, "Ada", "the first words are kept");
    }

    #[test]
    fn only_places_that_open_outside_the_reader_are_offered() {
        let offered = said(concat!(
            r##"<a href="#section">a fragment</a>"##,
            r#"<a href="postio-verb:unsubscribe">a verb</a>"#,
            r#"<a href="javascript:alert(1)">a script</a>"#,
            r#"<a>no target</a>"#,
            r#"<a href="http://example.com/">plain http</a>"#,
        ));
        assert_eq!(
            offered,
            [("plain http".to_owned(), "http://example.com/".to_owned())]
        );
    }

    #[test]
    fn a_link_with_no_words_is_offered_as_its_target() {
        let offered = said(r#"<a href="https://example.com/x"><img alt="" src="cid:a"></a>"#);
        assert_eq!(offered[0].0, "https://example.com/x");
    }

    #[test]
    fn a_page_with_no_links_offers_none() {
        assert!(links("<p>No links here.</p>").is_empty());
        assert!(links("").is_empty());
    }
}
