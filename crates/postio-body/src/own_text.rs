//! The newest message's own text (spec 007 FR-104, T115).
//!
//! What its sender wrote in it: not the history it quotes, not the
//! signature it ends with. The needs-action detector reads this text, and a
//! marker's excerpt and offset point into it (data-model.md, `markers`).
//!
//! # Read what is drawn
//!
//! Spike S5 (spec 007 research R2) measured it. A sentence is highlighted by
//! finding its words again in what the reader draws, so the text they come
//! from must be the part the reader draws: the HTML, flattened, when there
//! is HTML, and the plain part otherwise. Read that way, 99.5% of the
//! corpus's sentences are found again; reading the plain part of a message
//! drawn from its HTML loses sentences the HTML never says. So:
//!
//! * the HTML is sanitized first, as the reader's is, which takes out the
//!   `<title>` and everything else the reader never receives;
//! * what an inline style hides (`display: none`, `visibility: hidden`) is
//!   left out, as the renderer's text index leaves it out: a preheader is
//!   the usual case;
//! * an image's `alt` text is left out: the picture is drawn, not its words.
//!
//! # One quote detector, one signature rule
//!
//! Quoted history is what [`crate::quote`] folds, found by the same
//! [`Stretch`]es, so the text never includes what the reader folds away. A
//! signature is what [`postio_model::signature::split`] finds, applied once
//! the history is out: a signature written above the quote of a top-posted
//! reply is then the end of the text, as RFC 3676 means it to be.
//!
//! # Offsets
//!
//! The text is a pure function of the body, so the same body always gives
//! the same text and an offset into it stays true. Offsets are in chars.

use markup5ever_rcdom::{Handle, NodeData};
use postio_model::MessageBody;

use crate::parse;
use crate::quote::{self, Stretch};
use crate::sanitize::{RemoteImages, sanitize_body};

/// The sender's own words in `body`: its drawn part, without quoted
/// history, the signature, or what the reader never draws. Empty when the
/// message says nothing of its own.
pub fn own_text(body: &MessageBody) -> String {
    let html = body.html.as_deref().filter(|html| !html.trim().is_empty());
    let text = body.text.as_deref().filter(|text| !text.trim().is_empty());
    // The part the reader draws (`postio_ui::reader::document::body_html`):
    // the HTML, unless it is over a cap, and then the plain part.
    if let Some(html) = html {
        let sanitized = sanitize_body(html, RemoteImages::Blocked);
        if sanitized.over_cap.is_none() {
            return from_html(&sanitized.html);
        }
    }
    text.map(from_text).unwrap_or_default()
}

/// A plain part's own words: what it says before earlier mail begins, every
/// run of `>` lines out, and the words either side of one kept apart by a
/// blank line.
fn from_text(text: &str) -> String {
    let text = before_history(&text.replace("\r\n", "\n"));
    let own: Vec<&str> = quote::text_stretches(&text)
        .into_iter()
        .filter_map(|stretch| match stretch {
            Stretch::Own(own) => Some(without_blank_edges(own)),
            Stretch::Quoted(_) => None,
        })
        .filter(|own| !own.is_empty())
        .collect();
    unsigned(&own.join("\n\n"))
}

/// Sanitized HTML's own words, flattened as the search index flattens a
/// body ([`crate::Document::to_search_text`]).
fn from_html(sanitized: &str) -> String {
    let mut own = String::with_capacity(sanitized.len());
    for stretch in quote::html_stretches(sanitized) {
        match stretch {
            Stretch::Own(markup) => own.push_str(markup),
            // An empty paragraph ends the one before it, so the words either
            // side of the history do not run together.
            Stretch::Quoted(_) => own.push_str("<p></p>"),
        }
    }
    let dom = parse::fragment(&own);
    leave_out_the_undrawn(&dom.document);
    let flat = parse::narrow(&dom).to_search_text();
    // A flattened line's edges are the spaces between the sender's tags,
    // which nobody wrote, and a signature's separator is only found at the
    // start of a line.
    let mut lines: Vec<&str> = Vec::new();
    for line in flat.lines().map(str::trim) {
        if !(line.is_empty() && lines.last().is_some_and(|last| last.is_empty())) {
            lines.push(line);
        }
    }
    unsigned(&before_history(&lines.join("\n")))
}

/// `text` up to where earlier mail begins, with the attribution line of a
/// quote left out.
///
/// Not every client prefixes history with `>`, but each marks where it
/// begins, and everything from there on is somebody else's mail: Outlook's
/// "Original Message" and its rule of underscores, a forward's banner, a
/// header block (`From:` over `Sent:` or `Date:`), a phone's "Sent from my"
/// and its attribution over unquoted text. An attribution over a `>` quote
/// is the quote's own line, and goes with it; over anything else, it is
/// where the unquoted history starts. The rule an `<hr>` leaves, just before
/// the history, goes too.
fn before_history(text: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut kept = String::with_capacity(text.len());
    for (at, line) in lines.iter().enumerate() {
        let content = line.trim();
        if begins_history(content, lines.get(at + 1).map(|next| next.trim())) {
            break;
        }
        if is_attribution(content) {
            let over_a_quote = lines[at + 1..]
                .iter()
                .map(|next| next.trim())
                .find(|next| !next.is_empty())
                .is_some_and(|next| next.starts_with('>'));
            if over_a_quote {
                continue;
            }
            break;
        }
        kept.push_str(line);
    }
    without_trailing_rules(&kept).to_owned()
}

