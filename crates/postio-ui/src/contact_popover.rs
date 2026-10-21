//! The known-sender popover, without a toolkit (specs/011-focus-contacts S6,
//! handoff screen 04).
//!
//! Everything the popover says or shapes is decided here: the avatar's
//! initials, colour and glyph, the three tiles and their captions, the open
//! lines, the arrival control and the three actions. A toolkit only draws
//! what [`view`] returns.

use chrono::{DateTime, Datelike, Local, Utc};
use postio_core::{CommandId, Keymap};
use postio_model::{
    Contact, ContactArrival, ContactId, ContactKind, ContactStats, OpenItem, OpenSide,
};

/// The avatar colours, muted and mid-dark so white initials read on each.
/// A person's is chosen by their id, so it is the same in every list, every
/// popover and both apps.
pub const PALETTE: [u32; 8] = [
    0x5b6b7a, 0x9a6a2f, 0xb0624a, 0x4f7a5a, 0x8a5a9a, 0x3d6f9a, 0x7a64b0, 0xa07a3c,
];

/// What a person's avatar draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Avatar {
    /// Up to two letters, or the kind's glyph for a sender that is no one.
    pub text: String,
    /// The fill, `0xRRGGBB`.
    pub rgb: u32,
    /// Whether the shape is a circle; a list, a newsletter and a robot are
    /// rounded squares.
    pub round: bool,
}

/// The glyph a kind stands for in place of initials, `None` for those drawn
/// with letters.
pub fn kind_glyph(kind: ContactKind) -> Option<&'static str> {
    match kind {
        ContactKind::List => Some("≡"),
        ContactKind::Newsletter => Some("✉"),
        ContactKind::Automated => Some("⚙"),
        ContactKind::Person | ContactKind::Company => None,
    }
}

/// The initials of `name`: the first letters of its first and last words,
/// uppercased; one letter for one word; `?` for nothing to go on.
pub fn initials(name: &str) -> String {
    // An address stands in for a name: only its local part is a word.
    let name = name.split('@').next().unwrap_or(name);
    let words: Vec<&str> = name
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | '_' | '-'))
        .filter(|word| word.chars().next().is_some_and(char::is_alphanumeric))
        .collect();
    let letter = |word: &str| word.chars().next().into_iter().flat_map(char::to_uppercase);
    match words.as_slice() {
        [] => "?".to_owned(),
        [only] => letter(only).collect(),
        [first, .., last] => letter(first).chain(letter(last)).collect(),
    }
}

/// A person's avatar.
pub fn avatar(id: ContactId, name: &str, kind: ContactKind) -> Avatar {
    let glyph = kind_glyph(kind);
    Avatar {
        text: glyph.map_or_else(|| initials(name), str::to_owned),
        rgb: PALETTE[id.get().unsigned_abs() as usize % PALETTE.len()],
        round: glyph.is_none(),
    }
}

/// One of the three figures under the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    /// "431", "today", "—".
    pub value: String,
    /// "messages".
    pub caption: String,
}

/// One thing open between the user and the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenLine {
    /// "Waiting on you", "Waiting on Ada".
    pub label: String,
    /// What it is.
    pub text: String,
    /// Whether the move is the user's, which the label is drawn in the
    /// accent for.
    pub yours: bool,
    /// The message it hangs on, to open it.
    pub message: postio_model::MessageId,
}

/// One segment of the arrival control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrivalOption {
    /// "Inbox", "Digest", "Filtered out".
    pub label: String,
    /// Where it sends mail.
    pub arrival: ContactArrival,
    /// Whether it is the one in force.
    pub on: bool,
}

/// One of the buttons along the foot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// "All mail", "Write", "Open card".
    pub label: String,
    /// The command it runs.
    pub command: CommandId,
    /// Its key as the keymap binds it, `None` when unbound.
    pub key: Option<String>,
    /// Whether it is the default (↩) action, drawn filled.
    pub primary: bool,
    /// Whether it can be used now.
    pub enabled: bool,
}

