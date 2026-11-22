//! Focus's command bar without a toolkit: routing, blend rows, search hits, what running a row asks
//! for.

use postio_core::{ActionId, CommandId, Context, Keymap};
use postio_model::MailboxId;
use postio_model::ids::MessageId;
use postio_search::{ResultOrder, SearchHit};

use crate::finder::{self, Blend, Destination, Place, PlaceKind};

pub const FOLDER_ROWS: u32 = 30;

pub const HITS: usize = 30;

pub const BLEND_ROWS: usize = 5;

pub const SAVED: [CommandId; 4] = [
    CommandId::SavedSearch1,
    CommandId::SavedSearch2,
    CommandId::SavedSearch3,
    CommandId::SavedSearch4,
];

/// Account verbs; also reached from the list's bar, acting on the account row Settings has focused.
pub const ACCOUNT_VERBS: [CommandId; 5] = [
    CommandId::ToggleAccountEnabled,
    CommandId::RemoveAccount,
    CommandId::RebuildAccountIndex,
    CommandId::SetDefaultAccount,
    CommandId::MapMailboxRole,
];

pub const SEARCH_DETAIL: &str = "subject, body, attachments";

pub const CORRESPONDENTS: char = '@';

pub const CORRESPONDENT_ROWS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route<'a> {
    Folder(&'a str),
    Plain,
    Correspondent(&'a str),
    Blend,
}

pub fn route(typed: &str) -> Route<'_> {
    if let Some(name) = typed.strip_prefix("in:").filter(|name| !name.contains(' ')) {
        return Route::Folder(name);
    }
    if let Some(name) = typed.strip_prefix(CORRESPONDENTS) {
        return Route::Correspondent(name);
    }
    if typed.is_empty() || typed.starts_with(finder::COMMANDS_ONLY) {
        Route::Plain
    } else {
        Route::Blend
    }
}

/// A plain word makes no chip; it gets commands, places and one search row.
pub fn chips(parsed: &postio_search::ParsedQuery) -> Option<Vec<String>> {
    if parsed.filters().next().is_some() || parsed.partials().next().is_some() {
        Some(
            parsed
                .tokens()
                .iter()
                .map(|token| token.raw.clone())
                .collect(),
        )
    } else {
        None
    }
}

pub fn echo(typed: &str) -> String {
    format!("You typed \u{201c}{typed}\u{201d}")
}

