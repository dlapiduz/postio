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

/// One `label: value` line lifted out of a plain part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    /// The left column: short, and written by the sender as a label.
    pub label: String,
    /// The right column: the fact itself.
    pub value: String,
}

/// The most rows a block may have.
///
/// The canvas draws three. Four is the ceiling because a block longer than
/// the message it summarises is not a summary — and a template that emits
/// twenty `label: value` lines is a table of contents, not a set of facts.
pub const FACTS_KEPT: usize = 4;

/// The fewest lines that make a block.
///
/// Repetition is most of the signal. One line of `label: value` is a
/// sentence with a colon in it far more often than it is a fact, and a
/// one-row table above somebody's mail looks fabricated — which is exactly
/// the failure this feature has to avoid.
pub const FACTS_MIN: usize = 2;

/// The longest a label may be, in characters.
///
/// A label is a word or two a sender put in front of a fact. Past this it is
/// a clause, and a clause before a colon is prose.
const LABEL_CHARS: usize = 24;

/// The most words a label may have, for the same reason.
const LABEL_WORDS: usize = 3;

/// Labels that mean this run is mail plumbing rather than facts.
///
/// A quoted or forwarded header block has exactly the shape this looks for —
/// short labels, repeated, one per line — and drawing it as a summary would
/// put `From`, `To` and `Subject` in a table above a message that already
/// shows all three in its own header. Only rejected when the *whole* run is
/// header names: a receipt whose first row happens to be `Date` is still a
/// receipt.
const HEADER_LABELS: &[&str] = &[
    "from", "to", "cc", "bcc", "subject", "date", "sent", "reply-to",
];

/// Value prefixes that mean the row is a link, not a fact.
///
/// A newsletter footer is `Read issue 214: https://...` over
/// `Unsubscribe: https://...` — two short labels, adjacent, both with a
/// value. Every rule above passes it, and the block reader view would draw
/// is the sender's footer restated as a summary.
///
/// A link is not a fact, and reader view already has somewhere to put one:
/// reduction keeps the primary call to action and collapses the rest behind
/// a count. Lifting a URL up here would say the same thing twice, in the one
/// place on the page that is supposed to be the facts.
const LINK_PREFIXES: &[&str] = &["http://", "https://", "mailto:", "www."];

/// Lift a `label: value` block out of a plain part.
///
/// Transactional mail buries its facts in a paragraph, and the canvas draws
/// them as a small table above the body copy: what the tracking number is,
/// what shipped, where it went. This is the parser that finds them.
///
/// # What makes a block
///
/// **Repetition, mostly.** A single `label: value` line is a sentence with a
/// colon in it far more often than it is a fact — so the unit here is a
/// *contiguous run* of at least [`FACTS_MIN`] lines that all have the shape,
/// broken by any line that does not (a blank line included). The first run
/// that qualifies wins; a message with two of them is being over-read
/// already, and taking the first is a rule a person can predict.
///
/// The shape itself is deliberately narrow. A label is short
/// ([`LABEL_CHARS`], [`LABEL_WORDS`]) and made of letters, digits, spaces and
/// hyphens — no sentence punctuation, which is what tells `ship to` from
/// `Please note the following, which matters`. Both sides must be non-empty.
/// A run whose labels are *all* message-header names is thrown out; see
/// [`HEADER_LABELS`].
///
/// Getting this wrong puts a fabricated-looking table above somebody's mail,
/// so every rule here refuses rather than guesses, and the result is capped
/// at [`FACTS_KEPT`].
///
/// # Plain text only
///
/// The argument is the sender's own plain part — the version they wrote for
/// reading. Nothing is lifted out of HTML: markup that *looks* like a table
/// is a layout decision, and reader view's whole premise is that a sender's
/// layout is not to be trusted.
pub fn facts(plain: &str) -> Vec<Fact> {
    let mut run: Vec<Fact> = Vec::new();
    for line in plain.lines() {
        match row(line) {
            Some(fact) => run.push(fact),
            // Any line that is not a row ends the run, blank or not: the
            // block the canvas draws is a block on the page too.
            None => {
                if let Some(block) = block(&mut run) {
                    return block;
                }
            }
        }
    }
    block(&mut run).unwrap_or_default()
}

/// A finished run, if it was long enough and was not a header block.
///
/// Clears `run` either way, so the caller can keep walking.
fn block(run: &mut Vec<Fact>) -> Option<Vec<Fact>> {
    let mut found = std::mem::take(run);
    if found.len() < FACTS_MIN || found.iter().all(is_header_label) {
        return None;
    }
    found.truncate(FACTS_KEPT);
    Some(found)
}

fn is_header_label(fact: &Fact) -> bool {
    HEADER_LABELS.contains(&fact.label.to_ascii_lowercase().as_str())
}