/// Everything the popover draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopoverView {
    /// The address the popover is about: what its commands name.
    pub address: String,
    /// The avatar.
    pub avatar: Avatar,
    /// The name, as the person asked for it styled.
    pub name: String,
    /// The address under it.
    pub subline: String,
    /// "Finance lead, Northwind Partners".
    pub role: String,
    /// Messages, from you, last.
    pub tiles: [Tile; 3],
    /// Up to [`MAX_OPEN`] lines.
    pub open: Vec<OpenLine>,
    /// "Mail from Ada arrives in".
    pub arrival_title: String,
    /// Inbox, Digest, Filtered out.
    pub arrival: Vec<ArrivalOption>,
    /// All mail, Write, Open card.
    pub actions: Vec<Action>,
    /// For a sender not yet saved, what the popover says in place of the
    /// tiles and open lines (specs/011-focus-contacts S9, handoff screen 05).
    pub intro: Option<IntroView>,
    /// The arrival control's own keys, as the keymap binds them: `←→
    /// choose`, `0 default rules`. Drawn beside its title, so `0` is not a
    /// hidden key.
    pub hints: Vec<crate::hints::Hint>,
}

/// What the new-sender popover says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntroView {
    /// "Sent 1 message \u{b7} today 13:47".
    pub sent: String,
    /// "Not saved yet. First email today; Ben Adeyemi is on copy."
    pub status: String,
    /// "From their signature"; `None` when the signature said nothing.
    pub signature_title: Option<String>,
    /// "Partnerships \u{b7} Harbor Data Co.", "+1 555 0142 \u{b7} harbordata.example".
    pub signature_lines: Vec<String>,
    /// "Read on this Mac. Added only if you save."
    pub signature_foot: Option<String>,
}

/// The most open lines the popover shows.
pub const MAX_OPEN: usize = 2;

/// What [`view`] is made from.
#[derive(Debug)]
pub struct Facts<'a> {
    /// The person.
    pub person: &'a Contact,
    /// The address the popover was asked for.
    pub address: &'a str,
    /// Their counts.
    pub stats: &'a ContactStats,
    /// What is open between the two, newest first.
    pub open: &'a [OpenItem],
    /// Where their mail arrives, `None` for by the default rules.
    pub arrival: Option<ContactArrival>,
    /// Where the default rules put it, which is what `None` shows.
    pub default_arrival: ContactArrival,
    /// Whether the person's card can be opened (the Contacts mode exists).
    pub card: bool,
    /// For a sender not yet saved: how they reached the user and what their
    /// signature said.
    pub introduction: Option<&'a postio_model::Introduction>,
}

/// The first name: the given name, else the first word of the shown name.
pub fn first_name(person: &Contact) -> String {
    let given = person.given_name.as_deref().map(str::trim).unwrap_or("");
    if !given.is_empty() {
        return given.to_owned();
    }
    person
        .display_name()
        .split_whitespace()
        .next()
        .unwrap_or("them")
        .to_owned()
}

/// The line under the address: title and company, or what the person is.
pub fn role_line(person: &Contact) -> String {
    let filled = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let parts: Vec<String> = [filled(&person.title), filled(&person.organization)]
        .into_iter()
        .flatten()
        .collect();
    if !parts.is_empty() {
        return parts.join(", ");
    }
    match person.kind {
        ContactKind::Company => "Company".to_owned(),
        ContactKind::List => "Mailing list".to_owned(),
        ContactKind::Newsletter => "Newsletter".to_owned(),
        ContactKind::Automated => "Automated sender".to_owned(),
        ContactKind::Person if person.is_saved() => "In your contacts".to_owned(),
        ContactKind::Person => "Seen in your mail".to_owned(),
    }
}

