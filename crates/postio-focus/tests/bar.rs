//! The command bar, search, go-to and the places, from the controller's side
//! (specs/009-focus-macos T081; research R2, slice 8).
//!
//! What GTK's window and bar did -- `go_to`, `go_to_inbox`, `go_to_role`,
//! `go_to_view`, the saved searches, `bar_action`, reopen-after-hit, the chip
//! editor and the places reads -- asserted here so both frontends get it: `/`
//! and `mod+k` open one bar two ways (C24), a command chosen in it acts on
//! what the cursor was on when it opened, a place chosen in it is opened as
//! the list with the cursor on its first row, the go-to keys go by role, and
//! the places are read in one request however many accounts there are.

use chrono::Utc;
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_focus::{
    BarLine, BarLineKind, BarMode, BarView, Effect, FocusController, Found, FoundRow, Host, Input,
    Intent, Opened, PageAnswer, PlacesRead, Policy, Reply, Request, RowFacts, Rows, SurfaceKind,
    Ticket, ToastKind,
};
use postio_model::{FocusScope, ListScope, MailboxId, MailboxRole, MessageId, ThreadId};
use postio_search::SearchHit;
use postio_ui::keymap::KeyContext;
use postio_ui::places::Entry;

/// A list of conversations.
struct List {
    rows: Vec<RowFacts>,
}

impl List {
    fn of(len: i64) -> Self {
        List {
            rows: (0..len)
                .map(|at| RowFacts {
                    id: MessageId::new(100 + at),
                    digest: false,
                    threads: vec![ThreadId::new(500 + at)],
                    writes: false,
                })
                .collect(),
        }
    }
}

impl Rows for List {
    fn len(&self) -> u32 {
        self.rows.len() as u32
    }
    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.rows.get(position as usize).cloned()
    }
    fn position_of(&self, message: MessageId) -> Option<u32> {
        self.rows
            .iter()
            .position(|row| row.id == message)
            .map(|at| at as u32)
    }
}

const RECEIPTS: MailboxId = MailboxId::new(7);

/// The Mac's controller as spec 009 built it: the bar's search half is the
/// blend GTK draws (`results_view` off), which every test above the
/// dropdown's section pins for GTK (spec 010 S2, D17).
fn mac() -> FocusController {
    let mut policy = Policy::for_platform(Platform::Apple);
    policy.caps.results_view = false;
    FocusController::new(policy)
}

fn shown(effects: &[Effect]) -> Vec<Intent> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Show(intent) => Some(intent.clone()),
            _ => None,
        })
        .collect()
}

fn asks(effects: &[Effect]) -> Vec<(Ticket, Request)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Ask(ticket, request) => Some((*ticket, request.clone())),
            _ => None,
        })
        .collect()
}

fn asked(effects: &[Effect]) -> Vec<Request> {
    asks(effects)
        .into_iter()
        .map(|(_, request)| request)
        .collect()
}

/// The one ask among `effects`.
fn ask(effects: &[Effect]) -> (Ticket, Request) {
    let asks = asks(effects);
    assert_eq!(asks.len(), 1, "one ask in {effects:?}");
    asks[0].clone()
}

/// The bar's last view among `effects`.
fn view(effects: &[Effect]) -> BarView {
    shown(effects)
        .into_iter()
        .rev()
        .find_map(|intent| match intent {
            Intent::BarLines(view) => Some(view),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no bar lines in {effects:?}"))
}

/// The line titled `title`.
fn line<'a>(view: &'a BarView, title: &str) -> &'a BarLine {
    view.lines
        .iter()
        .find(|line| line.title == title)
        .unwrap_or_else(|| panic!("no line {title:?} in {:?}", view.lines))
}

fn run(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    focus.handle_on(Input::Command(command), rows)
}

fn typed(focus: &mut FocusController, text: &str, rows: &List) -> Vec<Effect> {
    focus.handle_on(
        Input::Typed {
            text: text.to_owned(),
        },
        rows,
    )
}

/// The places one account with an inbox, Sent, Archive and Receipts gives.
fn places() -> PlacesRead {
    let mailbox = |id: i64, name: &str, role: MailboxRole| {
        let mut mailbox = postio_model::Mailbox::new(postio_model::AccountId::new(1), name, None);
        mailbox.id = MailboxId::new(id);
        mailbox.role = role;
        mailbox
    };
    let mailboxes = [
        mailbox(1, "INBOX", MailboxRole::Inbox),
        mailbox(2, "Sent", MailboxRole::Sent),
        mailbox(3, "Archive", MailboxRole::Archive),
        mailbox(7, "Receipts", MailboxRole::Regular),
    ];
    let mut entries: Vec<Entry> = mailboxes
        .iter()
        .map(postio_ui::places::mailbox_entry)
        .collect();
    entries.extend(postio_ui::places::view_entries());
    PlacesRead {
        entries,
        places: mailboxes
            .iter()
            .map(postio_ui::places::mailbox_place)
            .collect(),
        folders: mailboxes
            .iter()
            .map(|mailbox| (mailbox.id, postio_ui::places::place_name(mailbox)))
            .collect(),
        owners: Vec::new(),
        contacts: Vec::new(),
    }
}

/// `focus` with the bar open by `command` and the places read.
fn bar_open(focus: &mut FocusController, command: CommandId, rows: &List) -> Vec<Effect> {
    let mut effects = run(focus, command, rows);
    let (ticket, request) = asks(&effects)
        .into_iter()
        .find(|(_, request)| *request == Request::Places)
        .expect("opening the bar reads the places");
    assert_eq!(request, Request::Places);
    effects.extend(focus.handle_on(Input::Reply(ticket, Reply::Places(Ok(places()))), rows));
    effects
}

fn hit(message: i64, subject: &str) -> SearchHit {
    SearchHit {
        message_id: MessageId::new(message),
        thread_id: Some(ThreadId::new(message + 400)),
        mailbox_id: MailboxId::new(1),
        subject: Some(subject.to_owned()),
        from: Some(postio_model::EmailAddress::new(
            Some("Ada Moreno"),
            "ada@example.com",
        )),
        received_at: Utc::now(),
        preview: None,
        snippet: String::new(),
        score: 0.0,
    }
}

/// Answer the one search among `effects` with `hits`.
fn found(focus: &mut FocusController, effects: &[Effect], hits: Vec<SearchHit>) -> Vec<Effect> {
    let (ticket, request) = asks(effects)
        .into_iter()
        .rev()
        .find(|(_, request)| matches!(request, Request::Search { .. }))
        .unwrap_or_else(|| panic!("a search in {effects:?}"));
    let Request::Search { stamp, .. } = request else {
        unreachable!()
    };
    focus.handle(Input::Reply(
        ticket,
        Reply::Search {
            stamp,
            answer: Ok(Found {
                hits,
                instead: None,
                held: Vec::new(),
            }),
        },
    ))
}

// -- Opening the bar (C24) ------------------------------------------------

