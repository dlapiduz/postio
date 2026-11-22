//! The reading pane's header as rendered strings, so every frontend shows the same sender (ADR
//! 0019).
//! Layout stays with each toolkit.

use chrono::{DateTime, Local, Utc};
use postio_core::{CommandId, Keymap};
use postio_model::DraftState;
use postio_model::address::EmailAddress;

pub const NO_SUBJECT: &str = "(no subject)";

/// Strings, not addresses: two frontends given the same address drew different senders (#1150).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageHeader {
    pub subject: String,
    pub from: String,
    /// The full list; `to_line` shortens what is drawn.
    pub to: Option<String>,
    pub to_short: Option<String>,
    pub cc: Option<String>,
    pub cc_count: usize,
    pub date: String,
}

impl MessageHeader {
    pub fn of(
        from: &[EmailAddress],
        to: &[EmailAddress],
        cc: &[EmailAddress],
        subject: Option<&str>,
        date: DateTime<Utc>,
        now: DateTime<Local>,
    ) -> Self {
        Self {
            subject: subject_text(subject),
            from: address_list(from),
            to: (!to.is_empty()).then(|| address_list(to)),
            to_short: (!to.is_empty()).then(|| recipient_line(to)),
            cc: (!cc.is_empty()).then(|| address_list(cc)),
            cc_count: cc.len(),
            date: absolute_date(date, now),
        }
    }

    /// Shortened and without a label: the view draws one shared label column, so an inline "To: "
    /// doubles it (#1437).
    pub fn to_line(&self) -> Option<String> {
        self.to_short.clone()
    }

    pub fn cc_toggle_label(&self) -> Option<String> {
        (self.cc_count > 0).then(|| format!("Cc ({})", self.cc_count))
    }
}

/// Matches `conversation::participants`' limit; the two lines sit one above the other.
pub const RECIPIENTS_SHOWN: usize = 3;

/// The first few, then a count: "and 197 others" shows a broadcast, an ellipsis reads as a bug.
/// A list that fits is returned untouched.
pub fn recipient_line(addresses: &[EmailAddress]) -> String {
    if addresses.len() <= RECIPIENTS_SHOWN {
        return address_list(addresses);
    }
    format!(
        "{} {}",
        address_list(&addresses[..RECIPIENTS_SHOWN]),
        others(addresses.len() - RECIPIENTS_SHOWN)
    )
}

pub fn others(hidden: usize) -> String {
    format!(
        "and {hidden} {}",
        if hidden == 1 { "other" } else { "others" }
    )
}

pub fn address_line(address: &EmailAddress) -> String {
    match address
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        Some(name) => format!("{name} <{}>", address.address),
        None => address.address.clone(),
    }
}

pub fn address_list(addresses: &[EmailAddress]) -> String {
    addresses
        .iter()
        .map(address_line)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Looked up by normalised address, never by name, which a forger controls.
pub fn with_user_names(
    addresses: &[EmailAddress],
    names: &std::collections::HashMap<String, String>,
) -> Vec<EmailAddress> {
    addresses
        .iter()
        .map(|address| match names.get(&address.address.to_lowercase()) {
            Some(name) => EmailAddress::new(Some(name.as_str()), address.address.as_str()),
            None => address.clone(),
        })
        .collect()
}

/// `Some` when the user's names changed what the header shows, so the sender as carried stays one
/// look away.
pub fn as_sent(shown: &[EmailAddress], raw: &[EmailAddress]) -> Option<String> {
    let raw_line = address_list(raw);
    (address_list(shown) != raw_line).then(|| format!("As sent: {raw_line}"))
}

pub fn subject_text(subject: Option<&str>) -> String {
    subject
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| NO_SUBJECT.to_string())
}

pub fn absolute_date(at: DateTime<Utc>, now: DateTime<Local>) -> String {
    let local = at.with_timezone(&now.timezone());
    local.format("%a, %-d %b %Y at %H:%M").to_string()
}

/// A verb the reader offers under the message; which verbs, in what order, is shared so frontends
/// agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderAction {
    Reply,
    ReplyAll,
    Forward,
    Archive,
    RetrySend,
    CancelSend,
}

