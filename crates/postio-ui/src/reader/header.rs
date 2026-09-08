//! The reading pane's header, in the part that has no toolkit in it (#1259):
//! how a sender reads, how a recipient list joins, what a missing subject
//! says, and how an opened message is dated.
//!
//! These were four private functions inside `postio-gtk`'s
//! `reader/message_header.rs`, which is precisely the shape ADR 0019 warns
//! about: the header is the surface where two frontends disagreeing is most
//! visible to a user, because it is where they read the sender's name before
//! deciding whether to trust anything below it. macOS had no header at all,
//! and the fix was either to write these four again in Swift or to move them
//! here. This is the twelfth block to move.
//!
//! Nothing here decides *layout* — where the account line sits, whether `Cc`
//! is a disclosure or always open. That stays with each toolkit, which is
//! why [`MessageHeader`] carries rendered strings rather than widgets.

use chrono::{DateTime, Local, Utc};
use postio_core::{CommandId, Keymap};
use postio_model::address::EmailAddress;

/// Shown in place of a blank line — a missing subject is a fact about the
/// message, not something to render as if it were not there.
pub const NO_SUBJECT: &str = "(no subject)";

/// Every line of the header, already rendered.
///
/// Strings rather than addresses, for the reason `RowFfi`'s `from` is a
/// string: two frontends handed the same `EmailAddress` drew different
/// senders for the same message once already (#1150), and a boundary that
/// carries the answer cannot be read two ways.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageHeader {
    /// The subject, or [`NO_SUBJECT`] — never empty.
    pub subject: String,
    /// Who it is from, as `Name <address>` joined by commas.
    pub from: String,
    /// Every recipient, joined — the full list, which stays reachable even
    /// when [`Self::to_line`] shortens what it draws.
    pub to: Option<String>,
    /// The recipients as they are drawn: the first few, then how many are
    /// left. See [`recipient_line`].
    pub to_short: Option<String>,
    /// The `Cc` addresses, joined; `None` when there are none, which is what
    /// lets a toolkit spend no space at all on the common case.
    pub cc: Option<String>,
    /// How many addresses `cc` holds, for the disclosure's own label.
    pub cc_count: usize,
    /// When it arrived, absolute and in the reader's own timezone.
    pub date: String,
}

impl MessageHeader {
    /// Render an envelope into the lines a header draws.
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

    /// The `To:` line as a header writes it, label included and shortened.
    ///
    /// [`Self::to`] keeps the full list. That split is the whole of spec
    /// Story 1 scenario 3: what is *drawn* shortens and says how many it hid,
    /// and what is *kept* is everything, so a disclosure or a tooltip can
    /// still answer "who exactly".
    pub fn to_line(&self) -> Option<String> {
        self.to_short.as_ref().map(|to| format!("To: {to}"))
    }

    /// What the `Cc` disclosure is called while it is offered — `Cc (2)`.
    pub fn cc_toggle_label(&self) -> Option<String> {
        (self.cc_count > 0).then(|| format!("Cc ({})", self.cc_count))
    }
}

/// How many recipients a header names before it starts counting the rest.
///
/// Three, matching [`crate::conversation::participants`]'s own limit, because
/// the two lines sit one above the other in the same pane and a reader should
/// not have to learn two different shapes of "there are more of these".
pub const RECIPIENTS_SHOWN: usize = 3;

/// The recipients as a header draws them: the first few, then how many are
/// left.
///
/// **The count is the information.** "Ada, Bob and 197 others" says at a
/// glance that this is a broadcast and that reply-all would be a mistake; an
/// ellipsis says nothing and reads as a rendering bug. Reply-all to two
/// hundred people is a mistake made because the header did not say so.
///
/// Unlike [`crate::conversation::participants`], this keeps the *first* names
/// and counts the rest rather than keeping both ends. Recipient order carries
/// no meaning worth preserving — nobody is the "most recent" recipient — so
/// there is no far end worth saving, and a plain count is easier to read than
/// an elision.
///
/// A list that fits is returned untouched: no "and 0 others".
pub fn recipient_line(addresses: &[EmailAddress]) -> String {
    if addresses.len() <= RECIPIENTS_SHOWN {
        return address_list(addresses);
    }
    let hidden = addresses.len() - RECIPIENTS_SHOWN;
    format!(
        "{} and {hidden} {}",
        address_list(&addresses[..RECIPIENTS_SHOWN]),
        if hidden == 1 { "other" } else { "others" }
    )
}

/// `"Name <address>"` when a display name is present, the bare address
/// otherwise — never a name repeated as its own address.
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

/// Several addresses on one line, comma-joined.
pub fn address_list(addresses: &[EmailAddress]) -> String {
    addresses
        .iter()
        .map(address_line)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The subject as the header shows it, or [`NO_SUBJECT`].
pub fn subject_text(subject: Option<&str>) -> String {
    subject
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| NO_SUBJECT.to_string())
}

