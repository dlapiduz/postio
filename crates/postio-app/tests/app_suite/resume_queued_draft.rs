//! Pressing Return on a *queued* draft in the Drafts folder must not reopen
//! it live while a send for it might still drain (#433).
//!
//! The sibling of `resume_draft.rs`: that one proves an ordinary draft opens
//! in the composer at all; this one proves that a draft still sitting in the
//! Drafts folder because its send has not drained yet does not hand the user
//! an editable buffer the drainer could build outgoing bytes from at the same
//! moment — cancelling the pending `Operation::Send` is what makes reopening
//! it safe. `postio-storage`'s `tests/drafts.rs` covers
//! `DraftRepository::cancel_send` on its own; this is the same "does the road
//! join up" check `resume_draft.rs` exists for, run against the queued path.
//!
//! Nothing here touches the network: `start_syncing` is never called, so
//! nothing ever picks the queued operation up on its own — the only thing
//! that removes it in this test is the resume path under test.
//!
//! One test function: GTK is single-threaded and initialised once per binary.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, actions, commands, compose, feed_the_window};
use postio_core::CommandId;
use postio_core::bridge::{Bridge, event_channel};
use postio_core::state::SharedState;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{Draft, DraftState, EmailAddress, MailboxRole, OperationTarget};
use postio_storage::repository::{DraftRepository, OperationQueueRepository};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

const SUBJECT: &str = "Tide gate interlock";

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

pub fn return_on_a_queued_draft_row_cancels_the_send_and_reopens_it_for_editing() {
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
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    // A draft handed to the operation queue for sending -- exactly what
    // `Composer::send` leaves behind, and never drained, so it is still
    // sitting in Drafts when the test activates its row.
    let (draft_id, queued_id) = {
        let connection = database.connection().expect("a connection");
        let mut draft = Draft::new(account);
        draft.subject = SUBJECT.to_owned();
        draft.to = vec![EmailAddress::new(None::<String>, "quinn@example.net")];
        draft.body.text = Some("Ready to go.".to_owned());
        let drafts = DraftRepository::new(&connection);
        drafts.save(&mut draft).expect("save the draft");
        let queued = drafts
            .queue_send(&mut draft, chrono::Utc::now())
            .expect("queue the send");
        assert_eq!(draft.state, DraftState::Queued);
        (draft.id, queued.id)
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
    click_folder(
        &window,
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
    assert!(
        expected > 0,
        "the queued draft never got a row to be listed by"
    );
    assert!(
        settle_until(|| list.model().n_items() == expected),
        "the Drafts folder drew {} rows and the store holds {expected}",
        list.model().n_items()
    );

    // ── put the cursor on the queued draft and press Return ─────────────
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
    assert!(
        found,
        "never found the queued draft's row in the Drafts folder"
    );

    list.test_activate_cursor();
    let opened = settle_until(|| window.composer().is_open());

    assert!(
        opened,
        "Return on the queued draft's row left the composer closed -- a \
         queued draft must still be reachable for editing, just not while \
         its send is still in the queue unresolved"
    );
    assert_eq!(window.composer().test_subject(), SUBJECT);

    let connection = database.connection().expect("a connection");
    assert_eq!(
        DraftRepository::new(&connection)
            .get(draft_id)
            .expect("get")
            .expect("still here")
            .state,
        DraftState::Editing,
        "opening a queued draft must cancel its pending send and return it \
         to Editing -- otherwise the drainer could still build outgoing \
         bytes from the row while the user is mid-edit"
    );
    assert!(
        OperationQueueRepository::new(&connection)
            .get(queued_id)
            .expect("get")
            .is_none(),
        "the Send operation this draft was queued under must be gone, or a \
         second, different message could still go out behind the one the \
         user is now editing"
    );
    assert!(
        !OperationQueueRepository::new(&connection)
            .has_pending(OperationTarget::Draft(draft_id))
            .expect("has_pending"),
        "nothing should still be queued against this draft"
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
