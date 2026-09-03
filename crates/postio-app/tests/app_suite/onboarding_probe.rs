//! The onboarding probe's *call site*, driven against a mock transport.
//!
//! `status_for` was already split out of `probe()` so the mapping could be
//! tested without a network, and it is. What stayed untestable is everything
//! around the mapping — the status transitions `probe()` drives, the "runtime
//! went away" branch, and the cancellation handoff #57 added — because
//! `probe()` constructed `PimalayaTransport` itself, three layers down inside
//! the spawned task. That is the `postio-bl2` shape: the wiring is where the
//! bugs are, and the wiring was the part no test could reach.
//!
//! #282 threads the transport through `onboarding::install` instead, so this
//! drives the real `probe()` — the same closure `Connect` and the address
//! field fire — over a transport that answers from a fixture.
//!
//! One test function, for the reason `wiring.rs` gives: GTK is initialised
//! once, per process, from one thread.
//!
//! Nothing here dials anything: the transport is a mock, the keyring is a
//! `MemorySecretStore`, and the command handler is a no-op.
//!
//! # Why this shares `app_suite`'s process (#973)
//!
//! None of the three reasons a test stays out of the suite applies. It is
//! not in the headless runner's watchdog name list (#272) -- of this
//! crate's tests only `e2e*` is. It needs no display of its own
//! (#45/#114): the suite already runs under the compositor and
//! initialises GTK once, which is the arrangement this wants too. And it
//! asserts no wall-clock budget (#841) -- every `Instant` here is a
//! settle deadline, which a shared process does not change.
//!
//! A mock transport, built per case: no server, and no real
//! network at all.
//!
//! It does set process-global environment -- its own XDG directories
//! -- and that is safe for the reason the modules beside it already
//! rely on: the harness runs one case at a time on one thread,
//! `postio_config::paths` caches nothing, and each case sets its
//! directories before it uses them.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::sync::{Arc, Mutex};

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
use postio_gtk::onboarding::{Onboarding, Status};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::{BlobStore, test_support};

// --- a transport that answers from a fixture ----------------------------

/// Answers the `.well-known` step with `xml`, if there is one, and fails
/// every other step — which is what a domain publishing nothing looks like.
#[derive(Default)]
struct MockTransport {
    autoconfig: Option<String>,
    /// A clone of the token the transport was handed, kept so the test can
    /// ask *afterwards* whether it observed the caller's cancellation. A
    /// transport handed a fresh token of its own would look identical at call
    /// time and never flip here — which is exactly the #57 bug.
    held: Mutex<Option<CancelToken>>,
    /// Whether the token was already spent when the call ran.
    saw_cancelled: Mutex<Vec<bool>>,
}

impl MockTransport {
    fn publishing(xml: &str) -> Self {
        Self {
            autoconfig: Some(xml.to_owned()),
            ..Self::default()
        }
    }

    fn publishing_nothing() -> Self {
        Self::default()
    }

    fn observe(&self, cancel: &CancelToken) {
        self.saw_cancelled
            .lock()
            .unwrap()
            .push(cancel.is_cancelled());
        *self.held.lock().unwrap() = Some(cancel.clone());
    }

    /// A clone of the token the transport was last handed.
    ///
    /// Taken rather than asked about: `held` is overwritten by the *next*
    /// call, and the next call happens on the runtime. Asserting on
    /// "whatever is held now" therefore races the second probe's own
    /// transport call — it passed alone and failed under a loaded suite,
    /// which is the shape `postio-9112` warns about. Holding the specific
    /// token makes the assertion about one probe rather than about timing.
    fn held_token(&self) -> Option<CancelToken> {
        self.held.lock().unwrap().clone()
    }

    fn was_called(&self) -> bool {
        !self.saw_cancelled.lock().unwrap().is_empty()
    }

    fn ever_called_with_a_spent_token(&self) -> bool {
        self.saw_cancelled
            .lock()
            .unwrap()
            .iter()
            .any(|spent| *spent)
    }
}

#[async_trait]
impl DiscoveryTransport for MockTransport {
    async fn autoconfig(
        &self,
        _endpoint: AutoconfigEndpoint<'_>,
        cancel: &CancelToken,
    ) -> Result<DiscoveryAutoconfig, TransportError> {
        self.observe(cancel);
        match &self.autoconfig {
            Some(xml) => Ok(serde_xml_rs::from_str(xml).expect("fixture parses")),
            None => Err(TransportError::new("no autoconfig document")),
        }
    }

