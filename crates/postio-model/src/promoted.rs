//! The three headers Focus reads before a body arrives: `List-Unsubscribe`, `Precedence`,
//! `Auto-Submitted`. Promoted (ADR 0025) so the filing pass can tell bulk and automated mail at
//! arrival; one reading here keeps the fetch, the body and `is:bulk`/`is:automated` in agreement.

use serde::{Deserialize, Serialize};

use crate::headers::Headers;

pub const PRECEDENCE_BULK: u8 = 1;
pub const PRECEDENCE_LIST: u8 = 2;
pub const PRECEDENCE_JUNK: u8 = 4;
/// `Auto-Submitted: auto-generated`, or any value but `no` and
/// `auto-replied`: RFC 3834 §5 asks a reader to treat an unknown `auto-`
/// keyword as automatic.
pub const AUTO_GENERATED: u8 = 8;
pub const AUTO_REPLIED: u8 = 16;

/// What `is:bulk` reads besides `List-Unsubscribe`.
pub const PRECEDENCE: u8 = PRECEDENCE_BULK | PRECEDENCE_LIST | PRECEDENCE_JUNK;
pub const AUTO_SUBMITTED: u8 = AUTO_GENERATED | AUTO_REPLIED;

/// A header only a bulk sending service writes, on mail that may carry no list header at all: a
/// bank's statement, a utility's bill (spec 007 T225).
pub const BULK_SENDER: u8 = 32;

/// Generic headers, not one provider's: a feedback loop's id and the senders' certification.
pub const BULK_SENDER_FIELDS: [&str; 2] = ["Feedback-ID", "X-CSA-Complaints"];

