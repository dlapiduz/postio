//! One-click unsubscribe: what a message offers, and what the privacy pane
//! says afterwards (#971).
//!
//! `PRODUCT.md` lists one-click unsubscribe among Postio's privacy features
//! and CLAUDE.md states the rule it keeps: **only on deliberate activation**.
//! Three decisions make that true and none of them is a widget — which list
//! a message belongs to, whether the reader may offer to leave it at all, and
//! what the banner says about it — so they live here rather than in either
//! frontend. The GTK banner (`postio_gtk::reader::banner::UnsubscribeBanner`)
//! and the macOS one call the same three functions, which is ADR 0019 Q6's
//! answer to a privacy rule forking silently across two readers.
//!
//! Nothing here contacts anything. These are words and a predicate; the
//! activation itself is a write the composition root makes, because it is
//! the layer that has a store.

use chrono::{DateTime, Utc};
use postio_model::{DraftState, EmailAddress, UnsubscribeActivation};

use crate::reader::header::ReaderAction;

/// What the button is labelled.
///
/// The verb, alone: "Unsubscribe" is what the sender's own footer calls it,
/// and a reader hunting for that word in a newsletter should find it in the
/// application's chrome instead. Shared so the two frontends do not end up
/// with a button that says "Leave list" on one platform.
pub const ACTION: &str = "Unsubscribe";

/// A message that came from a list, and what the reader offers about it.
///
/// Absent — `None` from [`offer`] — is the ordinary case for personal mail
/// and the mandatory one for outgoing mail; see [`ReaderAction::unsubscribable`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// What a click would be recorded against: the `List-Id` when the message
    /// carries one, the sender's domain otherwise. Not a URL — see the module
    /// documentation on `postio_model::unsubscribe`.
    pub list_identifier: String,
    /// The sentence the banner shows.
    pub summary: String,
}

/// Which list a message counts as being from.
///
/// `List-Id` (RFC 2919) when the sender set one, and the sender's domain
/// otherwise — because the fallback is what makes the banner useful on the
/// bulk mail that does *not* announce itself as a list, which is most of it.
/// Moved here from `postio-app`'s reading wiring, where it was the only copy
/// and the macOS reader could not reach it.
///
/// `None` only when a message has neither, which means it has no sender
/// either: a row that is on its way out of the store.
pub fn list_identifier(list_id: Option<&str>, from: &[EmailAddress]) -> Option<String> {
    list_id
        .map(str::to_owned)
        .or_else(|| from.first()?.domain().map(str::to_owned))
}

/// What the banner says above the message.
pub fn summary(list: &str) -> String {
    format!("This message is from {list}")
}

/// What the reader offers for this message, or nothing.
///
/// `send_state` is the gate, not an afterthought: the identifier falls back
/// to the sender's domain, and the sender of an outgoing message is the user,
/// so a banner over the Outbox offers to unsubscribe somebody from their own
/// account (#1525). [`ReaderAction::unsubscribable`] is the one place that
/// rule is written.
pub fn offer(
    send_state: Option<DraftState>,
    list_id: Option<&str>,
    from: &[EmailAddress],
) -> Option<Offer> {
    if !ReaderAction::unsubscribable(send_state) {
        return None;
    }
    let list_identifier = list_identifier(list_id, from)?;
    Some(Offer {
        summary: summary(&list_identifier),
        list_identifier,
    })
}

/// The date an activation is listed under: `2026-09-21`.
///
/// A plain ISO date rather than a relative span. The privacy pane's question
/// is "what have I left, and when", which is answered by a date somebody can
/// match against their mail — not by "3 weeks ago", which they would have to
/// do arithmetic on to compare with anything.
pub fn activated_on(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d").to_string()
}

/// The whole row as one sentence, for a screen reader: `Left news.example.org
/// on 2026-09-21`.
///
/// The pane draws the list and the date in two columns, which a screen reader
/// would otherwise announce as two unrelated fragments.
pub fn activation_label(list: &str, at: DateTime<Utc>) -> String {
    format!("Left {list} on {}", activated_on(at))
}

