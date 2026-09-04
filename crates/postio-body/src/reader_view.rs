//! Reader view: what bulk mail looks like when the sender stops designing it.
//!
//! Marketing and transactional HTML renders as a wall of blue underlined
//! links against a dark theme, because the sender laid it out for a white
//! page in nested tables with their own colours, fonts and widths. Postio's
//! sanitizer already drops `<style>` and the `style` attribute
//! ([`crate::sanitize`]) — what survives is still a *layout*: tables that
//! were columns, spacer images that were gutters, and thirty links where a
//! person needed one.
//!
//! Reader view goes further and reduces the markup to the handful of tags
//! that carry meaning rather than arrangement. The sender's original stays
//! one keystroke away, rendered untouched on a sheet of its own — never
//! repainted in Postio's palette, which is how other clients turn a
//! white-background logo into an unreadable smear.
//!
//! # Everything here is a rule over a string
//!
//! No toolkit, no document template, no policy about what the reader *does*
//! with the answer. That belongs to `postio_ui::reader::document`, which is
//! also where the plain-part preference lives — it needs the whole
//! [`MessageBody`], and this module only ever sees markup.
//!
//! [`MessageBody`]: postio_model::message::MessageBody

use std::collections::HashSet;

use html5ever::driver::ParseOpts;
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// The tags reader view keeps.
///
/// Meaning, not arrangement: emphasis, code, lists, quotes, paragraphs,
/// breaks and links. Everything else is unwrapped — its children survive,
/// its own box does not — so a sentence inside six nested `<td>`s comes out
/// as a sentence rather than disappearing with the table.
const KEPT: [&str; 11] = [
    "b",
    "strong",
    "i",
    "em",
    "code",
    "a",
    "ul",
    "ol",
    "li",
    "blockquote",
    "p",
];

/// The one void element worth keeping: a line break carries meaning that
/// nothing else does.
const KEPT_VOID: [&str; 1] = ["br"];

/// The attributes that survive on a kept tag.
///
/// `href` and nothing else. Not `style` (the sanitizer drops it already, and
/// this is defence in depth), not `width`, not `bgcolor`, not `class` — a
/// sender's class names mean nothing here and a sender's `class="dark"`
/// meeting Postio's own stylesheet is exactly the collision reader view
/// exists to end.
const KEPT_ATTRIBUTES: [&str; 1] = ["href"];

/// What reduction produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reduced {
    /// The reduced markup.
    pub html: String,
    /// How many links survived — the calls to action.
    pub links_kept: usize,
    /// How many were collapsed away.
    ///
    /// The canvas draws this as `1 link kept of 23`, which is
    /// `links_kept` of `links_kept + links_dropped`. A number, because
    /// "some links were hidden" is the kind of sentence that makes people
    /// distrust a reader.
    pub links_dropped: usize,
}

impl Reduced {
    /// Every link the message had, kept or not.
    pub fn links_total(&self) -> usize {
        self.links_kept + self.links_dropped
    }
}

/// How many links reader view keeps.
///
/// One. A marketing message has one thing it wants you to do and twenty-two
/// ways to say it — the header logo, the footer, the social icons, the
/// unsubscribe — and keeping "the ones that look important" is a judgement
/// this cannot make. Keeping exactly the first is a rule a person can learn:
/// the primary call to action is what a sender puts first in the body.
pub const LINKS_KEPT: usize = 1;

/// Reduce sender markup to what carries meaning.
///
/// Expects markup that has already been through [`crate::sanitize`] — this
/// is a readability pass, not a security one, and running it on raw sender
/// HTML would be relying on it for something it does not promise.
pub fn reduce(html: &str) -> Reduced {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let kept: HashSet<&str> = KEPT.into_iter().collect();
    let void: HashSet<&str> = KEPT_VOID.into_iter().collect();
    let attributes: HashSet<&str> = KEPT_ATTRIBUTES.into_iter().collect();

    let mut out = String::new();
    let mut links = Links::default();
    walk(
        &dom.document,
        &kept,
        &void,
        &attributes,
        &mut links,
        &mut out,
    );

    Reduced {
        html: out.trim().to_owned(),
        links_kept: links.kept,
        links_dropped: links.dropped,
    }
}

/// How many anchors have survived so far, and how many have not.
///
/// Threaded through the walk rather than counted afterwards, because "which
/// link is the first one" is a fact about document order and the walk is
/// what knows it.
#[derive(Default)]
struct Links {
    kept: usize,
    dropped: usize,
}