/// One line, if it has the shape.
fn row(line: &str) -> Option<Fact> {
    let (label, value) = line.trim().split_once(':')?;
    let label = label.trim();
    let value = value.trim();
    if label.is_empty() || value.is_empty() {
        return None;
    }
    let lowered = value.to_ascii_lowercase();
    if LINK_PREFIXES
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
    {
        return None;
    }
    if label.chars().count() > LABEL_CHARS || label.split_whitespace().count() > LABEL_WORDS {
        return None;
    }
    // No sentence punctuation, which is the whole test: a label is a name,
    // and a name does not contain a full stop or a comma.
    if !label
        .chars()
        .all(|c| c.is_alphanumeric() || c == ' ' || c == '-')
    {
        return None;
    }
    Some(Fact {
        label: label.to_owned(),
        value: value.to_owned(),
    })
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

#[cfg(test)]
mod facts_tests {
    use super::*;

    fn labels(plain: &str) -> Vec<String> {
        facts(plain).into_iter().map(|fact| fact.label).collect()
    }

    #[test]
    fn a_repeated_label_value_shape_is_a_block() {
        let found = facts(
            "Your order has shipped.\n\
             \n\
             tracking: EXTEST0042199317\n\
             item: Type-C Upgrade Small Board Replacement x 1\n\
             ship to: 1 Example Way, Springfield\n\
             \n\
             Follow the parcel from your orders page.\n",
        );
        assert_eq!(
            found,
            vec![
                Fact {
                    label: "tracking".to_owned(),
                    value: "EXTEST0042199317".to_owned()
                },
                Fact {
                    label: "item".to_owned(),
                    value: "Type-C Upgrade Small Board Replacement x 1".to_owned(),
                },
                Fact {
                    label: "ship to".to_owned(),
                    value: "1 Example Way, Springfield".to_owned(),
                },
            ],
        );
    }

    #[test]
    fn one_good_row_on_its_own_is_still_not_a_block() {
        // The row itself is impeccable -- short label, real value, not a
        // header name -- so the only thing that can refuse it is the
        // requirement that the shape repeat.
        assert!(
            facts(
                "Thanks for your order.\n\
                 \n\
                 tracking: EXTEST0042199317\n\
                 \n\
                 It will arrive next week.\n",
            )
            .is_empty(),
            "one row is not a block, however well formed it is"
        );
    }

    #[test]
    fn one_line_on_its_own_is_a_sentence_with_a_colon_in_it() {
        assert!(
            facts("Hello Ada,\n\nHere is the thing you asked about: the meeting is moved.\n")
                .is_empty(),
            "a single matching line is not a block"
        );
    }

    #[test]
    fn a_body_that_merely_mentions_a_header_name_does_not_grow_a_table() {
        assert!(
            facts(
                "I could not find your message.\n\
                 \n\
                 Subject: quarterly numbers\n\
                 \n\
                 Was that the one? Let me know.\n",
            )
            .is_empty(),
            "one quoted header line is not a block"
        );
    }

    #[test]
    fn a_quoted_header_block_is_not_a_facts_block() {
        assert!(
            facts(
                "See below.\n\
                 \n\
                 From: Ada Lovelace <ada@example.com>\n\
                 To: Postio <postio@example.net>\n\
                 Subject: quarterly numbers\n\
                 Date: Mon, 1 Jun 2026 09:00:00 +0000\n\
                 \n\
                 The numbers are attached.\n",
            )
            .is_empty(),
            "a forwarded header block is mail plumbing, not a set of facts"
        );
    }

    #[test]
    fn a_run_longer_than_the_cap_is_trimmed_to_it() {
        let plain = (1..=9)
            .map(|n| format!("field {n}: value {n}\n"))
            .collect::<String>();
        let found = facts(&plain);
        // Spelled out rather than compared against `FACTS_KEPT`, which is
        // what the constant would say about itself.
        assert_eq!(
            found.len(),
            4,
            "nine rows is a contents page, not a summary"
        );
        assert_eq!(found[0].label, "field 1", "and it keeps the first four");
        assert_eq!(found[3].label, "field 4");
    }

    // The three label rules below are tested one at a time, on input that
    // only that rule refuses. Together they read like one rule; separately
    // each has to earn its place, and a rule no test can break is a rule
    // nobody can justify.

    #[test]
    fn a_label_carrying_sentence_punctuation_is_prose() {
        // Short, and few enough words -- only the punctuation is wrong.
        assert!(
            facts(
                "Good news, everyone: your order shipped\n\
                   One thing, though: it is late\n"
            )
            .is_empty(),
            "a comma before the colon means a sentence, not a label"
        );
    }

    #[test]
    fn a_label_of_too_many_words_is_prose() {
        // Short words, no punctuation: only the count is wrong.
        assert!(
            facts(
                "we will be in touch: soon\n\
                   and you can call us: today\n"
            )
            .is_empty(),
            "five words before a colon is a clause"
        );
    }

    #[test]
    fn a_label_that_runs_long_is_prose() {
        // Three words, no punctuation: only the length is wrong.
        assert!(
            facts(
                "delivery confirmation notification: sent\n\
                   shipment verification acknowledgement: done\n"
            )
            .is_empty(),
            "a label a person would not write as a column heading"
        );
    }

    #[test]
    fn a_row_needs_something_on_both_sides_of_the_separator() {
        assert!(
            facts("tracking:\nitem:\nship to:\n").is_empty(),
            "a label with no value is not a fact"
        );
    }

    #[test]
    fn a_blank_line_ends_the_run() {
        // Nothing but whitespace separates these four rows, and every one of
        // them would pass on its own. The block the canvas draws is a block
        // on the page too, so only the first pair is one.
        assert_eq!(
            labels(
                "tracking: EXTEST0042199317\n\
                 item: One Small Board\n\
                 \n\
                 colour: blue\n\
                 size: large\n",
            ),
            vec!["tracking", "item"],
        );
    }

    #[test]
    fn only_one_run_is_taken_and_it_is_the_first() {
        assert_eq!(
            labels(
                "tracking: EXTEST0042199317\n\
                 item: One Small Board\n\
                 \n\
                 Some prose in between.\n\
                 \n\
                 colour: blue\n\
                 size: large\n",
            ),
            vec!["tracking", "item"],
        );
    }
}