/// When the last message was, in a tile: `today`, `yesterday`, a weekday
/// within the week, then a date, with the year past this one.
pub fn last_word(last: Option<DateTime<Utc>>, now: DateTime<Local>) -> String {
    let Some(last) = last else {
        return "—".to_owned();
    };
    let local = last.with_timezone(&now.timezone());
    let days = (now.date_naive() - local.date_naive()).num_days();
    match days {
        ..=0 => "today".to_owned(),
        1 => "yesterday".to_owned(),
        2..=6 => local.format("%A").to_string().to_lowercase(),
        _ if local.year() == now.year() => local.format("%-d %b").to_string(),
        _ => local.format("%b %Y").to_string(),
    }
}

fn count_tile(count: u32, one: &str, many: &str) -> Tile {
    Tile {
        value: count.to_string(),
        caption: if count == 1 { one } else { many }.to_owned(),
    }
}

/// How long they take to reply, as a tile says it: `~45 min`, `~3 h`,
/// `~2 d`; `—` when the ring has too few answers to say.
pub fn reply_word(median: Option<chrono::Duration>) -> String {
    let Some(median) = median else {
        return "—".to_owned();
    };
    let minutes = median.num_minutes().max(1);
    if minutes < 60 {
        format!("~{minutes} min")
    } else if minutes < 48 * 60 {
        format!("~{} h", (minutes + 30) / 60)
    } else {
        format!("~{} d", (minutes + 12 * 60) / (24 * 60))
    }
}

/// The three tiles: messages, how fast they reply, and the last message.
pub fn tiles(stats: &ContactStats, now: DateTime<Local>) -> [Tile; 3] {
    [
        count_tile(stats.messages, "message", "messages"),
        Tile {
            value: reply_word(stats.reply_median()),
            caption: "replies in".to_owned(),
        },
        Tile {
            value: last_word(stats.last, now),
            caption: "last".to_owned(),
        },
    ]
}

/// The popover for `facts`.
pub fn view(facts: &Facts<'_>, keymap: &Keymap, now: DateTime<Local>) -> PopoverView {
    let person = facts.person;
    let first = first_name(person);
    let name = person.shown_as().into_owned();
    let open = facts
        .open
        .iter()
        .take(MAX_OPEN)
        .map(|item| OpenLine {
            label: match item.side {
                OpenSide::WaitingOnYou => "Waiting on you".to_owned(),
                OpenSide::WaitingOnThem => format!("Waiting on {first}"),
            },
            text: item.text.clone(),
            yours: item.side == OpenSide::WaitingOnYou,
            message: item.message,
        })
        .collect();
    let in_force = facts.arrival.unwrap_or(facts.default_arrival);
    let arrival = [
        ("Inbox", ContactArrival::Inbox),
        ("Digest", ContactArrival::Digest),
        ("Filtered out", ContactArrival::Filtered),
    ]
    .into_iter()
    .map(|(label, arrival)| ArrivalOption {
        label: label.to_owned(),
        arrival,
        on: arrival == in_force,
    })
    .collect();
    let action = |label: &str, command: CommandId, primary: bool, enabled: bool| Action {
        label: label.to_owned(),
        command,
        key: keymap.binding(command).map(str::to_owned),
        primary,
        enabled,
    };
    let intro = facts
        .introduction
        .map(|introduction| intro_view(introduction, facts.stats, now));
    let actions = if intro.is_some() {
        vec![
            action("Add contact", CommandId::ContactAdd, true, true),
            action("Not now", CommandId::Back, false, true),
            action("Ignore sender", CommandId::ContactIgnore, false, true),
        ]
    } else {
        vec![
            action("All mail", CommandId::ContactShowMail, true, true),
            action("Write", CommandId::ContactCompose, false, true),
            action("Open card", CommandId::ContactOpenCard, false, facts.card),
        ]
    };
    PopoverView {
        intro,
        address: facts.address.to_owned(),
        avatar: avatar(person.id, &name, person.kind),
        name,
        subline: person
            .addresses
            .iter()
            .find(|owned| {
                owned
                    .address
                    .address
                    .eq_ignore_ascii_case(facts.address.trim())
            })
            .map_or_else(
                || facts.address.to_owned(),
                |owned| owned.address.address.clone(),
            ),
        role: role_line(person),
        tiles: tiles(facts.stats, now),
        open,
        hints: [
            crate::hints::pair(
                keymap,
                CommandId::ContactArrivalPrev,
                CommandId::ContactArrivalNext,
                "choose",
            ),
            crate::hints::hint(keymap, CommandId::ContactSetArrival, "default rules"),
        ]
        .into_iter()
        .flatten()
        .collect(),
        arrival_title: format!("Mail from {first} arrives in"),
        arrival,
        actions,
    }
}

