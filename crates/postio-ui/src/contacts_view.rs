//! The Contacts view's words and shapes, without a toolkit
//! (specs/011-focus-contacts S7, handoff screens 01 to 03).
//!
//! The strip with its tabs, the review line, a row of the list, the card
//! (a person's, and a newsletter's), and the footer are decided here: what
//! each says, in what order, and what the Mac may and may not draw. A toolkit
//! draws what these return and adds no word of its own.

use chrono::{DateTime, Datelike, Local, Utc, Weekday};
use postio_core::{CommandId, Keymap};
use postio_model::{
    CARD_RECENT, ContactArrival, ContactCardRead, ContactKind, ContactListRow, ContactTab,
    ContactsOverview, RecentMessage, ReviewSummary,
};

use crate::contact_popover::{Avatar, Tile, avatar, first_name, reply_word};
use crate::hints::Hint;

/// A count with its thousands separated: `1,533`.
pub fn thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A tab's title.
pub fn tab_title(tab: ContactTab) -> &'static str {
    match tab {
        ContactTab::All => "All",
        ContactTab::People => "People",
        ContactTab::Companies => "Companies",
        ContactTab::Lists => "Lists and newsletters",
        ContactTab::Automated => "Automated",
    }
}

/// One tab in the strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabView {
    /// Which.
    pub tab: ContactTab,
    /// "People".
    pub title: String,
    /// "612"; `None` for All, whose count is the strip's total.
    pub count: Option<String>,
    /// Whether it is the one chosen.
    pub on: bool,
}

/// The "Worth a look" line over the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewLine {
    /// "Worth a look:".
    pub lead: String,
    /// "3 contacts may be the same person · 2 addresses bounced recently".
    pub text: String,
    /// "Review".
    pub button: String,
    /// Its key.
    pub key: Option<String>,
    /// Whether the review exists yet: the button is dimmed until it does.
    pub enabled: bool,
}

/// The strip over the list: the title, the tabs, the filter and New contact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripView {
    /// "Contacts".
    pub title: String,
    /// The key that comes here, `g c`.
    pub key: Option<String>,
    /// "1,533".
    pub total: String,
    /// The five tabs.
    pub tabs: Vec<TabView>,
    /// "Filter contacts".
    pub filter_placeholder: String,
    /// The filter's key.
    pub filter_key: Option<String>,
    /// "New contact".
    pub new_label: String,
    /// Its key.
    pub new_key: Option<String>,
    /// Whether New contact does anything yet.
    pub new_enabled: bool,
    /// The review line, hidden when there is nothing to review.
    pub review: Option<ReviewLine>,
}

/// "3 contacts may be the same person · 2 addresses bounced recently · 5
/// people you write to often aren't saved"; empty with nothing to say.
pub fn review_text(review: &ReviewSummary) -> String {
    let plural = |n: u32, one: &str, many: &str| -> String {
        let count = if n >= postio_model::REVIEW_CAP {
            format!("{}+", postio_model::REVIEW_CAP)
        } else {
            n.to_string()
        };
        format!("{count} {}", if n == 1 { one } else { many })
    };
    let mut parts = Vec::new();
    if review.duplicates > 0 {
        parts.push(plural(
            review.duplicates,
            "contact may be the same person",
            "contacts may be the same person",
        ));
    }
    if review.bounced > 0 {
        parts.push(plural(
            review.bounced,
            "address bounced recently",
            "addresses bounced recently",
        ));
    }
    if review.frequent_unsaved > 0 {
        parts.push(plural(
            review.frequent_unsaved,
            "person you write to often isn't saved",
            "people you write to often aren't saved",
        ));
    }
    parts.join(" · ")
}

/// The strip for `overview`, with `tab` chosen.
pub fn strip(overview: &ContactsOverview, tab: ContactTab, keymap: &Keymap) -> StripView {
    let review_words = review_text(&overview.review);
    StripView {
        title: "Contacts".to_owned(),
        key: keymap.binding(CommandId::OpenContacts).map(str::to_owned),
        total: thousands(overview.total),
        tabs: overview
            .tabs
            .iter()
            .map(|(each, count)| TabView {
                tab: *each,
                title: tab_title(*each).to_owned(),
                count: (*each != ContactTab::All).then(|| thousands(*count)),
                on: *each == tab,
            })
            .collect(),
        filter_placeholder: "Filter contacts".to_owned(),
        filter_key: keymap.binding(CommandId::ContactsFilter).map(str::to_owned),
        new_label: "New contact".to_owned(),
        new_key: keymap.binding(CommandId::ContactNew).map(str::to_owned),
        new_enabled: true,
        review: (!review_words.is_empty()).then(|| ReviewLine {
            lead: "Worth a look:".to_owned(),
            text: review_words,
            button: "Review".to_owned(),
            key: keymap
                .binding(CommandId::ContactsSuggestions)
                .map(str::to_owned),
            enabled: false,
        }),
    }
}

/// The letter a row's sort key files it under: its first letter, uppercased,
/// or `#` for a name that starts with anything else.
pub fn letter_of(sort_key: &str) -> char {
    sort_key
        .chars()
        .next()
        .filter(|c| c.is_alphabetic())
        .and_then(|c| c.to_uppercase().next())
        .unwrap_or('#')
}

