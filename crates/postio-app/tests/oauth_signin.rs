//! The browser sign-in, end to end (#534).
//!
//! Everything runs on loopback and in memory: the provider is a user-overlay
//! preset row, the IdP is a scripted HTTP server on a thread, the browser is
//! a fake that captures the URL so the test can play the redirect, the IMAP
//! server is the in-crate `TestServer` speaking XOAUTH2, and the keyring is a
//! `MemorySecretStore`. What this proves:
//!
//! 1. A preset-known OAuth provider is added **without a password field** —
//!    the wizard resolves the row, opens the browser, and finishes off the
//!    redirect.
//! 2. The exchange really carried PKCE and the authorization code.
//! 3. The access token authenticated a real IMAP session *before* anything
//!    persisted — the connection proof runs with XOAUTH2, not PLAIN.
//! 4. The account row lands with `auth = xoauth2` and the composition data;
//!    the refresh token reaches the keyring under its derived key and never
//!    the database.
//!
//! One test function, for the reason `wiring.rs` gives: GTK initialises once
//! per process.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe; these run before the app starts.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use adw::prelude::*;
use async_trait::async_trait;
use gtk::{gdk, glib};
use postio_app::notifications;
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::onboarding::{Onboarding, Status};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_imap::discovery::{
    AutoconfigEndpoint, CancelToken, DiscoveryAutoconfig, DiscoverySrvReport, DiscoveryTransport,
    TransportError,
};
use postio_imap::oauth::BrowserOpener;
use postio_imap::secret::{AccountKey, MemorySecretStore, SecretStore};
use postio_imap::test_server::{TestMailbox, TestServer};
use postio_model::AuthMethod;
use postio_session::Wiring;
use postio_storage::repository::AccountRepository;
use postio_storage::{BlobStore, test_support};
use url::Url;

const ADDRESS: &str = "ada@example.test";
const ACCESS_TOKEN: &str = "access-token-from-the-idp";
const REFRESH_TOKEN: &str = "refresh-token-from-the-idp";

// --- a transport that must never be needed ------------------------------

/// Fails every step: the preset row answers first, so the probe never asks —
/// and a test that accidentally left preset range fails loudly here instead
/// of dialing out.
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
}

// --- the fake browser and the scripted IdP ------------------------------

/// Captures the authorization URL instead of opening anything.
#[derive(Clone, Default, Debug)]
struct FakeBrowser {
    opened: Arc<Mutex<Option<Url>>>,
}

impl BrowserOpener for FakeBrowser {
    fn open(&self, url: &Url) -> std::io::Result<()> {
        *self.opened.lock().expect("no poisoned browser") = Some(url.clone());
        Ok(())
    }
}

/// A one-shot token endpoint: answers one POST with the token JSON and keeps
/// the request it answered, so the test can assert on what the exchange sent.
struct MockIdp {
    url: String,
    handle: thread::JoinHandle<Vec<u8>>,
}

impl MockIdp {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the IdP");
        let port = listener.local_addr().expect("addr").port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("one exchange");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut request = Vec::new();
            let mut line = String::new();
            let mut content_length = 0usize;
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                request.extend_from_slice(line.as_bytes());
                let header = line.trim_end().to_ascii_lowercase();
                if let Some(value) = header
                    .strip_prefix("content-length:")
                    .map(str::trim)
                    .and_then(|v| v.parse().ok())
                {
                    content_length = value;
                }
                if line.trim_end().is_empty() {
                    break;
                }
            }
            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).expect("the form body");
            request.extend_from_slice(&body);

            let json = format!(
                r#"{{"access_token":"{ACCESS_TOKEN}","token_type":"Bearer","expires_in":3600,"refresh_token":"{REFRESH_TOKEN}"}}"#
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{json}",
                json.len()
            );
            stream.write_all(response.as_bytes()).expect("answer");
            request
        });
        Self {
            url: format!("http://127.0.0.1:{port}/token"),
            handle,
        }
    }
}

/// Plays the user's part: reads the redirect URI and state off the captured
/// authorization URL and delivers the code the way a browser would.
fn play_the_browser(authorize_url: &Url, code: &str) {
    let pairs: std::collections::HashMap<_, _> = authorize_url.query_pairs().into_owned().collect();
    let mut callback: Url = pairs["redirect_uri"].parse().expect("a redirect URI");
    callback
        .query_pairs_mut()
        .append_pair("code", code)
        .append_pair("state", &pairs["state"]);

    let address = format!(
        "{}:{}",
        callback.host_str().expect("a host"),
        callback.port().expect("a port")
    );
    let mut stream = TcpStream::connect(&address).expect("the loopback listener answers");
    let request = format!(
        "GET {}?{} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n",
        callback.path(),
        callback.query().unwrap_or_default()
    );
    stream
        .write_all(request.as_bytes())
        .expect("deliver the code");
    let mut sink = Vec::new();
    let _ = stream.read_to_end(&mut sink);
}

// --- harness ------------------------------------------------------------

/// Run the main loop until `done` or the budget runs out.
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

