use crate::search_support::{asks, shown, shown_but_the_footer};
use postio_config::paths::Platform;
use postio_core::CommandId;
use postio_focus::{
    BarLine, BarLineKind, BarMode, BarView, Effect, FocusController, Host, Input, Intent, Opened,
    PageAnswer, PlacesRead, Policy, Reply, Request, RowFacts, Rows, SurfaceKind, Ticket, ToastKind,
};
use postio_model::{FocusScope, ListScope, MailboxId, MailboxRole, MessageId, ThreadId};
use postio_ui::keymap::KeyContext;
use postio_ui::places::Entry;

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

fn mac() -> FocusController {
    FocusController::new(Policy::for_platform(Platform::Apple))
}

fn asked(effects: &[Effect]) -> Vec<Request> {
    asks(effects)
        .into_iter()
        .map(|(_, request)| request)
        .collect()
}

fn ask(effects: &[Effect]) -> (Ticket, Request) {
    let asks = asks(effects);
    assert_eq!(asks.len(), 1, "one ask in {effects:?}");
    asks[0].clone()
}

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
        messages: 18_204,
    }
}

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
        asked(&effects)
            .iter()
            .filter(|request| **request == Request::Places)
            .count(),
        1,
        "one read, not one per account: {:?}",
        asked(&effects)
    );
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::Dropdown(view)
                if view.state == postio_focus::DropdownState::Empty)),
        "the empty dropdown: {effects:?}"
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
fn the_bar_searches_people_on_both_platforms() {
    // from: and to: that know people (specs/011-focus-contacts S4), on the
    // Mac and, since its search adopted the dropdown, on Linux (#1825).
    let rows = List::of(3);
    for platform in [Platform::Apple, Platform::Freedesktop] {
        let mut focus = FocusController::new(Policy::for_platform(platform));
        let _ = bar_open(&mut focus, CommandId::Search, &rows);
        let effects = typed(&mut focus, "from:lovelace atlas", &rows);
        let aware: Vec<bool> = asked(&effects)
            .iter()
            .filter_map(|request| match request {
                Request::Conversations { query, .. } => Some(query.is_contact_aware()),
                _ => None,
            })
            .collect();
        assert!(
            !aware.is_empty() && aware.iter().all(|aware| *aware),
            "{platform:?}: {:?}",
            asked(&effects)
        );
    }
}

#[test]
fn a_command_chosen_in_the_bar_acts_on_the_aim_captured_when_it_opened() {
    let rows = List::of(3);
    let mut twin = mac();
    let _ = twin.landed(&rows, true);
    let _ = run(&mut twin, CommandId::NextMessage, &rows);
    let from_the_list = asked(&run(&mut twin, CommandId::Archive, &rows));

    let mut focus = mac();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::NextMessage, &rows);
    let _ = bar_open(&mut focus, CommandId::CommandPalette, &rows);
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
        shown_but_the_footer(&effects),
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

#[test]
fn receipts_from_the_places_opens_as_the_list_with_the_cursor_on_its_first_row() {
    let rows = List::of(3);
    let mut focus = mac();
    let _ = focus.landed(&rows, true);
    let _ = run(&mut focus, CommandId::LastMessage, &rows);
    let (ticket, _) = ask(&run(&mut focus, CommandId::GoToFolders, &rows));
    let _ = focus.handle(Input::Reply(ticket, Reply::Places(Ok(places()))));
    let receipts = focus.places("receipts");
    let effects = focus.handle_on(Input::OpenPlace(receipts[0].0), &rows);
    let intents = shown(&effects);
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
fn the_go_to_keys_go_to_their_places() {
    let rows = List::of(2);
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
        let mut focus = mac();
        let (ticket, _) = ask(&run(&mut focus, command, &rows));
        assert!(
            focus
                .handle(Input::Reply(ticket, Reply::RoleFolder(None)))
                .is_empty()
        );
    }
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

fn flagged_elsewhere() -> Input {
    Input::Event(postio_core::Event::MessagesChanged {
        account: postio_model::AccountId::new(1),
        messages: vec![MessageId::new(100)],
    })
}

/// While the popover is up, changing mail has its places read again so the counts follow (#2153);
/// changes during a read are one more read, not one each.
#[test]
fn the_folders_popover_reads_its_counts_again_as_mail_changes() {
    let rows = List::of(1);
    let mut focus = mac();
    let (opening, _) = ask(&run(&mut focus, CommandId::GoToFolders, &rows));
    let _ = focus.handle(Input::PlacesShown(true));

    assert!(asked(&focus.handle(flagged_elsewhere())).is_empty());
    assert!(asked(&focus.handle(flagged_elsewhere())).is_empty());
    let landed = focus.handle(Input::Reply(opening, Reply::Places(Ok(places()))));
    let (again, request) = ask(&landed);
    assert_eq!(request, Request::Places, "one more read for both changes");
    let landed = focus.handle(Input::Reply(again, Reply::Places(Ok(places()))));
    assert!(
        asked(&landed).is_empty(),
        "nothing changed since: {landed:?}"
    );

    let (_, request) = ask(&focus.handle(flagged_elsewhere()));
    assert_eq!(request, Request::Places);

    focus.invalidate();
    let (later, _) = ask(&focus.handle(flagged_elsewhere()));
    let _ = focus.handle(Input::Reply(later, Reply::Places(Ok(places()))));

    let _ = focus.handle(Input::PlacesShown(false));
    assert!(
        asked(&focus.handle(flagged_elsewhere())).is_empty(),
        "a popover that has gone reads nothing"
    );
}

#[test]
fn mail_changing_reads_no_places_while_no_popover_is_up() {
    let rows = List::of(1);
    let mut focus = mac();
    let (opening, _) = ask(&run(&mut focus, CommandId::GoToFolders, &rows));
    let _ = focus.handle(Input::Reply(opening, Reply::Places(Ok(places()))));
    assert!(asked(&focus.handle(flagged_elsewhere())).is_empty());
}

/// A folder whose last pass failed is marked in the places, with why, and the sync label names it
/// until a pass completes (#1731).
#[test]
fn a_folder_that_cannot_sync_is_marked_in_the_places_and_named_by_the_label() {
    let rows = List::of(1);
    let mut focus = mac();
    let effects = run(&mut focus, CommandId::GoToFolders, &rows);
    let (ticket, _) = ask(&effects);
    let _ = focus.handle(Input::Reply(ticket, Reply::Places(Ok(places()))));
    let account = postio_model::AccountId::new(1);
    let said = |focus: &mut FocusController, failure: Option<&str>| {
        focus.handle(Input::Event(postio_core::Event::MailboxSyncChanged {
            account,
            mailbox: MailboxId::new(2),
            failure: failure.map(str::to_owned),
        }))
    };
    let failures = |focus: &FocusController| -> Vec<(String, String)> {
        focus
            .places("")
            .into_iter()
            .filter_map(|(_, entry)| Some((entry.name, entry.failure?)))
            .collect()
    };
    let label = |effects: &[Effect]| {
        effects.iter().find_map(|effect| match effect {
            Effect::Show(Intent::SyncLabel(label)) => Some(label.clone()),
            _ => None,
        })
    };
    let _ = focus.handle(Input::Event(postio_core::Event::ConnectionChanged {
        account,
        state: postio_core::ConnectionState::Online,
    }));

    let effects = said(&mut focus, Some("SELECT refused: Mailbox is locked"));
    assert!(
        shown(&effects).contains(&Intent::PlacesChanged),
        "{effects:?}"
    );
    assert_eq!(
        failures(&focus),
        vec![(
            "Sent".to_owned(),
            "SELECT refused: Mailbox is locked".to_owned()
        )]
    );
    let drawn = label(&effects).expect("the label is drawn again");
    assert_eq!(drawn.text, "Sent can't sync");
    assert_eq!(
        drawn.detail.as_deref(),
        Some("Sent: SELECT refused: Mailbox is locked")
    );
    let sent = focus
        .places("sent")
        .into_iter()
        .next()
        .expect("Sent is listed");
    let opened = focus.handle_on(Input::OpenPlace(sent.0), &rows);
    assert!(!opened.is_empty(), "the marked row still opens");

    let effects = said(&mut focus, None);
    assert!(
        shown(&effects).contains(&Intent::PlacesChanged),
        "{effects:?}"
    );
    assert_eq!(failures(&focus), Vec::<(String, String)>::new());
    assert_ne!(
        label(&effects).map(|label| label.text),
        Some("Sent can't sync".to_owned())
    );
}

#[test]
fn a_folder_that_fails_before_the_places_are_read_is_named_once_they_are() {
    let rows = List::of(1);
    let mut focus = mac();
    let account = postio_model::AccountId::new(1);
    let _ = focus.handle(Input::Event(postio_core::Event::ConnectionChanged {
        account,
        state: postio_core::ConnectionState::Online,
    }));
    let effects = focus.handle(Input::Event(postio_core::Event::MailboxSyncChanged {
        account,
        mailbox: MailboxId::new(2),
        failure: Some("Mailbox is locked".to_owned()),
    }));
    let label = |effects: &[Effect]| {
        effects.iter().find_map(|effect| match effect {
            Effect::Show(Intent::SyncLabel(label)) => Some(label.text.clone()),
            _ => None,
        })
    };
    assert_eq!(label(&effects).as_deref(), Some("A folder can't sync"));
    let (ticket, _) = asks(&effects)
        .into_iter()
        .find(|(_, request)| *request == Request::Places)
        .expect("a folder with no name yet reads the places");
    let effects = focus.handle_on(Input::Reply(ticket, Reply::Places(Ok(places()))), &rows);
    assert_eq!(label(&effects).as_deref(), Some("Sent can't sync"));
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

/// The popover open by then is told the places changed, and a token it read before is spent rather
/// than opening whatever moved into its row.
#[test]
fn counts_landing_after_the_places_change_them() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = focus.handle(Input::Filtering(true));
    let Effect::Ask(counted, _) = focus.refresh_counts() else {
        panic!("counts are asked for");
    };
    let (ticket, _) = ask(&run(&mut focus, CommandId::GoToFolders, &rows));
    let _ = focus.handle(Input::Reply(ticket, Reply::Places(Ok(places()))));
    let names = |focus: &FocusController| -> Vec<String> {
        focus
            .places("")
            .into_iter()
            .map(|(_, entry)| entry.name)
            .collect()
    };
    assert!(!names(&focus).contains(&"Filtered".to_owned()), "not yet");
    let archive = focus.places("archive")[0].0;

    let counts = postio_client::protocol::FocusCounts {
        filtered_today: 4,
        ..Default::default()
    };
    let effects = focus.handle(Input::Reply(counted, Reply::FocusCounts(Ok(counts))));
    assert!(
        shown(&effects).contains(&Intent::PlacesChanged),
        "{:?}",
        shown(&effects)
    );
    assert!(names(&focus).contains(&"Filtered".to_owned()));
    let effects = focus.handle_on(Input::OpenPlace(archive), &rows);
    assert!(
        shown(&effects).is_empty() && asked(&effects).is_empty(),
        "a token from before opens nothing: {effects:?}"
    );

    let Effect::Ask(counted, _) = focus.refresh_counts() else {
        panic!("counts are asked for");
    };
    let effects = focus.handle(Input::Reply(counted, Reply::FocusCounts(Ok(counts))));
    assert!(!shown(&effects).contains(&Intent::PlacesChanged));
}

#[test]
fn filtering_turned_off_changes_the_places() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = focus.handle(Input::Filtering(true));
    let (ticket, _) = ask(&run(&mut focus, CommandId::GoToFolders, &rows));
    let _ = focus.handle(Input::Reply(ticket, Reply::Places(Ok(places()))));

    let effects = focus.handle(Input::Filtering(false));
    assert!(
        shown(&effects).contains(&Intent::PlacesChanged),
        "{:?}",
        shown(&effects)
    );
    let effects = focus.handle(Input::Filtering(false));
    assert!(
        !shown(&effects).contains(&Intent::PlacesChanged),
        "unchanged"
    );
}