/// How a sender not yet saved is introduced.
pub fn intro_view(
    introduction: &postio_model::Introduction,
    stats: &ContactStats,
    now: DateTime<Local>,
) -> IntroView {
    let when = introduction.last_at.map(|at| {
        let local = at.with_timezone(&now.timezone());
        format!("{} {}", last_word(Some(at), now), local.format("%H:%M"))
    });
    let sent = match (stats.messages, when) {
        (1, Some(when)) => format!("Sent 1 message \u{b7} {when}"),
        (n, Some(when)) => format!("Sent {n} messages \u{b7} last {when}"),
        (1, None) => "Sent 1 message".to_owned(),
        (n, None) => format!("Sent {n} messages"),
    };
    let first = if stats.messages <= 1 {
        let day = introduction
            .last_at
            .map_or_else(|| "today".to_owned(), |at| last_word(Some(at), now));
        format!("First email {day}")
    } else {
        format!("{} emails so far", stats.messages)
    };
    let with = match introduction.with.as_slice() {
        [] => String::new(),
        [one] => format!("; {one} is on copy"),
        [one, two] => format!("; {one} and {two} are on copy"),
        [one, two, more @ ..] => format!("; {one}, {two} and {} more are on copy", more.len()),
    };
    let details = &introduction.details;
    let work: Vec<&str> = [details.title.as_deref(), details.company.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    let reach: Vec<&str> = details
        .phones
        .first()
        .map(String::as_str)
        .into_iter()
        .chain(details.websites.first().map(String::as_str))
        .collect();
    let signature_lines: Vec<String> = [work.join(" \u{b7} "), reach.join(" \u{b7} ")]
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect();
    let said = !signature_lines.is_empty();
    IntroView {
        sent,
        status: format!("Not saved yet. {first}{with}."),
        signature_title: said.then(|| "From their signature".to_owned()),
        signature_lines,
        signature_foot: said.then(|| "Read on this Mac. Added only if you save.".to_owned()),
    }
}

/// The arrival one segment to the left or right of `from`, stopping at the
/// ends rather than wrapping: a segmented control is a line, not a ring.
pub fn step_arrival(from: Option<ContactArrival>, by: i32) -> ContactArrival {
    const ORDER: [ContactArrival; 3] = [
        ContactArrival::Inbox,
        ContactArrival::Digest,
        ContactArrival::Filtered,
    ];
    let at = ORDER
        .iter()
        .position(|arrival| Some(*arrival) == from)
        .unwrap_or(0) as i32;
    ORDER[(at + by).clamp(0, 2) as usize]
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use postio_model::{AddressId, ContactAddress, EmailAddress, MessageId};

    use super::*;

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 26, 16, 5, 0).unwrap()
    }

    fn ada() -> Contact {
        let address = ContactAddress::new(
            AddressId::new(1),
            EmailAddress::new(Some("Ada Moreno"), "ada@example.org"),
        );
        let mut person = Contact::new(address);
        person.id = ContactId::new(7);
        person.title = Some("Finance lead".into());
        person.organization = Some("Northwind Partners".into());
        person
    }

    fn stats() -> ContactStats {
        ContactStats {
            messages: 431,
            from_you: 88,
            opened: 400,
            last: Some(now().with_timezone(&Utc)),
            months: Vec::new(),
            recent: Vec::new(),
        }
    }

    fn keymap() -> Keymap {
        Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        )
    }

    #[test]
    fn initials_are_the_first_and_last_words_first_letters() {
        assert_eq!(initials("Ada Moreno"), "AM");
        assert_eq!(initials("ada maria moreno"), "AM");
        assert_eq!(initials("Cher"), "C");
        assert_eq!(initials("ada.moreno@example.org"), "AM");
        assert_eq!(initials("  "), "?");
        assert_eq!(initials("Émile Zola"), "ÉZ");
    }

    #[test]
    fn a_person_is_a_circle_and_a_robot_is_a_glyph_in_a_rounded_square() {
        let person = avatar(ContactId::new(7), "Ada Moreno", ContactKind::Person);
        assert_eq!((person.text.as_str(), person.round), ("AM", true));
        for (kind, glyph) in [
            (ContactKind::List, "≡"),
            (ContactKind::Newsletter, "✉"),
            (ContactKind::Automated, "⚙"),
        ] {
            let sender = avatar(ContactId::new(7), "Field Notes", kind);
            assert_eq!((sender.text.as_str(), sender.round), (glyph, false));
        }
        assert_eq!(
            avatar(ContactId::new(7), "Acme", ContactKind::Company).text,
            "A"
        );
    }

    #[test]
    fn a_colour_belongs_to_the_person_and_never_changes() {
        let one = avatar(ContactId::new(7), "Ada Moreno", ContactKind::Person);
        let again = avatar(ContactId::new(7), "Ada Moreno", ContactKind::Person);
        assert_eq!(one.rgb, again.rgb);
        assert!(PALETTE.contains(&one.rgb));
        let colours: std::collections::HashSet<u32> = (1..=8)
            .map(|id| avatar(ContactId::new(id), "x", ContactKind::Person).rgb)
            .collect();
        assert_eq!(colours.len(), PALETTE.len(), "eight people, eight colours");
    }

    #[test]
    fn the_popover_says_who_what_and_how_much() {
        let open = [
            OpenItem {
                side: OpenSide::WaitingOnYou,
                text: "Approve the Q3 numbers by Friday".into(),
                message: MessageId::new(1),
            },
            OpenItem {
                side: OpenSide::WaitingOnThem,
                text: "Q4 headcount numbers".into(),
                message: MessageId::new(2),
            },
            OpenItem {
                side: OpenSide::WaitingOnYou,
                text: "a third never shows".into(),
                message: MessageId::new(3),
            },
        ];
        let person = ada();
        let stats = stats();
        let popover = view(
            &Facts {
                person: &person,
                address: "ada@example.org",
                stats: &stats,
                open: &open,
                arrival: None,
                default_arrival: ContactArrival::Inbox,
                introduction: None,
                card: false,
            },
            &keymap(),
            now(),
        );
        assert_eq!(popover.name, "Ada Moreno");
        assert_eq!(popover.subline, "ada@example.org");
        let shouted = view(
            &Facts {
                person: &person,
                address: "ADA@Example.ORG",
                stats: &stats,
                open: &[],
                arrival: None,
                default_arrival: ContactArrival::Inbox,
                introduction: None,
                card: false,
            },
            &keymap(),
            now(),
        );
        assert_eq!(shouted.subline, "ada@example.org", "as the store spells it");
        assert_eq!(popover.role, "Finance lead, Northwind Partners");
        let tiles: Vec<_> = popover
            .tiles
            .iter()
            .map(|tile| (tile.value.as_str(), tile.caption.as_str()))
            .collect();
        assert_eq!(
            tiles,
            [("431", "messages"), ("—", "replies in"), ("today", "last")]
        );
        let lines: Vec<_> = popover
            .open
            .iter()
            .map(|line| (line.label.as_str(), line.text.as_str(), line.yours))
            .collect();
        assert_eq!(
            lines,
            [
                ("Waiting on you", "Approve the Q3 numbers by Friday", true),
                ("Waiting on Ada", "Q4 headcount numbers", false),
            ]
        );
        assert_eq!(popover.arrival_title, "Mail from Ada arrives in");
    }

    #[test]
    fn a_sender_not_yet_saved_is_introduced_by_how_they_reached_you_and_their_signature() {
        let mut person = ada();
        person.saved_at = None;
        let stats = ContactStats {
            messages: 1,
            ..stats()
        };
        let introduction = postio_model::Introduction {
            details: postio_model::SignatureDetails {
                name: None,
                title: Some("Partnerships".into()),
                company: Some("Harbor Data Co.".into()),
                phones: vec!["+1 555 0142".into()],
                websites: vec!["harbordata.example".into()],
            },
            last_at: Some(now().with_timezone(&Utc)),
            with: vec!["Ben Adeyemi".into()],
        };
        let popover = view(
            &Facts {
                person: &person,
                address: "ada@example.org",
                stats: &stats,
                open: &[],
                arrival: None,
                default_arrival: ContactArrival::Inbox,
                card: true,
                introduction: Some(&introduction),
            },
            &keymap(),
            now(),
        );
        let intro = popover.intro.expect("an introduction");
        assert_eq!(intro.sent, "Sent 1 message \u{b7} today 16:05");
        assert_eq!(
            intro.status,
            "Not saved yet. First email today; Ben Adeyemi is on copy."
        );
        assert_eq!(
            intro.signature_title.as_deref(),
            Some("From their signature")
        );
        assert_eq!(
            intro.signature_lines,
            [
                "Partnerships \u{b7} Harbor Data Co.",
                "+1 555 0142 \u{b7} harbordata.example"
            ]
        );
        assert_eq!(
            intro.signature_foot.as_deref(),
            Some("Read on this Mac. Added only if you save.")
        );
        let actions: Vec<(&str, Option<&str>)> = popover
            .actions
            .iter()
            .map(|a| (a.label.as_str(), a.key.as_deref()))
            .collect();
        assert_eq!(
            actions,
            [
                ("Add contact", Some("cmd+Return")),
                ("Not now", Some("Escape")),
                ("Ignore sender", Some("cmd+BackSpace"))
            ]
        );
        assert!(popover.actions[0].primary);
    }

    #[test]
    fn a_signature_that_said_nothing_leaves_the_card_out() {
        let introduction = postio_model::Introduction {
            details: postio_model::SignatureDetails::default(),
            last_at: None,
            with: Vec::new(),
        };
        let stats = ContactStats {
            messages: 3,
            ..stats()
        };
        let intro = intro_view(&introduction, &stats, now());
        assert_eq!(intro.signature_title, None);
        assert!(intro.signature_lines.is_empty() && intro.signature_foot.is_none());
        assert_eq!(intro.status, "Not saved yet. 3 emails so far.");
    }

    #[test]
    fn the_arrival_control_shows_the_one_in_force_and_inbox_for_none() {
        let person = ada();
        let stats = stats();
        let on = |arrival| {
            let facts = Facts {
                person: &person,
                address: "ada@example.org",
                stats: &stats,
                open: &[],
                arrival,
                default_arrival: ContactArrival::Inbox,
                introduction: None,
                card: false,
            };
            view(&facts, &keymap(), now())
                .arrival
                .iter()
                .filter(|option| option.on)
                .map(|option| option.label.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(on(None), ["Inbox"]);
        assert_eq!(on(Some(ContactArrival::Digest)), ["Digest"]);
        assert_eq!(on(Some(ContactArrival::Filtered)), ["Filtered out"]);
    }

    #[test]
    fn with_no_rule_it_shows_where_the_default_rules_put_their_mail() {
        let person = ada();
        let stats = stats();
        let on = |arrival, default_arrival| {
            view(
                &Facts {
                    person: &person,
                    address: "ada@example.org",
                    stats: &stats,
                    open: &[],
                    arrival,
                    default_arrival,
                    introduction: None,
                    card: false,
                },
                &keymap(),
                now(),
            )
            .arrival
            .iter()
            .filter(|option| option.on)
            .map(|option| option.label.clone())
            .collect::<Vec<_>>()
        };
        assert_eq!(on(None, ContactArrival::Filtered), ["Filtered out"]);
        assert_eq!(on(None, ContactArrival::Digest), ["Digest"]);
        assert_eq!(
            on(Some(ContactArrival::Inbox), ContactArrival::Filtered),
            ["Inbox"],
            "a rule of theirs outranks the default"
        );
    }

    #[test]
    fn the_arrival_controls_own_keys_are_drawn_with_it() {
        let person = ada();
        let stats = stats();
        let popover = view(
            &Facts {
                person: &person,
                address: "ada@example.org",
                stats: &stats,
                open: &[],
                arrival: None,
                default_arrival: ContactArrival::Inbox,
                introduction: None,
                card: false,
            },
            &keymap(),
            now(),
        );
        let hints: Vec<_> = popover
            .hints
            .iter()
            .map(|hint| (hint.key.as_str(), hint.label.as_str()))
            .collect();
        assert_eq!(hints, [("Left/Right", "choose"), ("0", "default rules")]);
    }

    #[test]
    fn the_actions_are_all_mail_write_and_a_card_that_waits_for_its_mode() {
        let person = ada();
        let stats = stats();
        let popover = view(
            &Facts {
                person: &person,
                address: "ada@example.org",
                stats: &stats,
                open: &[],
                arrival: None,
                default_arrival: ContactArrival::Inbox,
                introduction: None,
                card: false,
            },
            &keymap(),
            now(),
        );
        let actions: Vec<_> = popover
            .actions
            .iter()
            .map(|action| (action.label.as_str(), action.primary, action.enabled))
            .collect();
        assert_eq!(
            actions,
            [
                ("All mail", true, true),
                ("Write", false, true),
                ("Open card", false, false)
            ]
        );
        assert_eq!(popover.actions[1].key.as_deref(), Some("c"));
        assert_eq!(popover.actions[2].key.as_deref(), Some("cmd+i"));
    }

    #[test]
    fn a_stranger_in_the_mail_is_not_called_a_contact() {
        let mut person = ada();
        person.title = None;
        person.organization = None;
        assert_eq!(role_line(&person), "Seen in your mail");
        person.saved_at = Some(Utc::now());
        assert_eq!(role_line(&person), "In your contacts");
        person.kind = ContactKind::Newsletter;
        assert_eq!(role_line(&person), "Newsletter");
    }

    #[test]
    fn the_last_tile_names_the_day_the_way_a_person_would() {
        let at = |days: i64| Some(now().with_timezone(&Utc) - chrono::Duration::days(days));
        assert_eq!(last_word(None, now()), "—");
        assert_eq!(last_word(at(0), now()), "today");
        assert_eq!(last_word(at(1), now()), "yesterday");
        assert_eq!(last_word(at(3), now()), "wednesday");
        assert_eq!(last_word(at(14), now()), "12 Sep");
        assert_eq!(last_word(at(400), now()), "Aug 2025");
    }

    #[test]
    fn one_message_is_not_messages() {
        let mut one = stats();
        one.messages = 1;
        let tiles = tiles(&one, now());
        assert_eq!(tiles[0].caption, "message");
    }

    #[test]
    fn a_reply_time_is_said_in_the_unit_a_person_would() {
        let said = |minutes| reply_word(Some(chrono::Duration::minutes(minutes)));
        assert_eq!(said(0), "~1 min");
        assert_eq!(said(45), "~45 min");
        assert_eq!(said(180), "~3 h");
        assert_eq!(said(60 * 47), "~47 h");
        assert_eq!(said(60 * 24 * 3), "~3 d");
        assert_eq!(reply_word(None), "—");
    }

    #[test]
    fn the_arrival_steps_along_the_line_and_stops_at_its_ends() {
        use ContactArrival::{Digest, Filtered, Inbox};
        assert_eq!(step_arrival(None, 1), Digest);
        assert_eq!(step_arrival(Some(Inbox), -1), Inbox);
        assert_eq!(step_arrival(Some(Digest), 1), Filtered);
        assert_eq!(step_arrival(Some(Filtered), 1), Filtered);
        assert_eq!(step_arrival(Some(Filtered), -1), Digest);
    }
}