#[test]
fn a_preset_oauth_provider_signs_in_with_the_browser_end_to_end() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let state_dir = scratch.path().join("state");
    let config_dir = scratch.path().join("config");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::create_dir_all(config_dir.join("postio")).unwrap();
    // SAFETY: first statements of a single-threaded test binary — set before
    // anything (the preset table's `LazyLock` included) reads the
    // environment.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &config_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── the servers ─────────────────────────────────────────────────────
    // An auxiliary runtime carries the test server; the app's own work runs
    // on the bridge's runtime as in production.
    let runtime = tokio::runtime::Runtime::new().expect("a runtime for the servers");
    let imap = runtime.block_on(async {
        TestServer::builder()
            .capabilities(["IMAP4rev1", "SASL-IR", "AUTH=XOAUTH2"])
            .access_token(ACCESS_TOKEN)
            .account(ADDRESS)
            .mailbox(TestMailbox::new("INBOX"))
            .start()
            .await
    });
    let idp = MockIdp::start();

    // ── the provider, as a user-overlay preset row ──────────────────────
    std::fs::write(
        config_dir.join("postio/providers.toml"),
        format!(
            r#"[provider.looptest]
display_name = "Loop Test"
domains = ["example.test"]
imap_host = "{host}"
imap_port = {port}
imap_security = "none"
smtp_host = "127.0.0.1"
smtp_port = 1
smtp_security = "none"
auth = ["oauth2"]

[provider.looptest.oauth]
authorize = "http://127.0.0.1:1/authorize"
token = "{token}"
scopes = ["mail.everything"]
sources = ["own-client"]
"#,
            host = imap.addr().ip(),
            port = imap.addr().port(),
            token = idp.url,
        ),
    )
    .expect("the overlay row");

    // ── the app ─────────────────────────────────────────────────────────
    let database = test_support::memory();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");
    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, events) = event_channel();
    let secrets = Arc::new(MemorySecretStore::new());
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        sink,
        bridge.commands(),
    )
    .with_secrets(secrets.clone());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    let notifier = notifications::Notifier::new(
        wiring.database.clone(),
        wiring.store.clone(),
        wiring.runtime.clone(),
        Default::default(),
    );
    let browser = FakeBrowser::default();
    postio_app::onboarding::install(
        &window,
        &wiring,
        Default::default(),
        Vec::new(),
        std::rc::Rc::new(std::cell::RefCell::new(Some(events))),
        notifier,
        None,
        Arc::new(DeadTransport),
        Arc::new(browser.clone()),
    );
    let screen = window
        .content()
        .and_downcast::<Onboarding>()
        .expect("the onboarding screen is the window's content");

    // ── the user's three actions: address, client id, one click ────────
    screen.set_address(ADDRESS);
    screen.probe();
    assert!(
        settle_until(|| matches!(screen.status(), Status::Found(_))),
        "the overlay preset never resolved: {:?}",
        screen.status()
    );
    let Status::Found(found) = screen.status() else {
        unreachable!()
    };
    assert!(
        found.oauth_sign_in,
        "a row preferring oauth2 must open the browser door, not a password \
         field"
    );

    screen.test_set_oauth_client("the-client-id", "");
    screen.submit();

    // ── the browser's part ──────────────────────────────────────────────
    assert!(
        settle_until(|| browser.opened.lock().unwrap().is_some()),
        "no authorization URL was ever opened: {:?}",
        screen.status()
    );
    assert!(
        matches!(screen.status(), Status::WaitingForBrowser),
        "while the browser is out, the screen must say so: {:?}",
        screen.status()
    );
    let authorize_url = browser.opened.lock().unwrap().clone().expect("captured");
    play_the_browser(&authorize_url, "the-code");

    assert!(
        settle_until(|| matches!(screen.status(), Status::Saved | Status::Failed(_))),
        "the sign-in never settled: {:?}",
        screen.status()
    );
    assert!(
        matches!(screen.status(), Status::Saved),
        "the sign-in failed: {:?}",
        screen.status()
    );

    // ── what must be true afterwards ────────────────────────────────────
    let connection = database.connection().expect("a connection");
    let account = AccountRepository::new(&connection)
        .list()
        .expect("accounts")
        .into_iter()
        .find(|account| account.address.address == ADDRESS)
        .expect("the account row landed");
    assert_eq!(account.auth, AuthMethod::XOAuth2);
    let oauth = account.oauth.expect("the composition data is on the row");
    assert_eq!(oauth.client_id, "the-client-id");
    assert_eq!(oauth.token_url, idp.url);

    let refresh = runtime
        .block_on(secrets.retrieve(&AccountKey::new(format!("{ADDRESS}#oauth-refresh"))))
        .expect("the refresh token is in the keyring");
    assert_eq!(refresh.expose(), REFRESH_TOKEN);
    assert!(
        runtime
            .block_on(secrets.retrieve(&AccountKey::new(ADDRESS)))
            .is_err(),
        "no password entry exists: this account never had one"
    );

    // The exchange really carried PKCE and the code — the IdP kept the
    // request it answered. (The tokens themselves travel in the *response*,
    // which is why asserting they never appear in the request is not the
    // secrecy proof; the keyring assertions above are.)
    let exchange =
        String::from_utf8_lossy(&idp.handle.join().expect("the IdP served")).into_owned();
    assert!(
        exchange.contains("grant_type=authorization_code"),
        "{exchange}"
    );
    assert!(exchange.contains("code=the-code"), "{exchange}");
    assert!(exchange.contains("code_verifier="), "{exchange}");

    bridge.shutdown();
}
