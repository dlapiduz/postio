//! A `mailto:` URI, read into the fields of a draft (RFC 6068).
//!
//! The desktop entry registers Postio for `x-scheme-handler/mailto`, so a
//! link in a browser or a "share by email" from another application arrives
//! as one of these. The grammar is small and the encoding is the whole of
//! the difficulty: the address list and every header value are
//! percent-encoded, a `+` is a literal plus (this is not a form), and header
//! names are matched case-insensitively. Only the headers a draft has fields
//! for are read — `to`, `cc`, `bcc`, `subject`, `body`; anything else in the
//! URI is ignored rather than smuggled into a message (RFC 6068 §6 says as
//! much: a client must not add arbitrary headers a URI names).

use crate::address::{EmailAddress, parse_list};
use crate::draft::Draft;
use crate::ids::AccountId;
use crate::message::MessageBody;

/// What a `mailto:` URI asked for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mailto {
    /// The addresses in the path, plus any `to=` in the query.
    pub to: Vec<EmailAddress>,
    /// `cc=`, as many as the query named.
    pub cc: Vec<EmailAddress>,
    /// `bcc=`, likewise.
    pub bcc: Vec<EmailAddress>,
    /// `subject=`, decoded; `None` when the link named none.
    pub subject: Option<String>,
    /// `body=`, decoded, as the plain-text body; `None` when the link named
    /// none. A link cannot ask for HTML.
    pub body: Option<String>,
}

impl Mailto {
    /// Reads `uri`, or `None` when it is not a `mailto:` URI at all.
    ///
    /// A `mailto:` with nothing in it (`mailto:`) is still `Some`: the link
    /// asked for a new message and an empty composer is the right answer.
    pub fn parse(uri: &str) -> Option<Self> {
        let rest = uri
            .trim()
            .get(..7)
            .filter(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
            .and_then(|_| uri.trim().get(7..))?;
        // RFC 6068 has no authority, but GIO gives every URI one:
        // `g_file_new_for_uri("mailto:ada@example.com")` reads back as
        // `mailto:///ada@example.com`, and that is the form the desktop hands
        // an application for a clicked link. `mailto://` is the same thing
        // typed by hand. Neither slash is part of an address.
        let rest = rest.trim_start_matches('/');
        let (path, query) = match rest.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (rest, None),
        };
        let mut mailto = Self {
            to: parse_list(&decode(path)),
            ..Self::default()
        };
        for pair in query.into_iter().flat_map(|query| query.split('&')) {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = decode(value);
            match name.to_ascii_lowercase().as_str() {
                "to" => mailto.to.extend(parse_list(&value)),
                "cc" => mailto.cc.extend(parse_list(&value)),
                "bcc" => mailto.bcc.extend(parse_list(&value)),
                "subject" => mailto.subject = Some(value),
                "body" => mailto.body = Some(value),
                _ => {}
            }
        }
        Some(mailto)
    }

    /// A new draft for `account` with these fields filled in.
    pub fn into_draft(self, account: AccountId) -> Draft {
        let mut draft = Draft::new(account);
        draft.to = self.to;
        draft.cc = self.cc;
        draft.bcc = self.bcc;
        draft.subject = self.subject.unwrap_or_default();
        draft.body = MessageBody {
            text: self.body,
            html: None,
        };
        draft
    }
}

