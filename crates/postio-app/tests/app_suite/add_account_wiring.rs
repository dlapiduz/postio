//! Issue #64: adding a second account to an application that is already
//! running.
//!
//! `settings_credential_wiring.rs` proves the *credential* dialog reuses
//! `Onboarding` over a live shell. This proves the other host ADR 0012 Q1
//! asked for -- a blank form, opened from a registered command rather than
//! from a row's context menu -- and the two things about it that only a real
//! window can show:
//!
//!   1. **It is reachable.** The dialog opens from `Ctrl+Shift+N` going
//!      through the window's own resolver, which is the same road the
//!      palette and the cheat sheet take. A surface with no command in front
//!      of it is the `postio-bl2` shape this repository keeps re-learning:
//!      the widget passes its tests and the application cannot reach it.
//!   2. **Closing it stops the probe.** ADR 0012 Q3: add-account is the
//!      first place a probe runs a second time in one process, over a shell
//!      that stays on screen, with a way for the user to walk away from it.
//!      A discovery request that outlives its dialog is a socket held open
//!      for an answer nobody will read (#57).
//!
//! # Why neither case submits
//!
//! `onboarding::submit` tests the credential against a real IMAP server
//! before it writes anything, and there is no mock seam in front of
//! `ImapSession::open` the way `DiscoveryTransport` sits in front of the
//! probe. `attach_account.rs` proves what happens *after* a successful
//! write instead, over a loopback server.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use crate::settle_until;
use std::sync::{Arc, Mutex};

use adw::prelude::*;
use async_trait::async_trait;
use gtk::gdk;
use postio_account::discovery::{
    AutoconfigEndpoint, CancelToken, DiscoveryAutoconfig, DiscoverySrvReport, DiscoveryTransport,
    TransportError,
};
use postio_account::secret::MemorySecretStore;
use postio_app::feed_the_window;
use postio_gtk::onboarding::{Onboarding, Status};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

// --- a transport that answers nothing until it is cancelled --------------

/// Holds every discovery step open until the caller's token is cancelled,
/// and keeps a clone of that token so the test can ask afterwards whether
/// the cancellation ever arrived.
///
/// A transport handed a *fresh* token of its own would look identical at
/// call time and never flip, which is exactly the shape of #57.
#[derive(Default)]
struct HangingTransport {
    held: Mutex<Option<CancelToken>>,
}

impl HangingTransport {
    /// Blocks this step until the probe is cancelled, recording the token.
    async fn hold(&self, cancel: &CancelToken) -> TransportError {
        *self.held.lock().unwrap() = Some(cancel.clone());
        cancel.cancelled().await;
        TransportError::new("the discovery probe was cancelled")
    }

    /// A clone of the token the transport was handed, once it has been.
    fn token(&self) -> Option<CancelToken> {
        self.held.lock().unwrap().clone()
    }
}

#[async_trait]
impl DiscoveryTransport for HangingTransport {
    async fn autoconfig(
        &self,
        _endpoint: AutoconfigEndpoint<'_>,
        cancel: &CancelToken,
    ) -> Result<DiscoveryAutoconfig, TransportError> {
        Err(self.hold(cancel).await)
    }

    async fn srv(
        &self,
        _domain: &str,
        cancel: &CancelToken,
    ) -> Result<DiscoverySrvReport, TransportError> {
        Err(self.hold(cancel).await)
    }
}

// --- harness ------------------------------------------------------------

/// A running application over a seeded store: a window with mail in it, its
/// panes fed, and the add-account command wired.
fn running_application() -> (
    Window,
    Wiring,
    postio_core::bridge::Bridge,
    tempfile::TempDir,
) {
    let database = test_support::memory();
    seed_small(&database, 51);

    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    let (bridge, _replies) =
        postio_core::bridge::Bridge::new(postio_core::bridge::handler_fn(|_, _| async {}))
            .expect("a runtime");
    let (sink, _events) = postio_core::bridge::event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands())
        .with_secrets(Arc::new(MemorySecretStore::new()));

    let window = Window::default();
    window.present();
    settle();
    feed_the_window(&window, &wiring).expect("the seeded store has an account");
    (window, wiring, bridge, directory)
}