impl ReaderAction {
    pub const ALL: [ReaderAction; 4] = [
        ReaderAction::Reply,
        ReaderAction::ReplyAll,
        ReaderAction::Forward,
        ReaderAction::Archive,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            ReaderAction::Reply => "Reply",
            ReaderAction::ReplyAll => "Reply all",
            ReaderAction::Forward => "Forward",
            ReaderAction::Archive => "Archive",
            ReaderAction::RetrySend => "Send again",
            ReaderAction::CancelSend => "Cancel send",
        }
    }

    /// Always a registry command, so undo and the draft resolver see it.
    pub const fn command(self) -> CommandId {
        match self {
            ReaderAction::Reply => CommandId::Reply,
            ReaderAction::ReplyAll => CommandId::ReplyAll,
            ReaderAction::Forward => CommandId::Forward,
            ReaderAction::Archive => CommandId::Archive,
            ReaderAction::RetrySend => CommandId::RetrySend,
            ReaderAction::CancelSend => CommandId::CancelSend,
        }
    }

    /// Archive acts on the whole thread; the other verbs act on the latest message.
    pub const fn scope(self) -> ActionScope {
        match self {
            ReaderAction::Reply | ReaderAction::ReplyAll | ReaderAction::Forward => {
                ActionScope::LatestMessage
            }
            ReaderAction::Archive => ActionScope::WholeConversation,
            ReaderAction::RetrySend | ReaderAction::CancelSend => ActionScope::LatestMessage,
        }
    }

    /// Tooltip and accessible name: says which message the verb acts on. One message gets the bare
    /// verb.
    pub fn describe(self, messages: usize) -> String {
        let title = self.title();
        if messages <= 1 {
            return title.to_owned();
        }
        match self.scope() {
            // "Forward to the latest message" would read as addressing a recipient.
            ActionScope::LatestMessage => match self {
                ReaderAction::Forward => format!("{title} the latest message"),
                _ => format!("{title} to the latest message"),
            },
            ActionScope::WholeConversation if messages == 2 => {
                format!("{title} both messages")
            }
            ActionScope::WholeConversation => format!("{title} all {messages} messages"),
        }
    }

    pub const fn primary(self) -> bool {
        matches!(self, ReaderAction::Reply | ReaderAction::RetrySend)
    }

    /// Never for outgoing mail: `list_identifier` falls back to the sender's domain, which is the
    /// user's own (#1525).
    pub fn unsubscribable(state: Option<DraftState>) -> bool {
        matches!(state, None | Some(DraftState::Sent))
    }
}

/// The bar is fixed: it does not retarget as the reader scrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionScope {
    LatestMessage,
    WholeConversation,
}