#[test]
fn slash_opens_the_bar_for_search_and_reads_the_places_once() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = run(&mut focus, CommandId::Search, &rows);
    assert_eq!(
        shown(&effects)[0],
        Intent::OpenBar {
            mode: BarMode::Search,
            text: String::new(),
            select: None,
        }
    );
    assert_eq!(
        asked(&effects),
        vec![Request::Places],
        "one read, not one per account"
    );
    let lines = view(&effects);
    assert!(
        lines
            .lines
            .iter()
            .any(|line| line.kind == BarLineKind::Hint),
        "the empty bar says what typing does: {lines:?}"
    );
    assert!(
        lines
            .lines
            .iter()
            .all(|line| !line.selectable || line.kind != BarLineKind::Hint)
    );
    assert_eq!(
        focus.key_context(),
        KeyContext::Search,
        "the bar has the keys"
    );
}

#[test]
fn mod_k_opens_the_bar_on_commands_with_the_prefix_typed() {
    let rows = List::of(3);
    let mut focus = mac();
    let effects = run(&mut focus, CommandId::CommandPalette, &rows);
    assert_eq!(
        shown(&effects)[0],
        Intent::OpenBar {
            mode: BarMode::Commands,
            text: ">".to_owned(),
            select: None,
        }
    );
    let lines = view(&effects);
    assert!(
        lines
            .lines
            .iter()
            .all(|line| matches!(line.kind, BarLineKind::Heading | BarLineKind::Command)),
        "commands only, and no search row: {lines:?}"
    );
    assert!(
        lines
            .lines
            .iter()
            .any(|line| line.kind == BarLineKind::Command)
    );
}

#[test]
fn typing_offers_the_one_search_row_and_asks_the_index() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    let effects = typed(&mut focus, "tide", &rows);
    let lines = view(&effects);
    let first = lines
        .lines
        .iter()
        .find(|line| line.selectable)
        .expect("a row to run");
    assert_eq!(first.kind, BarLineKind::Search);
    assert_eq!(first.title, "Search mail for \u{201c}tide\u{201d}");
    assert_eq!(
        lines.echo.as_deref(),
        Some("You typed \u{201c}tide\u{201d}")
    );
    assert!(matches!(
        asked(&effects).as_slice(),
        [Request::Search { .. }]
    ));
    // The same words again, as a frontend echoes its own field: nothing.
    assert!(typed(&mut focus, "tide", &rows).is_empty());
}

#[test]
fn a_single_letter_asks_the_index_nothing_yet() {
    // The first keystroke of "southwest" is "s": in most of a mailbox, and
    // over a large store a search for it ran for minutes and held every
    // later keystroke's search behind it.
    let rows = List::of(3);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    let effects = typed(&mut focus, "s", &rows);
    assert!(
        !asked(&effects)
            .iter()
            .any(|request| matches!(request, Request::Search { .. })),
        "{:?}",
        asked(&effects)
    );
    let effects = typed(&mut focus, "so", &rows);
    assert!(
        asked(&effects)
            .iter()
            .any(|request| matches!(request, Request::Search { .. })),
        "two letters are a word"
    );
}

#[test]
fn what_the_bar_searches_for_forgives_a_near_word() {
    // A person searching: "tickt" should still find the ticket. A rule's
    // query is never forgiving (ADR 0037, as amended), so it is the bar
    // that asks for it.
    let rows = List::of(3);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    let effects = typed(&mut focus, "tickt", &rows);
    let forgiving = asked(&effects)
        .iter()
        .any(|request| matches!(request, Request::Search { query, .. } if query.is_forgiving()));
    assert!(forgiving, "{:?}", asked(&effects));
}

// -- What a row runs -------------------------------------------------------

#[test]
fn a_command_chosen_in_the_bar_acts_on_the_aim_captured_when_it_opened() {
    let rows = List::of(3);
    // What `a` on the second row sends, from the list.
    let mut twin = mac();
    let _ = twin.landed(&rows, true);
    let _ = run(&mut twin, CommandId::NextMessage, &rows);
    let from_the_list = asked(&run(&mut twin, CommandId::Archive, &rows));

    let mut focus = mac();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = bar_open(&mut focus, CommandId::CommandPalette, &rows);
    // The pointer moved the cursor behind the bar.
    let _ = focus.handle_on(Input::Point(2), &rows);
    let lines = view(&typed(&mut focus, ">archive", &rows));
    let archive = lines
        .lines
        .iter()
        .find(|line| line.command == Some(CommandId::Archive))
        .unwrap_or_else(|| panic!("no Archive row in {lines:?}"))
        .clone();
    assert!(archive.selectable);
    let effects = focus.handle_on(Input::BarRun(archive.token), &rows);
    let intents = shown(&effects);
    assert_eq!(intents[0], Intent::CloseSurface(SurfaceKind::Bar));
    assert!(
        intents.contains(&Intent::Cursor {
            position: 1,
            to_top: false
        }),
        "the cursor goes back to what the bar opened over: {intents:?}"
    );
    assert_eq!(
        asked(&effects),
        from_the_list,
        "the bar's Archive is the list's, on the row it opened over"
    );
    assert_eq!(focus.key_context(), KeyContext::List, "the bar is gone");
}

#[test]
fn a_command_the_controller_does_not_answer_is_handed_back_to_run() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::CommandPalette, &rows);
    let lines = view(&typed(&mut focus, ">settings", &rows));
    let settings = lines
        .lines
        .iter()
        .find(|line| line.command == Some(CommandId::Settings))
        .unwrap_or_else(|| panic!("no Settings row in {lines:?}"))
        .clone();
    let effects = focus.handle_on(Input::BarRun(settings.token), &rows);
    // The keyboard goes home first: a command the frontend runs may open
    // nothing of its own (Refresh), and one that does takes it from there.
    assert_eq!(
        shown(&effects),
        vec![
            Intent::CloseSurface(SurfaceKind::Bar),
            Intent::KeyboardHome,
            Intent::Run(CommandId::Settings)
        ]
    );
}

#[test]
fn compose_from_the_bar_opens_the_composer_itself() {
    // Compose was handed back to `Run` until the composer was the
    // controller's (slice 7).
    let rows = List::of(1);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::CommandPalette, &rows);
    let lines = view(&typed(&mut focus, ">compose", &rows));
    let compose = lines
        .lines
        .iter()
        .find(|line| line.command == Some(CommandId::Compose))
        .unwrap_or_else(|| panic!("no Compose row in {lines:?}"))
        .clone();
    let effects = focus.handle_on(Input::BarRun(compose.token), &rows);
    assert_eq!(
        shown(&effects),
        vec![
            Intent::CloseSurface(SurfaceKind::Bar),
            Intent::KeyboardHome,
            Intent::Composer {
                kind: postio_focus::ComposerKind::New,
                message: None,
            }
        ]
    );
    assert_eq!(focus.key_context(), KeyContext::Composer);
}

#[test]
fn a_token_from_lines_since_redrawn_runs_nothing() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::CommandPalette, &rows);
    let old = view(&typed(&mut focus, ">archive", &rows));
    let _ = typed(&mut focus, ">compose", &rows);
    let stale = old
        .lines
        .iter()
        .find(|line| line.command == Some(CommandId::Archive))
        .expect("an Archive row")
        .token;
    assert!(focus.handle_on(Input::BarRun(stale), &rows).is_empty());
}

// -- Places --------------------------------------------------------------