/// The onboarding screen open anywhere in `window`'s tree, dialogs included
/// -- an add-account form is never the window's content, unlike first run's.
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

/// Everything a display-needing case in this suite has to do first.
///
/// `None` when there is no display; `Some` holds the state directory's
/// cleanup guard, which the caller must keep bound for the rest of its own
/// case body.
fn display() -> Option<tempfile::TempDir> {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: the suite runs its cases in sequence on one thread.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);
    Some(state_dir)
}

// --- the cases ----------------------------------------------------------

pub fn the_add_account_key_opens_a_blank_form_over_the_running_window() {
    let Some(_state_dir) = display() else {
        return;
    };
    let (window, wiring, bridge, _directory) = running_application();
    postio_app::add_account::install(&window, &wiring);

    assert!(
        settle_until(|| window.list().model().n_items() > 0),
        "the list should already have mail before this case asks anything of it"
    );
    let mail_before = window.list().model().n_items();
    assert!(
        find_onboarding(&window).is_none(),
        "an add-account form appeared with nobody asking for one"
    );

    // The registry's binding, through the window's own resolver: the same
    // road a palette row takes, and the one thing a unit test of the dialog
    // cannot prove.
    window.handle_key(
        gdk::Key::from_name("N").unwrap(),
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
    );
    assert!(
        settle_until(|| find_onboarding(&window).is_some()),
        "Ctrl+Shift+N reached no handler: the add-account command resolves \
         and then hits nothing, which is the postio-bl2 shape"
    );

    let screen = find_onboarding(&window).expect("the dialog's onboarding screen");
    assert_eq!(
        screen.address(),
        "",
        "the add-account form arrived with an address already in it; it is a \
         new account, not a repair of the one already there"
    );
    assert!(
        matches!(screen.status(), Status::Idle),
        "the add-account form did not arrive idle: {:?}",
        screen.status()
    );

    // ── and none of that disturbed the application behind it ────────────
    assert_eq!(
        window.list().model().n_items(),
        mail_before,
        "opening the add-account dialog changed what the running window \
         shows -- it must float over the shell, never replace its content \
         the way first run's own host does"
    );
    assert!(
        window.content().and_downcast::<Onboarding>().is_none(),
        "the add-account dialog replaced the window's content instead of \
         floating over it"
    );

    bridge.shutdown();
}

pub fn closing_the_dialog_stops_the_probe_it_started() {
    let Some(_state_dir) = display() else {
        return;
    };
    let (window, wiring, bridge, _directory) = running_application();

    let transport = Arc::new(HangingTransport::default());
    let dialog = postio_app::add_account::open(&window, &wiring, transport.clone());

    let screen = find_onboarding(&window).expect("the dialog's onboarding screen");
    screen.set_address("ada@example.com");
    screen.probe();
    assert!(
        matches!(screen.status(), Status::Probing),
        "the probe did not put the form into its waiting state: {:?}",
        screen.status()
    );
    assert!(
        settle_until(|| transport.token().is_some()),
        "the probe never reached the transport it was given"
    );
    let token = transport.token().expect("the token the probe was handed");
    assert!(
        !token.is_cancelled(),
        "the transport was handed a token that was already spent"
    );

    // Walking away from the dialog is the cancellation ADR 0012 Q3 names:
    // there is no Cancel button, because `Esc` and the close gesture are
    // what a dialog already offers.
    dialog.close();
    settle();

    assert!(
        token.is_cancelled(),
        "closing the add-account dialog left its discovery request running: \
         the socket is open for an answer that now lands on a form which is \
         no longer in the tree (#57, ADR 0012 Q3)"
    );

    bridge.shutdown();
}
