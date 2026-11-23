//! Reader view: reduces bulk mail to the tags that carry meaning, since sanitised newsletters keep
//! their sender's nested-table layout and colours. The original stays one keystroke away.
//! Every rule here is over a string; what the reader does with the answer lives in
//! `postio_ui::reader::document`.

use std::collections::HashSet;

use html5ever::driver::ParseOpts;
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// Meaning, not arrangement. Unkept elements are unwrapped, so a sentence inside nested `<td>`s
/// survives.
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

const KEPT_VOID: [&str; 1] = ["br"];

/// `href` only. Dropping `style` is the control, not a backstop: `sanitize` keeps a sender's
/// `style`, so this list alone keeps a sender's palette out of reader view.
const KEPT_ATTRIBUTES: [&str; 1] = ["href"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reduced {
    pub html: String,
    pub links_kept: usize,
    pub links_dropped: usize,
}

impl Reduced {
    pub fn links_total(&self) -> usize {
        self.links_kept + self.links_dropped
    }
}

/// One: the primary call to action is what a sender puts first, a rule a person can learn; judging
/// which links are important is not possible here.
pub const LINKS_KEPT: usize = 1;

/// Expects sanitized markup: this is a readability pass, not a security one.
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

/// Threaded through the walk because "the first link" is a fact about document order.
#[derive(Default)]
struct Links {
    kept: usize,
    dropped: usize,
}

/// An unkept element is unwrapped, not deleted. `<script>`/`<style>` subtrees are dropped here too,
/// so this is not wrong on raw markup.
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
            // An anchor past the budget keeps its words: dropping it whole makes a newsletter read
            // as a list of gaps.
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

/// Written out rather than taken from a crate: this is the output side of a reduction that must
/// interpret nothing a sender wrote.
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

/// A plain part that is the sender's HTML flattened to markdown. Matches only `](http` links:
/// a false positive costs the facts block, so it fires on unambiguously generated text.
pub fn reads_as_flattened_markup(plain: &str) -> bool {
    plain.contains("](http://") || plain.contains("](https://")
}

/// A heuristic over arrangement, not content: nested layout tables, or many links. Either alone is
/// enough, since each is unusual in correspondence and being wrong is cheap (`View original` is one
/// keystroke away).
/// Styling is not counted: it was always absent when this was written, and inline styling is how
/// rich-text composers look too; reviving it is a measurement.
/// A `List-Unsubscribe` header is a better signal but not visible here; callers with headers should
/// prefer it.
pub fn reads_as_bulk(html: &str) -> bool {
    let signals = count_signals(html);
    signals.nested_tables > 0 || signals.links >= BULK_LINKS
}

const BULK_LINKS: usize = 10;

#[derive(Default, Debug)]
struct Signals {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    pub label: String,
    pub value: String,
}

/// Four, because a block longer than the message it summarises is not a summary.
pub const FACTS_KEPT: usize = 4;

/// One `label: value` line is a sentence with a colon far more often than a fact; repetition is the
/// signal.
pub const FACTS_MIN: usize = 2;

/// Past this it is a clause, and a clause before a colon is prose.
const LABEL_CHARS: usize = 24;

const LABEL_WORDS: usize = 3;

/// A quoted or forwarded header block has this shape; it is rejected only when the whole run is
/// header names.
const HEADER_LABELS: &[&str] = &[
    "from", "to", "cc", "bcc", "subject", "date", "sent", "reply-to",
];

/// A link is not a fact: reduction already keeps the primary call to action, so lifting a URL here
/// would say it twice.
const LINK_PREFIXES: &[&str] = &["http://", "https://", "mailto:", "www."];

/// A plain part with its facts block taken out. Facts come only from plain text, since markup that
/// looks like a table is layout; the first contiguous run of `FACTS_MIN` or more `label: value`
/// lines wins, capped at `FACTS_KEPT`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lifted {
    pub rows: Vec<Fact>,
    pub body: String,
}

/// The rows leave the body copy, or the reader shows them twice. Only lines that became rows are
/// removed: a run past `FACTS_KEPT` keeps its remainder in the body.
pub fn lift(plain: &str) -> Lifted {
    let lines: Vec<&str> = plain.lines().collect();
    let mut run: Vec<Fact> = Vec::new();
    let mut start = 0;
    let mut found = None;
    for (index, line) in lines.iter().enumerate() {
        match row(line) {
            Some(fact) => {
                if run.is_empty() {
                    start = index;
                }
                run.push(fact);
            }
            None => {
                if let Some(block) = block(&mut run) {
                    found = Some((start, block));
                    break;
                }
            }
        }
    }
    let found = found.or_else(|| block(&mut run).map(|block| (start, block)));
    let Some((start, rows)) = found else {
        return Lifted {
            rows: Vec::new(),
            body: plain.to_owned(),
        };
    };

    let lifted = start..start + rows.len();
    let mut body = String::new();
    for (index, line) in lines.iter().enumerate() {
        if lifted.contains(&index) {
            continue;
        }
        body.push_str(line);
        body.push('\n');
    }
    Lifted {
        rows,
        body: close_the_gap(&body),
    }
}

fn close_the_gap(body: &str) -> String {
    let mut out = body.trim_start_matches('\n').to_owned();
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out
}

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