#[test]
fn in_receipts_opens_that_folder_as_the_list_with_the_cursor_on_its_first_row() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::LastMessage, &rows);
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    let lines = view(&typed(&mut focus, "Receipts", &rows));
    let receipts = line(&lines, "in:Receipts").clone();
    assert_eq!(receipts.kind, BarLineKind::Place);
    let effects = focus.handle_on(Input::BarRun(receipts.token), &rows);
    let intents = shown(&effects);
    assert_eq!(intents[0], Intent::CloseSurface(SurfaceKind::Bar));
    assert!(
        intents.contains(&Intent::Place {
            name: "Receipts".to_owned()
        }),
        "the strip names the place: {intents:?}"
    );
    let (ticket, request) = ask(&effects);
    assert_eq!(
        request,
        Request::OpenScope {
            scope: ListScope::Mailbox(RECEIPTS),
            splices: false
        }
    );
    // Counted, changed over, its first page landed: the cursor is on row 0.
    let effects = focus.handle(Input::Reply(
        ticket,
        Reply::Opened(Opened {
            scope: ListScope::Mailbox(RECEIPTS),
            accounts: vec![postio_model::AccountId::new(1)],
            folders: vec![(MailboxId::new(1), true), (RECEIPTS, false)],
            total: Ok(2),
            surfaced: None,
        }),
    ));
    assert_eq!(shown(&effects), vec![Intent::ReplaceSource { total: 2 }]);
    let (ticket, request) = ask(&focus.page_wanted(0, 1));
    let Request::Page {
        page,
        stamp,
        start,
        count,
        ..
    } = request
    else {
        panic!("a page, not {request:?}");
    };
    let _ = focus.handle(Input::Reply(
        ticket,
        Reply::Page {
            page,
            stamp,
            start,
            count,
            answer: Ok(PageAnswer::Threads {
                total: 2,
                rows: Vec::new(),
                labels: Vec::new(),
            }),
        },
    ));
    let receipts_rows = List::of(2);
    let opened = focus.take_opened();
    let effects = focus.landed(&receipts_rows, opened);
    assert!(
        shown(&effects).contains(&Intent::Cursor {
            position: 0,
            to_top: true
        }),
        "Receipts opened with the cursor on its first row"
    );
    assert_eq!(focus.cursor(), Some(0));
}

#[test]
fn in_rec_lists_the_folders_conversations_under_its_heading() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    let effects = typed(&mut focus, "in:Rec", &rows);
    let (ticket, request) = ask(&effects);
    let Request::Folder { mailbox, stamp } = request else {
        panic!("the folder's rows, not {request:?}");
    };
    assert_eq!(mailbox, RECEIPTS);
    let row = |id: i64, subject: &str| FoundRow {
        message: MessageId::new(id),
        from: Some("Ada Moreno".to_owned()),
        subject: subject.to_owned(),
        preview: None,
        at: Utc::now(),
    };
    let effects = focus.handle(Input::Reply(
        ticket,
        Reply::Folder {
            stamp,
            count: Ok(3),
            rows: Ok(vec![
                row(1, "Coffee beans"),
                row(2, "Bookshop order"),
                row(3, "Train ticket"),
            ]),
        },
    ));
    let lines = view(&effects);
    assert_eq!(
        lines.heading.as_deref(),
        Some("Receipts \u{b7} folder \u{b7} 3 conversations \u{b7} newest first")
    );
    let subjects: Vec<&str> = lines
        .lines
        .iter()
        .filter(|line| line.kind == BarLineKind::Message)
        .map(|line| line.title.as_str())
        .collect();
    assert_eq!(subjects, ["Coffee beans", "Bookshop order", "Train ticket"]);
}

#[test]
fn the_go_to_keys_go_to_their_places() {
    let rows = List::of(2);
    // `g i`: Focus's own inbox.
    let mut focus = mac();
    let effects = run(&mut focus, CommandId::GoToInbox, &rows);
    assert!(shown(&effects).contains(&Intent::Place {
        name: "Inbox".to_owned()
    }));
    assert_eq!(
        asked(&effects),
        vec![Request::OpenScope {
            scope: ListScope::Focus(FocusScope::Inbox),
            splices: true
        }]
    );
    // `g t`, `g s`, `g r`: a folder by its role, in the account Focus
    // writes from.
    for (command, role, name) in [
        (CommandId::GoToDrafts, MailboxRole::Drafts, "Drafts"),
        (CommandId::GoToSent, MailboxRole::Sent, "Sent"),
        (CommandId::GoToArchive, MailboxRole::Archive, "Archive"),
    ] {
        let mut focus = mac();
        let (ticket, request) = ask(&run(&mut focus, command, &rows));
        assert_eq!(request, Request::RoleFolder(role), "{command:?}");
        let effects = focus.handle(Input::Reply(
            ticket,
            Reply::RoleFolder(Some((MailboxId::new(40), name.to_owned()))),
        ));
        assert!(
            shown(&effects).contains(&Intent::Place {
                name: name.to_owned()
            }),
            "{command:?}: {effects:?}"
        );
        assert_eq!(
            asked(&effects),
            vec![Request::OpenScope {
                scope: ListScope::Mailbox(MailboxId::new(40)),
                splices: false
            }],
            "{command:?}"
        );
        // No such folder anywhere: nothing moves.
        let mut focus = mac();
        let (ticket, _) = ask(&run(&mut focus, command, &rows));
        assert!(
            focus
                .handle(Input::Reply(ticket, Reply::RoleFolder(None)))
                .is_empty()
        );
    }
    // `g z`: Snoozed, a view over every account's mail.
    let mut focus = mac();
    let effects = run(&mut focus, CommandId::GoToSnoozed, &rows);
    assert!(shown(&effects).contains(&Intent::Place {
        name: "Snoozed".to_owned()
    }));
    assert_eq!(
        asked(&effects),
        vec![Request::OpenScope {
            scope: ListScope::Focus(FocusScope::Snoozed),
            splices: false
        }]
    );
    // `g f`: Filtered, which is a view of its own.
    let mut focus = mac();
    assert_eq!(
        shown(&run(&mut focus, CommandId::GoToFiltered, &rows)),
        vec![Intent::ShowFiltered]
    );
    for command in [
        CommandId::GoToInbox,
        CommandId::GoToDrafts,
        CommandId::GoToSent,
        CommandId::GoToArchive,
        CommandId::GoToSnoozed,
        CommandId::GoToFlagged,
        CommandId::GoToFiltered,
        CommandId::GoToFolders,
        CommandId::Search,
        CommandId::CommandPalette,
        CommandId::SavedSearch1,
    ] {
        assert!(mac().answers(command), "{command:?} is the controller's");
    }
}

#[test]
fn going_somewhere_lets_the_selection_and_has_action_go() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::ToggleSelection, &rows);
    let _ = run(&mut focus, CommandId::ToggleHasAction, &rows);
    assert!(focus.has_action());
    let effects = run(&mut focus, CommandId::GoToSnoozed, &rows);
    assert!(!focus.has_action(), "Snoozed is not narrowed by `!`");
    assert!(shown(&effects).contains(&Intent::SingleHeading(None)));
    assert_eq!(focus.selection(), Default::default());
}