/// The tag pill on a row: where their mail is sent, or how it is drawn.
/// One at most, the most telling first.
pub fn tag_of(row: &ContactListRow) -> Option<String> {
    match row.arrival {
        Some(ContactArrival::Filtered) => Some("filtered".to_owned()),
        Some(ContactArrival::Digest) => Some(match &row.digest_cadence {
            Some(cadence) => format!("digest {cadence}"),
            None => "digest".to_owned(),
        }),
        _ if row.notify => Some("notify".to_owned()),
        _ if row.html_original => Some("always original".to_owned()),
        _ => None,
    }
}

/// How long ago, on a row or a tile: `today`, a weekday inside the week, a
/// day and month inside the year, else a month and year.
pub fn when_word(at: Option<DateTime<Utc>>, now: DateTime<Local>) -> String {
    let Some(at) = at else {
        return String::new();
    };
    let local = at.with_timezone(&now.timezone());
    let days = (now.date_naive() - local.date_naive()).num_days();
    match days {
        ..=0 => "today".to_owned(),
        1..=6 => local.format("%a").to_string(),
        _ if local.year() == now.year() => local.format("%-d %b").to_string(),
        _ => local.format("%b %Y").to_string(),
    }
}

fn noun(n: u32, one: &str, many: &str) -> String {
    format!("{} {}", thousands(n), if n == 1 { one } else { many })
}

/// The right-hand line of a row: `431 messages`, `list · 112 messages`,
/// `newsletter · 48 issues`.
pub fn count_line(row: &ContactListRow) -> String {
    match row.kind {
        ContactKind::Person => noun(row.messages, "message", "messages"),
        ContactKind::Company => format!("company · {}", noun(row.messages, "message", "messages")),
        ContactKind::List => format!("list · {}", noun(row.messages, "message", "messages")),
        ContactKind::Newsletter => {
            format!("newsletter · {}", noun(row.messages, "issue", "issues"))
        }
        ContactKind::Automated => {
            format!("automated · {}", noun(row.messages, "message", "messages"))
        }
    }
}

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowView {
    /// The person.
    pub id: postio_model::ContactId,
    /// The avatar.
    pub avatar: Avatar,
    /// The name.
    pub name: String,
    /// The tag pill beside it.
    pub tag: Option<String>,
    /// The primary address, with `+2` for the rest.
    pub address: String,
    /// When they were last in touch.
    pub when: String,
    /// What the mail adds up to.
    pub count_line: String,
    /// The letter heading above this row when it starts one.
    pub letter: Option<char>,
}

/// A row, with its letter heading when it follows a row under another letter
/// (`before` is that row's sort key; `None` for the first row of the list).
pub fn row(row: &ContactListRow, before: Option<&str>, now: DateTime<Local>) -> RowView {
    let letter = letter_of(&row.sort_key);
    let address = match (&row.preferred, row.address_count) {
        (Some(address), n) if n > 1 => format!("{address} +{}", n - 1),
        (Some(address), _) => address.clone(),
        (None, _) => String::new(),
    };
    RowView {
        id: row.id,
        avatar: avatar(row.id, &row.name, row.kind),
        name: row.name.clone(),
        tag: tag_of(row),
        address,
        when: when_word(row.last_seen_at, now),
        count_line: count_line(row),
        letter: before
            .map(letter_of)
            .is_none_or(|previous| previous != letter)
            .then_some(letter),
    }
}

/// A button along the card's head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardButton {
    /// "Write".
    pub label: String,
    /// The command it runs.
    pub command: CommandId,
    /// Its key as bound.
    pub key: Option<String>,
    /// The filled one.
    pub primary: bool,
    /// Whether it does anything yet.
    pub enabled: bool,
}

/// One address on the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressLine {
    /// The address.
    pub address: String,
    /// The star: replies go here.
    pub primary: bool,
    /// "primary · replies go here", "last used March 2025 · 6 messages".
    pub note: String,
    /// "bounced 2 Sep", drawn in orange.
    pub bounced: Option<String>,
}

/// One switch under "How mail from X arrives".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchLine {
    /// "Notify me when Ada writes".
    pub title: String,
    /// "A quiet badge, even when Focus is in the background".
    pub note: String,
    /// Whether it is on.
    pub on: bool,
    /// What a click on it changes; `None` for a line that says something the
    /// arrival control changes (a newsletter's digest).
    pub switch: Option<postio_model::CardSwitch>,
}

/// The twelve-month strip.
#[derive(Debug, Clone, PartialEq)]
pub struct MonthStrip {
    /// Twelve heights, oldest first, 0.0 to 1.0 of the tallest month.
    pub bars: Vec<f32>,
    /// "Oct 2025".
    pub from: String,
    /// "messages per month".
    pub caption: String,
    /// "Sep 2026".
    pub to: String,
}

/// One line of "Open between you" or "Recent issues".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardLine {
    /// "Waiting on you", "Issue", a date.
    pub label: String,
    /// What it is.
    pub text: String,
    /// "opened", "not opened".
    pub state: String,
    /// Whether the move is the user's: the label is in the accent.
    pub yours: bool,
}

/// A file chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChipView {
    /// `XLSX`, `PDF`.
    pub kind: String,
    /// "Atlas-Q3-budget.xlsx".
    pub name: String,
}