/// The palette lists a context's own commands; these are `Accounts`'.
pub fn add_account_verbs(
    found: &mut Vec<crate::palette::Entry>,
    keymap: &Keymap,
    state: postio_core::Availability,
    query: &str,
) {
    let extra: Vec<_> = crate::palette::entries(keymap, Context::Accounts, state, query)
        .into_iter()
        .filter(|entry| {
            matches!(entry.id, ActionId::Builtin(id) if ACCOUNT_VERBS.contains(&id))
                && !found.iter().any(|have| have.id == entry.id)
        })
        .collect();
    found.extend(extra);
    found.sort_by_key(|entry| std::cmp::Reverse(entry.score));
    found.truncate(crate::palette::MAX_ROWS);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarAction {
    Open {
        message: MessageId,
        subject: String,
    },
    Command(CommandId),
    Go {
        destination: Destination,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Heading,
    Message {
        message: MessageId,
        subject: String,
    },
    Command(ActionId),
    Place(Destination, String),
    Search,
    /// The word typed found nothing and was answered with another (ADR 0037); runs it quoted,
    /// exactly.
    Instead(String),
    Order,
    Correspondent(String),
    Hint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Run {
    Action(BarAction),
    Search,
    SearchFor(String),
    ToggleOrder,
    Nothing,
}

impl Row {
    pub fn run(self) -> Run {
        match self {
            Row::Heading | Row::Hint => Run::Nothing,
            Row::Search => Run::Search,
            Row::Instead(typed) => Run::SearchFor(format!("\"{typed}\"")),
            Row::Order => Run::ToggleOrder,
            Row::Correspondent(query) => Run::SearchFor(query),
            Row::Message { message, subject } => Run::Action(BarAction::Open { message, subject }),
            Row::Command(ActionId::Builtin(command)) => Run::Action(BarAction::Command(command)),
            Row::Command(ActionId::Ext(_)) => Run::Nothing,
            Row::Place(destination, name) => Run::Action(BarAction::Go { destination, name }),
        }
    }

    pub fn is_selectable(&self) -> bool {
        !matches!(self, Row::Heading | Row::Hint)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Heading(String),
    Row {
        row: Row,
        title: String,
        detail: Option<String>,
        key: Option<String>,
    },
}

pub fn blend_lines(blend: &Blend<'_>) -> Vec<Line> {
    let mut lines = Vec::new();
    if blend.search_first() {
        lines.extend(search_line(blend));
    }
    if !blend.commands.is_empty() {
        lines.push(Line::Heading("Commands".to_owned()));
        for entry in blend.commands.iter().take(BLEND_ROWS) {
            lines.push(Line::Row {
                row: Row::Command(entry.id),
                title: entry.title.to_owned(),
                detail: None,
                key: entry.binding.clone(),
            });
        }
    }
    if !blend.places.is_empty() {
        lines.push(Line::Heading("Go to".to_owned()));
        for hit in blend.places.iter().take(BLEND_ROWS) {
            lines.push(Line::Row {
                row: Row::Place(hit.place.destination.clone(), hit.place.name.clone()),
                title: format!("in:{}", hit.place.name),
                detail: hit.place.count.map(|count| format!("{count} threads")),
                key: hit.binding.clone(),
            });
        }
    }
    if !blend.search_first() {
        lines.extend(search_line(blend));
    }
    lines
}

fn search_line(blend: &Blend<'_>) -> Option<Line> {
    blend.search.as_ref().map(|query| Line::Row {
        row: Row::Search,
        title: search_title(query, None),
        detail: Some(SEARCH_DETAIL.to_owned()),
        key: None,
    })
}

pub fn empty_lines() -> Vec<Line> {
    let mode = |marker: &str| {
        finder::MODES
            .iter()
            .find(|mode| mode.marker == marker)
            .map_or("", |mode| mode.purpose)
    };
    vec![
        Line::Row {
            row: Row::Hint,
            title: "Search mail".to_owned(),
            detail: Some("just start typing".to_owned()),
            key: None,
        },
        Line::Row {
            row: Row::Hint,
            title: mode(">").to_owned(),
            detail: Some("start with".to_owned()),
            key: Some(">".to_owned()),
        },
        Line::Row {
            row: Row::Hint,
            title: mode("@").to_owned(),
            detail: Some("start with".to_owned()),
            key: Some("@".to_owned()),
        },
    ]
}

pub fn correspondent_lines(hits: &[finder::ContactHit]) -> Vec<Line> {
    if hits.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![Line::Heading("Correspondents".to_owned())];
    for hit in hits.iter().take(CORRESPONDENT_ROWS) {
        lines.push(Line::Row {
            row: Row::Correspondent(finder::contact_query(hit)),
            title: hit.name.clone(),
            detail: Some(hit.address.clone()),
            key: None,
        });
    }
    lines
}

pub fn folder_for(places: &[Place], name: &str) -> Option<(String, MailboxId)> {
    let wanted = name.to_lowercase();
    places
        .iter()
        .filter(|place| matches!(place.kind, PlaceKind::Mailbox | PlaceKind::Folder))
        .find(|place| place.name.to_lowercase().starts_with(&wanted))
        .and_then(|place| match place.destination {
            Destination::Mailbox(mailbox) => Some((place.name.clone(), mailbox)),
            _ => None,
        })
}

pub fn folder_heading(name: &str, count: u32) -> String {
    format!("{name} \u{b7} folder \u{b7} {count} threads \u{b7} newest first")
}

pub fn conversations(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut seen = Vec::new();
    let mut rows = Vec::new();
    for hit in hits {
        let conversation = hit
            .thread_id
            .map(|thread| thread.get())
            .unwrap_or(-hit.message_id.get());
        if seen.contains(&conversation) {
            continue;
        }
        seen.push(conversation);
        rows.push(hit);
        if rows.len() == HITS {
            break;
        }
    }
    rows
}

/// With `instead` set, the results are for a correction (ADR 0037) and the row names it, so no two
/// rows name one query.
pub fn search_title(query: &str, instead: Option<&str>) -> String {
    let words = instead.unwrap_or(query);
    if words.contains('"') {
        format!("Search mail for {words}")
    } else {
        format!("Search mail for \u{201c}{words}\u{201d}")
    }
}

pub fn results_heading(count: usize) -> String {
    format!(
        "Threads \u{b7} {count} match{}",
        if count == 1 { "" } else { "es" }
    )
}

pub fn showing_results_for(term: &str) -> String {
    format!("Showing results for {term}")
}

pub fn search_instead(typed: &str) -> (String, &'static str) {
    (
        format!("Search instead for \u{201c}{typed}\u{201d}"),
        "exactly as typed",
    )
}

pub fn order_words(order: ResultOrder) -> (String, String) {
    let (now, other) = match order {
        ResultOrder::Relevance => ("relevance", "date"),
        ResultOrder::Newest => ("date", "relevance"),
    };
    (format!("Sorted by {now}"), format!("switch to {other}"))
}

/// Held mail shows where it waits (`held`: rule and delivered flag) in place of its folder.
pub fn result_place(
    hit: &SearchHit,
    held: &[(MessageId, String, bool)],
    folders: &[(MailboxId, String)],
) -> Option<String> {
    match held
        .iter()
        .find(|(message, _, _)| *message == hit.message_id)
    {
        Some((_, rule, delivered)) => Some(crate::digest::held_place(rule, *delivered)),
        None => folders
            .iter()
            .find(|(id, _)| *id == hit.mailbox_id)
            .map(|(_, name)| format!("in:{name}")),
    }
}

pub fn said_of(from: &postio_model::EmailAddress) -> String {
    from.name.clone().unwrap_or_else(|| from.address.clone())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use postio_model::ids::ThreadId;

    use super::*;

    fn hit(message: i64, thread: Option<i64>) -> SearchHit {
        SearchHit {
            message_id: MessageId::new(message),
            thread_id: thread.map(ThreadId::new),
            mailbox_id: MailboxId::new(1),
            subject: None,
            from: None,
            received_at: Utc::now(),
            preview: None,
            snippet: String::new(),
            score: 0.0,
        }
    }

    fn place(name: &str, kind: PlaceKind, id: i64) -> Place {
        Place {
            kind,
            name: name.to_owned(),
            count: None,
            go: None,
            destination: Destination::Mailbox(MailboxId::new(id)),
        }
    }

    fn first_row(lines: &[Line]) -> &Row {
        lines
            .iter()
            .find_map(|line| match line {
                Line::Row { row, .. } => Some(row),
                Line::Heading(_) => None,
            })
            .expect("a row")
    }

    #[test]
    fn the_first_row_is_the_search_unless_a_command_is_named() {
        let keymap = Keymap::resolve(&postio_config::KeyBindings::default());
        let state = postio_core::Availability::open(postio_core::Scope::Account(
            postio_model::AccountId::new(1),
        ));
        let lines =
            |typed: &str| blend_lines(&finder::blend(typed, &[], &keymap, Context::List, state));
        assert_eq!(first_row(&lines("tide")), &Row::Search);
        assert!(matches!(first_row(&lines("arch")), Row::Command(_)));
    }

    #[test]
    fn at_offers_correspondents_and_a_pick_searches_their_mail() {
        assert_eq!(route("@"), Route::Correspondent(""));
        assert_eq!(route("@ada"), Route::Correspondent("ada"));
        let contacts = [postio_model::Contact::new(
            postio_model::ContactAddress::new(
                postio_model::AddressId::new(1),
                postio_model::EmailAddress::new(Some("Ada Moreno"), "ada@example.com"),
            ),
        )];
        let lines = correspondent_lines(&finder::contacts(&contacts, ""));
        let Some(Line::Row {
            row, title, detail, ..
        }) = lines.get(1)
        else {
            panic!("no correspondent row: {lines:?}");
        };
        assert_eq!(title, "Ada Moreno");
        assert_eq!(detail.as_deref(), Some("ada@example.com"));
        assert_eq!(
            row.clone().run(),
            Run::SearchFor("with:ada@example.com".to_owned())
        );
        assert!(correspondent_lines(&[]).is_empty());
    }

    #[test]
    fn typing_routes_by_its_prefix() {
        assert_eq!(route("in:rec"), Route::Folder("rec"));
        assert_eq!(route("in:two words"), Route::Blend);
        assert_eq!(route(""), Route::Plain);
        assert_eq!(route(">arch"), Route::Plain);
        assert_eq!(route("invoice"), Route::Blend);
    }

    #[test]
    fn chips_only_when_the_words_are_a_search() {
        let parse = |text: &str| {
            postio_search::natural::lower(
                text,
                chrono::NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
                &|_| None,
            )
        };
        assert_eq!(chips(&parse("invoice")), None);
        assert_eq!(
            chips(&parse("from:ada invoice")),
            Some(vec!["from:ada".to_owned(), "invoice".to_owned()])
        );
    }

    #[test]
    fn hits_are_one_per_conversation_and_stop_at_the_limit() {
        let hits = vec![hit(1, Some(7)), hit(2, Some(7)), hit(3, None), hit(4, None)];
        let ids: Vec<i64> = conversations(hits)
            .iter()
            .map(|hit| hit.message_id.get())
            .collect();
        assert_eq!(ids, vec![1, 3, 4]);
        let many: Vec<SearchHit> = (1..=100).map(|id| hit(id, None)).collect();
        assert_eq!(conversations(many).len(), HITS);
    }

    #[test]
    fn in_completes_the_first_folder_by_prefix() {
        let places = vec![
            place("Archive", PlaceKind::Mailbox, 1),
            place("Receipts", PlaceKind::Folder, 2),
            place("Receipts-old", PlaceKind::Folder, 3),
            place("Travel", PlaceKind::Label, 4),
        ];
        assert_eq!(
            folder_for(&places, "REC"),
            Some(("Receipts".to_owned(), MailboxId::new(2)))
        );
        assert_eq!(folder_for(&places, "tra"), None);
    }

    #[test]
    fn the_empty_bar_says_what_typing_does_and_runs_nothing() {
        let lines = empty_lines();
        let said: Vec<(&str, Option<&str>)> = lines
            .iter()
            .map(|line| match line {
                Line::Row {
                    title, key, row, ..
                } => {
                    assert_eq!(row.clone().run(), Run::Nothing);
                    assert!(!row.is_selectable());
                    (title.as_str(), key.as_deref())
                }
                Line::Heading(_) => panic!("hints are rows"),
            })
            .collect();
        assert_eq!(
            said,
            [
                ("Search mail", None),
                ("Run a command", Some(">")),
                ("Find a correspondent", Some("@")),
            ]
        );
    }

    #[test]
    fn rows_say_what_they_run() {
        assert_eq!(Row::Heading.run(), Run::Nothing);
        assert_eq!(
            Row::Instead("ada".into()).run(),
            Run::SearchFor("\"ada\"".into())
        );
        assert_eq!(
            Row::Command(ActionId::Builtin(CommandId::Archive)).run(),
            Run::Action(BarAction::Command(CommandId::Archive))
        );
    }

    #[test]
    fn the_search_row_names_the_query_the_results_are_for() {
        assert_eq!(
            search_title("harbour", None),
            "Search mail for \u{201c}harbour\u{201d}"
        );
        let shown = search_title("harbourO", Some("harbour"));
        assert_eq!(shown, "Search mail for \u{201c}harbour\u{201d}");
        assert!(!shown.contains("harbourO"));
    }

    #[test]
    fn a_quoted_phrase_is_not_quoted_twice() {
        assert_eq!(
            search_title("\"factura telefonica\"", None),
            "Search mail for \"factura telefonica\""
        );
    }

    #[test]
    fn the_words_the_bar_says() {
        assert_eq!(results_heading(1), "Threads \u{b7} 1 match");
        assert_eq!(results_heading(3), "Threads \u{b7} 3 matches");
        assert_eq!(
            order_words(ResultOrder::Newest),
            (
                "Sorted by date".to_owned(),
                "switch to relevance".to_owned()
            )
        );
        assert_eq!(
            folder_heading("Receipts", 4),
            "Receipts \u{b7} folder \u{b7} 4 threads \u{b7} newest first"
        );
    }

    #[test]
    fn held_mail_says_where_it_waits_rather_than_its_folder() {
        let folders = vec![(MailboxId::new(1), "Archive".to_owned())];
        let held = vec![(MessageId::new(5), "News".to_owned(), false)];
        assert_eq!(
            result_place(&hit(5, None), &held, &folders).as_deref(),
            Some("held \u{b7} News")
        );
        assert_eq!(
            result_place(&hit(6, None), &held, &folders).as_deref(),
            Some("in:Archive")
        );
    }
}