#[test]
fn the_places_are_one_request_and_a_place_opens_from_the_popover() {
    let rows = List::of(1);
    let mut focus = mac();
    let effects = run(&mut focus, CommandId::GoToFolders, &rows);
    assert_eq!(shown(&effects), vec![Intent::OpenPlaces]);
    let (ticket, request) = ask(&effects);
    assert_eq!(request, Request::Places, "one read, however many accounts");
    let effects = focus.handle(Input::Reply(ticket, Reply::Places(Ok(places()))));
    assert_eq!(shown(&effects), vec![Intent::PlacesChanged]);

    let names: Vec<String> = focus
        .places("")
        .into_iter()
        .map(|(_, entry)| entry.name)
        .collect();
    assert_eq!(
        names,
        ["Inbox", "Sent", "Snoozed", "Archive", "Flagged", "Receipts"]
    );
    let receipts = focus.places("rec");
    assert_eq!(receipts.len(), 1, "filtered as typed: {receipts:?}");
    let effects = focus.handle_on(Input::OpenPlace(receipts[0].0), &rows);
    assert!(shown(&effects).contains(&Intent::Place {
        name: "Receipts".to_owned()
    }));
    assert_eq!(
        asked(&effects),
        vec![Request::OpenScope {
            scope: ListScope::Mailbox(RECEIPTS),
            splices: false
        }]
    );
    // Snoozed is a view, and runs the command its key runs.
    let snoozed = focus.places("snoozed");
    let effects = focus.handle_on(Input::OpenPlace(snoozed[0].0), &rows);
    assert!(asked(&effects).contains(&Request::OpenScope {
        scope: ListScope::Focus(FocusScope::Snoozed),
        splices: false
    }));
}

#[test]
fn the_popover_lists_filtered_while_focus_files_mail() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = focus.handle(Input::Filtering(true));
    let counts = postio_client::protocol::FocusCounts {
        filtered_today: 4,
        ..Default::default()
    };
    let Effect::Ask(ticket, _) = focus.refresh_counts() else {
        panic!("counts are asked for");
    };
    let _ = focus.handle(Input::Reply(ticket, Reply::FocusCounts(Ok(counts))));
    let (ticket, _) = ask(&run(&mut focus, CommandId::GoToFolders, &rows));
    let _ = focus.handle(Input::Reply(ticket, Reply::Places(Ok(places()))));
    let filtered = focus.places("filtered");
    assert_eq!(filtered.len(), 1);
    let effects = focus.handle_on(Input::OpenPlace(filtered[0].0), &rows);
    assert_eq!(shown(&effects), vec![Intent::ShowFiltered]);
}

#[test]
fn a_bar_place_that_is_an_inbox_goes_to_focus_s_inbox() {
    let rows = List::of(1);
    let mut focus = mac();
    // The inbox is known as one from opening it.
    let (ticket, _) = ask(&focus.open(ListScope::Focus(FocusScope::Inbox)));
    let _ = focus.handle(Input::Reply(
        ticket,
        Reply::Opened(Opened {
            scope: ListScope::Focus(FocusScope::Inbox),
            accounts: vec![postio_model::AccountId::new(1)],
            folders: vec![(MailboxId::new(1), true)],
            total: Ok(1),
            surfaced: Some(Ok(Vec::new())),
        }),
    ));
    let (ticket, _) = ask(&run(&mut focus, CommandId::GoToFolders, &rows));
    let _ = focus.handle(Input::Reply(ticket, Reply::Places(Ok(places()))));
    let inbox = focus.places("inbox");
    let effects = focus.handle_on(Input::OpenPlace(inbox[0].0), &rows);
    assert_eq!(
        asked(&effects),
        vec![Request::OpenScope {
            scope: ListScope::Focus(FocusScope::Inbox),
            splices: true
        }]
    );
}

// -- Saved searches --------------------------------------------------------

#[test]
fn alt_numbers_open_the_saved_searches() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = focus.handle(Input::SavedSearches(vec![
        ("Budget".to_owned(), "subject:budget".to_owned()),
        ("Ada".to_owned(), "from:ada".to_owned()),
    ]));
    let effects = run(&mut focus, CommandId::SavedSearch2, &rows);
    assert_eq!(
        shown(&effects)[0],
        Intent::OpenBar {
            mode: BarMode::Search,
            text: "from:ada".to_owned(),
            select: None,
        }
    );
    assert!(
        asked(&effects)
            .iter()
            .any(|request| matches!(request, Request::Search { .. })),
        "and it runs: {effects:?}"
    );
    assert_eq!(view(&effects).saved, ["Budget", "Ada"], "the saved row");

    let mut focus = mac();
    let effects = run(&mut focus, CommandId::SavedSearch3, &rows);
    assert_eq!(
        shown(&effects),
        vec![Intent::Toast {
            text: postio_ui::focus_target::no_saved_search(2),
            kind: ToastKind::Notice,
        }]
    );
}

#[test]
fn mod_s_saves_the_bars_query() {
    let rows = List::of(1);
    let mut focus = mac();
    assert!(
        run(&mut focus, CommandId::SaveSearch, &rows).is_empty(),
        "nothing to save with the bar closed"
    );
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    let _ = typed(&mut focus, "from:ada invoice", &rows);
    let effects = run(&mut focus, CommandId::SaveSearch, &rows);
    assert_eq!(
        shown(&effects),
        vec![Intent::SaveSearch {
            query: "from:ada invoice".to_owned()
        }]
    );
    let effects = focus.handle(Input::SearchSaved(Ok(vec![(
        "from-ada-invoice".to_owned(),
        "from:ada invoice".to_owned(),
    )])));
    let intents = shown(&effects);
    assert!(intents.contains(&Intent::Toast {
        text: postio_ui::focus_target::search_saved("from:ada invoice"),
        kind: ToastKind::Notice,
    }));
    assert_eq!(view(&effects).saved, ["from-ada-invoice"]);
    let effects = focus.handle(Input::SearchSaved(Err("no file".to_owned())));
    assert_eq!(
        shown(&effects),
        vec![Intent::Toast {
            text: "no file".to_owned(),
            kind: ToastKind::Notice,
        }]
    );
}

// -- The chips -------------------------------------------------------------

#[test]
fn tab_steps_into_the_chips_and_back_to_words_returns_to_what_was_typed() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    let lines = view(&typed(&mut focus, "from:ada  invoice", &rows));
    assert_eq!(lines.chips, ["from:ada", "invoice"]);
    assert_eq!(lines.editing, None);

    let effects = focus.handle(Input::BarTab);
    assert_eq!(
        shown(&effects)[0],
        Intent::OpenBar {
            mode: BarMode::Search,
            text: "from:ada invoice".to_owned(),
            select: Some((0, 8)),
        },
        "the field holds the chips, the first selected"
    );
    let editing = view(&effects);
    assert_eq!(editing.editing, Some(0));
    assert!(
        editing
            .echo
            .as_deref()
            .is_some_and(|echo| echo.contains("editing from:")),
        "{editing:?}"
    );
    let effects = focus.handle(Input::BarTab);
    assert_eq!(
        shown(&effects)[0],
        Intent::OpenBar {
            mode: BarMode::Search,
            text: "from:ada invoice".to_owned(),
            select: Some((9, 16)),
        }
    );
    let effects = run(&mut focus, CommandId::BackToWords, &rows);
    assert_eq!(
        shown(&effects)[0],
        Intent::OpenBar {
            mode: BarMode::Search,
            text: "from:ada  invoice".to_owned(),
            select: None,
        },
        "the words as typed"
    );
    assert_eq!(view(&effects).editing, None);
    assert!(
        run(&mut focus, CommandId::BackToWords, &rows).is_empty(),
        "nothing to go back to now"
    );
    // Words that make no chip have none to step into.
    let _ = typed(&mut focus, "tide", &rows);
    assert!(focus.handle(Input::BarTab).is_empty());
}

