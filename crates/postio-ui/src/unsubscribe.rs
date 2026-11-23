//! One-click unsubscribe: what a message offers and what the privacy pane says afterwards (#971).
//! Only on deliberate activation; nothing here contacts anything.

use chrono::{DateTime, Utc};
use postio_config::AfterUnsubscribe;
use postio_model::{DraftState, EmailAddress, UnsubscribeActivation};

use crate::reader::header::ReaderAction;

pub const ACTION: &str = "Unsubscribe";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// The `List-Id`, else the sender's domain. Not a URL.
    pub list_identifier: String,
    pub summary: String,
}

/// `List-Id` (RFC 2919), else the sender's domain, so the banner works on bulk mail that does not
/// announce itself.
pub fn list_identifier(list_id: Option<&str>, from: &[EmailAddress]) -> Option<String> {
    list_id
        .map(str::to_owned)
        .or_else(|| from.first()?.domain().map(str::to_owned))
}

pub fn summary(list: &str) -> String {
    format!("This message is from {list}")
}

/// `send_state` gates it: an outgoing message's identifier is the user's own domain (#1525).
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

/// The band appears only when the message has a `List-Id` or `List-Unsubscribe` (`None` while
/// unknown). `offer` stays the rule for `U`.
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

/// A plain ISO date, which can be matched against mail, unlike "3 weeks ago".
pub fn activated_on(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d").to_string()
}

pub fn activation_label(list: &str, at: DateTime<Utc>) -> String {
    format!("Left {list} on {}", activated_on(at))
}

/// Newest first, ties by row id descending to match the per-account log: the log is read per
/// account, so a stable sort by `activated_at` alone would order ties by account.
pub fn newest_first(activations: &mut [UnsubscribeActivation]) {
    activations.sort_by_key(|activation| {
        std::cmp::Reverse((activation.activated_at, activation.id.get()))
    });
}

/// Narrower than `list_identifier`: its domain fallback is wrong for moving mail (`gmail.com` is
/// not a list), so the fallback here is the exact sender address (#1931).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SweepScope {
    ListId(String),
    Sender(String),
}

impl SweepScope {
    pub fn name(&self) -> &str {
        match self {
            Self::ListId(list) | Self::Sender(list) => list,
        }
    }
}

pub fn sweep_scope(list_id: Option<&str>, from: &[EmailAddress]) -> Option<SweepScope> {
    match list_id.map(str::trim).filter(|list| !list.is_empty()) {
        Some(list) => Some(SweepScope::ListId(list.to_owned())),
        None => from
            .first()
            .map(|sender| SweepScope::Sender(sender.normalized())),
    }
}

pub fn sweep_verb(action: AfterUnsubscribe) -> &'static str {
    match action {
        AfterUnsubscribe::Archive => "Archive",
        AfterUnsubscribe::Delete => "Delete",
    }
}

/// Begins with the confirmation, naming the same list or address the sweep acts on. `more` means
/// the storage page was full, so the count is a floor.
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

pub fn sweep_button(action: AfterUnsubscribe, count: usize, more: bool) -> String {
    format!(
        "{} {count}{}",
        sweep_verb(action),
        if more { "+" } else { "" }
    )
}

/// Ignoring it, or `Esc`, keeps everything; the button is one undoable bulk command (#1952).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepNotice {
    pub text: String,
    pub button: String,
    pub action: AfterUnsubscribe,
}

impl SweepNotice {
    pub fn command(&self, ids: Vec<postio_model::MessageId>) -> postio_core::Command {
        let target = postio_core::MessageTarget::Messages(ids);
        match self.action {
            AfterUnsubscribe::Archive => postio_core::Command::Archive { target },
            AfterUnsubscribe::Delete => postio_core::Command::Delete { target },
        }
    }
}

/// The key is never silent (#1953).
pub const NO_OFFER: &str = "No unsubscribe offer to take up";

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

/// The sweep's own scope: the exact sender address when no list is named, never the activation
/// log's domain.
pub fn confirmation_name(list_id: Option<&str>, from: &[EmailAddress]) -> Option<String> {
    sweep_scope(list_id, from).map(|scope| scope.name().to_owned())
}

pub const SETTING_HEADING: &str = "After you unsubscribe";

/// The one place a person learns the notice never acts by itself.
pub const SETTING_NOTE: &str = "The notice after you unsubscribe offers this for the mail the list \
     already sent. Nothing moves unless you press it, and Undo takes it back.";

pub const SETTING_CHOICES: [AfterUnsubscribe; 2] =
    [AfterUnsubscribe::Archive, AfterUnsubscribe::Delete];

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
        assert_eq!(
            confirmation_name(None, &sender("Shop@Mail.Example.org")).as_deref(),
            Some("shop@mail.example.org")
        );
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
        // Ties break by `id DESC`, as each account's own read does, so one account and two read the
        // same order.
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
        // Almost no commercial mail sets `List-Id`; the domain fallback is what makes the banner
        // appear.
        assert_eq!(
            list_identifier(None, &from("weekly@news.example.org")),
            Some("news.example.org".to_owned())
        );
    }

    #[test]
    fn a_message_with_neither_offers_nothing_to_leave() {
        assert_eq!(list_identifier(None, &[]), None);
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
