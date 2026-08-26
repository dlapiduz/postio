//! Opening a session with a bearer token rather than a password.
//!
//! #193, the wiring slice of #2: a token that exists but cannot authenticate a
//! session is not OAuth support. `ImapSession::open` builds `SaslPlainCreds`
//! for a password account; it has to build the OAUTHBEARER or XOAUTH2
//! credentials instead when the account says so.
//!
//! Against [`TestServer`] on a loopback port, so this exercises the real
//! `io-imap` auth path and the real SASL mechanisms rather than a transcript
//! that could only ever replay what was written down. Nothing here touches the
//! network.

use postio_imap::backend::BackendError;
use postio_imap::imap::{ImapSession, RustlsConnector};
use postio_imap::secret::Password;
use postio_imap::test_server::{TestMailbox, TestServer};
use postio_model::AuthMethod;

const TOKEN: &str = "ya29.a0-test-access-token";

fn connector() -> RustlsConnector {
    RustlsConnector::new().expect("a connector")
}

/// A server that accepts a bearer token and nothing else — so a test that
/// passes here cannot be passing because PLAIN quietly still worked.
async fn oauth_server(mechanism: &str) -> TestServer {
    TestServer::builder()
        .capabilities(["IMAP4rev1", "SASL-IR", &format!("AUTH={mechanism}")])
        .access_token(TOKEN)
        .mailbox(TestMailbox::new("INBOX"))
        .start()
        .await
}

#[tokio::test]
async fn a_session_opens_with_an_oauthbearer_token() {
    let server = oauth_server("OAUTHBEARER").await;
    let settings = server.settings().with_auth(AuthMethod::OAuth2);

    let session = ImapSession::open(&settings, &Password::new(TOKEN), &connector())
        .await
        .expect("OAUTHBEARER should authenticate against a server that offers it");

    assert!(
        !session.capabilities().names().is_empty(),
        "the post-auth capability re-read should still have happened — a \
         bearer session is a session like any other once it is open"
    );
}

#[tokio::test]
async fn a_session_opens_with_an_xoauth2_token() {
    let server = oauth_server("XOAUTH2").await;
    let settings = server.settings().with_auth(AuthMethod::XOAuth2);

    ImapSession::open(&settings, &Password::new(TOKEN), &connector())
        .await
        .expect("XOAUTH2 should authenticate against a server that offers it");
}

/// The token is the credential, so the wrong one has to be refused — a test
/// that only ever presents the right token cannot tell "the mechanism works"
/// from "the server accepts anything".
#[tokio::test]
async fn a_wrong_bearer_token_is_an_auth_failure() {
    let server = oauth_server("OAUTHBEARER").await;
    let settings = server.settings().with_auth(AuthMethod::OAuth2);

    let error = ImapSession::open(&settings, &Password::new("not-the-token"), &connector())
        .await
        .expect_err("a bad token must not open a session");

    assert!(
        matches!(error, BackendError::Auth { .. }),
        "a rejected token is an Auth failure, never a blind retry: {error:?}"
    );
}

/// The existing path, unchanged. `AuthMethod::Password` and `AppPassword` both
/// mean PLAIN, and an account that never mentions auth gets PLAIN by default —
/// #193's third acceptance criterion, and the one a regression would be
/// quietest about.
#[tokio::test]
async fn the_password_path_is_unchanged() {
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX"))
        .start()
        .await;

    // The default: nothing said about auth at all.
    ImapSession::open(
        &server.settings(),
        &Password::new(server.password()),
        &connector(),
    )
    .await
    .expect("an account that says nothing about auth still uses PLAIN");

    // And both password-shaped methods, said explicitly.
    for method in [AuthMethod::Password, AuthMethod::AppPassword] {
        let settings = server.settings().with_auth(method);
        ImapSession::open(&settings, &Password::new(server.password()), &connector())
            .await
            .unwrap_or_else(|error| panic!("{method:?} should still be PLAIN: {error:?}"));
    }
}