// -- A hit, and back to the results ----------------------------------------

#[test]
fn a_hit_opens_walks_the_results_and_closing_it_reopens_the_bar_on_it() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    let effects = typed(&mut focus, "tide", &rows);
    let effects = found(
        &mut focus,
        &effects,
        vec![hit(1, "Tide tables"), hit(2, "Tide again")],
    );
    let lines = view(&effects);
    assert!(
        lines
            .lines
            .iter()
            .any(|line| line.title == "Conversations \u{b7} 2 matches")
    );
    let second = line(&lines, "Tide again").clone();
    assert_eq!(second.kind, BarLineKind::Message);
    assert_eq!(second.sender.as_deref(), Some("Ada Moreno"));

    let effects = focus.handle_on(Input::BarRun(second.token), &rows);
    assert_eq!(
        shown(&effects),
        vec![
            Intent::CloseSurface(SurfaceKind::Bar),
            Intent::OpenMessage {
                message: MessageId::new(2),
                index: 1,
                total: 2,
                host: Host::Own,
            }
        ]
    );
    assert_eq!(focus.key_context(), KeyContext::Reader);
    // `k` walks the results, not the list behind them.
    let effects = run(&mut focus, CommandId::PrevMessage, &rows);
    assert_eq!(
        shown(&effects),
        vec![Intent::OpenMessage {
            message: MessageId::new(1),
            index: 0,
            total: 2,
            host: Host::Own,
        }]
    );
    // Back closes it, and the bar comes back on the words, the highlight
    // on the hit last read.
    let effects = run(&mut focus, CommandId::Back, &rows);
    let intents = shown(&effects);
    assert!(intents.contains(&Intent::CloseSurface(SurfaceKind::Message)));
    assert!(
        intents.contains(&Intent::OpenBar {
            mode: BarMode::Search,
            text: "tide".to_owned(),
            select: None,
        }),
        "{intents:?}"
    );
    let effects = found(
        &mut focus,
        &effects,
        vec![hit(1, "Tide tables"), hit(2, "Tide again")],
    );
    let lines = view(&effects);
    assert_eq!(
        lines.highlight,
        Some(line(&lines, "Tide tables").token),
        "back where they left the results"
    );
}

#[test]
fn return_on_the_search_row_goes_to_the_first_hit() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    let effects = typed(&mut focus, "tide", &rows);
    let lines = view(&found(&mut focus, &effects, vec![hit(5, "Tide tables")]));
    let search = lines
        .lines
        .iter()
        .find(|line| line.kind == BarLineKind::Search)
        .expect("the search row")
        .token;
    let effects = focus.handle_on(Input::BarRun(search), &rows);
    let lines = view(&effects);
    assert_eq!(lines.highlight, Some(line(&lines, "Tide tables").token));
}

#[test]
fn an_answer_for_words_since_changed_is_not_drawn() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    let first = typed(&mut focus, "tid", &rows);
    let _ = typed(&mut focus, "tide", &rows);
    assert!(found(&mut focus, &first, vec![hit(5, "Tide tables")]).is_empty());
}

#[test]
fn closing_the_bar_forgets_it() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    let lines = view(&typed(&mut focus, "Receipts", &rows));
    let token = line(&lines, "in:Receipts").token;
    let effects = focus.handle_on(Input::SurfaceClosed(SurfaceKind::Bar), &rows);
    assert_eq!(shown(&effects), vec![Intent::KeyboardHome]);
    assert!(focus.handle_on(Input::BarRun(token), &rows).is_empty());
    assert!(
        focus
            .handle(Input::Typed {
                text: "x".to_owned()
            })
            .is_empty()
    );
}

#[test]
fn back_closes_the_bar() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    assert!(focus.answers(CommandId::Back));
    // Off the stack here, as a line run takes it off: the frontend's
    // `SurfaceClosed(Bar)` is then a repeat, never the thing that closes
    // it -- an echo that did could land after the bar had been reopened.
    assert_eq!(
        shown(&run(&mut focus, CommandId::Back, &rows)),
        vec![Intent::CloseSurface(SurfaceKind::Bar), Intent::KeyboardHome]
    );
    assert!(!focus.answers(CommandId::BackToWords), "the bar is gone");
    assert!(
        focus
            .handle(Input::SurfaceClosed(SurfaceKind::Bar))
            .is_empty(),
        "the frontend's report changes nothing"
    );
}

#[test]
fn the_bar_answers_its_own_keys_while_open() {
    let rows = List::of(1);
    let mut focus = mac();
    assert!(!focus.answers(CommandId::BackToWords), "no bar, no chips");
    assert!(!focus.answers(CommandId::ToggleResultOrder));
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    for command in [
        CommandId::BackToWords,
        CommandId::ToggleResultOrder,
        CommandId::SaveSearch,
        CommandId::SavedSearch1,
    ] {
        assert!(focus.answers(command), "{command:?}");
    }
    assert!(!focus.answers(CommandId::Archive), "a letter is typing");
}

#[test]
fn the_order_row_switches_the_order_and_asks_again() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    let effects = typed(&mut focus, "tide", &rows);
    let lines = view(&found(&mut focus, &effects, vec![hit(5, "Tide tables")]));
    let order = lines
        .lines
        .iter()
        .find(|line| line.kind == BarLineKind::Order)
        .expect("the order row")
        .clone();
    assert_eq!(order.title, "Sorted by relevance");
    let effects = focus.handle_on(Input::BarRun(order.token), &rows);
    let Some(Request::Search { order, .. }) = asked(&effects).into_iter().next() else {
        panic!("asked again: {effects:?}");
    };
    assert_eq!(order, postio_search::ResultOrder::Newest);
    let lines = view(&found(&mut focus, &effects, vec![hit(5, "Tide tables")]));
    let row = lines
        .lines
        .iter()
        .find(|line| line.kind == BarLineKind::Order)
        .expect("the order row");
    assert_eq!(row.title, "Sorted by date");
    assert_eq!(
        lines.highlight,
        Some(row.token),
        "the highlight stays on it"
    );
}

// -- The search dropdown (spec 010 step 2, screens 01 and 03) ---------------
//
// With `results_view` on -- the Mac's policy -- the bar's search half is the
// dropdown: empty, it lists the recent and saved searches and the cheat
// sheet; with words, the top hits, Narrow to and Show all. Everything above
// this section runs with it off, which is GTK's bar, unchanged.

mod dropdown {
    use super::*;
    use chrono::{DateTime, Local, NaiveDate, TimeZone};
    use postio_client::protocol::RecentSearch;
    use postio_focus::{DropdownRowKind, DropdownState, DropdownView, Lane, RunStyle};
    use postio_model::{AddressId, EmailAddress, LabelId};
    use postio_search::facets::{Count, SearchFacets};
    use postio_search::results::{
        ConversationHit, ConversationKey, ConversationResults, FacetNames, Match, Passage, Source,
    };