/// A person's or a newsletter's card, drawn whole.
#[derive(Debug, Clone, PartialEq)]
pub struct CardView {
    /// The person.
    pub id: postio_model::ContactId,
    /// The avatar, drawn at 64.
    pub avatar: Avatar,
    /// The name.
    pub name: String,
    /// "Person · Finance lead, Northwind Partners · saved · groups: Atlas core".
    pub role: String,
    /// The head's buttons.
    pub buttons: Vec<CardButton>,
    /// "Addresses".
    pub addresses_title: String,
    /// The addresses, the preferred first.
    pub addresses: Vec<AddressLine>,
    /// "Also seen as “A. Moreno” and “Ada M.” in headers".
    pub also_seen: Option<String>,
    /// "How mail from Ada arrives".
    pub arrival_title: String,
    /// Inbox, Digest, Filtered out.
    pub arrival: Vec<crate::contact_popover::ArrivalOption>,
    /// The switches.
    pub switches: Vec<SwitchLine>,
    /// "Notes".
    pub notes_title: String,
    /// The note's text.
    pub notes: Option<String>,
    /// "Kept on this Mac."
    pub notes_foot: String,
    /// "Your mail together", "Its mail".
    pub stats_title: String,
    /// Four tiles.
    pub tiles: Vec<Tile>,
    /// The strip under them.
    pub months: MonthStrip,
    /// "Open between you", "Recent issues".
    pub lines_title: String,
    /// What is open, or the recent issues.
    pub lines: Vec<CardLine>,
    /// "Recent files from Ada"; `None` for a newsletter.
    pub files_title: Option<String>,
    /// The chips.
    pub files: Vec<FileChipView>,
}

/// Whether a kind is one of the people-like ones, whose card is "your mail
/// together" with Write first; the rest are somebody's mail you read.
pub fn is_correspondent(kind: ContactKind) -> bool {
    matches!(kind, ContactKind::Person | ContactKind::Company)
}

fn kind_word(kind: ContactKind) -> &'static str {
    match kind {
        ContactKind::Person => "Person",
        ContactKind::Company => "Company",
        ContactKind::List => "Mailing list",
        ContactKind::Newsletter => "Newsletter",
        ContactKind::Automated => "Automated sender",
    }
}

/// The line under the name.
pub fn role_line(card: &ContactCardRead) -> String {
    let person = &card.person;
    let mut parts = vec![kind_word(person.kind).to_owned()];
    if is_correspondent(person.kind) {
        let work: Vec<String> = [&person.title, &person.organization]
            .into_iter()
            .flatten()
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
            .collect();
        if !work.is_empty() {
            parts.push(work.join(", "));
        }
        parts.push(
            if person.is_saved() {
                "saved"
            } else {
                "not saved yet"
            }
            .to_owned(),
        );
    } else {
        parts.push(
            match person.kind {
                ContactKind::Newsletter => "recognised from its unsubscribe header",
                ContactKind::List => "recognised from its list headers",
                _ => "recognised as automated",
            }
            .to_owned(),
        );
        parts.push("stays in Postio, not synced".to_owned());
    }
    if !card.groups.is_empty() {
        parts.push(format!("groups: {}", card.groups.join(", ")));
    }
    parts.join(" · ")
}

/// The three-letter label of a file chip: its extension, else its type.
pub fn file_kind(name: &str, mime: &str) -> String {
    let from_name = name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| !ext.is_empty() && ext.len() <= 5 && ext.chars().all(char::is_alphanumeric));
    from_name
        .or_else(|| mime.rsplit('/').next())
        .unwrap_or("FILE")
        .chars()
        .take(4)
        .collect::<String>()
        .to_uppercase()
}

/// How often a sender sends, from the dates of what they sent: the word
/// (`daily`, `weekly`, `monthly`, `every 3 days`) and the weekday they most
/// often land on, for a weekly one.
pub fn frequency(recent: &[RecentMessage]) -> Option<(String, Option<Weekday>)> {
    let mut ins: Vec<DateTime<Utc>> = recent
        .iter()
        .filter(|message| !message.outgoing)
        .map(|message| message.at)
        .collect();
    if ins.len() < 3 {
        return None;
    }
    ins.sort();
    let mut gaps: Vec<i64> = ins
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).num_hours())
        .collect();
    gaps.sort();
    let days = gaps[gaps.len() / 2] as f64 / 24.0;
    let word = if days < 1.5 {
        "daily".to_owned()
    } else if (5.0..=9.0).contains(&days) {
        "weekly".to_owned()
    } else if (12.0..=16.0).contains(&days) {
        "fortnightly".to_owned()
    } else if (25.0..=35.0).contains(&days) {
        "monthly".to_owned()
    } else {
        format!("every {} days", days.round() as i64)
    };
    let mut by_day = [0u32; 7];
    for at in &ins {
        by_day[at.weekday().num_days_from_monday() as usize] += 1;
    }
    let (best, count) = by_day
        .iter()
        .enumerate()
        .max_by_key(|(_, count)| **count)
        .map(|(day, count)| (day, *count))?;
    let weekday = (word == "weekly" && count * 2 >= ins.len() as u32)
        .then(|| Weekday::try_from(best as u8).ok())
        .flatten();
    Some((word, weekday))
}

fn weekday_plural(day: Weekday) -> String {
    format!("on {}s", day)
        .replace("Mon", "Monday")
        .replace("Tue", "Tuesday")
        .replace("Wed", "Wednesday")
        .replace("Thu", "Thursday")
        .replace("Fri", "Friday")
        .replace("Sat", "Saturday")
        .replace("Sun", "Sunday")
}

