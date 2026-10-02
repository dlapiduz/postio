//! The backend choice, end to end, through the host (#545, ADR 0018 Q5).
//!
//! Two adds against user-overlay preset rows that advertise
//! `backend = ["jmap", "imap"]`, all on loopback, from a client's side:
//!
//! 1. **The JMAP proof works** -- a scripted session endpoint accepts the
//!    password as a bearer -- and the account row stores `jmap` with the
//!    session URL, ready for the engine to pick the adapter.
//! 2. **The JMAP proof is refused** (401) -- the add falls back to the IMAP
//!    proof against the in-crate `TestServer` and stores `imap`: a credential
//!    that only speaks IMAP still lands, no dead ends.
//!
//! Its own binary, run again as a child: the overlay is found through
//! `XDG_CONFIG_HOME`, this crate forbids the `unsafe` that setting it
//! in-process takes, and the provider table is computed once per process
//! from the environment (`tests/oauth.rs`, #973).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

use postio_account::discovery::{
    AutoconfigEndpoint, CancelToken, DiscoveryAutoconfig, DiscoverySrvReport, DiscoveryTransport,
    TransportError,
};
use postio_account::secret::MemorySecretStore;
use postio_account::test_server::{TestMailbox, TestServer};
use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_host::Host;
use postio_model::account::Backend;
use postio_storage::repository::AccountRepository;
use postio_storage::test_support;
use postio_ui::onboarding::{Status, Submission};

/// Set in the child that does the work.
const INNER: &str = "POSTIO_BACKEND_CHOICE_INNER";

/// Fails every step: the preset rows answer first.
struct DeadTransport;

#[async_trait::async_trait]
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

/// Resolve `address` from the overlay and add it with `password`.
fn add(rt: &tokio::runtime::Runtime, client: &Client, address: &str, password: &str) {
    let found = rt
        .block_on(client.discover(address.into()))
        .expect("discovery");
    let Status::Found(settings) = found else {
        panic!("{address} never resolved from the overlay: {found:?}");
    };
    rt.block_on(client.add_account(Submission {
        address: address.into(),
        name: "Test".into(),
        password: password.into(),
        settings,
        oauth_client: None,
    }))
    .unwrap_or_else(|error| panic!("the add of {address} failed: {}", error.message()));
}

#[test]
fn the_add_stores_the_first_backend_whose_proof_succeeds() {
    if std::env::var_os(INNER).is_some() {
        drive();
        return;
    }
    let config = tempfile::tempdir().expect("a config directory");
    let ran = std::process::Command::new(std::env::current_exe().expect("this test"))
        .args([
            "the_add_stores_the_first_backend_whose_proof_succeeds",
            "--exact",
            "--nocapture",
        ])
        .env(INNER, "1")
        .env("XDG_CONFIG_HOME", config.path())
        .output()
        .expect("the child runs");
    assert!(
        ran.status.success(),
        "{}{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
}

/// The scenario, with `XDG_CONFIG_HOME` pointing somewhere of its own.
fn drive() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let imap = rt.block_on(
        TestServer::builder()
            .account("grace@fallback.test")
            .password("imap-only-password")
            .mailbox(TestMailbox::new("INBOX"))
            .start(),
    );
    let jmap_ok = session_server("the-api-token");
    // The refusing endpoint: every bearer is 401, so the fallback row's
    // JMAP proof always fails.
    let jmap_refusing = session_server("nothing-ever-matches");

    let config = std::path::PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").expect("set"));
    std::fs::create_dir_all(config.join("postio")).unwrap();
    std::fs::write(
        config.join("postio/providers.toml"),
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

    let database = rt.block_on(test_support::memory());
    let blobs_dir = tempfile::tempdir().unwrap();
    let blobs =
        postio_storage::BlobStore::open(blobs_dir.path().to_path_buf(), &test_support::blob_keys())
            .unwrap();
    let host = Host::start(database.clone(), blobs, |wiring| {
        wiring
            .with_secrets(Arc::new(MemorySecretStore::new()))
            .with_discovery(Arc::new(DeadTransport))
    })
    .expect("a host");
    let client = host.connect(ClientKind::Tui);

    // Add 1: the JMAP proof works and jmap is stored.
    add(&rt, &client, "ada@example.test", "the-api-token");
    // Add 2: the JMAP proof is refused, IMAP lands, imap is stored.
    add(&rt, &client, "grace@fallback.test", "imap-only-password");

    let connection = rt.block_on(database.connect()).expect("a connection");
    let accounts = rt
        .block_on(AccountRepository::new(&connection).list())
        .expect("list");
    let native = accounts
        .iter()
        .find(|account| account.address.address == "ada@example.test")
        .expect("the native add landed");
    assert_eq!(
        native.backend,
        Backend::Jmap {
            session_url: format!("http://127.0.0.1:{jmap_ok}/jmap/session/"),
        },
        "the working JMAP proof is what the engine will read back"
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
    drop(connection);
    drop(client);
    drop(host);
}