    /// Saturday 26 September 2026, mid-afternoon, as the screens are.
    fn today() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 26, 15, 0, 0).unwrap()
    }

    fn mac_search() -> FocusController {
        let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
        assert!(
            focus.policy().caps.results_view,
            "the Mac draws the dropdown"
        );
        let _ = focus.handle(Input::Clock(Some(today())));
        focus
    }

    /// The dropdown's last view among `effects`.
    fn dropdown(effects: &[Effect]) -> DropdownView {
        try_dropdown(effects).unwrap_or_else(|| panic!("no dropdown in {effects:?}"))
    }

    fn try_dropdown(effects: &[Effect]) -> Option<DropdownView> {
        shown(effects)
            .into_iter()
            .rev()
            .find_map(|intent| match intent {
                Intent::Dropdown(view) => Some(view),
                _ => None,
            })
    }

    fn text(runs: &[postio_focus::Run]) -> String {
        runs.iter().map(|run| run.text.as_str()).collect()
    }

    fn recent(query: &str, days_ago: i64, hits: u64) -> RecentSearch {
        RecentSearch {
            query: query.to_owned(),
            last_run_at: (today() - chrono::TimeDelta::days(days_ago)).to_utc(),
            hits,
        }
    }

    /// The pinned saved searches the screens show.
    fn saved() -> Vec<(String, String)> {
        [
            ("Waiting on reply", "from:juno"),
            ("Atlas", "subject:atlas"),
            ("Receipts this month", "in:Receipts"),
            ("From school", "from:northfield"),
        ]
        .into_iter()
        .map(|(name, query)| (name.to_owned(), query.to_owned()))
        .collect()
    }

    /// Answer the one request among `effects` that `pick` takes.
    fn answer(
        focus: &mut FocusController,
        effects: &[Effect],
        pick: impl Fn(&Request) -> bool,
        reply: impl FnOnce(&Request) -> Reply,
        rows: &List,
    ) -> Vec<Effect> {
        let (ticket, request) = asks(effects)
            .into_iter()
            .rev()
            .find(|(_, request)| pick(request))
            .unwrap_or_else(|| panic!("no such request in {effects:?}"));
        let reply = reply(&request);
        focus.handle_on(Input::Reply(ticket, reply), rows)
    }

    /// `/` with the places, the recent searches and the saved counts read.
    fn opened(focus: &mut FocusController, rows: &List) -> Vec<Effect> {
        let _ = focus.handle(Input::SavedSearches(saved()));
        let mut effects = bar_open(focus, CommandId::Search, rows);
        let more = answer(
            focus,
            &effects,
            |request| *request == Request::RecentSearches,
            |_| {
                Reply::RecentSearches(Ok(vec![
                    recent("atlas budget", 1, 48),
                    recent("from:ada invoice", 5, 6),
                    recent("has:attachment in:Receipts after:2026-09-01", 5, 19),
                    recent("harbor", 8, 3),
                ]))
            },
            rows,
        );
        effects.extend(more);
        let more = answer(
            focus,
            &effects,
            |request| matches!(request, Request::SavedCounts { .. }),
            |request| {
                let Request::SavedCounts { searches, .. } = request else {
                    unreachable!()
                };
                Reply::SavedCounts(Ok(searches
                    .iter()
                    .zip([5, 38, 19, 4])
                    .map(|((key, _), total)| (key.clone(), total, 0))
                    .collect()))
            },
            rows,
        );
        effects.extend(more);
        effects
    }

    fn ada() -> EmailAddress {
        EmailAddress::new(Some("Ada Moreno"), "ada@example.com")
    }

    fn tomas() -> EmailAddress {
        EmailAddress::new(Some("Tom\u{e1}s Reyes"), "tomas@example.com")
    }

    fn conversation(message: i64, from: EmailAddress, subject: &str) -> ConversationHit {
        ConversationHit {
            key: ConversationKey::Thread(ThreadId::new(message + 400)),
            best: MessageId::new(message),
            mailbox_id: MailboxId::new(1),
            subject: Some(subject.to_owned()),
            from: Some(from),
            newest_match: Local
                .with_ymd_and_hms(2026, 9, 26, 9, 0, 0)
                .unwrap()
                .to_utc(),
            messages: 3,
            unread: false,
            has_attachments: false,
            labels: Vec::new(),
            score: 1.0,
            reasons: Vec::new(),
            matches: vec![Match {
                source: Source::Body,
                passage: None,
                when: None,
            }],
        }
    }

    /// What `atlas budget` finds on the screens: four of 48, with facets.
    fn atlas_budget() -> ConversationResults {
        ConversationResults {
            hits: vec![
                conversation(1, ada(), "Re: Atlas Q3 budget, final numbers"),
                conversation(2, tomas(), "Atlas staffing plan for Q4"),
                conversation(3, ada(), "Contractor invoices for September"),
                conversation(4, ada(), "Atlas budget template v2"),
            ],
            total: 48,
            capped: false,
            messages_searched: 18_204,
            corpus_complete: true,
            contents_complete: true,
            facets: SearchFacets {
                senders: vec![
                    Count {
                        id: AddressId::new(1),
                        conversations: 21,
                    },
                    Count {
                        id: AddressId::new(2),
                        conversations: 9,
                    },
                ],
                labels: vec![Count {
                    id: LabelId::new(3),
                    conversations: 30,
                }],
                attachment: 12,
                ..SearchFacets::default()
            },
            files: 12,
            people: 6,
            names: FacetNames {
                people: vec![(AddressId::new(1), ada()), (AddressId::new(2), tomas())],
                labels: vec![(LabelId::new(3), "Atlas".to_owned())],
                label_colors: Vec::new(),
                folders: vec![(MailboxId::new(1), "Inbox".to_owned())],
            },
            elapsed: std::time::Duration::from_millis(38),
        }
    }

    fn is_conversations(request: &Request) -> bool {
        matches!(request, Request::Conversations { .. })
    }

    /// Type `words` and answer the conversation search with `results`.
    fn searched(
        focus: &mut FocusController,
        words: &str,
        results: ConversationResults,
        rows: &List,
    ) -> Vec<Effect> {
        let effects = typed(focus, words, rows);
        answer(
            focus,
            &effects,
            is_conversations,
            |request| {
                let Request::Conversations { stamp, .. } = request else {
                    unreachable!()
                };
                Reply::Conversations {
                    stamp: *stamp,
                    answer: Ok(Box::new(results)),
                }
            },
            rows,
        )
    }

    #[test]
    fn the_empty_dropdown_lists_three_recents_the_saved_searches_and_the_cheat_sheet() {
        let rows = List::of(3);
        let mut focus = mac_search();
        let effects = opened(&mut focus, &rows);
        assert!(
            !shown(&effects)
                .iter()
                .any(|intent| matches!(intent, Intent::BarLines(_))),
            "the Mac draws the dropdown, not spec 009's lines"
        );
        let view = dropdown(&effects);
        assert_eq!(view.state, DropdownState::Empty);
        let titles: Vec<&str> = view.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Recent", "Saved searches", "Search by"]);

        let recent = &view.sections[0];
        assert_eq!(recent.note.as_deref(), Some("forgets one"));
        assert_eq!(recent.note_key.as_deref(), Some("alt+BackSpace"));
        let said: Vec<(String, String, Option<&str>)> = recent
            .rows
            .iter()
            .map(|row| (text(&row.title), text(&row.detail), row.right.as_deref()))
            .collect();
        assert_eq!(
            said,
            [
                (
                    "atlas budget".into(),
                    "48 results".into(),
                    Some("yesterday")
                ),
                ("from:ada invoice".into(), "6 results".into(), Some("Mon")),
                (
                    "has:attachment in:Receipts after:2026-09-01".into(),
                    "19 results".into(),
                    Some("Mon")
                ),
            ],
            "at most three, newest first"
        );
        assert!(
            recent
                .rows
                .iter()
                .all(|row| row.kind == DropdownRowKind::Recent && row.selectable)
        );
        assert_eq!(
            recent.rows[0].title[0].style,
            RunStyle::Plain,
            "words read as words"
        );
        assert_eq!(
            recent.rows[1].title[0].style,
            RunStyle::Mono,
            "a query reads as one"
        );
        assert_eq!(
            view.highlight,
            Some(recent.rows[0].token),
            "the newest is focused"
        );

        let pills: Vec<(&str, Option<&str>, Option<&str>)> = view.sections[1]
            .pills
            .iter()
            .map(|pill| {
                (
                    pill.label.as_str(),
                    pill.count.as_deref(),
                    pill.key.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            pills,
            [
                ("Waiting on reply", Some("5"), Some("alt+1")),
                ("Atlas", Some("38"), Some("alt+2")),
                ("Receipts this month", Some("19"), Some("alt+3")),
                ("From school", Some("4"), Some("alt+4")),
            ]
        );

        let by = &view.sections[2];
        let sheet: Vec<(String, String)> = by
            .rows
            .iter()
            .filter(|row| row.kind == DropdownRowKind::CheatSheet)
            .map(|row| (text(&row.title), text(&row.detail)))
            .collect();
        assert_eq!(sheet.len(), 8);
        assert_eq!(
            sheet[0],
            ("from:".to_owned(), "person or address".to_owned())
        );
        let example = by
            .rows
            .iter()
            .find(|row| row.kind == DropdownRowKind::Example)
            .expect("the plain-English example");
        assert!(by.rows.iter().all(|row| !row.selectable), "nothing to run");
        assert_eq!(
            text(&example.title),
            "Or just type it: invoices from ada last month"
        );
        assert!(
            text(&example.detail).contains("after:2026-08-01 before:2026-09-01"),
            "lowered live against today: {:?}",
            text(&example.detail)
        );
        let hints: Vec<&str> = view.hints.iter().map(|hint| hint.label.as_str()).collect();
        assert_eq!(hints, ["move", "run again", "saved", "commands"]);
    }

    #[test]
    fn the_saved_counts_are_asked_for_every_saved_search() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = focus.handle(Input::SavedSearches(saved()));
        let effects = bar_open(&mut focus, CommandId::Search, &rows);
        let counts = asked(&effects)
            .into_iter()
            .find_map(|request| match request {
                Request::SavedCounts {
                    searches,
                    today: day,
                } => Some((searches, day)),
                _ => None,
            })
            .expect("the saved searches are counted as the bar opens");
        assert_eq!(counts.0, saved(), "every one, keyed by its name");
        assert_eq!(counts.1, NaiveDate::from_ymd_opt(2026, 9, 26).unwrap());
    }

    #[test]
    fn words_ask_for_four_conversations_and_draw_hits_narrow_to_and_show_all() {
        let rows = List::of(3);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = typed(&mut focus, "atlas budget", &rows);
        let request = asked(&effects)
            .into_iter()
            .find(is_conversations)
            .expect("words are searched as conversations");
        let Request::Conversations { limit, order, .. } = &request else {
            unreachable!()
        };
        assert_eq!(*limit, 4, "the dropdown's top hits");
        assert_eq!(*order, postio_search::results::ConversationOrder::BestMatch);
        assert_eq!(request.lane(), Some(Lane::Conversations));

        let effects = answer(
            &mut focus,
            &effects,
            is_conversations,
            |request| {
                let Request::Conversations { stamp, .. } = request else {
                    unreachable!()
                };
                Reply::Conversations {
                    stamp: *stamp,
                    answer: Ok(Box::new(atlas_budget())),
                }
            },
            &rows,
        );
        let view = dropdown(&effects);
        assert_eq!(view.state, DropdownState::Words);
        let titles: Vec<&str> = view.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Top hits", "Narrow to", ""]);

        let hits = &view.sections[0].rows;
        assert_eq!(hits.len(), 4);
        assert_eq!(
            text(&hits[0].title),
            "Ada Moreno \u{b7} Re: Atlas Q3 budget, final numbers"
        );
        let marked: Vec<&str> = hits[0]
            .title
            .iter()
            .filter(|run| run.highlighted)
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(
            marked,
            ["Atlas", "budget"],
            "the matched words, from the engine"
        );
        assert_eq!(hits[0].folder.as_deref(), Some("in:Inbox"));
        assert_eq!(hits[0].right.as_deref(), Some("26 Sep"));
        assert!(
            hits.iter()
                .all(|row| row.kind == DropdownRowKind::Hit && row.selectable)
        );

        let pills: Vec<String> = view.sections[1]
            .pills
            .iter()
            .map(|pill| {
                format!(
                    "{} {} {}",
                    pill.op.as_deref().unwrap_or_default(),
                    pill.label,
                    pill.count.as_deref().unwrap_or_default()
                )
            })
            .collect();
        assert_eq!(
            pills,
            [
                "from: Ada Moreno 21",
                "from: Tom\u{e1}s Reyes 9",
                "has: attachment 12",
                "label: Atlas 30",
            ]
        );

        let show_all = &view.sections[2].rows[0];
        assert_eq!(show_all.kind, DropdownRowKind::ShowAll);
        assert_eq!(text(&show_all.title), "Show all 48 results");
        assert_eq!(
            show_all.key.as_deref(),
            focus.keymap().binding(CommandId::ShowAllResults)
        );
        assert_eq!(
            view.highlight,
            Some(show_all.token),
            "Show all is focused by default"
        );
        assert_eq!(view.count.as_deref(), Some("48 matches \u{b7} 38 ms"));
    }

    #[test]
    fn the_hits_passages_are_read_after_the_hits_land() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = searched(&mut focus, "atlas budget", atlas_budget(), &rows);
        let (ticket, request) = asks(&effects)
            .into_iter()
            .find(|(_, request)| matches!(request, Request::Passages { .. }))
            .expect("the four hits' passages are asked for");
        assert_eq!(request.lane(), Some(Lane::Passages));
        let Request::Passages { hits, stamp, .. } = request else {
            unreachable!()
        };
        assert_eq!(hits.len(), 4);
        assert_eq!(hits[1], (MessageId::new(2), vec![Source::Body]));
        let passage = Passage {
            text: "moves the Atlas budget up by about 9%".to_owned(),
            ranges: vec![10..15, 16..22],
            elided_start: false,
            elided_end: false,
        };
        let effects = focus.handle(Input::Reply(
            ticket,
            Reply::Passages {
                stamp,
                answer: Ok(vec![(
                    MessageId::new(2),
                    vec![Match {
                        source: Source::Body,
                        passage: Some(passage),
                        when: None,
                    }],
                )]),
            },
        ));
        let view = dropdown(&effects);
        let second = &view.sections[0].rows[1];
        assert_eq!(
            text(&second.detail),
            "moves the Atlas budget up by about 9%"
        );
        let marked: Vec<&str> = second
            .detail
            .iter()
            .filter(|run| run.highlighted)
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(marked, ["Atlas", "budget"]);
    }

    #[test]
    fn an_answer_for_words_since_changed_is_dropped() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let early = typed(&mut focus, "atla", &rows);
        let _ = typed(&mut focus, "atlas", &rows);
        let effects = answer(
            &mut focus,
            &early,
            is_conversations,
            |request| {
                let Request::Conversations { stamp, .. } = request else {
                    unreachable!()
                };
                Reply::Conversations {
                    stamp: *stamp,
                    answer: Ok(Box::new(atlas_budget())),
                }
            },
            &rows,
        );
        assert_eq!(try_dropdown(&effects), None, "{effects:?}");
    }

    #[test]
    fn tab_adds_the_first_narrow_to_pills_term_and_searches_again() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let _ = searched(&mut focus, "atlas budget", atlas_budget(), &rows);
        let effects = focus.handle(Input::BarTab);
        assert!(
            shown(&effects).contains(&Intent::OpenBar {
                mode: BarMode::Search,
                text: "atlas budget from:ada@example.com".to_owned(),
                select: None,
            }),
            "{effects:?}"
        );
        let request = asked(&effects)
            .into_iter()
            .find(is_conversations)
            .expect("the panel re-runs");
        let Request::Conversations { query, .. } = request else {
            unreachable!()
        };
        assert_eq!(query.filters().count(), 1);
    }

    #[test]
    fn tab_with_no_pill_to_add_is_the_toolkits() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        assert!(
            focus.handle(Input::BarTab).is_empty(),
            "empty: nothing to add"
        );
        let _ = typed(&mut focus, "atlas", &rows);
        assert!(focus.handle(Input::BarTab).is_empty(), "no answer yet");
    }

    #[test]
    fn alt_backspace_on_a_recent_forgets_it_and_the_next_moves_up() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let effects = opened(&mut focus, &rows);
        let first = dropdown(&effects).sections[0].rows[0].token;
        let effects = focus.handle(Input::SearchForget(first));
        assert!(
            asked(&effects).contains(&Request::ForgetSearch {
                query: "atlas budget".to_owned()
            }),
            "{effects:?}"
        );
        let view = dropdown(&effects);
        let left: Vec<String> = view.sections[0]
            .rows
            .iter()
            .map(|row| text(&row.title))
            .collect();
        assert_eq!(
            left,
            [
                "from:ada invoice",
                "has:attachment in:Receipts after:2026-09-01",
                "harbor"
            ],
            "gone at once, and the fourth moves up"
        );
        assert_eq!(view.highlight, Some(view.sections[0].rows[0].token));

        // The key itself forgets the row the arrows rest on.
        let second = view.sections[0].rows[1].token;
        let _ = focus.handle(Input::SearchHighlighted(second));
        assert!(focus.answers(CommandId::ForgetRecent));
        let effects = run(&mut focus, CommandId::ForgetRecent, &rows);
        assert!(asked(&effects).contains(&Request::ForgetSearch {
            query: "has:attachment in:Receipts after:2026-09-01".to_owned()
        }));
        // Unmoved, the key forgets the row focused by default: the newest.
        let mut fresh = mac_search();
        let effects = opened(&mut fresh, &rows);
        assert!(!dropdown(&effects).sections[0].rows.is_empty());
        let effects = run(&mut fresh, CommandId::ForgetRecent, &rows);
        assert!(asked(&effects).contains(&Request::ForgetSearch {
            query: "atlas budget".to_owned()
        }));
        // A token that is no recent forgets nothing.
        assert!(asked(&focus.handle(Input::SearchForget(999_999))).is_empty());
    }

    #[test]
    fn return_on_a_hit_opens_it_and_remembers_the_query() {
        let rows = List::of(3);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = searched(&mut focus, "atlas budget", atlas_budget(), &rows);
        let second = dropdown(&effects).sections[0].rows[1].token;
        let effects = focus.handle_on(Input::BarRun(second), &rows);
        assert!(asked(&effects).contains(&Request::RememberSearch {
            query: "atlas budget".to_owned(),
            hits: 48,
        }));
        assert_eq!(
            shown(&effects),
            vec![
                Intent::CloseSurface(SurfaceKind::Bar),
                Intent::OpenMessage {
                    message: MessageId::new(2),
                    index: 1,
                    total: 4,
                    host: Host::Own,
                }
            ]
        );
    }

    #[test]
    fn show_all_takes_the_dropdown_down_for_the_results() {
        // Step 3: the results view, which `tests/results.rs` drives; the
        // query is remembered once its results land there.
        let rows = List::of(3);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let _ = searched(&mut focus, "atlas budget", atlas_budget(), &rows);
        assert!(focus.answers(CommandId::ShowAllResults));
        let effects = run(&mut focus, CommandId::ShowAllResults, &rows);
        assert!(shown(&effects).contains(&Intent::CloseSurface(SurfaceKind::Bar)));
        assert!(try_dropdown(&effects).is_none());
        assert!(focus.in_results());
        assert!(
            asked(&effects)
                .iter()
                .any(|request| matches!(request, Request::ResultsPage { .. }))
        );
        // Counted by the dropdown already: kept now, so leaving before the
        // results land keeps it too.
        assert!(asked(&effects).contains(&Request::RememberSearch {
            query: "atlas budget".to_owned(),
            hits: 48,
        }));
    }

    #[test]
    fn a_recent_search_run_puts_its_query_in_the_field() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let effects = opened(&mut focus, &rows);
        let first = dropdown(&effects).sections[0].rows[1].token;
        let effects = focus.handle_on(Input::BarRun(first), &rows);
        assert!(shown(&effects).contains(&Intent::OpenBar {
            mode: BarMode::Search,
            text: "from:ada invoice".to_owned(),
            select: None,
        }));
        assert!(
            asked(&effects)
                .into_iter()
                .any(|request| is_conversations(&request))
        );
    }

    #[test]
    fn commands_and_in_keep_spec_009s_lines() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = typed(&mut focus, ">mark", &rows);
        assert!(try_dropdown(&effects).is_none());
        let _ = view(&effects);
        let effects = typed(&mut focus, "in:Rec", &rows);
        assert!(try_dropdown(&effects).is_none());
        let effects = typed(&mut focus, "", &rows);
        assert_eq!(dropdown(&effects).state, DropdownState::Empty, "and back");
    }

    #[test]
    fn the_gtk_bar_asks_nothing_of_the_dropdown() {
        let rows = List::of(1);
        let mut focus = FocusController::new(Policy::for_platform(Platform::Freedesktop));
        assert!(!focus.policy().caps.results_view);
        let effects = run(&mut focus, CommandId::Search, &rows);
        assert_eq!(asked(&effects), [Request::Places]);
        let effects = typed(&mut focus, "atlas budget", &rows);
        assert!(
            asked(&effects)
                .iter()
                .all(|r| matches!(r, Request::Search { .. }))
        );
        assert!(!focus.answers(CommandId::ShowAllResults));
    }
}