pub const FIELDS: [&str; 5] = [
    "List-Unsubscribe",
    "Precedence",
    "Auto-Submitted",
    BULK_SENDER_FIELDS[0],
    BULK_SENDER_FIELDS[1],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct PromotedHeaders {
    pub unsubscribe_offered: bool,
    pub automation: u8,
}

impl PromotedHeaders {
    /// What `headers` say. Every occurrence of a field counts: a message
    /// that says `Precedence` twice is read for both.
    pub fn from_headers(headers: &Headers) -> Self {
        let unsubscribe_offered = headers
            .get_all("List-Unsubscribe")
            .iter()
            .any(|value| !value.trim().is_empty());
        let precedence =
            headers
                .get_all("Precedence")
                .into_iter()
                .map(|value| match keyword(value).as_str() {
                    "bulk" => PRECEDENCE_BULK,
                    "list" => PRECEDENCE_LIST,
                    "junk" => PRECEDENCE_JUNK,
                    _ => 0,
                });
        let auto_submitted = headers.get_all("Auto-Submitted").into_iter().map(|value| {
            match keyword(value).as_str() {
                "" | "no" => 0,
                "auto-replied" => AUTO_REPLIED,
                _ => AUTO_GENERATED,
            }
        });
        let bulk_sender = BULK_SENDER_FIELDS.iter().map(|name| {
            if headers
                .get_all(name)
                .iter()
                .any(|value| !value.trim().is_empty())
            {
                BULK_SENDER
            } else {
                0
            }
        });
        Self {
            unsubscribe_offered,
            automation: precedence
                .chain(auto_submitted)
                .chain(bulk_sender)
                .fold(0, |bits, bit| bits | bit),
        }
    }
}

fn keyword(value: &str) -> String {
    value
        .trim()
        .split(|character: char| character.is_whitespace() || matches!(character, ';' | '('))
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(fields: &[(&str, &str)]) -> PromotedHeaders {
        let mut headers = Headers::new();
        for (name, value) in fields {
            headers.push(*name, *value);
        }
        PromotedHeaders::from_headers(&headers)
    }

    type Case<'a> = (&'a [(&'a str, &'a str)], bool, u8);

    #[test]
    fn every_field_read_is_fetched() {
        for name in ["List-Unsubscribe", "Precedence", "Auto-Submitted"]
            .into_iter()
            .chain(BULK_SENDER_FIELDS)
        {
            assert!(FIELDS.contains(&name), "{name} is read but never fetched");
        }
    }

    #[test]
    fn each_field_sets_what_it_says_and_nothing_else() {
        let cases: &[Case<'_>] = &[
            (&[], false, 0),
            (
                &[("List-Unsubscribe", "<https://news.example.org/u/8f21c9>")],
                true,
                0,
            ),
            (&[("Precedence", "bulk")], false, PRECEDENCE_BULK),
            (&[("Precedence", "list")], false, PRECEDENCE_LIST),
            (&[("Precedence", "junk")], false, PRECEDENCE_JUNK),
            (
                &[("Auto-Submitted", "auto-generated")],
                false,
                AUTO_GENERATED,
            ),
            (&[("Auto-Submitted", "auto-replied")], false, AUTO_REPLIED),
            // RFC 3834 §5: any keyword but `no` is automatic.
            (
                &[("Auto-Submitted", "auto-notified")],
                false,
                AUTO_GENERATED,
            ),
            (&[("Auto-Submitted", "no")], false, 0),
            (&[("PRECEDENCE", "  Bulk ")], false, PRECEDENCE_BULK),
            (
                &[("precedence", "list (mailing list)")],
                false,
                PRECEDENCE_LIST,
            ),
            (
                &[(
                    "Auto-Submitted",
                    "Auto-Replied; owner-email=\"ada@example.com\"",
                )],
                false,
                AUTO_REPLIED,
            ),
            (&[("Precedence", "first-class")], false, 0),
            // A bulk sending service's own headers, on mail that carries no list header (T225).
            (
                &[("Feedback-ID", "4821:statements:bank.example:sender")],
                false,
                BULK_SENDER,
            ),
            (
                &[("X-CSA-Complaints", "csa-complaints@example.net")],
                false,
                BULK_SENDER,
            ),
            (&[("Feedback-ID", "  ")], false, 0),
            (&[("List-Unsubscribe", "   ")], false, 0),
            (
                &[
                    ("List-Unsubscribe", "<mailto:leave@lists.example.org>"),
                    ("Precedence", "list"),
                    ("Auto-Submitted", "auto-generated"),
                ],
                true,
                PRECEDENCE_LIST | AUTO_GENERATED,
            ),
        ];
        for (fields, unsubscribe_offered, automation) in cases {
            assert_eq!(
                read(fields),
                PromotedHeaders {
                    unsubscribe_offered: *unsubscribe_offered,
                    automation: *automation,
                },
                "{fields:?}"
            );
        }
    }

    #[test]
    fn a_parsed_message_says_what_its_promoted_headers_say() {
        let at = chrono::DateTime::<chrono::Utc>::UNIX_EPOCH;
        let (account, mailbox) = (crate::AccountId::new(1), crate::MailboxId::new(1));
        let newsletter = b"From: Ledger <news@ledger.example>\r\n\
                           List-Unsubscribe: <https://ledger.example/u/9>\r\n\
                           Precedence: bulk\r\n\
                           Subject: This week\r\n\r\nThe numbers.\r\n";
        for parsed in [
            crate::mime::parse(newsletter),
            crate::mime::parse_headers(newsletter),
        ] {
            assert_eq!(
                parsed.into_message(account, mailbox, at).promoted,
                Some(PromotedHeaders {
                    unsubscribe_offered: true,
                    automation: PRECEDENCE_BULK,
                })
            );
        }
        let plain = b"From: Ada <ada@example.com>\r\nSubject: Lunch\r\n\r\nNoon?\r\n";
        assert_eq!(
            crate::mime::parse(plain)
                .into_message(account, mailbox, at)
                .promoted,
            Some(PromotedHeaders::default()),
            "headers that say none of the three are known to say nothing"
        );
        assert_eq!(
            crate::mime::parse(b"")
                .into_message(account, mailbox, at)
                .promoted,
            None,
            "bytes with no header block say nothing either way"
        );
    }

    #[test]
    fn a_field_said_twice_is_read_both_times() {
        assert_eq!(
            read(&[("Precedence", "list"), ("Precedence", "bulk")]).automation,
            PRECEDENCE_LIST | PRECEDENCE_BULK
        );
    }

    #[test]
    fn the_precedence_and_auto_submitted_bits_do_not_overlap() {
        assert_eq!(PRECEDENCE & AUTO_SUBMITTED, 0);
        assert_eq!(PRECEDENCE | AUTO_SUBMITTED, 31, "five bits, each one field");
    }
}