/// Emit `node` and its children into `out`, keeping only what carries
/// meaning.
///
/// An unkept element is **unwrapped, not deleted**: its children are still
/// visited, so a sentence inside six nested `<td>`s survives its tables. The
/// exception is a `<script>`/`<style>` subtree, which is content nobody is
/// meant to read — the sanitizer removes those already, and doing it here
/// too costs one comparison and means this function is not wrong when
/// somebody calls it on raw markup.
fn walk(
    node: &Handle,
    kept: &HashSet<&str>,
    void: &HashSet<&str>,
    attributes: &HashSet<&str>,
    links: &mut Links,
    out: &mut String,
) {
    match &node.data {
        NodeData::Text { contents } => {
            escape_into(&contents.borrow(), out);
            return;
        }
        NodeData::Element { name, attrs, .. } => {
            let tag = name.local.to_string();
            if matches!(tag.as_str(), "script" | "style" | "head" | "title") {
                return;
            }
            if void.contains(tag.as_str()) {
                out.push_str("<br>");
                return;
            }
            // An anchor past the budget keeps its words and loses its href:
            // the text was a sentence before it was a link, and dropping it
            // whole is what makes a reduced newsletter read as a list of
            // gaps.
            let anchor = tag == "a";
            let keep_anchor = anchor && links.kept < LINKS_KEPT;
            if anchor && !keep_anchor {
                links.dropped += 1;
            } else if anchor {
                links.kept += 1;
            }

            if kept.contains(tag.as_str()) && (!anchor || keep_anchor) {
                out.push('<');
                out.push_str(&tag);
                for attribute in attrs.borrow().iter() {
                    let name = attribute.name.local.to_string();
                    if !attributes.contains(name.as_str()) {
                        continue;
                    }
                    out.push(' ');
                    out.push_str(&name);
                    out.push_str("=\"");
                    escape_into(&attribute.value, out);
                    out.push('"');
                }
                out.push('>');
                for child in node.children.borrow().iter() {
                    walk(child, kept, void, attributes, links, out);
                }
                out.push_str("</");
                out.push_str(&tag);
                out.push('>');
                return;
            }
        }
        _ => {}
    }
    for child in node.children.borrow().iter() {
        walk(child, kept, void, attributes, links, out);
    }
}

