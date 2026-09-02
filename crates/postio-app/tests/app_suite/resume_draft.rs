//! Pressing Return on a draft in the Drafts folder, all the way to the composer.
//!
//! The sibling of `keystroke.rs` and `bulk_keystroke.rs`, for #166. The chain
//! is: `DraftRepository::save` writes the row that puts a draft in the folder;
//! the list pages it like any other message; activating it finds the draft
//! behind the row and hands it to the composer instead of to the reader.
//!
//! Every joint has its own tests — `postio-storage`'s `tests/drafts.rs` for
//! both halves of the link, `postio-gtk`'s `tests/gtk_composer_resume.rs` for
//! the composer's side. What none of them can see is whether the road joins
//! up, which is the failure `keystroke.rs` exists for: before this, activating
//! a draft's row opened the *reader* on a snapshot of a buffer, and every one
//! of those tests still passed.
//!
//! Nothing here touches the network: `start_syncing` is never called, so the
//! queue row the save leaves simply waits.
//!
//! One test function: GTK is single-threaded and initialised once per binary.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, actions, commands, compose, feed_the_window};
use postio_core::CommandId;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{Draft, EmailAddress, MailboxRole};
use postio_storage::repository::DraftRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

const SUBJECT: &str = "Tide gate interlock";

pub fn return_on_a_draft_row_opens_the_composer_on_that_draft() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let report = seed_small(&database, 9);
    let account = report.account.id;
    let drafts_folder = report
        .mailbox(MailboxRole::Drafts)
        .expect("the fixture has a Drafts folder")
        .clone();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    // A draft, written and autosaved, exactly as the composer's own wiring
    // would have left it — and never uploaded, so nothing about this depends
    // on a server having been reachable.
    let draft_id = {
        let connection = database.connection().expect("a connection");
        let mut draft = Draft::new(account);
        draft.subject = SUBJECT.to_owned();
        draft.to = vec![EmailAddress::new(None::<String>, "quinn@example.net")];
        draft.body.text = Some("Half a sentence, still being had.".to_owned());
        DraftRepository::new(&connection)
            .save(&mut draft)
            .expect("save the draft")
    };

    let state = SharedState::default();
    let bus = actions::wire(
        postio_core::dispatch::DispatcherBuilder::new(),
        actions::Actions::new(database.clone(), state.clone()),
    )
    .build();
    let wired: Vec<CommandId> = bus.wired().collect();
    let (bridge, _replies) = Bridge::new(bus).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs.clone(),
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let feeds = feed_the_window(&window, &wiring)
        .expect("the seeded store has an account")
        .feeds;
    commands::install(&window, &feeds, state, wiring.commands.clone(), wired);
    // The wiring under test. `recover` runs inside it and reopens the draft,
    // so the composer is closed first: a composer that was already holding
    // this draft would make the assertion below true without anything having
    // been activated.
    compose::install(
        &window,
        account,
        database.clone(),
        blobs,
        bridge.handle(),
        postio_app::reading::Showing::default(),
    );
    window.composer().close();
    while glib::MainContext::default().iteration(false) {}
    assert!(
        !window.composer().is_open(),
        "the composer has to start closed or this test cannot mean anything"
    );

    // ── open the Drafts folder ──────────────────────────────────────────
    // Clicked, not `Sidebar::select`: that one is documented as selecting
    // "without reporting it back as a user action", so the feed would never
    // hear and the list would go on showing the inbox.
    click_folder(
        &window,
        // Among only itself: the seed has one folder per role, so the
        // Drafts folder is trivially its role's primary (#501).
        &postio_gtk::sidebar::display_name(&drafts_folder, std::slice::from_ref(&drafts_folder)),
    );
    let list = window.list();
    let expected = {
        let connection = database.connection().expect("a connection");
        postio_storage::repository::MessageRepository::new(&connection)
            .count(&postio_storage::repository::ListQuery {
                scope: postio_storage::repository::ListScope::Mailbox(drafts_folder.id),
                limit: 50,
                after: None,
            })
            .expect("a count")
    };
    assert!(expected > 0, "the draft never got a row to be listed by");
    assert!(
        settle_until(|| list.model().n_items() == expected),
        "the Drafts folder drew {} rows and the store holds {expected}",
        list.model().n_items()
    );

    // ── put the cursor on the draft and press Return ────────────────────
    // `j` down the folder until the cursor is on a row that leads back to a
    // draft. The fixture seeds other mail into Drafts, so the draft is not
    // reliably the first row and finding it by walking is what a person does.
    let is_the_draft = || {
        let Some(id) = list.cursor_id() else {
            return false;
        };
        let connection = database.connection().expect("a connection");
        DraftRepository::new(&connection)
            .by_message(id)
            .ok()
            .flatten()
            .is_some_and(|draft| draft.id == draft_id)
    };
    // `g g` first: the cursor sits on the top row before any key is pressed,
    // so a bare `j` starts by *leaving* it — and the newest row is where a
    // draft saved a moment ago is.
    for key in ["g", "g"] {
        window.handle_key(
            gdk::Key::from_name(key).unwrap(),
            gdk::ModifierType::empty(),
        );
    }
    while glib::MainContext::default().iteration(false) {}
    let mut found = is_the_draft();
    for _ in 0..list.model().n_items() {
        if found {
            break;
        }
        window.handle_key(
            gdk::Key::from_name("j").unwrap(),
            gdk::ModifierType::empty(),
        );
        while glib::MainContext::default().iteration(false) {}
        found = is_the_draft();
    }
    assert!(found, "never found the draft's row in the Drafts folder");

    // `Return`, through `GtkListView`'s own `list.activate-item` action. A key
    // put through `Window::handle_key` never reaches the widget, and the
    // keyboard is not in the list in a test that has been driving it by hand.
    list.test_activate_cursor();
    let opened = settle_until(|| window.composer().is_open());

    assert!(
        opened,
        "Return on the draft's row left the composer closed. Before #166 this \
         opened the reader on a snapshot of the buffer instead, which is a \
         message you can look at and not finish."
    );
    assert_eq!(
        window.composer().test_subject(),
        SUBJECT,
        "and it is the draft that row was listing, not a fresh composition"
    );

    bridge.shutdown();
}

/// Select a folder the way a pointer does, so the sidebar reports it.
fn click_folder(window: &Window, label: &str) {
    let row = folder_rows(window)
        .into_iter()
        .find(|(text, _)| text == label)
        .map(|(_, row)| row)
        .unwrap_or_else(|| panic!("the sidebar draws a {label} row"));
    let list = row
        .parent()
        .and_then(|parent| parent.downcast::<gtk::ListBox>().ok())
        .expect("a folder row lives in a list box");
    list.select_row(Some(&row));
}

fn folder_rows(window: &Window) -> Vec<(String, gtk::ListBoxRow)> {
    let mut found = Vec::new();
    walk(
        window.sidebar().upcast_ref::<gtk::Widget>(),
        &mut |widget| {
            if let Some(row) = widget.downcast_ref::<gtk::ListBoxRow>()
                && let Some(label) = first_label(row.upcast_ref::<gtk::Widget>())
            {
                found.push((label, row.clone()));
            }
        },
    );
    found
}

fn first_label(widget: &gtk::Widget) -> Option<String> {
    let mut found = None;
    walk(widget, &mut |node| {
        if found.is_none()
            && let Some(label) = node.downcast_ref::<gtk::Label>()
            && !label.text().is_empty()
        {
            found = Some(label.text().to_string());
        }
    });
    found
}

fn walk(widget: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(node) = child {
        walk(&node, visit);
        child = node.next_sibling();
    }
}
