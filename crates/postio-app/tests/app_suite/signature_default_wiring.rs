//! Issue #394: a per-mailbox signature default reaching the running app.
//!
//! `postio_model::signature_default::resolve` is pure logic, unit-tested in
//! `postio-model`. `gtk_composer_signature_default.rs` proves the composer's
//! own seam reacts to whatever a provider answers. Neither proves the two
//! ever meet: that pressing `c` on a real, seeded database, with a real
//! sidebar selection, actually reads the mailbox row and the account row and
//! signs with what they say. This is the wiring `postio_app::compose::
//! install_signature_default` builds, exercised end to end.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use gtk::prelude::*;
use gtk::gdk;
use postio_app::feed_the_window;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{Identity, MailboxRole, Signature};
use postio_session::Wiring;
use postio_storage::repository::{
    AccountRepository, IdentityRepository, MailboxRepository, SignatureRepository,
};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};



fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        settle();
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

fn press(window: &Window, key: &str, modifiers: gdk::ModifierType) {
    window.handle_key(gdk::Key::from_name(key).unwrap(), modifiers);
    settle();
}

pub fn compose_signs_with_the_selected_mailbox_or_account_default() {
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
    let report = seed_small(&database, 31);
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    let connection = database.connection().expect("a connection");
    let mut account = AccountRepository::new(&connection)
        .get(report.account.id)
        .expect("read the seeded account")
        .expect("the seeded account exists");

    let mut support = Signature::new("Support", "Support team");
    let support_id = SignatureRepository::new(&connection)
        .create(account.id, &mut support)
        .expect("insert the mailbox's signature");
    let mut sales = Signature::new("Sales", "Sales team");
    let sales_id = SignatureRepository::new(&connection)
        .create(account.id, &mut sales)
        .expect("insert the account's default signature");

    account.default_signature_id = Some(sales_id);
    AccountRepository::new(&connection)
        .update(&mut account)
        .expect("save the account default");

    // `test_support::account` (which `seed_small` builds on) creates no
    // identity of its own -- nothing composes without one, so this test
    // needs its own, unsigned so the account default and the mailbox
    // override are the only sources of a signature to tell apart. After
    // the account `update` above, which otherwise deletes any identity not
    // in the snapshot it was given (`account.identities` was fetched
    // before this one existed).
    let mut identity = Identity::new(account.id, account.address.clone());
    identity.is_default = true;
    IdentityRepository::new(&connection)
        .create(&mut identity)
        .expect("insert a sending identity");

    let overridden = report
        .mailbox(MailboxRole::Sent)
        .expect("a Sent mailbox from the seed")
        .clone();
    let mut overridden = overridden;
    overridden.signature_id = Some(support_id);
    MailboxRepository::new(&connection)
        .update(&overridden)
        .expect("save the mailbox override");

    let plain = report
        .mailbox(MailboxRole::Inbox)
        .expect("an Inbox mailbox from the seed")
        .clone();
    drop(connection);

    let (bridge, _replies) =
        postio_core::bridge::Bridge::new(postio_core::bridge::handler_fn(|_, _| async {}))
            .expect("a runtime");
    let (sink, _events) = postio_core::bridge::event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs.clone(),
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    settle();

    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    assert!(
        settle_until(|| window.list().model().n_items() > 0),
        "the list is empty, so selecting a mailbox proves nothing"
    );
    let composer = window.composer();

    // ── the mailbox's own override wins ──────────────────────────────────
    window.sidebar().select(overridden.id);
    assert!(
        settle_until(|| window.sidebar().selected() == Some(overridden.id)),
        "the sidebar never reported the Sent mailbox as selected"
    );
    press(&window, "c", gdk::ModifierType::empty());
    assert!(
        composer.is_open(),
        "`c` on a selected mailbox opened nothing"
    );
    let body = composer.draft().body.text.unwrap_or_default();
    assert!(
        body.contains("Support team"),
        "the mailbox's own signature override did not reach the compose: {body:?}"
    );
    composer.discard();
    settle();

    // ── no override on this mailbox: the account default applies ────────
    window.sidebar().select(plain.id);
    settle();
    press(&window, "c", gdk::ModifierType::empty());
    assert!(
        composer.is_open(),
        "`c` on the plain mailbox opened nothing"
    );
    assert!(
        composer
            .draft()
            .body
            .text
            .unwrap_or_default()
            .contains("Sales team"),
        "the account's default signature did not reach a mailbox with no \
         override of its own"
    );
    composer.discard();
    settle();

    bridge.shutdown();
}