/// The card for `card`, as of `now`.
pub fn card(card: &ContactCardRead, keymap: &Keymap, now: DateTime<Local>) -> CardView {
    let person = &card.person;
    let name = person.shown_as().into_owned();
    let first = first_name(person);
    let correspondent = is_correspondent(person.kind);
    let bind = |command| keymap.binding(command).map(str::to_owned);
    let button = |label: &str, command, primary, enabled| CardButton {
        label: label.to_owned(),
        command,
        key: bind(command),
        primary,
        enabled,
    };
    let buttons = if correspondent {
        vec![
            button("Write", CommandId::ContactCompose, true, true),
            button("All mail", CommandId::ContactShowMail, false, true),
            button("Edit", CommandId::ContactEdit, false, false),
        ]
    } else {
        let all = if person.kind == ContactKind::Newsletter {
            "All issues"
        } else {
            "All messages"
        };
        vec![
            button(all, CommandId::ContactShowMail, true, true),
            button("Unsubscribe", CommandId::Unsubscribe, false, false),
            button("Edit", CommandId::ContactEdit, false, false),
        ]
    };
    let preferred = person.preferred_address().map(|owned| owned.id);
    let mut addresses: Vec<AddressLine> = person
        .addresses
        .iter()
        .map(|owned| {
            let primary = Some(owned.id) == preferred;
            let note = if primary {
                "primary · replies go here".to_owned()
            } else {
                match owned.last_seen_at {
                    Some(at) => format!(
                        "last used {} · {}",
                        at.with_timezone(&now.timezone()).format("%B %Y"),
                        noun(owned.times_seen, "message", "messages")
                    ),
                    None => noun(owned.times_seen, "message", "messages"),
                }
            };
            AddressLine {
                address: owned.address.address.clone(),
                primary,
                note,
                bounced: card
                    .address_notes
                    .iter()
                    .find(|note| note.address == owned.id)
                    .and_then(|note| note.bounced_at)
                    .map(|at| {
                        format!(
                            "bounced {}",
                            at.with_timezone(&now.timezone()).format("%-d %b")
                        )
                    }),
            }
        })
        .collect();
    addresses.sort_by_key(|line| !line.primary);
    let seen: Vec<String> = {
        let shown = person.display_name().to_owned();
        let mut names: Vec<String> = Vec::new();
        for owned in &person.addresses {
            if let Some(name) = owned
                .address
                .name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty() && *name != shown)
                && !names.iter().any(|known| known == name)
            {
                names.push(name.to_owned());
            }
        }
        names
            .into_iter()
            .map(|name| format!("\u{201c}{name}\u{201d}"))
            .collect()
    };
    let also_seen = match seen.as_slice() {
        [] => None,
        [one] => Some(format!("Also seen as {one} in headers")),
        [rest @ .., last] => Some(format!(
            "Also seen as {} and {last} in headers",
            rest.join(", ")
        )),
    };
    let in_force = card.settings.arrival.unwrap_or(card.default_arrival);
    let arrival = [
        ("Inbox", ContactArrival::Inbox),
        ("Digest", ContactArrival::Digest),
        ("Filtered out", ContactArrival::Filtered),
    ]
    .into_iter()
    .map(|(label, each)| crate::contact_popover::ArrivalOption {
        label: label.to_owned(),
        arrival: each,
        on: each == in_force,
    })
    .collect();
    let html_on = card.settings.html_treatment.as_deref() == Some("paper");
    let switches = if correspondent {
        vec![
            SwitchLine {
                title: format!("Notify me when {first} writes"),
                note: "A quiet badge, even when Focus is in the background".to_owned(),
                on: card.settings.notify,
                switch: Some(postio_model::CardSwitch::Notify),
            },
            SwitchLine {
                title: format!("Always show {first}\u{2019}s HTML as sent"),
                note: if html_on {
                    "Their own background and fonts, whatever the app colours say"
                } else {
                    "Off: the usual rule decides"
                }
                .to_owned(),
                on: html_on,
                switch: Some(postio_model::CardSwitch::HtmlAsSent),
            },
            SwitchLine {
                title: "Remind if no reply, by default".to_owned(),
                note: match card.settings.remind_days {
                    Some(days) => format!(
                        "When you write to {first}, turn on {} for {days} {}",
                        keymap
                            .binding(CommandId::RemindIfNoReply)
                            .map(|key| crate::hints::short(key)
                                .replace("cmd+", "\u{2318}")
                                .to_uppercase())
                            .unwrap_or_else(|| "it".to_owned()),
                        if days == 1 { "day" } else { "days" }
                    ),
                    None => format!("Off: nothing waits on {first}'s reply unless you ask"),
                },
                on: card.settings.remind_days.is_some(),
                switch: Some(postio_model::CardSwitch::Remind),
            },
        ]
    } else {
        let digest = match (&card.settings.digest, in_force) {
            (Some(digest), _) => {
                let day = digest
                    .day
                    .as_deref()
                    .map(|day| format!(", {day}"))
                    .unwrap_or_default();
                format!("In the {} digest{day} {}", digest.cadence, digest.at)
            }
            (None, ContactArrival::Digest) => "In a digest".to_owned(),
            (None, _) => "Not in a digest".to_owned(),
        };
        vec![
            SwitchLine {
                title: digest,
                note: "Edit the rule with d".to_owned(),
                on: in_force == ContactArrival::Digest,
                switch: None,
            },
            SwitchLine {
                title: "Always show its HTML as sent".to_owned(),
                note: if html_on {
                    "It sets its own background"
                } else {
                    "Off: the usual rule decides"
                }
                .to_owned(),
                on: html_on,
                switch: Some(postio_model::CardSwitch::HtmlAsSent),
            },
            SwitchLine {
                title: match card.settings.archive_after_days {
                    Some(days) => format!("Archive issues after {days} days"),
                    None => "Archive issues automatically".to_owned(),
                },
                note: "Read or not; they stay searchable".to_owned(),
                on: card.settings.archive_after_days.is_some(),
                switch: Some(postio_model::CardSwitch::Archive),
            },
        ]
    };
    let stats = &card.stats;
    let year: u32 = stats.months.iter().map(|(_, n)| *n).sum();
    let tiles = if correspondent {
        vec![
            Tile {
                value: thousands(stats.messages),
                caption: if stats.messages == 1 {
                    "message"
                } else {
                    "messages"
                }
                .to_owned(),
            },
            Tile {
                value: thousands(stats.from_you),
                caption: "from you".to_owned(),
            },
            Tile {
                value: reply_word(stats.reply_median()),
                caption: "replies in".to_owned(),
            },
            Tile {
                value: when_word(stats.last, now),
                caption: match stats.last {
                    Some(at) => {
                        format!("last {}", at.with_timezone(&now.timezone()).format("%H:%M"))
                    }
                    None => "last".to_owned(),
                },
            },
        ]
    } else {
        let (cadence, day) = frequency(&stats.recent).unwrap_or_else(|| ("—".to_owned(), None));
        vec![
            Tile {
                value: thousands(year),
                caption: if year == 1 {
                    "issue this year"
                } else {
                    "issues this year"
                }
                .to_owned(),
            },
            Tile {
                value: thousands(stats.opened),
                caption: "you opened".to_owned(),
            },
            Tile {
                value: cadence,
                caption: day
                    .map(weekday_plural)
                    .unwrap_or_else(|| "between issues".to_owned()),
            },
            Tile {
                value: when_word(stats.last, now),
                caption: "last issue".to_owned(),
            },
        ]
    };
    let this_month = postio_model::month_key(now.with_timezone(&Utc));
    let by_month = |month: i64| {
        stats
            .months
            .iter()
            .find(|(key, _)| *key == month)
            .map_or(0, |(_, n)| *n)
    };
    let counts: Vec<u32> = (0..12)
        .map(|back| by_month(this_month - 11 + back))
        .collect();
    let tallest = counts.iter().copied().max().unwrap_or(0).max(1) as f32;
    let month_label = |month: i64| {
        let (year, index) = (month.div_euclid(12), month.rem_euclid(12));
        chrono::NaiveDate::from_ymd_opt(year as i32, index as u32 + 1, 1)
            .map(|day| day.format("%b %Y").to_string())
            .unwrap_or_default()
    };
    let months = MonthStrip {
        bars: counts.iter().map(|n| *n as f32 / tallest).collect(),
        from: month_label(this_month - 11),
        caption: "messages per month".to_owned(),
        to: month_label(this_month),
    };
    let (lines_title, lines) = if correspondent {
        (
            "Open between you".to_owned(),
            card.open
                .iter()
                .take(CARD_RECENT as usize)
                .map(|item| CardLine {
                    label: match item.side {
                        postio_model::OpenSide::WaitingOnYou => "Waiting on you".to_owned(),
                        postio_model::OpenSide::WaitingOnThem => format!("Waiting on {first}"),
                    },
                    text: item.text.clone(),
                    state: String::new(),
                    yours: item.side == postio_model::OpenSide::WaitingOnYou,
                })
                .collect(),
        )
    } else {
        (
            "Recent issues".to_owned(),
            card.issues
                .iter()
                .map(|issue| CardLine {
                    label: when_word(Some(issue.at), now),
                    text: issue.subject.clone(),
                    state: if issue.opened { "opened" } else { "not opened" }.to_owned(),
                    yours: false,
                })
                .collect(),
        )
    };
    CardView {
        id: person.id,
        avatar: avatar(person.id, &name, person.kind),
        name,
        role: role_line(card),
        buttons,
        addresses_title: "Addresses".to_owned(),
        addresses,
        also_seen,
        arrival_title: format!("How mail from {first} arrives"),
        arrival,
        switches,
        notes_title: "Notes".to_owned(),
        notes: person
            .note
            .as_deref()
            .map(str::trim)
            .filter(|note| !note.is_empty())
            .map(str::to_owned),
        notes_foot: "Kept on this Mac.".to_owned(),
        stats_title: if correspondent {
            "Your mail together"
        } else {
            "Its mail"
        }
        .to_owned(),
        tiles,
        months,
        lines_title,
        lines,
        files_title: correspondent.then(|| format!("Recent files from {first}")),
        files: if correspondent {
            card.files
                .iter()
                .map(|file| FileChipView {
                    kind: file_kind(&file.name, &file.mime),
                    name: file.name.clone(),
                })
                .collect()
        } else {
            Vec::new()
        },
    }
}