#[test]
fn a_bar_place_that_is_an_inbox_goes_to_focus_s_inbox() {
    let rows = List::of(1);
    let mut focus = mac();
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

#[test]
fn alt_numbers_open_the_saved_searches() {
    let rows = List::of(1);
    let mut focus = mac();
    let saved = |key: &str, name: &str, query: &str| postio_ui::saved_search::SavedSearch {
        key: key.to_owned(),
        name: name.to_owned(),
        query: query.to_owned(),
        notify: false,
    };
    let _ = focus.handle(Input::SavedSearches(vec![
        saved("budget", "Budget", "subject:budget"),
        saved("ada", "Ada", "from:ada"),
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
            .any(|request| matches!(request, Request::Conversations { .. })),
        "and it runs: {effects:?}"
    );

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
        vec![Intent::SaveSearch(postio_focus::SaveSearch {
            query: "from:ada invoice".to_owned(),
            name: None,
            pin: true,
            notify: false,
            dates: postio_ui::saved_search::Dates::AsTyped,
        })]
    );
    let effects = focus.handle(Input::SearchSaved(Ok(postio_focus::SearchesSaved {
        searches: vec![postio_ui::saved_search::SavedSearch {
            key: "from-ada-invoice".to_owned(),
            name: "from-ada-invoice".to_owned(),
            query: "from:ada invoice".to_owned(),
            notify: false,
        }],
        key: Some("from-ada-invoice".to_owned()),
    })));
    let intents = shown(&effects);
    assert!(
        intents.contains(&Intent::Toast {
            text: postio_ui::focus_target::search_saved("from:ada invoice"),
            kind: ToastKind::Completed {
                undoable: false,
                seconds: None,
            },
        }),
        "a confirmation, not a warning's notice: {intents:?}"
    );
    assert!(
        shown(&effects)
            .iter()
            .any(|intent| matches!(intent, Intent::Dropdown(_))),
        "the dropdown drawn again: {intents:?}"
    );
    let effects = focus.handle(Input::SearchSaved(Err("no file".to_owned())));
    assert_eq!(
        shown(&effects),
        vec![Intent::Toast {
            text: "no file".to_owned(),
            kind: ToastKind::Notice,
        }]
    );
}

#[test]
fn closing_the_bar_forgets_it() {
    let rows = List::of(1);
    let mut focus = mac();
    let _ = bar_open(&mut focus, CommandId::CommandPalette, &rows);
    let lines = view(&typed(&mut focus, ">archive", &rows));
    let token = lines
        .lines
        .iter()
        .find(|line| line.command == Some(CommandId::Archive))
        .expect("an Archive row")
        .token;
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
    // Off the stack here, as a line run takes it off: an echoed `SurfaceClosed(Bar)` could land
    // after the bar was reopened.
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
    assert!(!focus.answers(CommandId::ShowAllResults));
    let _ = bar_open(&mut focus, CommandId::Search, &rows);
    for command in [
        CommandId::BackToWords,
        CommandId::SaveSearch,
        CommandId::SavedSearch1,
        CommandId::ShowAllResults,
        CommandId::ForgetRecent,
        CommandId::ExcludeSuggestion,
    ] {
        assert!(focus.answers(command), "{command:?}");
    }
    assert!(!focus.answers(CommandId::Archive), "a letter is typing");
}

mod dropdown {
    use super::*;
    use chrono::{DateTime, Local, NaiveDate, TimeZone};
    use postio_client::protocol::{RecentSearch, SavedCount};
    use postio_focus::{DropdownRowKind, DropdownState, DropdownView, Lane, RunStyle};
    use postio_model::{AddressId, EmailAddress, LabelId};
    use postio_search::facets::{Count, SearchFacets};
    use postio_search::query::Field;
    use postio_search::results::{
        ConversationHit, ConversationKey, ConversationResults, Counted, FacetNames, Match, Passage,
        Source,
    };
    use postio_search::suggest::{Completion, Person, Suggestions};

    fn today() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 26, 15, 0, 0).unwrap()
    }

    fn mac_search() -> FocusController {
        let mut focus = FocusController::new(Policy::for_platform(Platform::Apple));
        let _ = focus.handle(Input::Clock(Some(today())));
        focus
    }

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
            capped: false,
        }
    }

    fn saved() -> Vec<postio_ui::saved_search::SavedSearch> {
        [
            ("waiting", "Waiting on reply", "from:juno", false),
            ("atlas", "Atlas", "subject:atlas", true),
            ("receipts", "Receipts this month", "in:Receipts", false),
            ("school", "From school", "from:northfield", false),
        ]
        .into_iter()
        .map(
            |(key, name, query, notify)| postio_ui::saved_search::SavedSearch {
                key: key.to_owned(),
                name: name.to_owned(),
                query: query.to_owned(),
                notify,
            },
        )
        .collect()
    }

    fn keyed(saved: &[postio_ui::saved_search::SavedSearch]) -> Vec<(String, String)> {
        saved
            .iter()
            .map(|search| (search.key.clone(), search.query.clone()))
            .collect()
    }

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
                    .zip([(5, 0), (38, 3), (19, 0), (4, 0)])
                    .map(|((key, _), (total, new))| exact(key, total, new))
                    .collect()))
            },
            rows,
        );
        effects.extend(more);
        effects
    }

    fn exact(key: &str, total: u64, new: u64) -> SavedCount {
        SavedCount {
            key: key.to_owned(),
            total: Counted {
                conversations: total,
                capped: false,
            },
            new: Counted {
                conversations: new,
                capped: false,
            },
        }
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
            best_number: 1,
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
            matched_messages: 1,
            matches: vec![Match {
                source: Source::Body,
                passage: None,
                when: None,
            }],
        }
    }

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
                own: Vec::new(),
                labels: vec![(LabelId::new(3), "Atlas".to_owned())],
                label_colors: Vec::new(),
                folders: vec![(MailboxId::new(1), "Inbox".to_owned())],
                held: Vec::new(),
            },
            elapsed: std::time::Duration::from_millis(38),
        }
    }

    fn is_conversations(request: &Request) -> bool {
        matches!(request, Request::Conversations { .. })
    }

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
    fn the_empty_dropdowns_footer_says_how_many_messages_the_index_holds() {
        let rows = List::of(3);
        let mut focus = mac_search();
        let effects = opened(&mut focus, &rows);
        let view = dropdown(&effects);
        assert_eq!(view.state, DropdownState::Empty);
        assert_eq!(
            view.count.as_deref(),
            Some("local index \u{b7} 18,204 messages"),
            "the places' maintained total, not a count per open"
        );
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

        type Pill<'a> = (&'a str, Option<&'a str>, Option<&'a str>, Option<&'a str>);
        let pills: Vec<Pill<'_>> = view.sections[1]
            .pills
            .iter()
            .map(|pill| {
                (
                    pill.label.as_str(),
                    pill.count.as_deref(),
                    pill.fresh.as_deref(),
                    pill.key.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            pills,
            [
                ("Waiting on reply", Some("5"), None, Some("alt+1")),
                ("Atlas", Some("38"), Some("3 new"), Some("alt+2")),
                ("Receipts this month", Some("19"), None, Some("alt+3")),
                ("From school", Some("4"), None, Some("alt+4")),
            ],
            "the search that notifies carries a quiet badge (D15)"
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
        assert_eq!(counts.0, keyed(&saved()), "every one, keyed by its key");
        assert_eq!(counts.1, NaiveDate::from_ymd_opt(2026, 9, 26).unwrap());
    }

    #[test]
    fn running_a_notify_search_marks_it_seen_and_its_badge_goes() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = run(&mut focus, CommandId::SavedSearch2, &rows);
        assert!(
            asked(&effects).contains(&Request::MarkSeen {
                key: "atlas".to_owned()
            }),
            "viewing it is seeing it: {:?}",
            asked(&effects)
        );

        let _ = run(&mut focus, CommandId::Back, &rows);
        let effects = run(&mut focus, CommandId::SavedSearch1, &rows);
        assert!(
            !asked(&effects)
                .iter()
                .any(|request| matches!(request, Request::MarkSeen { .. })),
            "a search that does not notify has nothing to clear"
        );
    }

    #[test]
    fn new_mail_recounts_only_the_searches_that_notify() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = focus.handle_on(
            Input::Event(postio_core::Event::NewMail {
                account: postio_model::AccountId::new(1),
                mailbox: postio_model::MailboxId::new(1),
                messages: vec![MessageId::new(9)],
            }),
            &rows,
        );
        let searches = asked(&effects)
            .into_iter()
            .find_map(|request| match request {
                Request::SavedCounts { searches, .. } => Some(searches),
                _ => None,
            })
            .expect("the badges are counted again");
        assert_eq!(
            searches,
            [("atlas".to_owned(), "subject:atlas".to_owned())],
            "only Atlas notifies"
        );
        let effects = answer(
            &mut focus,
            &effects,
            |request| matches!(request, Request::SavedCounts { .. }),
            |_| Reply::SavedCounts(Ok(vec![exact("atlas", 39, 4)])),
            &rows,
        );
        let view = dropdown(&effects);
        let pills: Vec<(Option<&str>, Option<&str>)> = view.sections[1]
            .pills
            .iter()
            .map(|pill| (pill.count.as_deref(), pill.fresh.as_deref()))
            .collect();
        assert_eq!(
            pills,
            [
                (Some("5"), None),
                (Some("39"), Some("4 new")),
                (Some("19"), None),
                (Some("4"), None),
            ],
            "the others keep the counts they had"
        );
    }

    #[test]
    fn a_saved_search_past_its_caps_draws_its_count_and_badge_as_floors() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = focus.handle_on(
            Input::Event(postio_core::Event::NewMail {
                account: postio_model::AccountId::new(1),
                mailbox: postio_model::MailboxId::new(1),
                messages: vec![MessageId::new(9)],
            }),
            &rows,
        );
        let effects = answer(
            &mut focus,
            &effects,
            |request| matches!(request, Request::SavedCounts { .. }),
            |_| {
                Reply::SavedCounts(Ok(vec![SavedCount {
                    key: "atlas".to_owned(),
                    total: Counted {
                        conversations: 5_000,
                        capped: true,
                    },
                    new: Counted {
                        conversations: 99,
                        capped: true,
                    },
                }]))
            },
            &rows,
        );
        let view = dropdown(&effects);
        let atlas = &view.sections[1].pills[1];
        assert_eq!(atlas.count.as_deref(), Some("5,000+"));
        assert_eq!(atlas.fresh.as_deref(), Some("99+ new"));
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

    /// You are the top sender in your own mail, so a pill to narrow to yourself is not offered. A
    /// capped search's facets are floors, so its pills are too (D30).
    #[test]
    fn a_hit_says_where_a_digest_holds_it_and_its_account() {
        let rows = List::of(3);
        let mut focus = mac_search();
        let effects = run(&mut focus, CommandId::Search, &rows);
        let (ticket, _) = asks(&effects)
            .into_iter()
            .find(|(_, request)| *request == Request::Places)
            .expect("opening the bar reads the places");
        let mut read = places();
        read.owners = read
            .folders
            .iter()
            .map(|(id, _)| (*id, "work@example.org".to_owned()))
            .collect();
        let _ = focus.handle_on(Input::Reply(ticket, Reply::Places(Ok(read))), &rows);
        let mut found = atlas_budget();
        let held = found.hits[0].best;
        found.names.held = vec![(held, "Newsletters".to_owned(), false)];
        let effects = searched(&mut focus, "atlas budget", found, &rows);
        let hits = &dropdown(&effects).sections[0].rows;
        assert_eq!(
            hits[0].folder.as_deref(),
            Some("held \u{b7} Newsletters \u{b7} work@example.org")
        );
        assert_eq!(
            hits[1].folder.as_deref(),
            Some("in:Inbox \u{b7} work@example.org")
        );
    }

    #[test]
    fn narrow_to_pills_over_a_capped_search_are_floors() {
        let rows = List::of(3);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let mut broad = atlas_budget();
        broad.total = 5_000;
        broad.capped = true;
        let effects = searched(&mut focus, "atlas budget", broad, &rows);
        let view = dropdown(&effects);
        let narrow = view
            .sections
            .iter()
            .find(|section| section.title == "Narrow to")
            .expect("the pills");
        assert!(!narrow.pills.is_empty());
        for pill in &narrow.pills {
            let count = pill.count.as_deref().expect("a count");
            assert!(count.ends_with('+'), "{count:?} is a floor");
        }
    }

    #[test]
    fn narrow_to_offers_no_pill_for_your_own_address() {
        let rows = List::of(3);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let mut results = atlas_budget();
        let you = AddressId::new(9);
        results.facets.senders.insert(
            0,
            Count {
                id: you,
                conversations: 30,
            },
        );
        results
            .names
            .people
            .push((you, EmailAddress::new(Some("You"), "you@example.com")));
        results.names.own.push(you);

        let effects = searched(&mut focus, "atlas budget", results, &rows);
        let view = dropdown(&effects);
        let pills: Vec<String> = view.sections[1]
            .pills
            .iter()
            .map(|pill| format!("{} {}", pill.op.as_deref().unwrap_or_default(), pill.label))
            .collect();
        assert_eq!(
            pills,
            [
                "from: Ada Moreno",
                "from: Tom\u{e1}s Reyes",
                "has: attachment",
                "label: Atlas",
            ],
            "the top senders after you"
        );
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
        assert_eq!(
            request.first_line(),
            Some(postio_search::passage::FirstLine::Avoided),
            "the hit's row shows no preview (screen 03): a match in the first \
             line is the passage"
        );
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

    /// The dropdown's row has room for about 45 characters after "Sender · Subject": a window cut
    /// for the results' row drew only the words before the match.
    #[test]
    fn a_hits_passage_leads_with_its_match() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = searched(&mut focus, "atlas budget", atlas_budget(), &rows);
        let (ticket, request) = asks(&effects)
            .into_iter()
            .find(|(_, request)| matches!(request, Request::Passages { .. }))
            .expect("the hits' passages are asked for");
        let Request::Passages { stamp, .. } = request else {
            unreachable!()
        };
        let cut = "over. Could you initial page four as well? The movers come on \
                   Monday and we sign the Atlas budget on Friday at ten.";
        let at = cut.find("Atlas").unwrap();
        let passage = Passage {
            text: cut.to_owned(),
            ranges: vec![at..at + 5, at + 6..at + 12],
            elided_start: true,
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
        let detail = &view.sections[0].rows[1].detail;
        let before: String = detail
            .iter()
            .take_while(|run| !run.highlighted)
            .map(|run| run.text.as_str())
            .collect();
        assert!(
            before.chars().count() <= 24,
            "the match is drawn within the room: {before:?}"
        );
        assert!(
            before.starts_with('\u{2026}'),
            "what was cut is said: {before:?}"
        );
        assert_eq!(
            text(detail),
            "\u{2026}and we sign the Atlas budget on Friday at ten."
        );
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

        let second = view.sections[0].rows[1].token;
        let _ = focus.handle(Input::SearchHighlighted(second));
        assert!(focus.answers(CommandId::ForgetRecent));
        let effects = run(&mut focus, CommandId::ForgetRecent, &rows);
        assert!(asked(&effects).contains(&Request::ForgetSearch {
            query: "has:attachment in:Receipts after:2026-09-01".to_owned()
        }));
        let view = dropdown(&effects);
        assert_eq!(view.select, Some(view.sections[0].rows[1].token));
        assert_eq!(text(&view.sections[0].rows[1].title), "harbor");
        let effects = run(&mut focus, CommandId::ForgetRecent, &rows);
        let view = dropdown(&effects);
        assert_eq!(view.select, Some(view.sections[0].rows[0].token));
        let mut fresh = mac_search();
        let effects = opened(&mut fresh, &rows);
        assert!(!dropdown(&effects).sections[0].rows.is_empty());
        let effects = run(&mut fresh, CommandId::ForgetRecent, &rows);
        assert!(asked(&effects).contains(&Request::ForgetSearch {
            query: "atlas budget".to_owned()
        }));
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
            capped: false,
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
    fn a_single_letter_asks_the_index_nothing_yet() {
        // The first keystroke of "southwest" is "s": in most of a mailbox,
        // and over a large store a search for it ran for minutes and held
        // every later keystroke's search behind it.
        let rows = List::of(3);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = typed(&mut focus, "s", &rows);
        assert!(
            !asked(&effects).iter().any(is_conversations),
            "{:?}",
            asked(&effects)
        );
        let effects = typed(&mut focus, "so", &rows);
        assert!(
            asked(&effects).iter().any(is_conversations),
            "two letters are a word"
        );
    }

    #[test]
    fn what_the_bar_searches_for_forgives_a_near_word() {
        // A person searching: "tickt" should still find the ticket. A
        // rule's query is never forgiving (ADR 0037, as amended), so it is
        // the bar that asks for it.
        let rows = List::of(3);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = typed(&mut focus, "tickt", &rows);
        let forgiving = asked(&effects).iter().any(|request| {
            matches!(request, Request::Conversations { query, .. } if query.is_forgiving())
        });
        assert!(forgiving, "{:?}", asked(&effects));
    }

    #[test]
    fn a_hit_opened_walks_the_hits_and_closing_it_reopens_the_bar_on_it() {
        let rows = List::of(3);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = searched(&mut focus, "atlas budget", atlas_budget(), &rows);
        let second = dropdown(&effects).sections[0].rows[1].token;
        assert!(!focus.among_hits(MessageId::new(2)), "nothing is open yet");
        let _ = focus.handle_on(Input::BarRun(second), &rows);
        assert_eq!(focus.key_context(), KeyContext::Reader);
        assert!(focus.among_hits(MessageId::new(2)));
        assert!(
            focus.among_hits(MessageId::new(1)),
            "and so is the hit k walks to"
        );
        let effects = run(&mut focus, CommandId::PrevMessage, &rows);
        assert_eq!(
            shown(&effects),
            vec![Intent::OpenMessage {
                message: MessageId::new(1),
                index: 0,
                total: 4,
                host: Host::Own,
            }]
        );
        let effects = run(&mut focus, CommandId::Back, &rows);
        let intents = shown(&effects);
        assert!(intents.contains(&Intent::CloseSurface(SurfaceKind::Message)));
        assert!(
            intents.contains(&Intent::OpenBar {
                mode: BarMode::Search,
                text: "atlas budget".to_owned(),
                select: None,
            }),
            "{intents:?}"
        );
        assert!(
            !focus.among_hits(MessageId::new(1)),
            "closed, it is a row again"
        );
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
        assert_eq!(
            view.select,
            Some(view.sections[0].rows[0].token),
            "back where they left the hits"
        );
    }

    #[test]
    fn replying_from_a_hit_does_not_bring_the_bar_back_over_the_composer() {
        let rows = List::of(3);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = searched(&mut focus, "atlas budget", atlas_budget(), &rows);
        let second = dropdown(&effects).sections[0].rows[1].token;
        let _ = focus.handle_on(Input::BarRun(second), &rows);
        // On the Mac the composer replaces the message's window (M4): the
        // message "closes", but the words are not coming back under a
        // composer.
        let effects = run(&mut focus, CommandId::ReplyAll, &rows);
        let intents = shown(&effects);
        assert!(
            intents
                .iter()
                .any(|intent| matches!(intent, Intent::Composer { .. })),
            "{intents:?}"
        );
        assert!(
            !intents
                .iter()
                .any(|intent| matches!(intent, Intent::OpenBar { .. })),
            "the dropdown must not open over the composer: {intents:?}"
        );
        assert_eq!(focus.key_context(), KeyContext::Composer);
    }

    #[test]
    fn at_lists_correspondents_and_one_run_searches_their_mail() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let mut effects = run(&mut focus, CommandId::Search, &rows);
        let (ticket, _) = asks(&effects)
            .into_iter()
            .find(|(_, request)| *request == Request::Places)
            .expect("the places");
        let mut read = places();
        read.contacts = vec![postio_model::Contact::new(
            postio_model::ContactAddress::new(AddressId::new(1), ada()),
        )];
        effects.extend(focus.handle_on(Input::Reply(ticket, Reply::Places(Ok(read))), &rows));
        let effects = typed(&mut focus, "@ada", &rows);
        assert!(try_dropdown(&effects).is_none(), "spec 009's lines");
        let lines = view(&effects);
        let ada = line(&lines, "Ada Moreno").clone();
        assert_eq!(ada.kind, BarLineKind::Correspondent);
        let effects = focus.handle_on(Input::BarRun(ada.token), &rows);
        assert_eq!(
            shown(&effects)[0],
            Intent::OpenBar {
                mode: BarMode::Search,
                text: "with:ada@example.com".to_owned(),
                select: None,
            }
        );
        assert!(try_dropdown(&effects).is_some(), "the words' dropdown");
        assert!(asked(&effects).iter().any(is_conversations));
    }

    #[test]
    fn words_typed_over_the_palette_are_the_dropdowns() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = bar_open(&mut focus, CommandId::CommandPalette, &rows);
        let effects = typed(&mut focus, "atlas budget", &rows);
        assert_eq!(dropdown(&effects).state, DropdownState::Words);
        assert!(asked(&effects).iter().any(is_conversations));
        assert!(
            !run(&mut focus, CommandId::ShowAllResults, &rows).is_empty(),
            "and ⌘↩ shows their results"
        );
    }

    #[test]
    fn a_capped_search_is_remembered_and_recalled_as_a_floor() {
        let rows = List::of(3);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let mut broad = atlas_budget();
        broad.total = 5_000;
        broad.capped = true;
        let effects = searched(&mut focus, "atlas budget", broad, &rows);
        let second = dropdown(&effects).sections[0].rows[1].token;
        let effects = focus.handle_on(Input::BarRun(second), &rows);
        assert!(asked(&effects).contains(&Request::RememberSearch {
            query: "atlas budget".to_owned(),
            hits: 5_000,
            capped: true,
        }));

        let mut focus = mac_search();
        let _ = focus.handle(Input::SavedSearches(saved()));
        let effects = bar_open(&mut focus, CommandId::Search, &rows);
        let effects = answer(
            &mut focus,
            &effects,
            |request| *request == Request::RecentSearches,
            |_| {
                Reply::RecentSearches(Ok(vec![RecentSearch {
                    capped: true,
                    ..recent("atlas budget", 1, 5_000)
                }]))
            },
            &rows,
        );
        let view = dropdown(&effects);
        assert_eq!(text(&view.sections[0].rows[0].detail), "5,000+ results");
    }

    #[test]
    fn show_all_takes_the_dropdown_down_for_the_results() {
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
        assert!(asked(&effects).contains(&Request::RememberSearch {
            query: "atlas budget".to_owned(),
            hits: 48,
            capped: false,
        }));
    }

    #[test]
    fn words_that_find_nothing_say_so_and_lead_to_looser_searches() {
        let rows = List::of(3);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let nothing = ConversationResults {
            hits: Vec::new(),
            total: 0,
            files: 0,
            people: 0,
            facets: SearchFacets::default(),
            ..atlas_budget()
        };
        let effects = searched(&mut focus, "atlas has:attachment", nothing, &rows);
        let view = dropdown(&effects);
        let [section] = view.sections.as_slice() else {
            panic!("not one section: {view:?}");
        };
        let [row] = section.rows.as_slice() else {
            panic!("not one row: {section:?}");
        };
        assert_eq!(row.kind, DropdownRowKind::ShowAll);
        assert_eq!(text(&row.title), "Nothing matches both filters");
        assert_eq!(text(&row.detail), "Return offers looser searches");
        assert_eq!(
            row.key.as_deref(),
            focus.keymap().binding(CommandId::ShowAllResults)
        );
        assert_eq!(view.highlight, Some(row.token));
        let effects = focus.handle_on(Input::BarRun(row.token), &rows);
        assert!(focus.in_results(), "{effects:?}");
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
    fn commands_keep_spec_009s_lines_and_in_is_the_dropdowns() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = typed(&mut focus, ">mark", &rows);
        assert!(try_dropdown(&effects).is_none());
        let _ = view(&effects);
        let effects = typed(&mut focus, "in:Rec", &rows);
        assert_eq!(dropdown(&effects).state, DropdownState::Operator);
        let effects = typed(&mut focus, "", &rows);
        assert_eq!(dropdown(&effects).state, DropdownState::Empty, "and back");
    }

    fn is_suggest(request: &Request) -> bool {
        matches!(request, Request::Suggest { .. })
    }

    fn completion(text: &str, query: &str, count: u64) -> Completion {
        Completion {
            text: text.to_owned(),
            query: query.to_owned(),
            count,
            capped: false,
        }
    }

    fn at_suggestions() -> Suggestions {
        let mut files = vec![
            completion("Atlas-Q3-budget.xlsx", "filename:atlas", 1),
            completion("Atlas-Sep-actuals.pdf", "filename:atlas", 1),
        ];
        files.extend((0..16).map(|n| completion(&format!("at-{n}.pdf"), "filename:at", 1)));
        Suggestions {
            ghost: Some("las".to_owned()),
            words: vec![completion("atlas", "atlas", 62)],
            labels: vec![completion("Atlas", "label:Atlas", 30)],
            lists: vec![completion(
                "atlas-planning.example.org",
                "list:atlas-planning.example.org",
                14,
            )],
            files,
            ..Suggestions::default()
        }
    }

    fn suggested(
        focus: &mut FocusController,
        effects: &[Effect],
        found: Suggestions,
        rows: &List,
    ) -> Vec<Effect> {
        answer(
            focus,
            effects,
            is_suggest,
            |request| {
                let Request::Suggest { stamp, .. } = request else {
                    unreachable!()
                };
                Reply::Suggest {
                    stamp: *stamp,
                    answer: Ok(Box::new(found)),
                }
            },
            rows,
        )
    }

    fn landed(
        focus: &mut FocusController,
        effects: &[Effect],
        results: ConversationResults,
        rows: &List,
    ) -> Vec<Effect> {
        answer(
            focus,
            effects,
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

    fn at_results() -> ConversationResults {
        let mut results = atlas_budget();
        results.hits.truncate(3);
        results.total = 214;
        results.elapsed = std::time::Duration::from_millis(12);
        results
    }

    #[test]
    fn a_capped_suggestion_count_reads_as_a_floor() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = typed(&mut focus, "a", &rows);
        let mut found = Suggestions {
            ghost: Some("s".to_owned()),
            words: vec![completion("as", "as", 1_000)],
            labels: vec![completion("Accounts", "label:Accounts", 4_512)],
            ..Suggestions::default()
        };
        found.words[0].capped = true;
        let effects = suggested(&mut focus, &effects, found, &rows);
        let view = dropdown(&effects);
        let ghost_row = &view.sections[0].rows[0];
        assert_eq!(ghost_row.key.as_deref(), Some("Tab"), "the ghost's word");
        assert_eq!(ghost_row.right.as_deref(), Some("1,000+"));
        assert_eq!(
            view.sections[0].rows[1].right.as_deref(),
            Some("4,512"),
            "an exact count, grouped as every count is"
        );
    }

    #[test]
    fn a_short_prefix_draws_the_ghost_suggestions_top_hits_so_far_and_show_all() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = typed(&mut focus, "at", &rows);
        let suggest = asked(&effects)
            .into_iter()
            .find(is_suggest)
            .expect("a prefix asks what it could become");
        assert!(
            matches!(&suggest, Request::Suggest { prefix, field: None, .. } if prefix == "at"),
            "{suggest:?}"
        );
        assert_eq!(suggest.lane(), Some(Lane::Suggest));
        let Request::Conversations { limit, .. } = asked(&effects)
            .into_iter()
            .find(is_conversations)
            .expect("and the top hits so far")
        else {
            unreachable!()
        };
        assert_eq!(limit, 3, "screen 02's three");
        assert_eq!(dropdown(&effects).state, DropdownState::Prefix);

        let more = suggested(&mut focus, &effects, at_suggestions(), &rows);
        let effects = [effects, more].concat();
        let effects = landed(&mut focus, &effects, at_results(), &rows);
        let view = dropdown(&effects);
        assert_eq!(view.state, DropdownState::Prefix);
        assert_eq!(view.ghost.as_deref(), Some("las"), "at|las");
        assert_eq!(view.operator, None, "words, no operator");
        assert_eq!(
            view.ghost_hint,
            Some(postio_ui::search_view::ghost_hint()),
            "Tab completes, in the field beside the ghost"
        );
        let titles: Vec<&str> = view.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Suggestions", "Top hits so far", ""]);
        assert_eq!(
            view.sections[0].note.as_deref(),
            Some("complete the word, or jump to a filter")
        );
        assert_eq!(
            view.sections[1].note.as_deref(),
            Some("update on every keystroke")
        );

        let rows_of = &view.sections[0].rows;
        let kinds: Vec<DropdownRowKind> = rows_of.iter().map(|row| row.kind).collect();
        assert_eq!(
            kinds,
            [
                DropdownRowKind::Word,
                DropdownRowKind::Label,
                DropdownRowKind::List,
                DropdownRowKind::File
            ]
        );
        let word = &rows_of[0];
        assert_eq!(text(&word.title), "atlas");
        let lit: Vec<&str> = word
            .title
            .iter()
            .filter(|run| run.highlighted)
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(lit, ["at"], "what was typed, in the find highlight");
        assert_eq!(text(&word.detail), "as a word");
        assert_eq!(word.right.as_deref(), Some("62"));
        assert_eq!(word.key.as_deref(), Some("Tab"));
        assert_eq!(view.highlight, Some(word.token), "the word is focused");

        let label = &rows_of[1];
        assert_eq!(text(&label.title), "label:Atlas");
        assert!(label.title.iter().all(|run| run.style == RunStyle::Mono));
        assert_eq!(text(&label.detail), "label");
        assert_eq!(label.right.as_deref(), Some("30"));
        assert_eq!(
            text(&rows_of[2].detail),
            "mailing list \u{b7} atlas-planning.example.org"
        );
        assert_eq!(rows_of[2].right.as_deref(), Some("14"));
        let files = &rows_of[3];
        assert_eq!(text(&files.title), "Files named \u{201c}at\u{2026}\u{201d}");
        assert_eq!(
            text(&files.detail),
            "Atlas-Q3-budget.xlsx, Atlas-Sep-actuals.pdf and 16 more"
        );
        assert_eq!(files.right.as_deref(), Some("18"));

        assert_eq!(view.sections[1].rows.len(), 3);
        assert_eq!(
            text(&view.sections[2].rows[0].title),
            "Show all 214 results for \u{201c}at\u{201d}"
        );
        assert_eq!(
            view.hints,
            postio_ui::search_view::prefix_hints(focus.keymap())
        );
        assert_eq!(view.count.as_deref(), Some("214 matches \u{b7} 12 ms"));

        let effects = focus.handle(Input::BarTab);
        assert!(
            shown(&effects).contains(&Intent::OpenBar {
                mode: BarMode::Search,
                text: "atlas".to_owned(),
                select: None,
            }),
            "{effects:?}"
        );
        assert!(asked(&effects).into_iter().any(|r| is_conversations(&r)));
        assert_eq!(dropdown(&effects).state, DropdownState::Words);
    }

    #[test]
    fn a_suggestion_run_puts_its_query_in_the_field() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let asked_at = typed(&mut focus, "at", &rows);
        let effects = suggested(&mut focus, &asked_at, at_suggestions(), &rows);
        let label = dropdown(&effects).sections[0].rows[1].token;
        let effects = focus.handle_on(Input::BarRun(label), &rows);
        assert!(
            shown(&effects).contains(&Intent::OpenBar {
                mode: BarMode::Search,
                text: "label:Atlas ".to_owned(),
                select: None,
            }),
            "the chip, and room to type on: {effects:?}"
        );
        assert!(try_dropdown(&suggested(&mut focus, &asked_at, at_suggestions(), &rows)).is_none());
        let _ = typed(&mut focus, "zq", &rows);
        assert!(focus.handle(Input::BarTab).is_empty());
    }

    fn person(name: &str, address: &str, received: u64, sent: u64) -> Person {
        Person {
            name: Some(name.to_owned()),
            address: address.to_owned(),
            received,
            sent,
            last: Some(today().to_utc()),
        }
    }

    fn people() -> Suggestions {
        Suggestions {
            people: vec![
                person("Ada Moreno", "ada@example.com", 400, 12),
                person("Ben Adeyemi", "ben@example.net", 80, 8),
            ],
            ..Suggestions::default()
        }
    }

    fn is_latest(request: &Request) -> bool {
        matches!(request, Request::LatestFrom { .. })
    }

    #[test]
    fn from_with_nobody_matching_says_so_and_drops_the_latest() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = typed(&mut focus, "from:", &rows);
        let effects = suggested(&mut focus, &effects, people(), &rows);
        let Request::LatestFrom { address, stamp } =
            asked(&effects).into_iter().find(is_latest).expect("Ada's")
        else {
            unreachable!()
        };
        let mut hers = atlas_budget();
        hers.hits.truncate(2);
        let effects = answer(
            &mut focus,
            &effects,
            is_latest,
            |_| Reply::LatestFrom {
                stamp,
                address: address.clone(),
                answer: Ok(Box::new(hers)),
            },
            &rows,
        );
        assert_eq!(dropdown(&effects).sections.len(), 2, "her latest is shown");

        let effects = typed(&mut focus, "from:zzz", &rows);
        let effects = suggested(&mut focus, &effects, Suggestions::default(), &rows);
        let view = dropdown(&effects);
        let [people] = view.sections.as_slice() else {
            panic!("not the one section: {:?}", view.sections);
        };
        assert_eq!(people.title, "People matching \u{201c}zzz\u{201d}");
        assert_eq!(people.note.as_deref(), Some("No one matches"));
        assert!(people.rows.is_empty());
        assert!(
            !view.hints.iter().any(|hint| hint.label == "add as chip"),
            "Return adds nothing: {:?}",
            view.hints
        );
    }

    #[test]
    fn from_lists_people_and_the_latest_from_the_highlighted_one() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = typed(&mut focus, "from:", &rows);
        let suggest = asked(&effects)
            .into_iter()
            .find(is_suggest)
            .expect("people are asked");
        assert!(
            matches!(&suggest, Request::Suggest { prefix, field: Some(Field::From), .. } if prefix.is_empty()),
            "{suggest:?}"
        );
        assert!(
            !asked(&effects).into_iter().any(|r| is_conversations(&r)),
            "an operator with no value searches nothing yet"
        );
        let effects = suggested(&mut focus, &effects, people(), &rows);
        let view = dropdown(&effects);
        assert_eq!(view.state, DropdownState::Operator);
        assert_eq!(
            view.operator.as_deref(),
            Some("from:"),
            "the field draws the operator tertiary, its value primary"
        );
        assert_eq!(view.sections[0].title, "People");
        assert_eq!(
            view.sections[0].note.as_deref(),
            Some("by how often you write to each other")
        );
        let ada = &view.sections[0].rows[0];
        assert_eq!(ada.kind, DropdownRowKind::Person);
        assert_eq!(text(&ada.title), "Ada Moreno");
        assert_eq!(
            text(&ada.detail),
            "ada@example.com \u{b7} 412 messages \u{b7} last today"
        );
        assert_eq!(ada.initials.as_deref(), Some("AM"));
        assert_eq!(view.highlight, Some(ada.token));
        assert!(
            view.sections[0]
                .rows
                .iter()
                .all(|row| row.highlighted == Some(postio_ui::search_view::adds_hint())),
            "whichever person is highlighted says Return adds"
        );
        assert_eq!(
            view.hints,
            postio_ui::search_view::operator_hints(focus.keymap(), "from")
        );
        assert_eq!(
            view.count.as_deref(),
            Some("Contacts and everyone you have mail with")
        );
        let latest = asked(&effects)
            .into_iter()
            .find(is_latest)
            .expect("the latest from the focused person");
        assert_eq!(
            latest,
            Request::LatestFrom {
                address: "ada@example.com".to_owned(),
                stamp: match &latest {
                    Request::LatestFrom { stamp, .. } => *stamp,
                    _ => unreachable!(),
                },
            }
        );
        assert_eq!(latest.lane(), Some(Lane::Latest));

        let ben = view.sections[0].rows[1].token;
        let effects = focus.handle(Input::SearchHighlighted(ben));
        let Request::LatestFrom { address, stamp } =
            asked(&effects).into_iter().find(is_latest).expect("Ben's")
        else {
            unreachable!()
        };
        assert_eq!(address, "ben@example.net");
        let mut his = atlas_budget();
        his.hits.truncate(2);
        let effects = answer(
            &mut focus,
            &effects,
            is_latest,
            |_| Reply::LatestFrom {
                stamp,
                address: "ben@example.net".to_owned(),
                answer: Ok(Box::new(his)),
            },
            &rows,
        );
        let view = dropdown(&effects);
        assert_eq!(view.sections[1].title, "Latest from Ben Adeyemi");
        assert_eq!(
            view.sections[1].note.as_deref(),
            Some("preview of the focused person")
        );
        assert_eq!(view.sections[1].rows.len(), 2);
        assert!(
            view.sections[1]
                .rows
                .iter()
                .all(|row| row.kind == DropdownRowKind::Hit)
        );

        let effects = focus.handle_on(Input::BarRun(ben), &rows);
        assert!(
            shown(&effects).contains(&Intent::OpenBar {
                mode: BarMode::Search,
                text: "from:ben@example.net ".to_owned(),
                select: None,
            }),
            "{effects:?}"
        );
        let effects = typed(&mut focus, "atlas from:ad", &rows);
        assert!(asked(&effects).into_iter().any(|request| matches!(
            request,
            Request::Suggest { prefix, field: Some(Field::From), .. } if prefix == "ad"
        )));
        let effects = suggested(&mut focus, &effects, people(), &rows);
        assert_eq!(
            dropdown(&effects).sections[0].title,
            "People matching \u{201c}ad\u{201d}"
        );
        let ada = dropdown(&effects).sections[0].rows[0].token;
        let effects = focus.handle(Input::SearchExclude(ada));
        assert!(
            shown(&effects).contains(&Intent::OpenBar {
                mode: BarMode::Search,
                text: "atlas -from:ada@example.com ".to_owned(),
                select: None,
            }),
            "the exclusion replaces what was typed after the words: {effects:?}"
        );

        let effects = typed(&mut focus, "from:", &rows);
        let effects = suggested(&mut focus, &effects, people(), &rows);
        let ben = dropdown(&effects).sections[0].rows[1].token;
        let _ = focus.handle(Input::SearchHighlighted(ben));
        assert!(focus.answers(CommandId::ExcludeSuggestion));
        let effects = run(&mut focus, CommandId::ExcludeSuggestion, &rows);
        assert!(shown(&effects).contains(&Intent::OpenBar {
            mode: BarMode::Search,
            text: "-from:ben@example.net ".to_owned(),
            select: None,
        }));
    }

    #[test]
    fn backspace_on_an_empty_value_goes_back_to_words() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let _ = typed(&mut focus, "atlas from:", &rows);
        let effects = typed(&mut focus, "atlas from", &rows);
        assert!(
            shown(&effects).contains(&Intent::OpenBar {
                mode: BarMode::Search,
                text: "atlas ".to_owned(),
                select: None,
            }),
            "{effects:?}"
        );
        assert_eq!(dropdown(&effects).state, DropdownState::Words);
        let _ = typed(&mut focus, "from:", &rows);
        let effects = typed(&mut focus, "from", &rows);
        assert!(
            shown(&effects).contains(&Intent::OpenBar {
                mode: BarMode::Search,
                text: String::new(),
                select: None,
            }),
            "{effects:?}"
        );
        let _ = typed(&mut focus, "from:a", &rows);
        let effects = typed(&mut focus, "from:", &rows);
        assert!(
            !shown(&effects)
                .iter()
                .any(|intent| matches!(intent, Intent::OpenBar { .. }))
        );
        assert_eq!(dropdown(&effects).state, DropdownState::Operator);
    }

    #[test]
    fn label_and_in_list_labels_and_folders_the_same_way() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = typed(&mut focus, "label:At", &rows);
        assert!(asked(&effects).into_iter().any(|request| matches!(
            request,
            Request::Suggest { prefix, field: Some(Field::Label), .. } if prefix == "At"
        )));
        let effects = suggested(
            &mut focus,
            &effects,
            Suggestions {
                labels: vec![completion("Atlas", "label:Atlas", 30)],
                ..Suggestions::default()
            },
            &rows,
        );
        let view = dropdown(&effects);
        assert_eq!(view.state, DropdownState::Operator);
        assert_eq!(view.sections[0].title, "Labels matching \u{201c}At\u{201d}");
        let atlas = &view.sections[0].rows[0];
        assert_eq!(atlas.kind, DropdownRowKind::Label);
        assert_eq!(text(&atlas.title), "Atlas");
        assert_eq!(atlas.right.as_deref(), Some("30"));
        assert_eq!(
            view.hints,
            postio_ui::search_view::operator_hints(focus.keymap(), "label")
        );
        let effects = focus.handle_on(Input::BarRun(atlas.token), &rows);
        assert!(shown(&effects).contains(&Intent::OpenBar {
            mode: BarMode::Search,
            text: "label:Atlas ".to_owned(),
            select: None,
        }));

        let effects = typed(&mut focus, "in:Rec", &rows);
        assert!(asked(&effects).into_iter().any(|request| matches!(
            request,
            Request::Suggest { prefix, field: Some(Field::In), .. } if prefix == "Rec"
        )));
        let effects = suggested(
            &mut focus,
            &effects,
            Suggestions {
                folders: vec![completion("Receipts", "in:Receipts", 19)],
                ..Suggestions::default()
            },
            &rows,
        );
        let receipts = &dropdown(&effects).sections[0].rows[0];
        assert_eq!(receipts.kind, DropdownRowKind::Folder);
        assert_eq!(text(&receipts.title), "Receipts");
        let effects = focus.handle(Input::SearchExclude(receipts.token));
        assert!(shown(&effects).contains(&Intent::OpenBar {
            mode: BarMode::Search,
            text: "-in:Receipts ".to_owned(),
            select: None,
        }));
    }

    #[test]
    fn plain_english_shows_what_it_understood_and_tab_makes_chips() {
        let rows = List::of(1);
        let mut focus = mac_search();
        let _ = opened(&mut focus, &rows);
        let effects = typed(&mut focus, "invoices last month", &rows);
        let Request::Conversations { query, order, .. } = asked(&effects)
            .into_iter()
            .find(is_conversations)
            .expect("the sentence is searched as what it became")
        else {
            unreachable!()
        };
        assert_eq!(query.filters().count(), 2, "after: and before:");
        assert_eq!(
            order,
            postio_search::results::ConversationOrder::Newest,
            "the design's results are newest first"
        );
        let view = dropdown(&effects);
        assert_eq!(view.state, DropdownState::PlainEnglish);
        let tiles: Vec<(String, String, String)> = view
            .understood
            .iter()
            .map(|tile| (tile.op.clone(), tile.value.clone(), tile.origin.clone()))
            .collect();
        assert_eq!(
            tiles,
            [
                (
                    String::new(),
                    "invoices".to_owned(),
                    "from \u{2018}invoices\u{2019}".to_owned()
                ),
                (
                    "after:".to_owned(),
                    "2026-08-01".to_owned(),
                    "from \u{2018}last month\u{2019}".to_owned()
                ),
                (
                    "before:".to_owned(),
                    "2026-09-01".to_owned(),
                    "from \u{2018}last month\u{2019}".to_owned()
                ),
            ],
            "one tile per term, each with the exact words it came from"
        );
        assert_eq!(
            view.hints,
            postio_ui::search_view::plain_hints(focus.keymap())
        );

        let mut found = atlas_budget();
        found.hits.truncate(3);
        found.total = 3;
        found.elapsed = std::time::Duration::from_millis(21);
        let effects = landed(&mut focus, &effects, found, &rows);
        let view = dropdown(&effects);
        let titles: Vec<&str> = view.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Results", ""]);
        assert_eq!(
            view.sections[0].note.as_deref(),
            Some("August 2026 \u{b7} newest first")
        );
        assert_eq!(
            view.count.as_deref(),
            Some("parsed on this Mac \u{b7} 3 matches \u{b7} 21 ms")
        );
        assert_eq!(
            view.highlight,
            Some(view.sections[0].rows[0].token),
            "the first result is focused"
        );

        let effects = typed(&mut focus, "invoices from:ada last month", &rows);
        let mut found = atlas_budget();
        found.hits.truncate(1);
        let effects = landed(&mut focus, &effects, found, &rows);
        let view = dropdown(&effects);
        assert_eq!(view.state, DropdownState::PlainEnglish);
        assert!(
            view.sections[0].rows[0]
                .title
                .iter()
                .all(|run| !run.highlighted),
            "{:?}",
            view.sections[0].rows[0].title
        );
        let _ = typed(&mut focus, "invoices last month", &rows);

        let effects = focus.handle(Input::BarTab);
        assert!(
            shown(&effects).contains(&Intent::OpenBar {
                mode: BarMode::Search,
                text: "invoices after:2026-08-01 before:2026-09-01".to_owned(),
                select: None,
            }),
            "{effects:?}"
        );
        assert_eq!(dropdown(&effects).state, DropdownState::Words);
        assert!(dropdown(&effects).understood.is_empty());

        let _ = typed(&mut focus, "invoices last month", &rows);
        let effects = run(&mut focus, CommandId::BackToWords, &rows);
        let view = dropdown(&effects);
        assert_eq!(view.state, DropdownState::Words);
        assert!(view.understood.is_empty());
        let Request::Conversations { query, .. } = asked(&effects)
            .into_iter()
            .find(is_conversations)
            .expect("searched again")
        else {
            unreachable!()
        };
        assert_eq!(
            query.filters().count(),
            0,
            "no operators: the words as typed"
        );
        assert!(
            !shown(&effects)
                .iter()
                .any(|intent| matches!(intent, Intent::OpenBar { .. })),
            "the field keeps the sentence"
        );
    }

    #[test]
    fn the_gtk_bar_is_the_dropdown() {
        let rows = List::of(1);
        let mut focus = FocusController::new(Policy::for_platform(Platform::Freedesktop));
        let effects = run(&mut focus, CommandId::Search, &rows);
        assert!(asked(&effects).contains(&Request::RecentSearches));
        let _ = typed(&mut focus, "atlas budget", &rows);
        assert!(focus.answers(CommandId::ShowAllResults));
    }
}

fn save(query: &str) -> postio_focus::SaveSearch {
    postio_focus::SaveSearch {
        query: query.to_owned(),
        name: None,
        pin: true,
        notify: false,
        dates: postio_ui::saved_search::Dates::AsTyped,
    }
}

#[test]
fn a_save_with_no_file_says_so_and_writes_nothing() {
    assert_eq!(
        postio_focus::save_search(None, &save("from:ada")),
        Err(postio_ui::focus_target::NO_CONFIG_TO_SAVE.to_owned())
    );
}

#[test]
fn a_save_writes_the_search_pinned_and_answers_with_every_pinned_one() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("config.toml");
    std::fs::write(&path, "[ui]\ntheme = \"dark\"\n").expect("the file");

    let saved = postio_focus::save_search(Some(&path), &save("from:ada")).expect("saved");

    assert_eq!(
        saved
            .searches
            .iter()
            .map(|search| search.query.as_str())
            .collect::<Vec<_>>(),
        ["from:ada"]
    );
    assert!(saved.key.is_some(), "the key it was written under");
    let written = std::fs::read_to_string(&path).expect("the file");
    assert!(written.contains("theme = \"dark\""), "{written}");
    assert!(written.contains("from:ada"), "{written}");
}
