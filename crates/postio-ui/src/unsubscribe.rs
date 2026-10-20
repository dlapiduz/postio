//! One-click unsubscribe: what a message offers, and what the privacy pane
//! says afterwards (#971).
//!
//! `PRODUCT.md` lists one-click unsubscribe among Postio's privacy features
//! and CLAUDE.md states the rule it keeps: **only on deliberate activation**.
//! Three decisions make that true and none of them is a widget — which list
//! a message belongs to, whether the reader may offer to leave it at all, and
//! what the banner says about it — so they live here rather than in either
//! frontend. The GTK banner
//! and the macOS one call the same three functions, which is ADR 0019 Q6's
//! answer to a privacy rule forking silently across two readers.
//!
//! Nothing here contacts anything. These are words and a predicate; the
//! activation itself is a write the composition root makes, because it is
//! the layer that has a store.

use chrono::{DateTime, Utc};
use postio_config::AfterUnsubscribe;
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
/// Moved here from the classic app's reading wiring, where it was the only copy
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

/// What the reader's notice band shows for this message, or nothing.
///
/// The band appears only when the message really offers to leave a list: it
/// has a `List-Id`, or it carries a `List-Unsubscribe`
/// (`messages.unsubscribe_offered`, `None` while unknown). Personal mail has
/// neither and stays clean. This gates the *band* only; [`offer`] stays the
/// rule for `U`, which works on any message that rule allows.
pub fn banner(
    send_state: Option<DraftState>,
    list_id: Option<&str>,
    unsubscribe_offered: Option<bool>,
    from: &[EmailAddress],
) -> Option<Offer> {
    if list_id.is_none() && unsubscribe_offered != Some(true) {
        return None;
    }
    offer(send_state, list_id, from)
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

/// Which of an account's mail "what this list sent" means (#1931).
///
/// Narrower than [`list_identifier`] on purpose: that one falls back to the
/// sender's *domain*, which is right for a note in the privacy pane and wrong
/// for something that moves mail, because `gmail.com` is not a list. Here the
/// fallback is the exact sender address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SweepScope {
    /// Messages carrying this `List-Id`.
    ListId(String),
    /// Messages from exactly this address, for a sender that names no list.
    Sender(String),
}

impl SweepScope {
    /// What the offer calls it: the list, or the sender's address.
    pub fn name(&self) -> &str {
        match self {
            Self::ListId(list) | Self::Sender(list) => list,
        }
    }
}

/// The mail an unsubscribe could also clear: the `List-Id` if the message has
/// one, else its first sender's exact address, else nothing.
pub fn sweep_scope(list_id: Option<&str>, from: &[EmailAddress]) -> Option<SweepScope> {
    match list_id.map(str::trim).filter(|list| !list.is_empty()) {
        Some(list) => Some(SweepScope::ListId(list.to_owned())),
        None => from
            .first()
            .map(|sender| SweepScope::Sender(sender.normalized())),
    }
}

/// The verb of the one action the notice offers: the registry's own word,
/// not a new one. `Delete` moves to the Trash and Undo takes it back.
pub fn sweep_verb(action: AfterUnsubscribe) -> &'static str {
    match action {
        AfterUnsubscribe::Archive => "Archive",
        AfterUnsubscribe::Delete => "Delete",
    }
}

/// What the offer says once the unsubscribe is recorded: how much mail the
/// list left behind, or `None` when there is nothing to offer.
///
/// It begins with the confirmation, [`crate::focus_target::unsubscribed`],
/// naming the same list or address the sweep acts on, so there is one
/// sentence on screen and not a confirmation and an offer that disagree.
///
/// `more` is the storage page being full: the count is then a floor, and
/// says so rather than claiming a number it did not read.
pub fn sweep_offer(scope: &SweepScope, count: usize, more: bool) -> Option<String> {
    if count == 0 {
        return None;
    }
    let plus = if more { "+" } else { "" };
    let (noun, verb) = if count == 1 && !more {
        ("message", "is")
    } else {
        ("messages", "are")
    };
    Some(format!(
        "{}. {count}{plus} {noun} from it {verb} still here",
        crate::focus_target::unsubscribed(scope.name())
    ))
}

/// The action's label with its count: `Archive 14`, `Delete 5000+`.
pub fn sweep_button(action: AfterUnsubscribe, count: usize, more: bool) -> String {
    format!(
        "{} {count}{}",
        sweep_verb(action),
        if more { "+" } else { "" }
    )
}