/// The footer: the keys that work here, and the counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterView {
    /// `j/k move`, `Return all mail with them`.
    pub hints: Vec<Hint>,
    /// "1,533 contacts · 640 saved".
    pub status: String,
}

/// The footer for `overview`.
pub fn footer(overview: &ContactsOverview, keymap: &Keymap) -> FooterView {
    use crate::hints::{hint, pair};
    let hints = [
        pair(
            keymap,
            CommandId::NextMessage,
            CommandId::PrevMessage,
            "move",
        ),
        hint(keymap, CommandId::ContactShowMail, "all mail with them"),
        hint(keymap, CommandId::ContactCompose, "write"),
        hint(keymap, CommandId::ContactEdit, "edit"),
        hint(
            keymap,
            CommandId::ContactPopover,
            "contact popover, anywhere",
        ),
        hint(keymap, CommandId::ContactJoin, "merge selected"),
        hint(keymap, CommandId::ContactsFilter, "filter"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut status = format!(
        "{} {} \u{b7} {} saved",
        thousands(overview.total),
        if overview.total == 1 {
            "contact"
        } else {
            "contacts"
        },
        thousands(overview.saved)
    );
    if let Some((made, of)) = overview.building {
        status = format!(
            "Building contacts, {} of {} \u{b7} {status}",
            thousands(made),
            thousands(of)
        );
    }
    FooterView { hints, status }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use postio_model::{
        AddressId, Contact, ContactAddress, ContactId, ContactSettings, ContactSource,
        ContactState, ContactStats, EmailAddress, FileChip, IssueRead, MessageId, OpenItem,
        OpenSide,
    };

    use super::*;

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 26, 16, 5, 0).unwrap()
    }

    fn keymap() -> Keymap {
        Keymap::resolve_on(
            &postio_config::KeyBindings::default(),
            postio_config::paths::Platform::Apple,
        )
    }

    fn list_row(kind: ContactKind, name: &str, messages: u32) -> ContactListRow {
        ContactListRow {
            id: ContactId::new(7),
            name: name.to_owned(),
            preferred: Some("ada@example.org".to_owned()),
            address_count: 3,
            last_seen_at: Some(now().with_timezone(&Utc)),
            source: ContactSource::Mail,
            state: ContactState::Live,
            sort_key: name.to_lowercase(),
            kind,
            messages,
            saved: true,
            arrival: None,
            notify: false,
            html_original: false,
            digest_cadence: None,
        }
    }

    #[test]
    fn counts_have_their_thousands_separated() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1533), "1,533");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn the_strip_has_five_tabs_with_the_chosen_one_on_and_all_without_a_count() {
        let overview = ContactsOverview {
            total: 1533,
            saved: 640,
            tabs: vec![
                (ContactTab::All, 1533),
                (ContactTab::People, 612),
                (ContactTab::Companies, 88),
                (ContactTab::Lists, 141),
                (ContactTab::Automated, 692),
            ],
            review: ReviewSummary::default(),
            building: None,
        };
        let strip = strip(&overview, ContactTab::People, &keymap());
        assert_eq!(strip.title, "Contacts");
        assert_eq!(strip.key.as_deref(), Some("g c"));
        assert_eq!(strip.total, "1,533");
        let tabs: Vec<_> = strip
            .tabs
            .iter()
            .map(|tab| (tab.title.as_str(), tab.count.as_deref(), tab.on))
            .collect();
        assert_eq!(
            tabs,
            [
                ("All", None, false),
                ("People", Some("612"), true),
                ("Companies", Some("88"), false),
                ("Lists and newsletters", Some("141"), false),
                ("Automated", Some("692"), false),
            ]
        );
        assert!(strip.review.is_none(), "nothing to review hides the line");
    }

    #[test]
    fn the_review_line_names_only_what_is_waiting() {
        let review = ReviewSummary {
            duplicates: 3,
            bounced: 2,
            frequent_unsaved: 5,
        };
        assert_eq!(
            review_text(&review),
            "3 contacts may be the same person · 2 addresses bounced recently · 5 people you write to often aren't saved"
        );
        assert_eq!(
            review_text(&ReviewSummary {
                duplicates: 0,
                bounced: 1,
                frequent_unsaved: 99,
            }),
            "1 address bounced recently · 99+ people you write to often aren't saved"
        );
        assert_eq!(review_text(&ReviewSummary::default()), "");
    }

    #[test]
    fn a_row_says_the_name_the_tag_the_address_and_the_count_line() {
        let mut ada = list_row(ContactKind::Person, "Ada Moreno", 431);
        ada.notify = true;
        let view = row(&ada, None, now());
        assert_eq!(view.name, "Ada Moreno");
        assert_eq!(view.tag.as_deref(), Some("notify"));
        assert_eq!(view.address, "ada@example.org +2");
        assert_eq!(view.when, "today");
        assert_eq!(view.count_line, "431 messages");
        assert_eq!(view.letter, Some('A'), "the first row starts its letter");
        let mut news = list_row(ContactKind::Newsletter, "Field Notes Weekly", 48);
        news.arrival = Some(ContactArrival::Digest);
        news.digest_cadence = Some("weekly".to_owned());
        news.address_count = 1;
        let view = row(&news, Some("ada moreno"), now());
        assert_eq!(view.tag.as_deref(), Some("digest weekly"));
        assert_eq!(view.count_line, "newsletter · 48 issues");
        assert_eq!(view.address, "ada@example.org");
        assert_eq!(view.letter, Some('F'));
        assert_eq!(row(&news, Some("field guide"), now()).letter, None);
        for (kind, line) in [
            (ContactKind::Company, "company · 57 messages"),
            (ContactKind::List, "list · 57 messages"),
            (ContactKind::Automated, "automated · 57 messages"),
        ] {
            assert_eq!(count_line(&list_row(kind, "X", 57)), line);
        }
        assert_eq!(
            count_line(&list_row(ContactKind::Person, "X", 1)),
            "1 message"
        );
    }

    #[test]
    fn a_tag_prefers_where_mail_goes_over_how_it_is_drawn() {
        let mut row = list_row(ContactKind::Person, "X", 1);
        row.html_original = true;
        assert_eq!(tag_of(&row).as_deref(), Some("always original"));
        row.notify = true;
        assert_eq!(tag_of(&row).as_deref(), Some("notify"));
        row.arrival = Some(ContactArrival::Filtered);
        assert_eq!(tag_of(&row).as_deref(), Some("filtered"));
        row.arrival = Some(ContactArrival::Inbox);
        assert_eq!(tag_of(&row).as_deref(), Some("notify"));
    }

    #[test]
    fn a_letter_is_the_first_letter_or_a_hash() {
        assert_eq!(letter_of("ada moreno"), 'A');
        assert_eq!(letter_of("émile"), 'É');
        assert_eq!(letter_of("3m"), '#');
        assert_eq!(letter_of(""), '#');
    }

    fn address(id: i64, email: &str, name: Option<&str>, seen: u32) -> ContactAddress {
        let mut owned = ContactAddress::new(AddressId::new(id), EmailAddress::new(name, email));
        owned.times_seen = seen;
        owned.last_seen_at = Some(Utc.with_ymd_and_hms(2025, 3, 4, 9, 0, 0).unwrap());
        owned
    }

    fn ada_card() -> ContactCardRead {
        let mut person = Contact::new(address(1, "ada@example.org", Some("Ada Moreno"), 431));
        person.id = ContactId::new(7);
        person.title = Some("Finance lead".into());
        person.organization = Some("Northwind Partners".into());
        person.saved_at = Some(Utc::now());
        person.note = Some("Prefers numbers in a sheet.".into());
        person
            .addresses
            .push(address(2, "a.moreno@example.net", Some("A. Moreno"), 14));
        person
            .addresses
            .push(address(3, "ada.moreno@example.com", Some("Ada M."), 6));
        let month = postio_model::month_key(now().with_timezone(&Utc));
        ContactCardRead {
            person,
            groups: vec!["Atlas core".into(), "Finance".into()],
            stats: ContactStats {
                messages: 431,
                from_you: 168,
                opened: 400,
                last: Some(now().with_timezone(&Utc)),
                months: vec![(month - 11, 10), (month - 1, 20), (month, 40)],
                recent: Vec::new(),
            },
            open: vec![
                OpenItem {
                    side: OpenSide::WaitingOnYou,
                    text: "Can you approve these by Friday".into(),
                    message: MessageId::new(1),
                },
                OpenItem {
                    side: OpenSide::WaitingOnThem,
                    text: "Q4 headcount numbers".into(),
                    message: MessageId::new(2),
                },
            ],
            settings: ContactSettings {
                notify: true,
                remind_days: Some(3),
                ..ContactSettings::default()
            },
            default_arrival: ContactArrival::Inbox,
            files: vec![FileChip {
                name: "Atlas-Q3-budget.xlsx".into(),
                mime: "application/vnd.ms-excel".into(),
            }],
            issues: Vec::new(),
            address_notes: Vec::new(),
        }
    }

    #[test]
    fn a_persons_card_says_who_where_how_much_and_what_is_open() {
        let view = card(&ada_card(), &keymap(), now());
        assert_eq!(view.name, "Ada Moreno");
        assert_eq!(
            view.role,
            "Person · Finance lead, Northwind Partners · saved · groups: Atlas core, Finance"
        );
        let buttons: Vec<_> = view
            .buttons
            .iter()
            .map(|button| (button.label.as_str(), button.primary, button.enabled))
            .collect();
        assert_eq!(
            buttons,
            [
                ("Write", true, true),
                ("All mail", false, true),
                ("Edit", false, false)
            ]
        );
        assert_eq!(view.addresses[0].address, "ada@example.org");
        assert!(view.addresses[0].primary);
        assert_eq!(view.addresses[0].note, "primary · replies go here");
        assert_eq!(view.addresses[1].note, "last used March 2025 · 14 messages");
        assert_eq!(
            view.also_seen.as_deref(),
            Some("Also seen as \u{201c}A. Moreno\u{201d} and \u{201c}Ada M.\u{201d} in headers")
        );
        assert_eq!(view.arrival_title, "How mail from Ada arrives");
        assert_eq!(
            view.arrival
                .iter()
                .filter(|o| o.on)
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            ["Inbox"]
        );
        let switches: Vec<_> = view
            .switches
            .iter()
            .map(|s| (s.title.as_str(), s.on))
            .collect();
        assert_eq!(
            switches,
            [
                ("Notify me when Ada writes", true),
                ("Always show Ada\u{2019}s HTML as sent", false),
                ("Remind if no reply, by default", true),
            ]
        );
        assert!(
            view.switches[2]
                .note
                .starts_with("When you write to Ada, turn on ")
        );
        assert_eq!(view.notes.as_deref(), Some("Prefers numbers in a sheet."));
        assert_eq!(view.stats_title, "Your mail together");
        let tiles: Vec<_> = view
            .tiles
            .iter()
            .map(|t| (t.value.as_str(), t.caption.as_str()))
            .collect();
        assert_eq!(
            tiles,
            [
                ("431", "messages"),
                ("168", "from you"),
                ("—", "replies in"),
                ("today", "last 16:05")
            ]
        );
        assert_eq!(view.months.bars.len(), 12);
        assert_eq!(view.months.bars[11], 1.0);
        assert_eq!(view.months.bars[10], 0.5);
        assert_eq!(view.months.bars[0], 0.25);
        assert_eq!(view.months.bars[5], 0.0);
        assert_eq!(
            (view.months.from.as_str(), view.months.to.as_str()),
            ("Oct 2025", "Sep 2026")
        );
        assert_eq!(view.lines_title, "Open between you");
        assert_eq!(view.lines[0].label, "Waiting on you");
        assert!(view.lines[0].yours);
        assert_eq!(view.lines[1].label, "Waiting on Ada");
        assert_eq!(view.files_title.as_deref(), Some("Recent files from Ada"));
        assert_eq!(view.files[0].kind, "XLSX");
    }

    #[test]
    fn a_newsletters_card_is_its_issues_and_how_often_they_come() {
        let mut news = ada_card();
        news.person.kind = ContactKind::Newsletter;
        news.person.name = Some("Field Notes Weekly".into());
        news.person.title = None;
        news.person.organization = None;
        news.person.note = None;
        news.person.addresses.truncate(1);
        news.settings = ContactSettings {
            arrival: Some(ContactArrival::Digest),
            digest: Some(postio_model::contact_settings::DigestSetting {
                name: "Newsletters".into(),
                cadence: "weekly".into(),
                day: Some("Saturday".into()),
                at: "16:00".into(),
            }),
            archive_after_days: Some(30),
            ..ContactSettings::default()
        };
        // Weekly, on Saturdays: 5 Sep to 26 Sep 2026 are Saturdays.
        news.stats.recent = (0..4)
            .map(|week| RecentMessage {
                message: MessageId::new(week + 1),
                at: Utc.with_ymd_and_hms(2026, 9, 5, 12, 0, 0).unwrap()
                    + chrono::Duration::days(7 * week),
                outgoing: false,
            })
            .collect();
        news.stats.opened = 31;
        news.files = Vec::new();
        news.issues = vec![
            IssueRead {
                message: MessageId::new(4),
                subject: "The quiet season".into(),
                at: Utc.with_ymd_and_hms(2026, 9, 26, 12, 0, 0).unwrap(),
                opened: false,
            },
            IssueRead {
                message: MessageId::new(3),
                subject: "Seed saving, done properly".into(),
                at: Utc.with_ymd_and_hms(2026, 9, 19, 12, 0, 0).unwrap(),
                opened: true,
            },
        ];
        let view = card(&news, &keymap(), now());
        assert_eq!(
            view.role,
            "Newsletter · recognised from its unsubscribe header · stays in Postio, not synced · groups: Atlas core, Finance"
        );
        let buttons: Vec<_> = view.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(buttons, ["All issues", "Unsubscribe", "Edit"]);
        assert_eq!(view.stats_title, "Its mail");
        let tiles: Vec<_> = view
            .tiles
            .iter()
            .map(|t| (t.value.as_str(), t.caption.as_str()))
            .collect();
        assert_eq!(
            tiles,
            [
                ("70", "issues this year"),
                ("31", "you opened"),
                ("weekly", "on Saturdays"),
                ("today", "last issue"),
            ]
        );
        assert_eq!(view.lines_title, "Recent issues");
        assert_eq!(
            view.lines
                .iter()
                .map(|l| (l.text.as_str(), l.state.as_str()))
                .collect::<Vec<_>>(),
            [
                ("The quiet season", "not opened"),
                ("Seed saving, done properly", "opened")
            ]
        );
        assert!(
            view.files_title.is_none() && view.files.is_empty(),
            "no files for a newsletter"
        );
        assert_eq!(
            view.switches[0].title,
            "In the weekly digest, Saturday 16:00"
        );
        assert!(view.switches[0].on);
        assert_eq!(view.switches[2].title, "Archive issues after 30 days");
        assert_eq!(
            view.arrival
                .iter()
                .filter(|o| o.on)
                .map(|o| o.label.as_str())
                .collect::<Vec<_>>(),
            ["Digest"]
        );
    }

    #[test]
    fn a_file_chip_is_lettered_by_its_extension() {
        assert_eq!(file_kind("Atlas-Q3-budget.xlsx", "application/x"), "XLSX");
        assert_eq!(file_kind("invoice-2026-08.pdf", "application/pdf"), "PDF");
        assert_eq!(file_kind("README", "text/plain"), "PLAI");
        assert_eq!(file_kind("a.verylongextension", "image/png"), "PNG");
    }

    #[test]
    fn the_footer_names_the_keys_and_says_how_far_the_catch_up_has_got() {
        let mut overview = ContactsOverview {
            total: 1533,
            saved: 640,
            ..ContactsOverview::default()
        };
        let footer_view = footer(&overview, &keymap());
        assert_eq!(footer_view.status, "1,533 contacts · 640 saved");
        let labels: Vec<_> = footer_view.hints.iter().map(|h| h.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "move",
                "all mail with them",
                "write",
                "edit",
                "contact popover, anywhere",
                "merge selected",
                "filter"
            ]
        );
        overview.building = Some((1200, 1533));
        assert_eq!(
            footer(&overview, &keymap()).status,
            "Building contacts, 1,200 of 1,533 · 1,533 contacts · 640 saved"
        );
    }

    #[test]
    fn a_sender_with_too_few_messages_has_no_frequency() {
        assert!(frequency(&[]).is_none());
    }
}
