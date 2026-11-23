//! What Settings' Filtering page says: whether Focus files spam and automated updates
//! away, and what `[focus.filter]` holds, from the file's values and the keymap.

use postio_config::{FocusConfig, NeverEntry, never_entry};
use postio_core::{CommandId, Keymap};

use crate::filtered;
use crate::hints::{self, Hint};

pub const SWITCH: &str = "Filter spam and automated updates";

pub const FILTERED: &str = "Filtered";

pub const OPEN: &str = "Open Filtered";

pub const NEVER: &str = "Never filtered";

pub const STOPPED: &str = "Markers turned off";

pub const KEPT: &str =
    "Nothing is deleted: filtered mail is archived, and Filtered lists all of it.";

pub const GUARDS: &str = "Whatever its headers say, mail is never filtered from anyone you have \
written to, from your own domain, in a thread you took part in, or from a sender listed \
here.";

pub const STOPPED_EMPTY: &str =
    "None. Dismissing one kind of marker from a sender three times turns it off here.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub on: bool,
    pub state: String,
    /// `None` while filtering is off or before the count is known.
    pub today: Option<String>,
    pub keys: Vec<Hint>,
    pub never: Vec<Listed>,
    pub never_empty: String,
    pub stopped: Vec<Listed>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub says: String,
    /// Whether the entry does what it was written to do; one that does not says why.
    pub acts: bool,
    pub undo: Undo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undo {
    Never(String),
    Marker { sender: String, kind: String },
}

impl Undo {
    pub fn label(&self) -> &'static str {
        match self {
            Undo::Never(_) => NEVER_UNDO,
            Undo::Marker { .. } => STOPPED_UNDO,
        }
    }
}

pub const NEVER_UNDO: &str = "Filter again";

pub const STOPPED_UNDO: &str = "Turn back on";

pub fn page(focus: &FocusConfig, filtered_today: Option<u32>, keymap: &Keymap) -> Page {
    let on = focus.filtering;
    let state = state(on);
    let today = today(on, filtered_today);
    let mut keys = Vec::new();
    keys.extend(hints::hint(
        keymap,
        CommandId::RestoreFiltered,
        "restore + never filter sender",
    ));
    if on {
        keys.extend(hints::hint(
            keymap,
            CommandId::SweepInbox,
            "sweep the inbox",
        ));
    }
    let never_empty = match hints::key(keymap, CommandId::RestoreFiltered) {
        Some(key) => {
            format!("Nobody yet. {key} in Filtered restores a message and lists its sender here.")
        }
        None => "Nobody yet. Restoring a message in Filtered lists its sender here.".to_owned(),
    };
    Page {
        on,
        state,
        today,
        keys,
        never: never_list(focus),
        never_empty,
        stopped: stopped_list(focus),
    }
}

/// A person's pin is listed once, by name: it covers every address they own.
fn never_list(focus: &FocusConfig) -> Vec<Listed> {
    let mut listed_people: Vec<i64> = Vec::new();
    let mut lines = Vec::new();
    for entry in &focus.filter.never {
        let Some(person) = focus.contact_pinned(entry) else {
            lines.push(never(entry));
            continue;
        };
        if listed_people.contains(&person.contact) {
            continue;
        }
        listed_people.push(person.contact);
        lines.push(Listed {
            says: format!("{} \u{b7} {}", person.name, person.addresses.join(", ")),
            acts: true,
            undo: Undo::Never(entry.trim().to_owned()),
        });
    }
    lines
}

fn stopped_list(focus: &FocusConfig) -> Vec<Listed> {
    let mut listed: Vec<(i64, String)> = Vec::new();
    let mut lines = Vec::new();
    for stop in &focus.filter.stop_markers {
        let Some(person) = focus.contact_stopped(&stop.sender, &stop.kind) else {
            lines.push(stopped(&stop.sender, &stop.kind, stop.is_well_formed()));
            continue;
        };
        let kind = stop.kind.trim().to_ascii_lowercase();
        if listed.contains(&(person.contact, kind.clone())) {
            continue;
        }
        listed.push((person.contact, kind.clone()));
        let kinds = if kind == "todo" {
            "To-dos"
        } else {
            "Questions"
        };
        lines.push(Listed {
            says: format!("{kinds} in mail from {}", person.name),
            acts: true,
            undo: Undo::Marker {
                sender: stop.sender.clone(),
                kind: stop.kind.clone(),
            },
        });
    }
    lines
}