/// Whether `line`, with `next` after it, is where earlier mail begins.
fn begins_history(line: &str, next: Option<&str>) -> bool {
    let lower = line.to_lowercase();
    let banner = |words: &str| lower.starts_with('-') && lower.contains(words);
    banner("original message")
        || banner("forwarded message")
        || lower.starts_with("begin forwarded message")
        || (line.chars().count() >= 10 && line.chars().all(|character| character == '_'))
        || lower.starts_with("sent from my ")
        || (lower.starts_with("from:")
            && next.is_some_and(|next| {
                let next = next.to_lowercase();
                next.starts_with("sent:") || next.starts_with("date:")
            }))
}

/// Whether `line` introduces somebody else's words: "On <when>, <who>
/// wrote:".
fn is_attribution(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.starts_with("on ") && lower.ends_with("wrote:")
}

/// `text` without the blank lines and horizontal rules at its end.
fn without_trailing_rules(text: &str) -> &str {
    let mut end = text.len();
    for line in text[..end].split_inclusive('\n').rev() {
        let content = line.trim();
        let rule = content.chars().count() >= 3
            && content
                .chars()
                .all(|character| matches!(character, '-' | '_' | '=' | '*'));
        if !(content.is_empty() || rule) {
            break;
        }
        end -= line.len();
    }
    &text[..end]
}

/// The lines that close a message: everything after one is its signature.
/// Matched whole, with a trailing comma, full stop or exclamation mark, so a
/// sentence that begins like one ("Thanks for the update.") is not one.
const CLOSINGS: &[&str] = &[
    "thanks",
    "thank you",
    "many thanks",
    "best",
    "best wishes",
    "best regards",
    "kind regards",
    "regards",
    "warm regards",
    "warmly",
    "cheers",
    "love",
    "all the best",
];

/// `text` up to the line that closes it, when one does.
fn before_closing(text: &str) -> &str {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let bare = line
            .trim()
            .trim_end_matches([',', '!', '.'])
            .trim_end()
            .to_lowercase();
        if CLOSINGS.contains(&bare.as_str()) {
            return &text[..offset];
        }
        offset += line.len();
    }
    text
}

/// `own` without the blank lines at either end, and without the spaces
/// that end its last line.
fn without_blank_edges(own: &str) -> &str {
    let own = own.trim_end();
    let first = own
        .split_inclusive('\n')
        .take_while(|line| line.trim().is_empty())
        .map(str::len)
        .sum::<usize>();
    &own[first..]
}

/// `text` up to its closing and its signature, if it has them, trimmed.
fn unsigned(text: &str) -> String {
    postio_model::signature::split(before_closing(text))
        .0
        .trim()
        .to_owned()
}

/// Take out of the tree what the reader never draws: every element an
/// inline style hides, and every image's `alt` text.
fn leave_out_the_undrawn(node: &Handle) {
    node.children.borrow_mut().retain(|child| !hidden(child));
    for child in node.children.borrow().iter() {
        if let NodeData::Element { name, attrs, .. } = &child.data
            && name.local.as_ref() == "img"
        {
            attrs
                .borrow_mut()
                .retain(|attribute| attribute.name.local.as_ref() != "alt");
        }
        leave_out_the_undrawn(child);
    }
}

/// Whether an element's inline style hides it.
fn hidden(node: &Handle) -> bool {
    let NodeData::Element { attrs, .. } = &node.data else {
        return false;
    };
    attrs
        .borrow()
        .iter()
        .any(|attribute| attribute.name.local.as_ref() == "style" && hides(&attribute.value))
}

