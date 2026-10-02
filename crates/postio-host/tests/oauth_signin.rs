//! The browser sign-in, end to end, through the host (#534).
//!
//! Everything runs on loopback and in memory: the provider is a user-overlay
//! preset row, the IdP is a scripted HTTP server on a thread, the person's
//! browser is played by a plain GET to the redirect the host announced, the
//! IMAP server is the in-crate `TestServer` speaking XOAUTH2, and the keyring
//! is a `MemorySecretStore`. What this proves, from a client's side:
//!
//! 1. A preset-known OAuth provider is added **without a password** -- the
//!    client begins the sign-in, gets the consent URL, and finishes off the
//!    redirect.
//! 2. The exchange really carried PKCE and the authorization code.
//! 3. The access token authenticated a real IMAP session before the account
//!    was saved -- the connection proof ran XOAUTH2, never PLAIN or LOGIN.
//! 4. The account row lands with `auth = xoauth2` and the composition data;
//!    the refresh token reaches the keyring under its derived key and never
//!    the database.
//!
//! Its own binary, run again as a child: the overlay is found through
//! `XDG_CONFIG_HOME`, this crate forbids the `unsafe` that setting it
//! in-process takes, and the provider table is computed once per process
//! from the environment (`tests/oauth.rs`, #973).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use postio_account::discovery::{
    AutoconfigEndpoint, CancelToken, DiscoveryAutoconfig, DiscoverySrvReport, DiscoveryTransport,
    TransportError,
};
use postio_account::oauth::Url;
use postio_account::secret::{AccountKey, MemorySecretStore, SecretStore};
use postio_account::test_server::{TestMailbox, TestServer};
use postio_client::protocol::{AfterSave, ClientKind};
use postio_host::Host;
use postio_model::AuthMethod;
use postio_storage::repository::AccountRepository;
use postio_storage::test_support;
use postio_ui::onboarding::{OAuthClientSubmission, Status, Submission};

const ADDRESS: &str = "ada@example.test";
const ACCESS_TOKEN: &str = "access-token-from-the-idp";
const REFRESH_TOKEN: &str = "refresh-token-from-the-idp";

/// Set in the child that does the work.
const INNER: &str = "POSTIO_OAUTH_SIGNIN_INNER";

/// Fails every step: the preset row answers first, so the probe never asks.
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

/// Plays the user's part: reads the redirect URI and state off the consent
/// URL and delivers the code the way a browser would.
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

#[test]
fn a_preset_oauth_provider_signs_in_through_the_host_end_to_end() {
    if std::env::var_os(INNER).is_some() {
        signs_in();
        return;
    }
    let config = tempfile::tempdir().expect("a config directory");
    let ran = std::process::Command::new(std::env::current_exe().expect("this test"))
        .args([
            "a_preset_oauth_provider_signs_in_through_the_host_end_to_end",
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

/// The test itself, with `XDG_CONFIG_HOME` pointing somewhere of its own.
fn signs_in() {
    let config = std::path::PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").expect("set"));
    std::fs::create_dir_all(config.join("postio")).unwrap();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let imap = rt.block_on(
        TestServer::builder()
            .capabilities(["IMAP4rev1", "SASL-IR", "AUTH=XOAUTH2"])
            .access_token(ACCESS_TOKEN)
            .account(ADDRESS)
            .mailbox(TestMailbox::new("INBOX"))
            .start(),
    );
    let idp = MockIdp::start();
    std::fs::write(
        config.join("postio/providers.toml"),
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

    let database = rt.block_on(test_support::memory());
    let blobs_dir = tempfile::tempdir().unwrap();
    let blobs =
        postio_storage::BlobStore::open(blobs_dir.path().to_path_buf(), &test_support::blob_keys())
            .unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let host = Host::start(database.clone(), blobs, {
        let secrets = Arc::clone(&secrets);
        move |wiring| {
            wiring
                .with_secrets(secrets)
                .with_discovery(Arc::new(DeadTransport))
        }
    })
    .expect("a host");
    let client = host.connect(ClientKind::Tui);

    // The person's three actions: address, client id, one click.
    let found = rt
        .block_on(client.discover(ADDRESS.into()))
        .expect("discovery");
    let Status::Found(settings) = found else {
        panic!("the overlay preset never resolved: {found:?}");
    };
    assert!(
        settings.oauth_sign_in,
        "a row preferring oauth2 opens the browser door, not a password field"
    );
    let submission = Submission {
        address: ADDRESS.into(),
        name: "Ada".into(),
        password: String::new(),
        settings,
        oauth_client: Some(OAuthClientSubmission {
            client_id: "the-client-id".into(),
            client_secret: None,
        }),
    };
    assert!(
        submission.password.is_empty(),
        "no password is asked for or sent"
    );
    let consent = rt
        .block_on(client.begin_oauth_then(submission, AfterSave::Wait))
        .expect("a consent URL");
    assert!(
        imap.commands().is_empty(),
        "nothing reached the IMAP server before the person consented: {:?}",
        imap.commands()
    );
    assert!(
        rt.block_on(AccountRepository::new(&rt.block_on(database.connect()).unwrap()).list())
            .unwrap()
            .iter()
            .all(|account| account.address.address != ADDRESS),
        "nothing is saved before the sign-in finishes"
    );

    // The browser's part, then the host finishes the sign-in.
    let authorize_url: Url = consent.authorize_url.parse().expect("a consent URL");
    play_the_browser(&authorize_url, "the-code");
    rt.block_on(client.finish_oauth(ADDRESS.into()))
        .expect("the sign-in finished");

    // What must be true afterwards.
    let commands = imap.commands();
    assert!(
        commands
            .iter()
            .any(|line| line.to_ascii_uppercase().contains("AUTHENTICATE XOAUTH2")),
        "the connection proof did not authenticate with XOAUTH2: {commands:?}"
    );
    assert!(
        commands
            .iter()
            .all(|line| !line.to_ascii_uppercase().contains(" LOGIN ")),
        "the proof fell back to a password login: {commands:?}"
    );
    let connection = rt.block_on(database.connect()).expect("a connection");
    let account = rt
        .block_on(AccountRepository::new(&connection).list())
        .expect("accounts")
        .into_iter()
        .find(|account| account.address.address == ADDRESS)
        .expect("the account row landed");
    assert_eq!(account.auth, AuthMethod::XOAuth2);
    let oauth = account.oauth.expect("the composition data is on the row");
    assert_eq!(oauth.client_id, "the-client-id");
    assert_eq!(oauth.token_url, idp.url);

    let refresh = rt
        .block_on(secrets.retrieve(&AccountKey::new(format!("{ADDRESS}#oauth-refresh"))))
        .expect("the refresh token is in the keyring");
    assert_eq!(refresh.expose(), REFRESH_TOKEN);
    assert!(
        rt.block_on(secrets.retrieve(&AccountKey::new(ADDRESS)))
            .is_err(),
        "no password entry exists: this account never had one"
    );

    let exchange =
        String::from_utf8_lossy(&idp.handle.join().expect("the IdP served")).into_owned();
    assert!(
        exchange.contains("grant_type=authorization_code"),
        "{exchange}"
    );
    assert!(exchange.contains("code=the-code"), "{exchange}");
    assert!(exchange.contains("code_verifier="), "{exchange}");
    drop(connection);
    drop(client);
    drop(host);
}