pub fn state(on: bool) -> String {
    if on {
        format!(
            "{} mail are archived as they arrive, each with its reason, and never reach the \
             inbox.",
            kinds()
        )
    } else {
        "Off: all new mail arrives in the inbox. What was filtered before stays in Filtered."
            .to_owned()
    }
}

/// `None` while filtering is off: a count shows only while its feature is in use.
pub fn today(on: bool, filtered_today: Option<u32>) -> Option<String> {
    filtered_today.filter(|_| on).map(|count| match count {
        0 => "Nothing filtered today".to_owned(),
        count => filtered::today(count),
    })
}

fn kinds() -> String {
    let names: Vec<String> = filtered::TABS
        .iter()
        .filter(|(reason, _)| reason.is_some())
        .map(|(_, name)| name.to_lowercase())
        .collect();
    let said = match names.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    };
    let mut letters = said.chars();
    letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default()
}

fn who(entry: &str) -> Option<String> {
    match never_entry(entry)? {
        NeverEntry::Address(address) => Some(address.to_owned()),
        NeverEntry::Domain(domain) => Some(format!("everyone at {domain}")),
    }
}

fn never(entry: &str) -> Listed {
    let undo = Undo::Never(entry.to_owned());
    match who(entry) {
        Some(says) => Listed {
            says,
            acts: true,
            undo,
        },
        None => Listed {
            says: format!(
                "{} \u{b7} neither an address nor a whole domain, so it pins nobody",
                entry.trim()
            ),
            acts: false,
            undo,
        },
    }
}