    async fn srv(
        &self,
        _domain: &str,
        cancel: &CancelToken,
    ) -> Result<DiscoverySrvReport, TransportError> {
        self.observe(cancel);
        Err(TransportError::new("NXDOMAIN"))
    }

    async fn mx(&self, _domain: &str, cancel: &CancelToken) -> Result<Vec<String>, TransportError> {
        self.observe(cancel);
        Err(TransportError::new("NXDOMAIN"))
    }
}

fn autoconfig_xml() -> String {
    r#"<clientConfig version="1.1">
  <emailProvider id="example.com">
    <domain>example.com</domain>
    <displayName>Example Mail</displayName>
    <incomingServer type="imap">
      <hostname>mail.example.com</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>send.example.com</hostname>
      <port>587</port>
      <socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#
        .to_owned()
}

// --- harness ------------------------------------------------------------

/// Run the main loop until `done` or the budget runs out.
///
/// The probe is answered on the runtime and crosses back over a channel, so
/// the status is not there the instant `probe()` returns.
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

/// A window with the onboarding screen installed over `transport`.
fn onboard(
    transport: Arc<dyn DiscoveryTransport>,
) -> (Window, Onboarding, Bridge, tempfile::TempDir) {
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
        transport,
        std::sync::Arc::new(postio_account::oauth::browser::SystemBrowserOpener),
    );

    let screen = window
        .content()
        .and_downcast::<Onboarding>()
        .expect("the onboarding screen");
    (window, screen, bridge, directory)
}

pub fn the_probe_call_site_drives_the_screen_from_a_transport_it_was_given() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── a domain that publishes settings ────────────────────────────────

    let transport = Arc::new(MockTransport::publishing(&autoconfig_xml()));
    let (_window, screen, bridge, _directory) = onboard(transport.clone());

    screen.set_address("ada@example.com");
    screen.probe();
    assert!(
        matches!(screen.status(), Status::Probing),
        "the probe did not put the screen into its waiting state: {:?}",
        screen.status()
    );

    let settled = settle_until(|| !screen.status().is_busy());
    assert!(settled, "the probe never came back: {:?}", screen.status());
    assert!(
        transport.was_called(),
        "the screen answered without ever asking the transport it was given, \
         so `probe()` is still building one of its own"
    );

    let Status::Found(settings) = screen.status() else {
        panic!(
            "a domain publishing a full autoconfig document did not land on \
             Found: {:?}",
            screen.status()
        );
    };
    assert_eq!(
        settings.imap.host, "mail.example.com",
        "the screen showed servers that did not come from the document"
    );
    assert_eq!(settings.smtp.host, "send.example.com");

    // ── the token the transport was handed is the caller's (#57) ────────

    assert!(
        !transport.ever_called_with_a_spent_token(),
        "the transport was handed a token that was already cancelled"
    );

    // A second probe restarts the cancellation, which must stop the first.
    // Driving it through the real closure rather than `ProbeCancellation`
    // directly is the point: the unit test for `restart()` passes either way.
    //
    // The first probe's token is taken *before* the second starts. `restart()`
    // runs synchronously inside `probe()`, so this is settled the moment the
    // call returns — no waiting on the runtime, and nothing for a loaded
    // machine to reorder.
    let first = transport
        .held_token()
        .expect("the transport was handed a token");
    screen.set_status(Status::Manual { suggestion: None });
    screen.probe();
    assert!(
        first.is_cancelled(),
        "starting a second probe left the first one's socket open — the \
         composition root is dropping the token again, which is #57"
    );

    bridge.shutdown();

    // ── a domain that publishes nothing falls back to manual entry ──────

    let quiet = Arc::new(MockTransport::publishing_nothing());
    let (_window, screen, bridge, _directory) = onboard(quiet.clone());

    screen.set_address("ada@example.com");
    screen.probe();
    let settled = settle_until(|| !screen.status().is_busy());
    assert!(settled, "the probe never came back: {:?}", screen.status());
    assert!(quiet.was_called(), "nothing asked the transport");

    assert!(
        matches!(screen.status(), Status::Manual { .. }),
        "a domain publishing nothing did not fall back to manual entry: {:?}",
        screen.status()
    );

    bridge.shutdown();

    // The window this test built joins GTK's toplevel list at
    // construction and stays there, holding a WebProcess, until it is
    // destroyed -- which at exit() is a segfault after a passing test
    // (#794). No harness here to sweep, so the test does it.
    postio_gtk::window::close_all_windows();
}