/// The header's date line: always absolute, unlike the list row's relative
/// [`crate::row::timestamp`] — a message once opened is not "3h ago" any
/// more, it is dated.
pub fn absolute_date(at: DateTime<Utc>, now: DateTime<Local>) -> String {
    let local = at.with_timezone(&now.timezone());
    local.format("%a, %-d %b %Y at %H:%M").to_string()
}

/// One verb the reading pane offers under the message.
///
/// The pointer's way to the same four verbs `e`, `E`, `f` and `a` already
/// reach from the keyboard (#498). Shared because *which four, in what
/// order* is a product decision, not a toolkit one: a reader offering three
/// verbs on one platform and five on the other is two applications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderAction {
    /// Reply to the sender — the primary.
    Reply,
    /// Reply to everyone on the message.
    ReplyAll,
    /// Forward it.
    Forward,
    /// Archive it.
    Archive,
}

impl ReaderAction {
    /// The four verbs, in canvas order.
    pub const ALL: [ReaderAction; 4] = [
        ReaderAction::Reply,
        ReaderAction::ReplyAll,
        ReaderAction::Forward,
        ReaderAction::Archive,
    ];

    /// What the button is labelled.
    ///
    /// `const`, because `postio-gtk`'s `ACTIONS` is a `const` array and a
    /// label it could not read at compile time would have to be written
    /// again there — which is the duplication this list exists to prevent.
    pub const fn title(self) -> &'static str {
        match self {
            ReaderAction::Reply => "Reply",
            ReaderAction::ReplyAll => "Reply all",
            ReaderAction::Forward => "Forward",
            ReaderAction::Archive => "Archive",
        }
    }

    /// Which registry command it runs.
    ///
    /// Never a local implementation, for `row::RowAction::command`'s reason:
    /// a button that did its own thing would be a second way to reply that
    /// the draft resolver and undo did not know about.
    pub const fn command(self) -> CommandId {
        match self {
            ReaderAction::Reply => CommandId::Reply,
            ReaderAction::ReplyAll => CommandId::ReplyAll,
            ReaderAction::Forward => CommandId::Forward,
            ReaderAction::Archive => CommandId::Archive,
        }
    }

    /// Whether it gets the primary treatment. Exactly one does.
    pub const fn primary(self) -> bool {
        matches!(self, ReaderAction::Reply)
    }
}

/// The four verbs with the key each currently carries, in canvas order.
///
/// From the keymap rather than the registry's defaults, the same rule
/// [`crate::row::hints`] follows: a button teaching the wrong key is worse
/// than one teaching none, so a verb whose key has been taken shows no hint
/// at all.
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
        // The count is the information. "Ada, Bob and 197 others" says this is
        // a broadcast; an ellipsis says nothing and reads as a rendering bug.
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
        // Shortening the line must not lose the addresses: reply-all to two
        // hundred people is a mistake made because the header did not say who
        // was on it.
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
        // Built in the local zone and handed over as UTC, the same way
        // `row.rs`'s own timestamp test does it: fixing both ends in UTC
        // would only pass in one timezone.
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
        assert_eq!(header.to_line().as_deref(), Some("To: bob@example.com"));
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
                (ReaderAction::Reply, Some("e".to_string())),
                (ReaderAction::ReplyAll, Some("E".to_string())),
                (ReaderAction::Forward, Some("f".to_string())),
                (ReaderAction::Archive, Some("a".to_string())),
            ]
        );
    }

    #[test]
    fn a_rebind_reaches_the_buttons_hint() {
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("reply".to_string(), "r".to_string());
        let keymap = Keymap::resolve(&overrides);
        let keys: Vec<_> = actions(&keymap).into_iter().map(|(_, key)| key).collect();
        assert_eq!(keys[0], Some("r".to_string()));
        assert_eq!(keys[1], Some("E".to_string()), "only Reply was rebound");
    }

    #[test]
    fn a_key_lost_to_another_command_is_never_shown_as_still_working() {
        // `undo`'s default is `u`; rebinding it to `a` takes Archive's own
        // default in every context they share, so Archive loses `a`. What it
        // shows instead is whatever binding it has left — `mod+shift+a`, its
        // alternate — and never `a`, which now runs undo. A button teaching
        // a key that does something else is the failure this guards;
        // teaching a longer key that still works is not.
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("undo".to_string(), "a".to_string());
        let keymap = Keymap::resolve(&overrides);
        let keys: Vec<_> = actions(&keymap).into_iter().map(|(_, key)| key).collect();

        assert_ne!(keys[3].as_deref(), Some("a"), "still teaching undo's key");
        assert_eq!(
            keys[3].as_deref(),
            keymap.binding(CommandId::Archive),
            "the hint is whatever the keymap says, or nothing"
        );
    }

    #[test]
    fn a_verb_with_no_binding_left_shows_no_hint_at_all() {
        // Both of Archive's bindings taken: `a` by undo, `mod+shift+a` by
        // flag. Nothing is left, and nothing is what the button says —
        // rather than a key that would archive nothing.
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("undo".to_string(), "a".to_string());
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
