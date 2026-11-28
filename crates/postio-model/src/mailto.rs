//! A `mailto:` URI read into draft fields (RFC 6068): percent-encoded, `+` is a literal plus,
//! header names case-insensitive.
//! Only `to`, `cc`, `bcc`, `subject`, `body` are read; other headers are ignored, not smuggled into
//! a message (RFC 6068 §6).

use crate::address::{EmailAddress, parse_list};
use crate::draft::Draft;
use crate::ids::AccountId;
use crate::message::MessageBody;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mailto {
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub subject: Option<String>,
    /// `None` when the link named none; a link cannot ask for HTML.
    pub body: Option<String>,
}

impl Mailto {
    /// `None` when `uri` is not a `mailto:` URI at all; a bare `mailto:` is still `Some`, since an
    /// empty composer is the right answer.
    pub fn parse(uri: &str) -> Option<Self> {
        let rest = uri
            .trim()
            .get(..7)
            .filter(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
            .and_then(|_| uri.trim().get(7..))?;
        // RFC 6068 has no authority, but GIO gives every URI one (`mailto:///ada@example.com`),
        // which is what the desktop hands over for a clicked link; `mailto://` is the same by hand.
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

/// `+` stays `+` (RFC 6068 §5); non-UTF-8 bytes are replaced, not refused: a mangled subject is
/// still a subject.
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
        let mailto = Mailto::parse("mailto:ada@example.com?In-Reply-To=%3Cx%40y%3E&subject=hi")
            .expect("a mailto uri");
        assert_eq!(mailto.subject.as_deref(), Some("hi"));
        assert_eq!(mailto.to.len(), 1);
    }

    #[test]
    fn the_slashes_gio_puts_after_the_scheme_are_not_part_of_the_address() {
        // GIO normalises a scheme with no authority to `mailto:///...`, the form the desktop hands
        // over; `mailto://` is the same fix.
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
