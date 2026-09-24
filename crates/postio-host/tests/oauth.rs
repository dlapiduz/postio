//! A browser sign-in begun from a frontend (US7 scenario 2, T086).
//!
//! Its own binary because the provider it signs in with is a user-overlay
//! row, and the provider table is computed once per process from the
//! environment (`postio-app/tests/oauth_signin.rs` says why at length).
//!
//! What it proves: the consent URL comes back to the frontend in full, and
//! nothing is opened or fetched until the person acts -- the host's browser
//! opener only reports the URL, and the provider's token endpoint has not
//! been contacted. Cancelling ends it, in words.
//!
//! The overlay is found through `XDG_CONFIG_HOME`, and this crate forbids the
//! `unsafe` that setting it in-process takes. So the test runs itself again as
//! a child with the variable set, and the child does the work.

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_account::discovery::{
    AutoconfigEndpoint, CancelToken, DiscoveryAutoconfig, DiscoverySrvReport, DiscoveryTransport,
    TransportError,
};
use postio_account::secret::MemorySecretStore;
use postio_client::protocol::ClientKind;
use postio_host::Host;
use postio_storage::test_support;
use postio_ui::onboarding::{OAuthClientSubmission, Status, Submission};

/// No network: discovery answers from the provider table.
struct Offline;

#[async_trait::async_trait]
impl DiscoveryTransport for Offline {
    async fn autoconfig(
        &self,
        _endpoint: AutoconfigEndpoint<'_>,
        _cancel: &CancelToken,
    ) -> Result<DiscoveryAutoconfig, TransportError> {
        Err(TransportError::new("offline"))
    }

    async fn srv(
        &self,
        _domain: &str,
        _cancel: &CancelToken,
    ) -> Result<DiscoverySrvReport, TransportError> {
        Err(TransportError::new("offline"))
    }

    async fn mx(
        &self,
        _domain: &str,
        _cancel: &CancelToken,
    ) -> Result<Vec<String>, TransportError> {
        Err(TransportError::new("offline"))
    }
}

/// Set in the child that does the work.
const INNER: &str = "POSTIO_OAUTH_TEST_INNER";

#[test]
fn the_consent_url_comes_back_whole_and_nothing_is_opened_or_fetched() {
    if std::env::var_os(INNER).is_some() {
        signs_in_as_asked();
        return;
    }
    let config = tempfile::tempdir().expect("a config directory");
    let ran = std::process::Command::new(std::env::current_exe().expect("this test"))
        .args([
            "the_consent_url_comes_back_whole_and_nothing_is_opened_or_fetched",
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
fn signs_in_as_asked() {
    // A token endpoint that counts who calls it.
    let token = TcpListener::bind("127.0.0.1:0").expect("a port");
    let token_url = format!("http://{}/token", token.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    std::thread::spawn({
        let calls = Arc::clone(&calls);
        move || {
            for stream in token.incoming() {
                if stream.is_ok() {
                    calls.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    });

    let config = std::path::PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").expect("set"));
    std::fs::create_dir_all(config.join("postio")).unwrap();
    std::fs::write(
        config.join("postio/providers.toml"),
        format!(
            r#"[provider.looptest]
display_name = "Loop Test"
domains = ["example.test"]
imap_host = "127.0.0.1"
imap_port = 1
imap_security = "none"
smtp_host = "127.0.0.1"
smtp_port = 1
smtp_security = "none"
auth = ["oauth2"]

[provider.looptest.oauth]
authorize = "http://127.0.0.1:1/authorize"
token = "{token_url}"
scopes = ["mail.everything"]
sources = ["own-client"]
"#
        ),
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let database = rt.block_on(test_support::memory());
    let blobs_dir = tempfile::tempdir().unwrap();
    let blobs =
        postio_storage::BlobStore::open(blobs_dir.path().to_path_buf(), &test_support::blob_keys())
            .unwrap();
    let host = Host::start(database, blobs, |wiring| {
        wiring
            .with_secrets(Arc::new(MemorySecretStore::new()))
            .with_discovery(Arc::new(Offline))
    })
    .expect("a host");
    let client = host.connect(ClientKind::Tui);

    let found = rt
        .block_on(client.discover("ada@example.test".into()))
        .expect("discovery");
    let Status::Found(settings) = found else {
        panic!("the overlay row was not found: {found:?}");
    };
    assert!(settings.oauth_sign_in, "the provider signs in in a browser");

    let submission = Submission {
        address: "ada@example.test".into(),
        name: "Ada".into(),
        password: String::new(),
        settings,
        oauth_client: Some(OAuthClientSubmission {
            client_id: "postio-test".into(),
            client_secret: None,
        }),
    };
    let consent = rt
        .block_on(client.begin_oauth(submission))
        .expect("a consent URL");
    assert!(
        consent
            .authorize_url
            .starts_with("http://127.0.0.1:1/authorize?"),
        "{}",
        consent.authorize_url
    );
    assert!(consent.authorize_url.contains("client_id=postio-test"));
    assert!(
        consent.redirect_uri.starts_with("http://127.0.0.1:"),
        "the redirect comes back to this machine: {}",
        consent.redirect_uri
    );
    assert_eq!(consent.provider, "Loop Test");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing was fetched");

    rt.block_on(client.cancel_oauth("ada@example.test".into()))
        .expect("cancelled");
    let finished = rt.block_on(client.finish_oauth("ada@example.test".into()));
    let error = finished.expect_err("a cancelled sign-in did not finish");
    assert!(error.message().contains("cancelled"), "{}", error.message());
    assert_eq!(calls.load(Ordering::SeqCst), 0, "and still nothing fetched");
}
