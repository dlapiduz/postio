//! What Settings' Filtering page says (spec 007 US9, FR-110 to FR-119;
//! screens.md, "Settings"): whether Focus files spam and automated updates
//! away, what that does, that nothing is lost and the key that goes to it,
//! and what `[focus.filter]` holds, in a person's words.
//!
//! The page is drawn by each frontend; the words are here, from the file's
//! values and the keymap, where a test holds them. Nothing here invents a
//! rule the engine does not have: the kinds are Filtered's own tabs, the
//! guards are FR-111's four, and the lists are the file's.

use postio_config::{FocusConfig, NeverEntry, never_entry};
use postio_core::{CommandId, Keymap};

use crate::filtered;
use crate::hints::{self, Hint};

/// The switch's label: what turning it on does.
pub const SWITCH: &str = "Filter spam and automated updates";

/// The heading over today's count and the way to Filtered.
pub const FILTERED: &str = "Filtered";

/// The button that goes to Filtered, with `g f` in it.
pub const OPEN: &str = "Open Filtered";

/// The heading over the guards and the senders never filtered.
pub const NEVER: &str = "Never filtered";

/// The heading over the marker kinds stopped for a sender.
pub const STOPPED: &str = "Markers turned off";

/// What is never deleted, whether filtering is on or off (FR-117).
pub const KEPT: &str =
    "Nothing is deleted: filtered mail is archived, and Filtered lists all of it.";

/// FR-111's guards, which win over every rule, as one sentence.
pub const GUARDS: &str = "Whatever its headers say, mail is never filtered from anyone you have \
written to, from your own domain, in a conversation you took part in, or from a sender listed \
here.";

/// What the stopped markers list says with nothing in it (FR-108).
pub const STOPPED_EMPTY: &str =
    "None. Dismissing one kind of marker from a sender three times turns it off here.";

/// Everything the Filtering page says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Whether filtering is on: `[focus] filtering`.
    pub on: bool,
    /// What filtering does now, in a sentence.
    pub state: String,
    /// "186 filtered today", "Nothing filtered today", or nothing: while
    /// filtering is off, or before the count is known (C10).
    pub today: Option<String>,
    /// The keys Filtered answers that the page teaches: restore, and the
    /// sweep while filtering is on. Those a `[keys]` edit unbound are left
    /// out.
    pub keys: Vec<Hint>,
    /// `[focus.filter] never`, one line each.
    pub never: Vec<Listed>,
    /// What the never list says with nobody in it.
    pub never_empty: String,
    /// `[focus.filter] stop_markers`, one line each.
    pub stopped: Vec<Listed>,
}

/// One entry of a `[focus.filter]` list, as the page says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The line.
    pub says: String,
    /// Whether the entry does what it was written to do. One that does not
    /// says why, in the validator's terms.
    pub acts: bool,
}

/// The page for `focus`, with `filtered_today` messages filtered so far
/// today when the count is known, its keys read from `keymap`.
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
        never: focus
            .filter
            .never
            .iter()
            .map(|entry| never(entry))
            .collect(),
        never_empty,
        stopped: focus
            .filter
            .stop_markers
            .iter()
            .map(|stop| stopped(&stop.sender, &stop.kind, stop.is_well_formed()))
            .collect(),
    }
}

/// What filtering does now, in a sentence: what it files away and where,
/// or, off, that all new mail arrives in the inbox.
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

/// Today's count, while filtering is `on` and the count is known: "186
/// filtered today", or "Nothing filtered today". Nothing while filtering is
/// off: a count shows only while its feature is in use (C10).
pub fn today(on: bool, filtered_today: Option<u32>) -> Option<String> {
    filtered_today.filter(|_| on).map(|count| match count {
        0 => "Nothing filtered today".to_owned(),
        count => filtered::today(count),
    })
}

/// What Focus files away, from Filtered's tabs: "Spam, promotions,
/// notifications, receipts, shipping and social".
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

/// Who an address or a whole domain names: the address, or "everyone at"
/// the domain.
fn who(entry: &str) -> Option<String> {
    match never_entry(entry)? {
        NeverEntry::Address(address) => Some(address.to_owned()),
        NeverEntry::Domain(domain) => Some(format!("everyone at {domain}")),
    }
}

/// A `never` entry, as the page lists it.
fn never(entry: &str) -> Listed {
    match who(entry) {
        Some(says) => Listed { says, acts: true },
        None => Listed {
            says: format!(
                "{} \u{b7} neither an address nor a whole domain, so it pins nobody",
                entry.trim()
            ),
            acts: false,
        },
    }
}

/// A `stop_markers` entry, as the page lists it.
fn stopped(sender: &str, kind: &str, well_formed: bool) -> Listed {
    let kinds = match kind.trim().to_ascii_lowercase().as_str() {
        "question" => Some("Questions"),
        "todo" => Some("To-dos"),
        _ => None,
    };
    match (well_formed, kinds, who(sender)) {
        (true, Some(kinds), Some(who)) => Listed {
            says: format!("{kinds} in mail from {who}"),
            acts: true,
        },
        _ => Listed {
            says: format!(
                "{} \u{b7} {} \u{b7} not a sender and a kind of marker, so it turns nothing off",
                sender.trim(),
                kind.trim()
            ),
            acts: false,
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
        assert_eq!(
            page.never,
            [
                Listed {
                    says: "ada@example.org".to_owned(),
                    acts: true
                },
                Listed {
                    says: "everyone at example.net".to_owned(),
                    acts: true
                },
                Listed {
                    says: "grace \u{b7} neither an address nor a whole domain, so it pins nobody"
                        .to_owned(),
                    acts: false
                },
            ]
        );
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
        assert!(GUARDS.contains("a conversation you took part in"));
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
        assert_eq!(
            page.stopped,
            [
                Listed {
                    says: "Questions in mail from news@ledger.example".to_owned(),
                    acts: true
                },
                Listed {
                    says: "To-dos in mail from everyone at example.net".to_owned(),
                    acts: true
                },
                Listed {
                    says: "news@ledger.example \u{b7} invite \u{b7} not a sender and a kind of \
                           marker, so it turns nothing off"
                        .to_owned(),
                    acts: false
                },
            ]
        );
        assert!(STOPPED_EMPTY.starts_with("None."));
    }
}