/// The non-blocking notice after leaving a list (#1952): what it says, and
/// the ONE action it offers, in the configured verb. Ignoring it, or `Esc`,
/// keeps everything; pressing the button is one undoable bulk command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepNotice {
    /// The sentence: the confirmation, then how much is still here.
    pub text: String,
    /// The button: `Archive 14`.
    pub button: String,
    /// What the button does with the mail.
    pub action: AfterUnsubscribe,
}

impl SweepNotice {
    /// The one command the button runs, on exactly the messages the notice
    /// counted: one bulk archive or delete, so one undo. The button, the
    /// registry's `clear_list_mail` key and the palette row all run this.
    pub fn command(&self, ids: Vec<postio_model::MessageId>) -> postio_core::Command {
        let target = postio_core::MessageTarget::Messages(ids);
        match self.action {
            AfterUnsubscribe::Archive => postio_core::Command::Archive { target },
            AfterUnsubscribe::Delete => postio_core::Command::Delete { target },
        }
    }
}

/// What `clear_list_mail` says when no notice is offering anything: the key
/// is never silent (#1953).
pub const NO_OFFER: &str = "No unsubscribe offer to take up";

/// The notice for a list that left `count` messages behind, or `None` when
/// nothing is left (the plain confirmation then says all there is to say).
pub fn sweep_notice(
    scope: &SweepScope,
    count: usize,
    more: bool,
    action: AfterUnsubscribe,
) -> Option<SweepNotice> {
    Some(SweepNotice {
        text: sweep_offer(scope, count, more)?,
        button: sweep_button(action, count, more),
        action,
    })
}

/// What the confirmation names for the message just left: the sweep's own
/// scope, so the exact sender address when the message names no list and
/// never the domain the activation log files it under.
pub fn confirmation_name(list_id: Option<&str>, from: &[EmailAddress]) -> Option<String> {
    sweep_scope(list_id, from).map(|scope| scope.name().to_owned())
}

/// The setting's heading in a settings pane (#1952).
pub const SETTING_HEADING: &str = "After you unsubscribe";

/// What the setting does, said plainly because it is the one place a person
/// learns that the notice never acts by itself.
pub const SETTING_NOTE: &str = "The notice after you unsubscribe offers this for the mail the list \
     already sent. Nothing moves unless you press it, and Undo takes it back.";

/// The choices a settings pane offers, the default first.
pub const SETTING_CHOICES: [AfterUnsubscribe; 2] =
    [AfterUnsubscribe::Archive, AfterUnsubscribe::Delete];

/// What a declined offer, or a finished one, leaves on screen is the
/// command's own notice ("Archived 14 messages"), so there is no wording here
/// for it: the verb is one the registry already says.
#[cfg(test)]
mod sweep_tests {
    use super::*;

    fn sender(address: &str) -> Vec<EmailAddress> {
        vec![EmailAddress::new(Some("Shop"), address)]
    }

    #[test]
    fn a_list_id_is_the_scope_and_the_domain_never_is() {
        assert_eq!(
            sweep_scope(Some("news.example.org"), &sender("a@mail.example.org")),
            Some(SweepScope::ListId("news.example.org".into()))
        );
        assert_eq!(
            sweep_scope(None, &sender("Shop@Example.COM")),
            Some(SweepScope::Sender("shop@example.com".into())),
            "the exact address, not example.com"
        );
        assert_eq!(
            sweep_scope(Some("  "), &sender("a@example.com")),
            Some(SweepScope::Sender("a@example.com".into())),
            "a blank List-Id names no list"
        );
        assert_eq!(sweep_scope(None, &[]), None);
    }

    #[test]
    fn the_offer_counts_and_is_silent_when_there_is_nothing() {
        let scope = SweepScope::ListId("news.example.org".into());
        assert_eq!(sweep_offer(&scope, 0, false), None);
        assert_eq!(
            sweep_offer(&scope, 1, false).unwrap(),
            "Unsubscribed from news.example.org. 1 message from it is still here"
        );
        assert!(
            sweep_offer(&scope, 14, false)
                .unwrap()
                .contains("14 messages")
        );
        assert!(
            sweep_offer(&scope, 5000, true)
                .unwrap()
                .contains("5000+ messages"),
            "a full page is a floor"
        );
    }

