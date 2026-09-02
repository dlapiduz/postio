//! Issue #464: updating an account's credential from the settings panel.
//!
//! `startup_repair.rs` proves `Status::Reauthenticate` arrives correctly
//! when the keyring loses a password automatically. This proves the manual
//! path #464 adds reaches the same screen, opened from a running
//! application's settings panel over an account whose engine is already
//! syncing -- and, critically, that *opening* it does not disturb the
//! running window: `settings_credential::install` builds the dialog and
//! wires the seams without calling `open_account` or touching
//! `window.set_content` at all, unlike `onboarding::install`'s own
//! first-run path.
//!
//! # Why this does not drive an actual submit
//!
//! `onboarding::submit` tests the credential against a real IMAP server
//! before writing anything -- `RustlsConnector`/`ImapSession::open`, with no
//! mock seam the way `probe`'s `DiscoveryTransport` has. Driving it here
//! would mean a genuine network attempt against `imap.example.com`, which
//! the repository rule forbids regardless of whether it fails fast. The
//! write itself (`onboarding::persist`) is exactly what the 21 existing
//! `onboarding` unit tests already exercise directly, unmodified by this
//! issue's refactor; what only a real window can prove is what this test
//! proves instead.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use crate::settle_until;
use adw::prelude::*;
use gtk::gdk;
use postio_app::feed_the_window;
use postio_gtk::onboarding::{Onboarding, Status};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::repository::IdentityRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn update_credential_opens_a_prefilled_dialog_without_disturbing_the_window() {
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
    let report = seed_small(&database, 51);

    // Onboarding never ran for this account (it was seeded directly), so it
    // has no identity of its own -- give it one, the same reason
    // `signature_default_wiring.rs` does, or nothing composes with it later
    // and this test would be exercising a shape no real account has.
    let connection = database.connection().expect("a connection");
    let mut identity =
        postio_model::Identity::new(report.account.id, report.account.address.clone());
    identity.is_default = true;
    IdentityRepository::new(&connection)
        .create(&mut identity)
        .expect("insert a sending identity");
    drop(connection);

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

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
        "the list should already have mail before this test asks anything of it"
    );
    let mail_before = window.list().model().n_items();

    // ── nothing showing yet: a dialog only opens when asked ──────────────
    assert!(
        find_onboarding(&window).is_none(),
        "a credential dialog appeared with nobody asking for one"
    );

    // ── Update credential opens a dialog prefilled from the account row ──
    postio_app::settings_credential::install(&window, &wiring, report.account.id);
    assert!(
        settle_until(|| find_onboarding(&window).is_some()),
        "Update credential did not open a dialog"
    );
    let screen = find_onboarding(&window).expect("the dialog's onboarding screen");
    assert!(
        matches!(screen.status(), Status::Reauthenticate(_)),
        "the dialog did not arrive looking like a reauthenticate: {:?}",
        screen.status()
    );
    assert_eq!(
        screen.address(),
        report.account.address.address,
        "the dialog made the user retype an address the store already had"
    );
    assert_eq!(
        screen.settings().imap.host,
        report.account.incoming.host,
        "the dialog made the user retype servers the store already had"
    );

    // ── and none of that touched the window that is already running ──────
    assert_eq!(
        window.list().model().n_items(),
        mail_before,
        "opening the credential dialog changed what the running window shows -- \
         install_reauthenticate must never call open_account or \
         window.set_content the way onboarding::install's first-run path does"
    );
    assert!(
        window.content().and_downcast::<Onboarding>().is_none(),
        "the credential dialog replaced the window's content instead of \
         floating over it"
    );

    bridge.shutdown();
}

/// The dialog's onboarding screen, if one is open anywhere in the window's
/// tree -- not only `window.content()`, the way `startup_repair.rs`'s own
/// `screen()` helper checks, since a credential dialog is not the window's
/// content at all.
fn find_onboarding(window: &Window) -> Option<Onboarding> {
    fn walk(widget: &gtk::Widget) -> Option<Onboarding> {
        if let Ok(found) = widget.clone().downcast::<Onboarding>() {
            return Some(found);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = walk(&current) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }
    walk(window.upcast_ref::<gtk::Widget>())
}
