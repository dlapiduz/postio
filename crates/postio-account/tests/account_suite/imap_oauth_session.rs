use postio_account::backend::BackendError;
use postio_account::imap::{ImapSession, RustlsConnector};
use postio_account::secret::Password;
use postio_account::test_server::{TestMailbox, TestServer};
use postio_model::AuthMethod;

const TOKEN: &str = "ya29.a0-test-access-token";

fn connector() -> RustlsConnector {
    RustlsConnector::new().expect("a connector")
}

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

/// A test that only presents the right token cannot tell "the mechanism works" from "the server
/// accepts anything".
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

#[tokio::test]
async fn the_password_path_is_unchanged() {
    let server = TestServer::builder()
        .mailbox(TestMailbox::new("INBOX"))
        .start()
        .await;

    ImapSession::open(
        &server.settings(),
        &Password::new(server.password()),
        &connector(),
    )
    .await
    .expect("an account that says nothing about auth still uses PLAIN");

    for method in [AuthMethod::Password, AuthMethod::AppPassword] {
        let settings = server.settings().with_auth(method);
        ImapSession::open(&settings, &Password::new(server.password()), &connector())
            .await
            .unwrap_or_else(|error| panic!("{method:?} should still be PLAIN: {error:?}"));
    }
}

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use postio_account::auth::TokenSource;
use postio_account::imap::{ConnectionPool, PoolConfig, Priority};
use postio_account::secret::{AccountKey, SecretError};

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

/// ADR 0006 Q5: a stale token is invalidated once and retried once with what the source hands back
/// next.
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

/// A source with nothing new (every stored password) must not be retried, or a wrong password
/// loops.
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

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use postio_account::oauth::{OwnClientTokenSource, TokenResponse};
use postio_account::secret::MemorySecretStore;

fn slow_counting_token_endpoint() -> (url::Url, Arc<AtomicUsize>) {
    let served = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let counter = Arc::clone(&served);

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let mut length = 0usize;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).expect("header line");
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut discard = vec![0u8; length];
            let _ = reader.read_exact(&mut discard);
            counter.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(150));

            let body = format!(r#"{{"access_token":"{TOKEN}","expires_in":3600}}"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    (
        format!("http://127.0.0.1:{port}/token").parse().unwrap(),
        served,
    )
}

/// Counts concurrent `access_token` calls: sequential opens would also refresh once, so the overlap
/// is asserted causally, not by stopwatch.
#[derive(Debug)]
struct Overlap {
    inner: Arc<dyn TokenSource>,
    live: AtomicUsize,
    peak: AtomicUsize,
}

#[async_trait]
impl TokenSource for Overlap {
    async fn access_token(&self, account: &AccountKey) -> Result<Password, SecretError> {
        let live = self.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(live, Ordering::SeqCst);
        let token = self.inner.access_token(account).await;
        self.live.fetch_sub(1, Ordering::SeqCst);
        token
    }

    async fn invalidate(&self, account: &AccountKey) {
        self.inner.invalidate(account).await;
    }
}

/// ADR 0006 Q5: three connections on an expired token refresh once, since a rotating provider
/// invalidates the others.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_sessions_on_an_expired_token_cost_one_refresh() {
    let server = oauth_server("OAUTHBEARER").await;
    let (token_url, refreshes) = slow_counting_token_endpoint();

    let source = Arc::new(OwnClientTokenSource::new(
        Arc::new(MemorySecretStore::new()),
        token_url,
        "client-1",
        None,
        None,
    ));
    let account = AccountKey::new(server.account());
    source
        .seed(
            &account,
            TokenResponse {
                access_token: Password::new("expired"),
                refresh_token: Some(Password::new("the-refresh-token")),
                expires_in: Some(Duration::from_secs(0)),
                token_type: "Bearer".to_owned(),
                scope: None,
            },
        )
        .await
        .expect("seed succeeds");

    let watched = Arc::new(Overlap {
        inner: Arc::clone(&source) as Arc<dyn TokenSource>,
        live: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
    });
    let pool = Arc::new(ConnectionPool::with_token_source(
        server.settings().with_auth(AuthMethod::OAuth2),
        account,
        Arc::clone(&watched) as Arc<dyn TokenSource>,
        Arc::new(connector()),
        PoolConfig::default(),
    ));

    let mut opening = tokio::task::JoinSet::new();
    for _ in 0..3 {
        let pool = Arc::clone(&pool);
        opening.spawn(async move { pool.acquire(Priority::Interactive).await.map(|_| ()) });
    }
    let mut opened = 0;
    while let Some(joined) = opening.join_next().await {
        joined
            .expect("the task should not panic")
            .expect("every session should open");
        opened += 1;
    }

    assert_eq!(opened, 3, "three sessions");
    assert_eq!(
        watched.peak.load(Ordering::SeqCst),
        3,
        "all three were asking for a credential at the same moment — without \
         that the refresh count below would pass for the wrong reason"
    );
    assert_eq!(
        refreshes.load(Ordering::SeqCst),
        1,
        "and one round trip to the token endpoint between them"
    );
    assert_eq!(
        authenticate_attempts(&server),
        3,
        "each session authenticated once — the refreshed token was right \
         first time, so nothing was retried"
    );
}
