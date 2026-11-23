//! The newest message's own text, without quoted history or signature, read from what the reader
//! draws (sanitized HTML flattened, else the plain part): 99.5% of sentences are then found again
//! to highlight.
//! Inline-hidden elements and image `alt` text are left out. Quotes are `quote`'s stretches,
//! signatures `postio_model::signature::split`. A pure function of the body; offsets are in chars.

use markup5ever_rcdom::{Handle, NodeData};
use postio_model::MessageBody;
use postio_model::mime::preview;

use crate::parse;
use crate::quote::{self, Stretch};
use crate::sanitize::{RemoteImages, sanitize_body};

pub fn own_text(body: &MessageBody) -> String {
    let html = body.html.as_deref().filter(|html| !html.trim().is_empty());
    let text = body.text.as_deref().filter(|text| !text.trim().is_empty());
    // The part the reader draws: the HTML unless over a cap, then the plain part.
    if let Some(html) = html {
        let sanitized = sanitize_body(html, RemoteImages::Blocked);
        if sanitized.over_cap.is_none() {
            return from_html(&sanitized.html);
        }
    }
    text.map(from_text).unwrap_or_default()
}

/// Empty when the message says nothing of its own, which tells an all-quote reply from one whose
/// body has not landed.
pub fn own_line(body: &MessageBody) -> String {
    preview(&own_text(body)).unwrap_or_default()
}

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

fn from_html(sanitized: &str) -> String {
    let mut own = String::with_capacity(sanitized.len());
    for stretch in quote::html_stretches(sanitized) {
        match stretch {
            Stretch::Own(markup) => own.push_str(markup),
            // An empty paragraph keeps the words either side of the history apart.
            Stretch::Quoted(_) => own.push_str("<p></p>"),
        }
    }
    let dom = parse::fragment(&own);
    leave_out_the_undrawn(&dom.document);
    let flat = parse::narrow(&dom).to_search_text();
    // A flattened line's edges are spaces between tags, and a signature separator is found only at
    // line start.
    let mut lines: Vec<&str> = Vec::new();
    for line in flat.lines().map(str::trim) {
        if !(line.is_empty() && lines.last().is_some_and(|last| last.is_empty())) {
            lines.push(line);
        }
    }
    unsigned(&before_history(&lines.join("\n")))
}

/// Outlook's "Original Message" and underscore rule, forward banners, header blocks, "Sent from my"
/// and attributions over unquoted text all mark where history begins.
/// An attribution over a `>` quote goes with it; the `<hr>` before history goes too.
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

fn is_attribution(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.starts_with("on ") && lower.ends_with("wrote:")
}

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

/// Matched whole with trailing punctuation, so "Thanks for the update." is not a closing.
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

fn without_blank_edges(own: &str) -> &str {
    let own = own.trim_end();
    let first = own
        .split_inclusive('\n')
        .take_while(|line| line.trim().is_empty())
        .map(str::len)
        .sum::<usize>();
    &own[first..]
}

fn unsigned(text: &str) -> String {
    postio_model::signature::split(before_closing(text))
        .0
        .trim()
        .to_owned()
}

fn leave_out_the_undrawn(node: &Handle) {
    node.children.borrow_mut().retain(|child| !hidden(child));
    for child in node.children.borrow().iter() {
        if let NodeData::Element { name, attrs, .. } = &child.data
            && (&*name.local) == "img"
        {
            attrs
                .borrow_mut()
                .retain(|attribute| (&*attribute.name.local) != "alt");
        }
        leave_out_the_undrawn(child);
    }
}

fn hidden(node: &Handle) -> bool {
    let NodeData::Element { attrs, .. } = &node.data else {
        return false;
    };
    attrs
        .borrow()
        .iter()
        .any(|attribute| (&*attribute.name.local) == "style" && hides(&attribute.value))
}

/// `visibility: hidden` or `collapse` match the renderer's text index; a descendant resetting
/// `visibility` is ignored, so the whole element goes.
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
    use postio_model::mime::PREVIEW_CHARS;

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
    fn a_top_posted_reply_gives_its_own_first_line() {
        let line = own_line(&text(
            "Looks good to me.\nShip it Friday.\n\nOn Mon, 21 Sep 2026, Ana wrote:\n> Here is the draft.\n> Thoughts?\n",
        ));
        assert_eq!(line, "Looks good to me. Ship it Friday.");
    }

    #[test]
    fn an_all_quote_message_has_no_own_line() {
        let line = own_line(&text(
            "On Mon, Ana wrote:\n> Here is the draft.\n> Thoughts?\n",
        ));
        assert_eq!(line, "");
    }

    #[test]
    fn a_long_line_is_cut_as_a_preview_is() {
        let words = "word ".repeat(100);
        let line = own_line(&text(&words));
        assert_eq!(line.chars().count(), PREVIEW_CHARS);
        assert!(line.ends_with('\u{2026}'), "{line}");
        assert!(line.starts_with("word word"));
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

    // Clients that do not prefix history with `>` still mark where it begins; the needs-action
    // detector must not read an ask there as the sender's own.

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