// ---------------------------------------------------------------------------
// A rejected token: one invalidate, one retry, then stop
// ---------------------------------------------------------------------------

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use postio_imap::auth::TokenSource;
use postio_imap::imap::{ConnectionPool, PoolConfig, Priority};
use postio_imap::secret::{AccountKey, SecretError};

/// Hands out `first` until invalidated, then `second`. Counts both calls, so
/// a test can assert the discipline rather than the outcome alone.
#[derive(Debug)]
struct RotatingSource {
    first: String,
    second: String,
    invalidated: AtomicUsize,
    handed_out: AtomicUsize,
}

impl RotatingSource {
    fn new(first: &str, second: &str) -> Self {
        Self {
            first: first.to_owned(),
            second: second.to_owned(),
            invalidated: AtomicUsize::new(0),
            handed_out: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl TokenSource for RotatingSource {
    async fn access_token(&self, _account: &AccountKey) -> Result<Password, SecretError> {
        self.handed_out.fetch_add(1, Ordering::SeqCst);
        Ok(Password::new(
            if self.invalidated.load(Ordering::SeqCst) == 0 {
                &self.first
            } else {
                &self.second
            },
        ))
    }

    async fn invalidate(&self, _account: &AccountKey) {
        self.invalidated.fetch_add(1, Ordering::SeqCst);
    }
}

fn authenticate_attempts(server: &TestServer) -> usize {
    server
        .commands()
        .iter()
        .filter(|line| line.to_ascii_uppercase().contains("AUTHENTICATE"))
        .count()
}

/// The discipline ADR 0006 Q5 asks for, at the layer that saw the rejection:
/// a stale token is invalidated once, retried once with whatever the source
/// hands back next, and that is the end of it.
#[tokio::test]
async fn a_stale_token_is_invalidated_once_and_retried_once() {
    let server = oauth_server("OAUTHBEARER").await;
    let source = Arc::new(RotatingSource::new("stale-token", TOKEN));

    let pool = ConnectionPool::with_token_source(
        server.settings().with_auth(AuthMethod::OAuth2),
        AccountKey::new(server.account()),
        Arc::clone(&source) as Arc<dyn TokenSource>,
        Arc::new(connector()),
        PoolConfig::default(),
    );

    pool.acquire(Priority::Interactive)
        .await
        .expect("the retry with a fresh token should open the session");

    assert_eq!(
        source.invalidated.load(Ordering::SeqCst),
        1,
        "the rejected token should be invalidated exactly once"
    );
    assert_eq!(
        source.handed_out.load(Ordering::SeqCst),
        2,
        "one token for the first attempt, one for the retry — never a third"
    );
    assert_eq!(
        authenticate_attempts(&server),
        2,
        "exactly one retry reaches the server"
    );
}

/// And it stops. A source with nothing new to offer — which is every stored
/// password, whose `invalidate` is a documented no-op — must not be retried
/// against, or a wrong password becomes an endless pair of round trips.
#[tokio::test]
async fn a_source_with_nothing_new_is_not_retried() {
    let server = oauth_server("OAUTHBEARER").await;
    let source = Arc::new(RotatingSource::new("wrong", "wrong"));

    let pool = ConnectionPool::with_token_source(
        server.settings().with_auth(AuthMethod::OAuth2),
        AccountKey::new(server.account()),
        Arc::clone(&source) as Arc<dyn TokenSource>,
        Arc::new(connector()),
        PoolConfig::default(),
    );

    let error = pool
        .acquire(Priority::Interactive)
        .await
        .expect_err("a token the source cannot improve on must fail");

    assert!(
        matches!(error, BackendError::Auth { .. }),
        "still an Auth failure, for the user to resolve: {error:?}"
    );
    assert_eq!(
        authenticate_attempts(&server),
        1,
        "re-presenting an identical credential is a wasted round trip, so the \
         retry must not happen at all"
    );
}