fn stopped(sender: &str, kind: &str, well_formed: bool) -> Listed {
    let kinds = match kind.trim().to_ascii_lowercase().as_str() {
        "question" => Some("Questions"),
        "todo" => Some("To-dos"),
        _ => None,
    };
    let undo = Undo::Marker {
        sender: sender.to_owned(),
        kind: kind.to_owned(),
    };
    match (well_formed, kinds, who(sender)) {
        (true, Some(kinds), Some(who)) => Listed {
            says: format!("{kinds} in mail from {who}"),
            acts: true,
            undo,
        },
        _ => Listed {
            says: format!(
                "{} \u{b7} {} \u{b7} not a sender and a kind of marker, so it turns nothing off",
                sender.trim(),
                kind.trim()
            ),
            acts: false,
            undo,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_config::Config;

    fn focus(text: &str) -> FocusConfig {
        Config::from_toml_str(text).expect("a config").focus
    }

    fn ada() -> postio_config::ContactRule {
        let mut settings = postio_model::ContactSettings {
            arrival: Some(postio_model::ContactArrival::Inbox),
            ..postio_model::ContactSettings::default()
        };
        settings.set_stop(postio_model::StoppedMarker::Todo, true);
        postio_config::ContactRule {
            contact: 7,
            name: "Ada Lovelace".into(),
            addresses: vec!["ada@work.example".into(), "ada@home.example".into()],
            settings,
        }
    }

    #[test]
    fn a_person_s_pin_and_markers_are_listed_by_name_once_whatever_their_addresses() {
        let focus = focus("[focus.filter]\nnever = [\"@example.net\"]\n").with_contacts(&[ada()]);
        let page = page(&focus, None, Keymap::defaults());

        let never: Vec<&str> = page.never.iter().map(|entry| entry.says.as_str()).collect();
        assert_eq!(
            never,
            [
                "everyone at example.net",
                "Ada Lovelace \u{b7} ada@work.example, ada@home.example"
            ],
            "one line for the person, not one per address"
        );
        let stopped: Vec<&str> = page
            .stopped
            .iter()
            .map(|entry| entry.says.as_str())
            .collect();
        assert_eq!(stopped, ["To-dos in mail from Ada Lovelace"]);
        assert!(page.never.iter().all(|entry| entry.acts));
        assert_eq!(
            page.never[1].undo,
            Undo::Never("ada@work.example".to_owned())
        );
    }

    #[test]
    fn on_it_says_what_is_filed_away_and_that_nothing_is_lost() {
        let page = page(&focus(""), Some(186), Keymap::defaults());
        assert!(page.on, "filtering starts on (FR-119)");
        assert_eq!(
            page.state,
            "Spam, promotions, notifications, receipts, shipping and social mail are archived \
             as they arrive, each with its reason, and never reach the inbox."
        );
        assert_eq!(page.today.as_deref(), Some("186 filtered today"));
        assert!(KEPT.contains("Nothing is deleted"));
        assert_eq!(
            hints::line(&page.keys),
            "R restore + never filter sender \u{b7} F sweep the inbox"
        );
    }

    #[test]
    fn a_day_with_nothing_filtered_says_so_and_an_unknown_count_says_nothing() {
        let keymap = Keymap::defaults();
        assert_eq!(
            page(&focus(""), Some(0), keymap).today.as_deref(),
            Some("Nothing filtered today")
        );
        assert_eq!(page(&focus(""), None, keymap).today, None);
    }

    #[test]
    fn off_it_says_mail_reaches_the_inbox_and_counts_nothing() {
        let page = page(
            &focus("[focus]\nfiltering = false\n"),
            Some(186),
            Keymap::defaults(),
        );
        assert!(!page.on);
        assert_eq!(
            page.state,
            "Off: all new mail arrives in the inbox. What was filtered before stays in Filtered."
        );
        assert_eq!(
            page.today, None,
            "the count shows only while filtering is on (C10)"
        );
        assert_eq!(
            hints::line(&page.keys),
            "R restore + never filter sender",
            "restore still works on what was filtered; there is nothing to sweep with"
        );
    }

    #[test]
    fn the_never_list_names_each_sender_and_says_which_pin_nobody() {
        let page = page(
            &focus("[focus.filter]\nnever = [\"ada@example.org\", \"@example.net\", \"grace\"]\n"),
            None,
            Keymap::defaults(),
        );
        let seen: Vec<(&str, bool)> = page
            .never
            .iter()
            .map(|entry| (entry.says.as_str(), entry.acts))
            .collect();
        assert_eq!(
            seen,
            [
                ("ada@example.org", true),
                ("everyone at example.net", true),
                (
                    "grace \u{b7} neither an address nor a whole domain, so it pins nobody",
                    false
                ),
            ]
        );
    }

    #[test]
    fn each_listed_entry_carries_the_write_that_takes_it_back() {
        let page = page(
            &focus(
                "[focus.filter]\nnever = [\"ada@example.org\"]\nstop_markers = [\
                 { sender = \"news@ledger.example\", kind = \"question\" }]\n",
            ),
            None,
            Keymap::defaults(),
        );
        assert_eq!(
            page.never[0].undo,
            Undo::Never("ada@example.org".to_owned())
        );
        assert_eq!(page.never[0].undo.label(), "Filter again");
        assert_eq!(
            page.stopped[0].undo,
            Undo::Marker {
                sender: "news@ledger.example".to_owned(),
                kind: "question".to_owned()
            }
        );
        assert_eq!(page.stopped[0].undo.label(), "Turn back on");
    }

    #[test]
    fn an_empty_never_list_teaches_the_key_that_fills_it() {
        let page = page(&focus(""), None, Keymap::defaults());
        assert!(page.never.is_empty());
        assert_eq!(
            page.never_empty,
            "Nobody yet. R in Filtered restores a message and lists its sender here."
        );
        assert!(GUARDS.contains("anyone you have written to"));
        assert!(GUARDS.contains("your own domain"));
        assert!(GUARDS.contains("a thread you took part in"));
    }

    #[test]
    fn the_keys_follow_a_rebind_and_an_unbound_one_is_left_out() {
        let config = Config::from_toml_str("[keys]\nsweep_inbox = \"S\"\n").expect("a config");
        let keymap = Keymap::resolve(&config.keys);
        let page = page(&config.focus, None, &keymap);
        assert_eq!(
            hints::line(&page.keys),
            "R restore + never filter sender \u{b7} S sweep the inbox"
        );

        let unbound = super::page(&config.focus, None, &Keymap::default());
        assert!(unbound.keys.is_empty(), "{:?}", unbound.keys);
        assert_eq!(
            unbound.never_empty,
            "Nobody yet. Restoring a message in Filtered lists its sender here."
        );
    }

    #[test]
    fn stopped_markers_read_as_what_is_no_longer_marked() {
        let page = page(
            &focus(
                "[focus.filter]\nstop_markers = [\
                 { sender = \"news@ledger.example\", kind = \"question\" }, \
                 { sender = \"@example.net\", kind = \"todo\" }, \
                 { sender = \"news@ledger.example\", kind = \"invite\" }]\n",
            ),
            None,
            Keymap::defaults(),
        );
        let seen: Vec<(&str, bool)> = page
            .stopped
            .iter()
            .map(|entry| (entry.says.as_str(), entry.acts))
            .collect();
        assert_eq!(
            seen,
            [
                ("Questions in mail from news@ledger.example", true),
                ("To-dos in mail from everyone at example.net", true),
                (
                    "news@ledger.example \u{b7} invite \u{b7} not a sender and a kind of \
                     marker, so it turns nothing off",
                    false
                ),
            ]
        );
        assert!(STOPPED_EMPTY.starts_with("None."));
    }
}
