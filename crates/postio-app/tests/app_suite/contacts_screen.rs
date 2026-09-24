//! The Contacts screen, driven the way a person drives it
//! (specs/005-contacts User Story 1).
//!
//! A store with mail in it, the same install the binary runs, and then the
//! keys: `g c` shows the people built from that mail -- the ones the user made
//! or wrote to, marked for which is which -- typing narrows them, `v e` shows
//! everyone from mail, "show mail" leaves a `with:` search in the box that
//! finds both directions of the correspondence, and `Esc` returns to exactly
//! where the keyboard was. And nothing left the machine while it happened.
//!
//! Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::{settle, settle_until};
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_core::Context;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::finder::Mode;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{ContactSource, EmailAddress, Message};
use postio_session::{Wiring, ensure_search_index};
use postio_storage::repository::{
    AccountRepository, ContactRepository, MailboxRepository, MessageRepository,
};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

fn press(window: &Window, key: &str) {
    window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
}

/// The names the Contacts list draws, top to bottom, as far as its rows have
/// arrived.
fn drawn(window: &Window) -> Vec<String> {
    let model = window.contacts().model();
    (0..model.n_items())
        .filter_map(|position| model.row(position).map(|row| row.name))
        .collect()
}

pub fn g_c_shows_the_people_from_the_mail_and_esc_returns() {
    crate::gtk_case(async {
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

        // ── a store with mail, one person the user made, one they wrote to ──
        let database = test_support::memory().await;
        seed_small(&database, 11).await;
        {
            let connection = database.connect().await.expect("a connection");
            let account = AccountRepository::new(&connection)
                .list()
                .await
                .expect("accounts")
                .into_iter()
                .next()
                .expect("the seed makes an account");
            let sent_folder = MailboxRepository::new(&connection)
                .list_for_account(account.id)
                .await
                .expect("folders")
                .into_iter()
                .find(|m| m.role == postio_model::MailboxRole::Sent)
                .expect("a Sent folder");
            let mut sent = Message::new(account.id, sent_folder.id, chrono::Utc::now());
            sent.from = vec![account.address.clone()];
            sent.to = vec![EmailAddress::new(Some("Quinn Abara"), "quinn@example.net")];
            sent.subject = Some("Lunch on Thursday".into());
            MessageRepository::new(&connection)
                .create(&mut sent)
                .await
                .expect("a sent message");
            ContactRepository::new(&connection)
                .record_message(&sent, std::slice::from_ref(&account.address))
                .await
                .expect("recorded as sync would");
            ContactRepository::new(&connection)
                .create(
                    Some("Ada Lovelace"),
                    &[EmailAddress::new(None::<String>, "ada@analytical.example")],
                )
                .await
                .expect("a person the user made");
        }
        // After the extra mail, as opening a synced store would have it: the
        // index covers every message the store holds.
        ensure_search_index(&database)
            .await
            .expect("the index is part of opening the store");
        let egress_before = egress_rows(&database).await;

        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let (bridge, replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
        let (sink, events) = event_channel();
        let wiring = Wiring::new(
            database.clone(),
            blobs,
            bridge.handle(),
            sink,
            bridge.commands(),
        );

        let window = Window::default();
        window.present();
        while glib::MainContext::default().iteration(false) {}
        let wired = feed_the_window(&window, &wiring)
            .await
            .expect("the seeded store has an account");
        let feeds = wired.feeds.clone();
        // What carries a search's results from the sink into the list, as the
        // binary composes it (`search_results.rs` does the same).
        let notifier = postio_app::notifications::Notifier::new(
            database.clone(),
            wiring.store.clone(),
            wiring.runtime.clone(),
            Default::default(),
        );
        let state = postio_core::state::SharedState::default();
        for stream in [events, replies] {
            postio_app::commands::drain(&window, &feeds, stream, notifier.clone(), state.clone());
        }
        assert!(
            settle_until(async || window.list().model().n_items() > 0).await,
            "the inbox never listed anything"
        );
        let before_context = window.context();
        let before_occupant = window.shell().reader_occupant();

        // ── `g c` ──────────────────────────────────────────────────────────
        press(&window, "g");
        press(&window, "c");
        assert!(window.contacts_open(), "`g c` opened nothing");
        assert_eq!(window.context(), Context::Contacts);
        assert!(
            settle_until(async || drawn(&window).len() == 2).await,
            "the default view drew {:?}",
            drawn(&window)
        );
        assert_eq!(
            drawn(&window),
            ["Ada Lovelace", "Quinn Abara"],
            "the people the user made or wrote to, by name -- and none of the \
             senders they only received mail from"
        );
        let pane = window.contacts();
        let ada = pane.model().row(0).expect("ada's row");
        assert_eq!(ada.source, ContactSource::User, "ada carries the made mark");
        assert_eq!(
            pane.model().row(1).expect("quinn's row").source,
            ContactSource::Mail,
            "quinn does not"
        );

        // ── typing narrows ─────────────────────────────────────────────────
        pane.filter().set_text("quinn");
        assert!(
            settle_until(async || drawn(&window) == ["Quinn Abara"]).await,
            "filtering by `quinn` drew {:?}",
            drawn(&window)
        );
        assert_eq!(
            pane.cursor_person().map(|row| row.name).as_deref(),
            Some("Quinn Abara"),
            "the first match is selected"
        );

        // ── "show mail" leaves a `with:` query that finds the sent message ──
        pane.dispatch(postio_core::CommandId::ContactShowMail);
        assert!(
            settle_until(async || !window.contacts_open()).await,
            "show mail left the screen up over the results"
        );
        // The search runs and the keyboard moves on to its results, closing
        // the box (`install_leave_to_list`) -- so what a person sees is the
        // results in the list, and the query they came from.
        let ran = settle_until(async || feeds.messages.showing_results()).await;
        assert!(
            ran,
            "show mail ran no search; the box holds {:?} (open: {}); outcome {:?}",
            window.finder().query().text,
            window.finder().is_open(),
            window.finder().live().and_then(|live| live.outcome())
        );
        assert_eq!(
            window.finder().query().text,
            "with:quinn@example.net",
            "the query names every address the person owns"
        );
        assert_eq!(window.finder().query().mode, Mode::Search);
        assert!(
            settle_until(async || window.list().model().n_items() >= 1).await,
            "the search for quinn's mail found nothing, though the user wrote \
             to quinn -- `with:` must match mail *to* an address as well as from it"
        );
        window.finder().press_escape();
        settle();

        // ── `v e`, then `Esc` back to where the keyboard was ───────────────
        press(&window, "g");
        press(&window, "c");
        pane.filter().set_text("");
        assert!(settle_until(async || drawn(&window).len() == 2).await);
        press(&window, "v");
        press(&window, "e");
        assert!(
            settle_until(async || drawn(&window).len() > 2).await,
            "everyone from mail drew only {:?}",
            drawn(&window)
        );
        press(&window, "Escape");
        assert!(!window.contacts_open());
        assert_eq!(window.context(), before_context);
        assert_eq!(window.shell().reader_occupant(), before_occupant);

        assert_eq!(
            egress_rows(&database).await,
            egress_before,
            "the Contacts screen made an outbound connection (SC-007, FR-060)"
        );
        bridge.shutdown();
    });
}

/// How many outbound connections the store has recorded (#151).
async fn egress_rows(database: &postio_storage::Store) -> i64 {
    let connection = database.connect().await.expect("a connection");
    postio_storage::sql::scalar(&connection, "SELECT count(*) FROM egress_log", ())
        .await
        .expect("count the egress log")
}