/// Put one merged activation log in the order the privacy pane reads it:
/// newest first, and within a millisecond, newest row first.
///
/// # Why this is not just a `sort_by_key`
///
/// The log is read one account at a time — `UnsubscribeRepository::for_account`
/// — so a pane that draws every account, which both of Postio's do, is
/// looking at several already-sorted lists stuck end to end. Sorting the
/// join by `activated_at` alone is a *stable* sort, so a pair of rows
/// sharing a millisecond keeps whatever order the concatenation happened to
/// put them in, which is the order the accounts came back from the account
/// table. The same two rows read through one account come back
/// `activated_at DESC, id DESC`. So the pane would show one order for a
/// machine with one account and the other for a machine with two, over
/// identical data.
///
/// `activated_at` is milliseconds in the store, and an activation is a
/// button press, so the tie is rare in a person's hands and routine in a
/// test that writes two rows in a loop — which is exactly the kind of
/// difference that shows up as an intermittent failure rather than as a bug
/// report. Breaking the tie by row id, descending, is what the log itself
/// does; doing the same here means the merged list and the per-account list
/// never disagree.
///
/// Shared because it is the same pane on both platforms and the answer is
/// arithmetic over `postio-model` rows — no toolkit, no store.
pub fn newest_first(activations: &mut [UnsubscribeActivation]) {
    activations.sort_by_key(|activation| {
        std::cmp::Reverse((activation.activated_at, activation.id.get()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One stored activation: whose account, which row, what list, and the
    /// millisecond it was written at.
    fn activation(account: i64, id: i64, list: &str, millis: i64) -> UnsubscribeActivation {
        UnsubscribeActivation {
            id: postio_model::ids::UnsubscribeActivationId::new(id),
            account_id: postio_model::ids::AccountId::new(account),
            list_identifier: list.to_owned(),
            activated_at: chrono::DateTime::from_timestamp_millis(millis).expect("an instant"),
        }
    }

    fn listed(activations: &[UnsubscribeActivation]) -> Vec<&str> {
        activations
            .iter()
            .map(|activation| activation.list_identifier.as_str())
            .collect()
    }

    #[test]
    fn every_accounts_activations_interleave_into_one_list_newest_first() {
        // What a caller hands over: each account's own log, already
        // newest-first on its own, one after the other. Two accounts whose
        // activations alternate in time are the whole reason the merged list
        // needs sorting at all.
        let mut merged = vec![
            activation(1, 7, "late.first.example.org", 3_000),
            activation(1, 3, "early.first.example.org", 1_000),
            activation(2, 9, "late.second.example.org", 4_000),
            activation(2, 5, "early.second.example.org", 2_000),
        ];
        newest_first(&mut merged);
        assert_eq!(
            listed(&merged),
            [
                "late.second.example.org",
                "late.first.example.org",
                "early.second.example.org",
                "early.first.example.org",
            ]
        );
    }

    #[test]
    fn two_activations_in_one_millisecond_are_ordered_by_row_the_way_the_log_is() {
        // `activated_at` is stored to the millisecond, and each account's own
        // read breaks a tie with `id DESC`. A merged list that broke it
        // differently would put a pair of rows in one order when the two
        // accounts are read together and the other when either is read alone
        // -- and which of those a reader saw would depend on how many
        // accounts they happen to have.
        let mut merged = vec![
            activation(1, 4, "older.row.example.org", 1_000),
            activation(2, 8, "newer.row.example.org", 1_000),
        ];
        newest_first(&mut merged);
        assert_eq!(
            listed(&merged),
            ["newer.row.example.org", "older.row.example.org"]
        );
    }

    fn from(address: &str) -> Vec<EmailAddress> {
        vec![EmailAddress::new(Some("Weekly Digest"), address)]
    }

    #[test]
    fn a_list_header_is_what_the_banner_names() {
        assert_eq!(
            list_identifier(Some("news.example.org"), &from("weekly@mail.example.org")),
            Some("news.example.org".to_owned()),
            "the sender said which list this is; nothing should second-guess it"
        );
    }

    #[test]
    fn bulk_mail_with_no_list_header_falls_back_to_the_senders_domain() {
        // The case that matters most in practice: almost no commercial mail
        // sets `List-Id`, and a banner that only worked for the mail that did
        // would be a feature nobody ever saw.
        assert_eq!(
            list_identifier(None, &from("weekly@news.example.org")),
            Some("news.example.org".to_owned())
        );
    }

    #[test]
    fn a_message_with_neither_offers_nothing_to_leave() {
        assert_eq!(list_identifier(None, &[]), None);
        // A sender with no `@` in it is not a domain, and half an address is
        // not something to log an activation against.
        assert_eq!(
            list_identifier(None, &[EmailAddress::new(None::<String>, "mailer-daemon")]),
            None
        );
    }

    #[test]
    fn the_banner_names_the_list_in_its_sentence() {
        assert_eq!(
            summary("news.example.org"),
            "This message is from news.example.org",
            "the sentence has to carry the list, or the button acts on something unnamed"
        );
    }

    #[test]
    fn an_ordinary_message_from_a_list_is_offered() {
        let offered = offer(
            None,
            Some("news.example.org"),
            &from("weekly@mail.example.org"),
        )
        .expect("a message from a list is offered an unsubscribe");
        assert_eq!(offered.list_identifier, "news.example.org");
        assert_eq!(offered.summary, summary("news.example.org"));
    }

    #[test]
    fn nothing_on_its_way_out_is_ever_offered_one() {
        // #1525, through the one predicate that decides it. The domain
        // fallback means the offer over the Outbox would name the user's own
        // account and the activation log would record them leaving it.
        for state in [
            DraftState::Queued,
            DraftState::Sending,
            DraftState::Failed,
            DraftState::Unconfirmed,
            DraftState::Editing,
        ] {
            assert_eq!(
                offer(
                    Some(state),
                    Some("news.example.org"),
                    &from("ada@example.com")
                ),
                None,
                "{state:?} is offered an unsubscribe from the user's own mail"
            );
        }
        // A draft the server has taken is ordinary mail in Sent, and the
        // gate is `ReaderAction::unsubscribable`'s to move, not this one's.
        assert_eq!(
            offer(
                Some(DraftState::Sent),
                Some("news.example.org"),
                &from("weekly@mail.example.org")
            )
            .is_some(),
            ReaderAction::unsubscribable(Some(DraftState::Sent)),
            "the offer and the predicate have to agree about a message already sent"
        );
    }

    #[test]
    fn an_activation_is_listed_under_a_plain_date() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-09-21T14:03:00Z")
            .expect("a fixed instant")
            .with_timezone(&Utc);
        assert_eq!(activated_on(at), "2026-09-21");
        assert_eq!(
            activation_label("news.example.org", at),
            "Left news.example.org on 2026-09-21"
        );
    }
}