/// Whether a `style` attribute hides what it is on: `display: none`, or
/// `visibility: hidden` or `collapse`, the two the renderer's text index
/// leaves out. A descendant could set `visibility` back, which mail does
/// not do, so the whole element goes either way.
fn hides(style: &str) -> bool {
    style.split(';').any(|declaration| {
        let Some((property, value)) = declaration.split_once(':') else {
            return false;
        };
        let value = value.trim().to_ascii_lowercase();
        let value = value.strip_suffix("!important").unwrap_or(&value).trim();
        match property.trim().to_ascii_lowercase().as_str() {
            "display" => value == "none",
            "visibility" => value == "hidden" || value == "collapse",
            _ => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(markup: &str) -> MessageBody {
        MessageBody {
            html: Some(markup.to_owned()),
            text: None,
        }
    }

    fn text(plain: &str) -> MessageBody {
        MessageBody {
            html: None,
            text: Some(plain.to_owned()),
        }
    }

    #[test]
    fn what_a_style_hides_is_not_read() {
        let own = own_text(&html(
            "<p>Can you send the plans?</p>\
             <div style=\"DISPLAY: none !important\">Or reply by noon, please.</div>\
             <span style=\"color:#fff; visibility:hidden\">Tracking text here.</span>\
             <p>See you then.</p>",
        ));
        assert_eq!(own, "Can you send the plans?\n\nSee you then.");
    }

    #[test]
    fn words_either_side_of_a_quote_stay_apart() {
        let own = own_text(&html(
            "<div>See below<blockquote><p>their words</p></blockquote>and above</div>",
        ));
        assert_eq!(own, "See below\n\nand above");
        let own = own_text(&text(
            "My answers:\n> first question\nYes\n> second question\nNo\n",
        ));
        assert_eq!(own, "My answers:\n\nYes\n\nNo");
    }

    #[test]
    fn markup_over_a_cap_is_read_as_the_plain_part_the_reader_draws() {
        let deep = "<div>".repeat(2_000);
        let body = MessageBody {
            html: Some(format!("{deep}Too deep to draw.")),
            text: Some("The plain part says this instead.".to_owned()),
        };
        assert_eq!(own_text(&body), "The plain part says this instead.");
    }

    // --- Earlier mail that is not quoted (spec 007 T117) --------------------
    //
    // Clients that do not prefix history with `>` still mark where it begins:
    // Outlook's "Original Message" and its underscore rule, a forward's
    // banner, a header block, a phone's attribution over unquoted text. The
    // needs-action detector must not read an ask in any of them as the
    // sender's own, and SC-013's gate measures it through this cutter.

    #[test]
    fn earlier_mail_after_a_client_s_separator_is_not_the_message() {
        for (body, why) in [
            (
                "Done, see attached.\n\n-----Original Message-----\nFrom: Ada <ada@example.com>\n\
                 Sent: Friday\n\nCould you send me the receipts?\n",
                "Outlook's original message",
            ),
            (
                "Done, see attached.\n\n________________________________\n\
                 From: Ada <ada@example.com>\nSent: Friday\n\nCan you confirm points 1 to 4?\n",
                "Outlook's rule",
            ),
            (
                "Done, see attached.\n\n---------- Forwarded message ---------\n\
                 From: Hana <hana@example.org>\n\nCan you send the questionnaire back?\n",
                "a forward's banner",
            ),
            (
                "Done, see attached.\n\nBegin forwarded message:\n\n\
                 From: Hana <hana@example.org>\n\nCan you send the questionnaire back?\n",
                "Apple Mail's forward",
            ),
            (
                "Done, see attached.\n\nFrom: Ada <ada@example.com>\nDate: Friday\n\
                 Subject: Receipts\n\nCould you send me the receipts?\n",
                "a header block with no separator",
            ),
            (
                "Done, see attached.\n\nSent from my phone\n\n\
                 On 25 Sep 2026, at 18:44, Quinn <quinn@example.net> wrote:\n\n\
                 Are you coming on Friday?\n",
                "a phone's attribution over unquoted text",
            ),
        ] {
            assert_eq!(own_text(&text(body)), "Done, see attached.", "{why}");
        }
    }

    #[test]
    fn a_quote_s_attribution_is_not_the_message_and_what_follows_the_quote_is() {
        let own = own_text(&text(
            "On Thu, 24 Sep 2026 at 14:05, Ada <ada@example.com> wrote:\n\
             > Could you send me the slides?\n\n\
             Here they are. Also, can you check slide 7 before Monday?\n",
        ));
        assert_eq!(
            own,
            "Here they are. Also, can you check slide 7 before Monday?"
        );
    }

    #[test]
    fn a_closing_ends_the_message_and_what_follows_it_is_the_signature() {
        let own = own_text(&text(
            "Hi Ada,\n\nConfirmed for Tuesday.\n\nKind regards,\nWalter Pike\n\
             Pike Lettings\nNeed a valuation? Book a free visit today.\n",
        ));
        assert_eq!(own, "Hi Ada,\n\nConfirmed for Tuesday.");
        let own = own_text(&text("See you Thursday.\n\nCheers!\nFelix\n"));
        assert_eq!(own, "See you Thursday.");
    }

    #[test]
    fn a_line_that_only_starts_like_a_closing_is_the_message() {
        let body = "Thanks for the update.\nBest of all, the room is free on Monday.\n";
        assert_eq!(own_text(&text(body)), body.trim());
    }

    #[test]
    fn an_html_reply_s_header_block_is_not_the_message() {
        let own = own_text(&html(
            "<p>Agreed on all points.</p><hr>\
             <p><b>From:</b> Ada &lt;ada@example.com&gt;<br><b>Sent:</b> Friday</p>\
             <p>Can you confirm you're happy with points 1 to 4?</p>",
        ));
        assert_eq!(own, "Agreed on all points.");
    }

    #[test]
    fn a_body_with_nothing_of_its_own_has_no_own_text() {
        assert_eq!(own_text(&text("Mine.\n> theirs\n")), "Mine.");
        assert_eq!(
            own_text(&text("> all of it quoted\n> and nothing else\n")),
            ""
        );
        assert_eq!(own_text(&MessageBody::default()), "");
    }
}