/// Keys come from the keymap, not the registry defaults; a verb whose key was taken shows no hint.
pub fn actions(keymap: &Keymap) -> Vec<(ReaderAction, Option<String>)> {
    ReaderAction::ALL
        .iter()
        .map(|action| {
            (
                *action,
                keymap.binding(action.command()).map(str::to_string),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_named_persons_addresses_take_the_users_name_and_nothing_else_does() {
        use super::{EmailAddress, with_user_names};
        let names = std::collections::HashMap::from([(
            "ada@work.example".to_owned(),
            "Ada Lovelace".to_owned(),
        )]);
        let shown = with_user_names(
            &[
                EmailAddress::new(Some("A. L."), "Ada@Work.example"),
                EmailAddress::new(Some("Ada Lovelace"), "not-ada@example.net"),
            ],
            &names,
        );
        assert_eq!(shown[0].name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(
            shown[0].address, "Ada@Work.example",
            "the address is untouched"
        );
        assert_eq!(
            shown[1].name.as_deref(),
            Some("Ada Lovelace"),
            "a stranger keeps his own header, whatever it claims"
        );
    }

    #[test]
    fn the_header_as_sent_is_offered_only_when_it_differs() {
        use super::{EmailAddress, as_sent};
        let raw = [EmailAddress::new(Some("A. L."), "ada@work.example")];
        let named = [EmailAddress::new(Some("Ada Lovelace"), "ada@work.example")];
        assert_eq!(
            as_sent(&named, &raw).as_deref(),
            Some("As sent: A. L. <ada@work.example>")
        );
        assert_eq!(as_sent(&raw, &raw), None, "nothing was substituted");
    }

    #[test]
    fn the_three_composing_verbs_act_on_the_latest_message() {
        for verb in [
            ReaderAction::Reply,
            ReaderAction::ReplyAll,
            ReaderAction::Forward,
        ] {
            assert_eq!(verb.scope(), ActionScope::LatestMessage, "{verb:?}");
        }
    }

    #[test]
    fn archive_acts_on_the_whole_conversation() {
        assert_eq!(
            ReaderAction::Archive.scope(),
            ActionScope::WholeConversation
        );
    }

    #[test]
    fn an_action_says_what_it_will_act_on_rather_than_naming_the_verb() {
        assert_eq!(
            ReaderAction::Reply.describe(6),
            "Reply to the latest message"
        );
        assert_eq!(ReaderAction::Archive.describe(6), "Archive all 6 messages");
    }

    #[test]
    fn forward_does_not_read_as_forwarding_to_someone() {
        assert_eq!(
            ReaderAction::Forward.describe(6),
            "Forward the latest message"
        );
        assert_eq!(
            ReaderAction::ReplyAll.describe(6),
            "Reply all to the latest message"
        );
    }

    #[test]
    fn a_two_message_thread_says_both_rather_than_all_two() {
        assert_eq!(ReaderAction::Archive.describe(2), "Archive both messages");
    }

    #[test]
    fn a_lone_message_is_not_described_as_a_conversation() {
        assert_eq!(ReaderAction::Reply.describe(1), "Reply");
        assert_eq!(ReaderAction::Archive.describe(1), "Archive");
    }

    fn many(count: usize) -> Vec<EmailAddress> {
        (0..count)
            .map(|n| EmailAddress::new(Some(&format!("Person {n}")), format!("p{n}@example.com")))
            .collect()
    }

    #[test]
    fn a_recipient_list_that_fits_is_not_touched() {
        let line = recipient_line(&many(RECIPIENTS_SHOWN));
        assert!(
            !line.contains("other"),
            "a list that fits must not be described as shortened: {line}"
        );
        assert!(line.contains("Person 0"), "{line}");
        assert!(
            line.contains(&format!("Person {}", RECIPIENTS_SHOWN - 1)),
            "{line}"
        );
    }

    #[test]
    fn a_long_recipient_list_says_how_many_it_hid() {
        let line = recipient_line(&many(200));
        assert!(
            line.contains(&format!("{} others", 200 - RECIPIENTS_SHOWN)),
            "the hidden count is the whole point: {line}"
        );
    }

    #[test]
    fn one_hidden_recipient_is_one_other_not_one_others() {
        let line = recipient_line(&many(RECIPIENTS_SHOWN + 1));
        assert!(line.contains("1 other"), "{line}");
        assert!(!line.contains("1 others"), "{line}");
    }

    #[test]
    fn the_full_recipient_list_stays_reachable() {
        let header = MessageHeader::of(
            &many(1),
            &many(200),
            &[],
            Some("Subject"),
            Utc.with_ymd_and_hms(2026, 8, 12, 14, 32, 0).unwrap(),
            Local::now(),
        );
        let full = header.to.as_deref().expect("a To line");
        assert!(full.contains("p199@example.com"), "the last recipient went");
        assert!(
            header
                .to_line()
                .expect("a rendered To line")
                .contains("others"),
            "what is drawn is the shortened form"
        );
    }

    use chrono::TimeZone;

    use super::*;

    fn addr(name: Option<&str>, address: &str) -> EmailAddress {
        EmailAddress::new(name, address)
    }

    #[test]
    fn a_named_address_shows_both_the_name_and_the_address() {
        let a = addr(Some("Ada Lovelace"), "ada@example.com");
        assert_eq!(address_line(&a), "Ada Lovelace <ada@example.com>");
    }

    #[test]
    fn an_unnamed_address_shows_just_the_address() {
        let a = addr(None, "ada@example.com");
        assert_eq!(address_line(&a), "ada@example.com");
    }

    #[test]
    fn a_blank_display_name_is_treated_as_absent() {
        let a = addr(Some("   "), "ada@example.com");
        assert_eq!(address_line(&a), "ada@example.com");
    }

    #[test]
    fn several_addresses_join_with_a_comma() {
        let list = [
            addr(Some("Ada"), "ada@example.com"),
            addr(None, "bob@example.com"),
        ];
        assert_eq!(
            address_list(&list),
            "Ada <ada@example.com>, bob@example.com"
        );
    }

    #[test]
    fn a_missing_subject_says_so_rather_than_showing_nothing() {
        assert_eq!(subject_text(None), NO_SUBJECT);
        assert_eq!(subject_text(Some("   ")), NO_SUBJECT);
    }

    #[test]
    fn a_real_subject_passes_through_verbatim() {
        assert_eq!(subject_text(Some("Dinner Friday?")), "Dinner Friday?");
    }

    #[test]
    fn the_date_line_is_always_absolute() {
        // Built in the local zone: fixing both ends in UTC passes in only one timezone.
        let local = |y, m, d, h, min| Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap();
        let now = local(2026, 8, 26, 9, 0);
        let at = local(2026, 8, 12, 14, 32).with_timezone(&Utc);

        assert_eq!(absolute_date(at, now), "Wed, 12 Aug 2026 at 14:32");
    }

    #[test]
    fn a_whole_envelope_renders_every_line_a_reader_asks_for() {
        let local = |y, m, d, h, min| Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap();
        let header = MessageHeader::of(
            &[addr(Some("Ada Lovelace"), "ada@example.com")],
            &[addr(None, "bob@example.com")],
            &[
                addr(Some("Grace"), "grace@example.com"),
                addr(None, "carol@example.com"),
            ],
            Some("Dinner Friday?"),
            local(2026, 8, 12, 14, 32).with_timezone(&Utc),
            local(2026, 8, 26, 9, 0),
        );

        assert_eq!(header.subject, "Dinner Friday?");
        assert_eq!(header.from, "Ada Lovelace <ada@example.com>");
        assert_eq!(header.to_line().as_deref(), Some("bob@example.com"));
        assert_eq!(header.cc_toggle_label().as_deref(), Some("Cc (2)"));
        assert_eq!(
            header.cc.as_deref(),
            Some("Grace <grace@example.com>, carol@example.com")
        );
        assert_eq!(header.date, "Wed, 12 Aug 2026 at 14:32");
    }

    #[test]
    fn a_message_with_no_recipients_offers_neither_line() {
        let header = MessageHeader::of(
            &[addr(None, "ada@example.com")],
            &[],
            &[],
            None,
            Utc::now(),
            Local::now(),
        );
        assert_eq!(header.to_line(), None);
        assert_eq!(header.cc_toggle_label(), None);
        assert_eq!(header.cc, None);
        assert_eq!(header.subject, NO_SUBJECT);
    }

    #[test]
    fn the_four_verbs_are_reply_reply_all_forward_and_archive_in_that_order() {
        let keymap = Keymap::resolve(&Default::default());
        assert_eq!(
            actions(&keymap),
            vec![
                (ReaderAction::Reply, Some("r".to_string())),
                (ReaderAction::ReplyAll, Some("a".to_string())),
                (ReaderAction::Forward, Some("f".to_string())),
                (ReaderAction::Archive, Some("e".to_string())),
            ]
        );
    }

    #[test]
    fn a_rebind_reaches_the_buttons_hint() {
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("reply".to_string(), "w".to_string());
        let keymap = Keymap::resolve(&overrides);
        let keys: Vec<_> = actions(&keymap).into_iter().map(|(_, key)| key).collect();
        assert_eq!(keys[0], Some("w".to_string()));
        assert_eq!(keys[1], Some("a".to_string()), "only Reply was rebound");
    }

    #[test]
    fn a_key_lost_to_another_command_is_never_shown_as_still_working() {
        // Archive loses `e` to undo; its alternate shows, never `e`.
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("undo".to_string(), "e".to_string());
        let keymap = Keymap::resolve(&overrides);
        let keys: Vec<_> = actions(&keymap).into_iter().map(|(_, key)| key).collect();

        assert_ne!(keys[3].as_deref(), Some("e"), "still teaching undo's key");
        assert_eq!(
            keys[3].as_deref(),
            keymap.binding(CommandId::Archive),
            "the hint is whatever the keymap says, or nothing"
        );
    }

    #[test]
    fn a_verb_with_no_binding_left_shows_no_hint_at_all() {
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("undo".to_string(), "e".to_string());
        overrides
            .overrides_mut()
            .insert("flag".to_string(), "mod+shift+a".to_string());
        let keymap = Keymap::resolve(&overrides);
        let keys: Vec<_> = actions(&keymap).into_iter().map(|(_, key)| key).collect();
        assert_eq!(keys[3], None);
    }

    #[test]
    fn exactly_one_verb_is_the_primary_and_it_is_reply() {
        let primary: Vec<_> = ReaderAction::ALL
            .iter()
            .filter(|action| action.primary())
            .collect();
        assert_eq!(primary, vec![&ReaderAction::Reply]);
    }
}

#[cfg(test)]
mod outgoing_tests {
    use super::*;

    #[test]
    fn a_message_on_its_way_is_never_offered_an_unsubscribe() {
        // #1525: an outgoing message's list id falls back to the user's own domain.
        for state in [
            DraftState::Queued,
            DraftState::Sending,
            DraftState::Failed,
            DraftState::Unconfirmed,
            DraftState::Editing,
        ] {
            assert!(
                !ReaderAction::unsubscribable(Some(state)),
                "{state:?} offers to unsubscribe from the user's own domain"
            );
        }
    }

    #[test]
    fn ordinary_mail_is_untouched() {
        assert!(ReaderAction::unsubscribable(None));
        assert!(ReaderAction::unsubscribable(Some(DraftState::Sent)));
    }
}