    #[test]
    fn the_notice_offers_one_action_and_archive_is_the_default() {
        let scope = SweepScope::ListId("news.example.org".into());
        let archive = sweep_notice(&scope, 14, false, AfterUnsubscribe::default()).unwrap();
        assert_eq!(archive.action, AfterUnsubscribe::Archive);
        assert_eq!(archive.button, "Archive 14");
        let delete = sweep_notice(&scope, 14, false, AfterUnsubscribe::Delete).unwrap();
        assert_eq!(delete.button, "Delete 14", "the setting picks the verb");
        assert_eq!(delete.text, archive.text, "the words do not change with it");
        assert_eq!(
            sweep_notice(&scope, 5000, true, AfterUnsubscribe::Archive)
                .unwrap()
                .button,
            "Archive 5000+"
        );
        assert_eq!(
            sweep_notice(&scope, 0, false, AfterUnsubscribe::Delete),
            None,
            "nothing left, nothing to offer"
        );
    }

    #[test]
    fn the_notice_command_acts_on_exactly_the_counted_messages() {
        use postio_core::{Command, MessageTarget};
        use postio_model::MessageId;
        let scope = SweepScope::ListId("news.example.org".into());
        let ids = vec![MessageId::new(4), MessageId::new(9)];
        let archive = sweep_notice(&scope, 2, false, AfterUnsubscribe::Archive).unwrap();
        assert_eq!(
            archive.command(ids.clone()),
            Command::Archive {
                target: MessageTarget::Messages(ids.clone())
            }
        );
        let delete = sweep_notice(&scope, 2, false, AfterUnsubscribe::Delete).unwrap();
        assert_eq!(
            delete.command(ids.clone()),
            Command::Delete {
                target: MessageTarget::Messages(ids)
            }
        );
    }

    #[test]
    fn clear_list_mail_is_a_bound_undoable_registry_command() {
        use postio_core::{CommandId, Recovery, registry};
        let spec = registry::get(CommandId::ClearListMail);
        assert_eq!(spec.default_binding, "C");
        assert_eq!(spec.recovery, Recovery::Undo);
        assert!(!NO_OFFER.is_empty());
    }

    #[test]
    fn the_confirmation_names_the_exact_sender_never_the_domain() {
        // No List-Id: the address the sweep acts on, not `example.org`.
        assert_eq!(
            confirmation_name(None, &sender("Shop@Mail.Example.org")).as_deref(),
            Some("shop@mail.example.org")
        );
        // A List-Id wins, as it does for the sweep.
        assert_eq!(
            confirmation_name(Some("news.example.org"), &sender("a@mail.example.org")).as_deref(),
            Some("news.example.org")
        );
        assert_eq!(confirmation_name(None, &[]), None);
    }

    #[test]
    fn the_setting_offers_archive_first_and_says_nothing_moves_by_itself() {
        assert_eq!(SETTING_CHOICES[0], AfterUnsubscribe::default());
        let named: Vec<&str> = SETTING_CHOICES.iter().map(|a| sweep_verb(*a)).collect();
        assert_eq!(named, ["Archive", "Delete"]);
        assert!(SETTING_NOTE.contains("Nothing moves") && SETTING_NOTE.contains("Undo"));
    }

    #[test]
    fn the_offer_opens_with_the_confirmation_it_replaces() {
        let scope = SweepScope::Sender("shop@example.com".into());
        let said = sweep_offer(&scope, 3, false).unwrap();
        assert!(
            said.starts_with(&crate::focus_target::unsubscribed(scope.name())),
            "{said}"
        );
    }
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
    fn the_band_shows_for_a_list_id_or_a_list_unsubscribe_and_not_for_personal_mail() {
        let sender = from("ada@example.org");
        let band = |send_state, list_id, offered| banner(send_state, list_id, offered, &sender);
        assert_eq!(
            band(None, Some("news.example.org"), Some(false)).map(|o| o.list_identifier),
            Some("news.example.org".to_owned()),
            "a List-Id is a list"
        );
        assert_eq!(
            band(None, None, Some(true)).map(|o| o.list_identifier),
            Some("example.org".to_owned()),
            "a List-Unsubscribe alone offers the sender's domain"
        );
        assert_eq!(band(None, None, Some(false)), None, "personal mail");
        assert_eq!(band(None, None, None), None, "not known is not offered");
        assert_eq!(
            band(Some(DraftState::Queued), Some("x.example.org"), Some(true)),
            None,
            "outgoing mail has none"
        );
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
