//! The backend choice, end to end (#545, ADR 0018 Q5).
//!
//! Two adds against user-overlay preset rows that advertise
//! `backend = ["jmap", "imap"]`, all on loopback:
//!
//! 1. **The JMAP proof works** — a scripted session endpoint accepts the
//!    password as a bearer — and the account row stores `jmap` with the
//!    session URL, ready for `engine::start` to pick the adapter.
//! 2. **The JMAP proof is refused** (401) — the wizard falls back to the
//!    IMAP proof against the in-crate `TestServer` and stores `imap`: a
//!    credential that only speaks IMAP still lands, no dead ends.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe; these run before the app starts.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

use adw::prelude::*;
use async_trait::async_trait;
use gtk::{gdk, glib};
use postio_account::discovery::{
    AutoconfigEndpoint, CancelToken, DiscoveryAutoconfig, DiscoverySrvReport, DiscoveryTransport,
    TransportError,
};
use postio_account::secret::MemorySecretStore;
use postio_account::test_server::{TestMailbox, TestServer};
use postio_app::notifications;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::onboarding::{Onboarding, Status};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::account::Backend;
use postio_session::Wiring;
use postio_storage::repository::AccountRepository;
use postio_storage::{BlobStore, test_support};

/// Fails every step: the preset rows answer first.
struct DeadTransport;

#[async_trait]
impl DiscoveryTransport for DeadTransport {
    async fn autoconfig(
        &self,
        _endpoint: AutoconfigEndpoint<'_>,
        _cancel: &CancelToken,
    ) -> Result<DiscoveryAutoconfig, TransportError> {
        Err(TransportError::new("this test resolves from presets only"))
    }

    async fn srv(
        &self,
        _domain: &str,
        _cancel: &CancelToken,
    ) -> Result<DiscoverySrvReport, TransportError> {
        Err(TransportError::new("this test resolves from presets only"))
    }

    async fn mx(
        &self,
        _domain: &str,
        _cancel: &CancelToken,
    ) -> Result<Vec<String>, TransportError> {
        Err(TransportError::new("this test resolves from presets only"))
    }
}

/// A JMAP session endpoint that accepts exactly one bearer.
fn session_server(accepted: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                continue;
            }
            let mut authorized = false;
            let mut content_length = 0usize;
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let header = line.trim_end();
                if let Some(value) = header
                    .to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse().ok())
                {
                    content_length = value;
                }
                if header.eq_ignore_ascii_case(&format!("authorization: Bearer {accepted}")) {
                    authorized = true;
                }
                if header.is_empty() {
                    break;
                }
            }
            let mut body = vec![0u8; content_length];
            let _ = reader.read_exact(&mut body);

            let response = if authorized {
                let body = format!(
                    r#"{{"username": "ada@example.test", "accounts": {{"acc1": {{"name": "Ada", "isPersonal": true, "isReadOnly": false, "accountCapabilities": {{}}}}}}, "primaryAccounts": {{"urn:ietf:params:jmap:mail": "acc1"}}, "capabilities": {{"urn:ietf:params:jmap:core": {{}}, "urn:ietf:params:jmap:mail": {{}}}}, "apiUrl": "http://127.0.0.1:{port}/jmap/api/", "downloadUrl": "http://127.0.0.1:{port}/d/{{accountId}}/{{blobId}}/{{name}}?t={{type}}", "uploadUrl": "http://127.0.0.1:{port}/u/{{accountId}}/", "eventSourceUrl": "http://127.0.0.1:{port}/e/", "state": "s1"}}"#
                );
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}"
                    .to_owned()
            };
            let _ = stream.write_all(response.as_bytes());
        }
    });
    port
}

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