/// Percent-decoding as RFC 3986 spells it: `%XX` is a byte, everything else
/// is itself. A `+` stays a `+` (RFC 6068 §5). Bytes that do not form UTF-8
/// are replaced rather than refused: a mangled subject is still a subject.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(i + 1).and_then(|b| (*b as char).to_digit(16)),
                bytes.get(i + 2).and_then(|b| (*b as char).to_digit(16)),
            )
        {
            out.push((high * 16 + low) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_address_becomes_the_recipient() {
        let mailto = Mailto::parse("mailto:ada@example.com").expect("a mailto uri");
        assert_eq!(mailto.to.len(), 1);
        assert_eq!(mailto.to[0].address, "ada@example.com");
        assert_eq!(mailto.subject, None);
        assert_eq!(mailto.body, None);
    }

    #[test]
    fn subject_and_body_are_percent_decoded_and_plus_is_literal() {
        let mailto = Mailto::parse(
            "mailto:ada@example.com?subject=Re%3A%20the%20plan&body=one%0Atwo+three%20%E2%9C%93",
        )
        .expect("a mailto uri");
        assert_eq!(mailto.subject.as_deref(), Some("Re: the plan"));
        assert_eq!(mailto.body.as_deref(), Some("one\ntwo+three ✓"));
    }

    #[test]
    fn every_recipient_field_is_read_and_header_names_are_case_insensitive() {
        let mailto = Mailto::parse(
            "MAILTO:ada@example.com,grace@example.net?To=alan@example.org&CC=lena%40example.com&bcc=q@example.test",
        )
        .expect("a mailto uri");
        let addresses =
            |list: &[EmailAddress]| list.iter().map(|a| a.address.clone()).collect::<Vec<_>>();
        assert_eq!(
            addresses(&mailto.to),
            ["ada@example.com", "grace@example.net", "alan@example.org"]
        );
        assert_eq!(addresses(&mailto.cc), ["lena@example.com"]);
        assert_eq!(addresses(&mailto.bcc), ["q@example.test"]);
    }

    #[test]
    fn a_header_a_draft_has_no_field_for_is_ignored() {
        // RFC 6068 §6: a client must not let a URI set arbitrary headers.
        let mailto = Mailto::parse("mailto:ada@example.com?In-Reply-To=%3Cx%40y%3E&subject=hi")
            .expect("a mailto uri");
        assert_eq!(mailto.subject.as_deref(), Some("hi"));
        assert_eq!(mailto.to.len(), 1);
    }

    #[test]
    fn the_slashes_gio_puts_after_the_scheme_are_not_part_of_the_address() {
        // `g_file_new_for_uri("mailto:ada@example.com").get_uri()` answers
        // `mailto:///ada@example.com`: GIO normalises a scheme with no
        // authority by inserting one. That is exactly the form the desktop
        // hands an application for a clicked link, so it has to read as the
        // link, not as an address that starts with three slashes. The
        // `mailto://` spelling seen in the wild is the same fix.
        for uri in [
            "mailto:///ada@example.com?subject=Lunch",
            "mailto://ada@example.com?subject=Lunch",
        ] {
            let mailto = Mailto::parse(uri).expect("a mailto uri");
            assert_eq!(mailto.to.len(), 1, "{uri}");
            assert_eq!(mailto.to[0].address, "ada@example.com", "{uri}");
            assert_eq!(mailto.subject.as_deref(), Some("Lunch"), "{uri}");
        }
    }

    #[test]
    fn an_empty_mailto_asks_for_an_empty_message() {
        assert_eq!(Mailto::parse("mailto:"), Some(Mailto::default()));
    }

    #[test]
    fn anything_else_is_not_a_mailto() {
        assert_eq!(Mailto::parse("https://example.com/"), None);
        assert_eq!(Mailto::parse("ada@example.com"), None);
        assert_eq!(Mailto::parse(""), None);
    }

    #[test]
    fn the_draft_carries_every_field_and_nothing_else() {
        let draft =
            Mailto::parse("mailto:ada@example.com?cc=grace@example.net&subject=Lunch&body=Noon%3F")
                .expect("a mailto uri")
                .into_draft(AccountId::new(7));
        assert_eq!(draft.account_id, AccountId::new(7));
        assert_eq!(draft.to[0].address, "ada@example.com");
        assert_eq!(draft.cc[0].address, "grace@example.net");
        assert!(draft.bcc.is_empty());
        assert_eq!(draft.subject, "Lunch");
        assert_eq!(draft.body.text.as_deref(), Some("Noon?"));
        assert_eq!(draft.body.html, None);
        assert!(draft.in_reply_to.is_none());
    }
}
