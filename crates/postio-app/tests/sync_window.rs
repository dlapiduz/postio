//! The sync-window step's own write (#876): whether `Start sync` actually
//! reaches `config.toml`.
//!
//! `gtk_onboarding_sync_window.rs` (in `postio-gtk`'s own suite) proves the
//! step renders and that `Start sync` fires the picker's own selection --
//! everything a display can prove without a network. This proves the other
//! half: that `onboarding::install`'s real `connect_start_sync` closure,
//! not a hand-written stand-in for it, patches `[sync].initial_sync_messages`
//! and leaves the rest of the file alone.
//!
//! `Status::SyncWindow` is reached by calling `set_status` directly rather
//! than by driving a real probe and `submit()`: proving the credential path
//! writes an account is `onboarding_probe.rs`'s and `attach_account.rs`'s
//! job, over a mock transport and a loopback server respectively, and
//! neither has a seam this test needs — the write under test happens after
//! the account is already saved, from the step that only shows once it is.
//!
//! One test function, for the reason `onboarding_probe.rs` gives: GTK is
//! initialised once, per process, from one thread.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::sync::Arc;

use adw::prelude::*;
use async_trait::async_trait;
use gtk::{gdk, glib};
use postio_account::discovery::{
    AutoconfigEndpoint, CancelToken, DiscoveryAutoconfig, DiscoverySrvReport, DiscoveryTransport,
    TransportError,
};
use postio_account::secret::MemorySecretStore;
use postio_app::notifications;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::onboarding::{Onboarding, Status, SyncWindow};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::{BlobStore, test_support};

/// A transport `install` needs a value for and this test never calls: no
/// probe runs on the path from `Status::SyncWindow` to `Start sync`.
struct UnusedTransport;

#[async_trait]
impl DiscoveryTransport for UnusedTransport {
    async fn autoconfig(
        &self,
        _endpoint: AutoconfigEndpoint<'_>,
        _cancel: &CancelToken,
    ) -> Result<DiscoveryAutoconfig, TransportError> {
        unreachable!("this test never probes")
    }

    async fn srv(
        &self,
        _domain: &str,
        _cancel: &CancelToken,
    ) -> Result<DiscoverySrvReport, TransportError> {
        unreachable!("this test never probes")
    }

    async fn mx(
        &self,
        _domain: &str,
        _cancel: &CancelToken,
    ) -> Result<Vec<String>, TransportError> {
        unreachable!("this test never probes")
    }
}

#[test]
fn picking_a_sync_window_and_pressing_start_sync_writes_it_to_config_toml() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    let config_path = state_dir.path().join("config.toml");
    // A comment on an unrelated section, the same shape `patch_sync`'s own
    // tests use: proves the write goes through `patch_sync` rather than a
    // whole-file reserialize, which would silently drop it (#874, #885).
    std::fs::write(
        &config_path,
        "# a hand-written comment nobody wants to lose\n[ui]\ntheme = \"dark\"\n",
    )
    .expect("a starter config.toml");
    // SAFETY: first statements of a single-threaded test, before the app
    // under test reads either variable.
    unsafe {
        std::env::set_var("XDG_STATE_HOME", state_dir.path());
        std::env::set_var("POSTIO_CONFIG", &config_path);
    }

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands())
        .with_secrets(Arc::new(MemorySecretStore::new()));

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let notifier = notifications::Notifier::new(
        wiring.database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    postio_app::onboarding::install(
        &window,
        &wiring,
        Default::default(),
        Vec::new(),
        std::rc::Rc::new(std::cell::RefCell::new(Some(events))),
        notifier,
        None,
        Arc::new(UnusedTransport) as Arc<dyn DiscoveryTransport>,
        Arc::new(postio_account::oauth::browser::SystemBrowserOpener),
    );

    let screen = window
        .content()
        .and_downcast::<Onboarding>()
        .expect("the onboarding screen");

    // The account and its credential are already written by the time this
    // step shows in the real flow (`submit`/`submit_oauth`) -- this test
    // starts from exactly that point rather than re-proving the write those
    // functions' own tests already cover.
    screen.set_status(Status::SyncWindow);
    screen.test_select_sync_window(SyncWindow::LastMonth);
    screen.start_sync();

    let written = std::fs::read_to_string(&config_path).expect("config.toml");
    let config = postio_config::Config::from_toml_str(&written).expect("the write still parses");
    assert_eq!(
        config.sync.initial_sync_messages,
        SyncWindow::LastMonth.message_count(),
        "Start sync did not reach [sync].initial_sync_messages: {written}"
    );
    assert!(
        written.contains("# a hand-written comment nobody wants to lose"),
        "the write touched more than [sync]: {written}"
    );
    assert!(
        written.contains("theme = \"dark\""),
        "the write touched more than [sync]: {written}"
    );

    bridge.shutdown();
}