/// Text into markup, with the four characters that would otherwise be
/// markup.
///
/// Written out rather than reached for from a crate: this is the *output*
/// side of a reduction whose whole point is that nothing a sender wrote is
/// interpreted, and an escape that is one dependency away from being wrong
/// is worth having in front of you.
fn escape_into(text: &str, out: &mut String) {
    for character in text.chars() {
        match character {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
}

/// Whether a body reads like bulk mail, and so should open in reader view.
///
/// A heuristic, and the honest word for it. Three signals, all of which are
/// about *arrangement* rather than content, because that is the difference
/// between a newsletter and a reply:
///
/// * **nested layout tables** — a person writing mail does not put a table
///   inside a table; a template does it to make columns.
/// * **many links** — a reply has a few, a campaign has dozens.
///
/// **Not styling**, though that was the obvious third one and this function
/// counted it first. It cannot work: [`crate::sanitize`] removes `<style>`
/// tag-and-contents and `style` is not in ammonia's attribute allow-list, so
/// by the time reader view sees the markup every style signal is already
/// zero. Counting it made the heuristic *look* careful — three signals,
/// two required — while quietly needing both of the other two, so the
/// corpus's own newsletter (3 tables, 13 cells, 2 links) was not recognised
/// as bulk. A signal that is always absent is worse than no signal, because
/// it raises the bar for everything else.
///
/// Deliberately not "does it have a `List-Unsubscribe` header" either: that
/// is a better signal and it is not available here, since this module only
/// ever sees markup. Whoever has the headers should prefer them and use this
/// as the fallback.
pub fn reads_as_bulk(html: &str) -> bool {
    let signals = count_signals(html);
    // Either signal alone is enough, because each one alone is already
    // unusual in correspondence. A quoted table is one table; a template
    // nests them to make columns. A reply has a handful of links; a campaign
    // has dozens. And the cost of being wrong is small and visible in both
    // directions: the notice says reader view is on and `View original` is
    // one keystroke away.
    signals.nested_tables > 0 || signals.links >= BULK_LINKS
}

/// How many links a message needs before that alone looks like a campaign.
const BULK_LINKS: usize = 10;

/// The arrangement signals, counted in one pass.
#[derive(Default, Debug)]
struct Signals {
    /// Tables inside tables. A person writing mail does not nest tables.
    nested_tables: usize,
    links: usize,
}

fn count_signals(html: &str) -> Signals {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let mut signals = Signals::default();
    count_into(&dom.document, 0, &mut signals);
    signals
}

fn count_into(node: &Handle, tables: usize, signals: &mut Signals) {
    let mut depth = tables;
    if let NodeData::Element { name, .. } = &node.data {
        match name.local.to_string().as_str() {
            "table" => {
                depth += 1;
                if depth > 1 {
                    signals.nested_tables += 1;
                }
            }
            "a" => signals.links += 1,
            _ => {}
        }
    }
    for child in node.children.borrow().iter() {
        count_into(child, depth, signals);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sentence_inside_six_nested_tables_comes_out_as_a_sentence() {
        // The whole point. The tables were columns; the words are the mail.
        let html = "<table><tr><td><table><tr><td><p>Your package is out \
                    for delivery.</p></td></tr></table></td></tr></table>";
        let reduced = reduce(html);
        assert!(
            reduced.html.contains("Your package is out for delivery."),
            "{}",
            reduced.html
        );
        assert!(
            !reduced.html.contains("<table") && !reduced.html.contains("<td"),
            "the arrangement went: {}",
            reduced.html
        );
    }

    #[test]
    fn emphasis_and_lists_and_quotes_survive() {
        let html = "<div><p>Hello <b>Ada</b> and <i>Bo</i></p>\
                    <ul><li>one</li><li>two</li></ul>\
                    <blockquote>quoted</blockquote></div>";
        let reduced = reduce(html);
        for tag in ["<b>", "<i>", "<ul>", "<li>", "<blockquote>", "<p>"] {
            assert!(reduced.html.contains(tag), "{tag} went: {}", reduced.html);
        }
    }

    #[test]
    fn a_line_break_is_kept_because_nothing_else_carries_it() {
        assert!(reduce("<div>one<br>two</div>").html.contains("<br"));
    }

    #[test]
    fn sender_styling_does_not_survive_even_one_attribute() {
        // Defence in depth: the sanitizer drops `style` before this runs, and
        // a sender's `bgcolor` on a kept tag would still be a sender deciding
        // what colour Postio's reader is.
        let html = r##"<p style="color:#f0f" bgcolor="#000" width="600" class="hero">text</p>"##;
        let reduced = reduce(html);
        assert!(reduced.html.contains("text"));
        for attribute in ["style", "bgcolor", "width", "class"] {
            assert!(
                !reduced.html.contains(attribute),
                "{attribute} survived: {}",
                reduced.html
            );
        }
    }

    #[test]
    fn the_first_link_is_kept_and_the_rest_are_counted() {
        // A campaign has one thing it wants you to do and twenty-two ways to
        // say it. `1 link kept of 23` is the canvas's own wording.
        let mut html =
            String::from(r#"<p><a href="https://example.com/track">Track delivery</a></p>"#);
        for index in 0..22 {
            html.push_str(&format!(
                r#"<p><a href="https://example.com/{index}">more</a></p>"#
            ));
        }
        let reduced = reduce(&html);
        assert_eq!(reduced.links_kept, 1);
        assert_eq!(reduced.links_dropped, 22);
        assert_eq!(reduced.links_total(), 23);
        assert!(
            reduced.html.contains("https://example.com/track"),
            "the primary call to action is the one that survives"
        );
    }

    #[test]
    fn a_collapsed_link_keeps_its_words_and_loses_its_href() {
        // The text was a sentence before it was a link. Dropping the anchor
        // and keeping the words is what stops a reduced newsletter reading
        // like a list of gaps.
        //
        // Document order decides which one survives — the first — so the
        // *second* link here is the collapsed one.
        let html = r#"<p><a href="https://example.com/track">Track delivery</a></p>
                      <p>Read our <a href="https://example.com/policy">privacy notice</a> today</p>"#;
        let reduced = reduce(html);
        assert!(
            reduced
                .html
                .contains(r#"<a href="https://example.com/track">Track delivery</a>"#),
            "the first link is the call to action and keeps its href: {}",
            reduced.html
        );
        assert!(
            reduced.html.contains("Read our privacy notice today"),
            "the collapsed link's words stay in the sentence they were in: {}",
            reduced.html
        );
        assert!(
            !reduced.html.contains("example.com/policy"),
            "and its href does not: {}",
            reduced.html
        );
    }

    #[test]
    fn a_reply_does_not_read_as_bulk() {
        let html = "<p>Hi Ada,</p><p>Friday works. See you then.</p>\
                    <blockquote><p>Can we do it Friday?</p></blockquote>";
        assert!(!reads_as_bulk(html));
    }

    #[test]
    fn a_campaign_reads_as_bulk() {
        let mut html = String::from("<table><tr><td><table><tr><td>");
        for index in 0..14 {
            html.push_str(&format!(
                r##"<a href="https://example.com/{index}" style="color:#06c">shop</a>"##
            ));
        }
        html.push_str("</td></tr></table></td></tr></table>");
        assert!(reads_as_bulk(&html));
    }

    #[test]
    fn a_template_with_barely_any_links_is_still_bulk() {
        // The corpus's own newsletter, in miniature: three tables, thirteen
        // cells, two links. The first version of this heuristic wanted two
        // signals out of three and one of the three was styling, which
        // `sanitize` has already erased by the time reader view runs -- so
        // this shape was not recognised at all. See `reads_as_bulk`.
        let html = "<table><tr><td><table><tr><td>\
                    <a href=\"https://example.com/a\">Shop now</a>\
                    </td></tr></table></td></tr></table>";
        assert!(reads_as_bulk(html));
    }

    #[test]
    fn plain_text_rendered_as_html_is_not_bulk() {
        // `quote::text_to_html` output must never be mistaken for a campaign,
        // or every plain-text message would open reduced for no reason.
        assert!(!reads_as_bulk("<p>Hello Diego —</p><p>Sincerely, Ada</p>"));
    }

    #[test]
    fn reducing_nothing_is_nothing() {
        let reduced = reduce("");
        assert_eq!(reduced.html, "");
        assert_eq!(reduced.links_total(), 0);
    }
}