fn drive() {
    // ── servers ─────────────────────────────────────────────────────────
    // An auxiliary runtime carries the test server; the app's own work runs
    // on the bridge's runtime as in production.
    let runtime = tokio::runtime::Runtime::new().expect("a runtime for the servers");
    let imap = runtime.block_on(async {
        TestServer::builder()
            .account("grace@fallback.test")
            .password("imap-only-password")
            .mailbox(TestMailbox::new("INBOX"))
            .start()
            .await
    });
    let jmap_ok = session_server("the-api-token");
    // The refusing endpoint: every bearer is 401, so the fallback row's
    // JMAP proof always fails.
    let jmap_refusing = session_server("nothing-ever-matches");

    // ── two overlay rows, both advertising jmap first ───────────────────
    let config_dir_guard = tempfile::tempdir().expect("a config directory");
    let config_dir = config_dir_guard.path();
    let provider_dir = config_dir.join("postio");
    std::fs::create_dir_all(&provider_dir).expect("a config dir");
    std::fs::write(
        provider_dir.join("providers.toml"),
        format!(
            r#"[provider.native]
display_name = "Native"
domains = ["example.test"]
imap_host = "127.0.0.1"
imap_port = 1
imap_security = "none"
smtp_host = "127.0.0.1"
smtp_port = 1
smtp_security = "none"
auth = ["app-password"]
backend = ["jmap", "imap"]

[provider.native.jmap]
session_url = "http://127.0.0.1:{jmap_ok}/jmap/session/"

[provider.fallback]
display_name = "Fallback"
domains = ["fallback.test"]
imap_host = "{imap_host}"
imap_port = {imap_port}
imap_security = "none"
smtp_host = "127.0.0.1"
smtp_port = 1
smtp_security = "none"
auth = ["app-password"]
backend = ["jmap", "imap"]

[provider.fallback.jmap]
session_url = "http://127.0.0.1:{jmap_refusing}/jmap/session/"
"#,
            imap_host = imap.addr().ip(),
            imap_port = imap.addr().port(),
        ),
    )
    .expect("the overlay rows");
    // SAFETY: before anything touches the preset table's LazyLock.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", config_dir) };

    // ── the app ─────────────────────────────────────────────────────────
    let database = test_support::memory();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        sink,
        bridge.commands(),
    )
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
        Arc::new(DeadTransport),
        Arc::new(postio_account::oauth::browser::SystemBrowserOpener),
    );
    let screen = window
        .content()
        .and_downcast::<Onboarding>()
        .expect("the onboarding screen is the window's content");

    // ── add 1: the JMAP proof works and jmap is stored ──────────────────
    screen.set_address("ada@example.test");
    screen.probe();
    assert!(
        settle_until(|| matches!(screen.status(), Status::Found(_))),
        "the native row never resolved: {:?}",
        screen.status()
    );
    screen.test_set_password("the-api-token");
    screen.submit();
    assert!(
        settle_until(|| matches!(screen.status(), Status::Saved | Status::Failed(_))),
        "the add never settled: {:?}",
        screen.status()
    );
    assert!(
        matches!(screen.status(), Status::Saved),
        "the JMAP add failed: {:?}",
        screen.status()
    );

    // ── add 2: the JMAP proof is refused, IMAP lands, imap is stored ────
    screen.set_status(Status::Idle);
    screen.set_address("grace@fallback.test");
    screen.probe();
    assert!(
        settle_until(|| matches!(screen.status(), Status::Found(_))),
        "the fallback row never resolved: {:?}",
        screen.status()
    );
    screen.test_set_password("imap-only-password");
    screen.submit();
    assert!(
        settle_until(|| matches!(screen.status(), Status::Saved | Status::Failed(_))),
        "the fallback add never settled: {:?}",
        screen.status()
    );
    assert!(
        matches!(screen.status(), Status::Saved),
        "a credential that only speaks IMAP must still land: {:?}",
        screen.status()
    );

    // ── what the rows say ───────────────────────────────────────────────
    let connection = database.connection().expect("a connection");
    let accounts = AccountRepository::new(&connection).list().expect("list");
    let native = accounts
        .iter()
        .find(|account| account.address.address == "ada@example.test")
        .expect("the native add landed");
    assert_eq!(
        native.backend,
        Backend::Jmap {
            session_url: format!("http://127.0.0.1:{jmap_ok}/jmap/session/"),
        },
        "the working JMAP proof is what engine::start will read back"
    );
    let fallback = accounts
        .iter()
        .find(|account| account.address.address == "grace@fallback.test")
        .expect("the fallback add landed");
    assert_eq!(
        fallback.backend,
        Backend::Imap,
        "the refused JMAP proof fell back rather than dead-ending"
    );

    bridge.shutdown();
}

#[test]
fn the_add_stores_the_first_backend_whose_proof_succeeds() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statements of a single-threaded test binary.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    drive();

    // The window this test built joins GTK's toplevel list at
    // construction and stays there, holding a WebProcess, until it is
    // destroyed -- which at exit() is a segfault after a passing test
    // (#794). No harness here to sweep, so the test does it.
    postio_gtk::window::close_all_windows();
}