fn row(line: &str) -> Option<Fact> {
    let (label, value) = line.trim().split_once(':')?;
    let label = label.trim();
    let value = value.trim();
    if label.is_empty() || value.is_empty() {
        return None;
    }
    // A CSS declaration is not a fact: a plain part made by stripping HTML keeps `<style>` text as
    // `name: value` lines (#1432). Braces are decisive.
    if value.contains('{') || value.contains('}') || label.contains('{') {
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
        // `sanitize` keeps a sender's `style`, so this asserts reader view still drops it.
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
        // Document order decides which link survives: the first, so the second is the collapsed
        // one.
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
        // The corpus newsletter in miniature (3 tables, 13 cells, 2 links); a styling signal would
        // have missed it.
        let html = "<table><tr><td><table><tr><td>\
                    <a href=\"https://example.com/a\">Shop now</a>\
                    </td></tr></table></td></tr></table>";
        assert!(reads_as_bulk(html));
    }

    #[test]
    fn plain_text_rendered_as_html_is_not_bulk() {
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

    fn facts(plain: &str) -> Vec<Fact> {
        lift(plain).rows
    }

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
        assert_eq!(
            found.len(),
            4,
            "nine rows is a contents page, not a summary"
        );
        assert_eq!(found[0].label, "field 1", "and it keeps the first four");
        assert_eq!(found[3].label, "field 4");
    }

    #[test]
    fn a_label_carrying_sentence_punctuation_is_prose() {
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
    fn the_lifted_lines_leave_the_body_copy() {
        let lifted = lift(
            "Your order has shipped.\n\
             \n\
             tracking: EXTEST0042199317\n\
             item: One Small Board\n\
             \n\
             Follow the parcel from your orders page.\n",
        );
        assert_eq!(lifted.rows.len(), 2);
        assert_eq!(
            lifted.body, "Your order has shipped.\n\nFollow the parcel from your orders page.\n",
            "the rows are gone and the hole they left is closed"
        );
    }

    #[test]
    fn a_body_with_no_block_is_handed_back_unchanged() {
        let plain = "Hi Ada,\n\nFriday works for me.\n";
        assert_eq!(lift(plain).body, plain);
    }

    #[test]
    fn a_run_past_the_cap_keeps_its_remainder_in_the_body() {
        // A summary may be partial but nothing may vanish.
        let plain = (1..=9)
            .map(|n| format!("field {n}: value {n}\n"))
            .collect::<String>();
        let lifted = lift(&plain);
        assert_eq!(lifted.rows.len(), 4);
        assert!(
            !lifted.body.contains("field 4:"),
            "a lifted line is still in the body: {}",
            lifted.body
        );
        for n in 5..=9 {
            assert!(
                lifted.body.contains(&format!("field {n}: value {n}")),
                "field {n} vanished: {}",
                lifted.body
            );
        }
    }

    #[test]
    fn a_blank_line_ends_the_run() {
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

#[cfg(test)]
mod css_is_not_a_fact {
    use super::*;

    /// #1432: a text alternative made by stripping HTML keeps the `<style>` block, and `lift` read
    /// `text-indent: -1em; } a {` as a row.
    #[test]
    fn a_stylesheet_swept_into_the_text_part_is_not_lifted() {
        let plain = "text-indent: -1em; } a { border: none !important; } li {\n\
                     text-indent: -10em; }\n\
                     ul > li {\n\
                     \n\
                     A few simple checks now can help protect your home.\n";
        let lifted = lift(plain);
        assert!(
            lifted.rows.is_empty(),
            "CSS was lifted into the facts block: {:?}",
            lifted.rows
        );
        assert!(
            lifted.body.contains("A few simple checks"),
            "the prose must survive: {:?}",
            lifted.body
        );
    }

    /// The control: without it, "never lift" would pass the test above.
    #[test]
    fn a_real_facts_block_still_lifts() {
        let plain = "Order: 4417\n\
                     Ship date: 21 August\n\
                     Carrier: Royal Mail\n\
                     \n\
                     Thanks for your order.\n";
        let lifted = lift(plain);
        assert_eq!(
            lifted.rows.len(),
            3,
            "an ordinary facts block stopped lifting: {:?}",
            lifted.rows
        );
    }
}

#[cfg(test)]
mod a_plain_part_that_is_really_markup {
    use super::reads_as_flattened_markup;

    /// A school's daily summary whose `text/plain` is its HTML run through a markdown converter.
    #[test]
    fn a_converters_markdown_is_not_prose() {
        let flattened = "Assignment.[Bridges Practice #1](https://app.example.test/a/849) 3:59 pm\n\
                         Event.[Even Day](https://app.example.test/p/420) Henderson Middle School";
        assert!(reads_as_flattened_markup(flattened));
        assert!(reads_as_flattened_markup(
            "see [the notice](http://example.test/n)"
        ));
    }

    /// The control: a false positive costs a transactional message the `lift` summary, since the
    /// HTML path has none.
    #[test]
    fn prose_is_not_mistaken_for_markup() {
        for prose in [
            "Your parcel EXTEST0042199317 is on its way.\nTracking: EXTEST0042199317",
            "* milk\n* eggs\n- and a note\n# not a heading, a comment",
            "The meeting (see https://example.test/agenda) is at three.",
            "I wrote [sic] in the margin (twice).",
            "",
        ] {
            assert!(
                !reads_as_flattened_markup(prose),
                "prose must keep the plain part and its facts block: {prose:?}"
            );
        }
    }
}
