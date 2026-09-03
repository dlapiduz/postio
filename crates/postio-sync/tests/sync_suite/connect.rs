//! Reconnection against the `MailBackend` mock: a flapping network, a refused
//! password, and a laptop lid.
//!
//! The schedule itself is unit-tested in `src/connect.rs` — it is arithmetic
//! and needs no backend. What needs one is the state machine around it: that a
//! drop is noticed, that recovery is automatic, and that some failures stop it
//! dead.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_account::backend::{Fault, MailBackend, MockBackend, MockMailbox};
use postio_sync::connect::{Blocker, Link, NetworkState, ReconnectPolicy, Supervisor};

/// A fixed entropy, so every delay below is exactly the midpoint of its jitter
/// window and the arithmetic in the assertions is legible.
const MIDPOINT: u64 = 500;

fn at(second: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap() + TimeDelta::seconds(second)
}

fn backend() -> MockBackend {
    MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .build()
}

/// The default policy with a stability window short enough to step over.
fn policy() -> ReconnectPolicy {
    ReconnectPolicy {
        stability: std::time::Duration::from_secs(30),
        ..ReconnectPolicy::default()
    }
}

fn retry_at(link: &Link) -> DateTime<Utc> {
    match link {
        Link::Waiting { retry_at, .. } => *retry_at,
        other => panic!("expected Waiting, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Coming up
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_first_poll_connects() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());

    let change = supervisor.poll(&backend, at(0), MIDPOINT).await;

    assert_eq!(change, Some(Link::Online { since: at(0) }));
    assert!(supervisor.link().is_online());
    assert_eq!(supervisor.attempts(), 0);
}

#[tokio::test]
async fn a_healthy_connection_is_left_alone() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());
    supervisor.poll(&backend, at(0), MIDPOINT).await;

    let before = backend.calls();
    assert_eq!(
        supervisor.poll(&backend, at(1), MIDPOINT).await,
        None,
        "nothing changed, so nothing is reported"
    );
    assert!(
        backend.calls() - before <= 1,
        "a liveness check, not a reconnection"
    );
}

// ---------------------------------------------------------------------------
// Going down and coming back
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_failed_attempt_backs_off_and_then_succeeds() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());

    backend.inject(Fault::Io("network is unreachable".to_owned()));
    let change = supervisor
        .poll(&backend, at(0), MIDPOINT)
        .await
        .expect("a change");

    assert_eq!(supervisor.attempts(), 1);
    // Equal jitter at the midpoint: half the 1s backoff plus half of the rest.
    assert_eq!(retry_at(&change), at(0) + TimeDelta::milliseconds(750));

    assert_eq!(
        supervisor.poll(&backend, at(0), MIDPOINT).await,
        None,
        "not due yet"
    );

    let change = supervisor.poll(&backend, at(1), MIDPOINT).await;
    assert_eq!(change, Some(Link::Online { since: at(1) }));
}

#[tokio::test]
async fn each_failure_waits_longer_than_the_last() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());
    let mut waits = Vec::new();
    let mut now = at(0);

    for _ in 0..6 {
        backend.inject(Fault::Io("network is unreachable".to_owned()));
        let change = supervisor
            .poll(&backend, now, MIDPOINT)
            .await
            .expect("a change");
        let next = retry_at(&change);
        waits.push(next - now);
        now = next;
    }

    for pair in waits.windows(2) {
        assert!(
            pair[1] > pair[0],
            "the wait must grow: {:?} then {:?}",
            pair[0],
            pair[1]
        );
    }
    assert_eq!(supervisor.attempts(), 6);
}

#[tokio::test]
async fn a_drop_noticed_by_a_command_puts_the_link_into_backoff() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());
    supervisor.poll(&backend, at(0), MIDPOINT).await;
    assert!(supervisor.link().is_online());

    // A fetch died mid-command; the drainer reports what it saw.
    let error = backend.status("nope").await.expect_err("no such mailbox");
    assert_eq!(
        supervisor.observe(&error, at(1)),
        None,
        "a refused command is the operation's problem, not the connection's"
    );
    assert!(supervisor.link().is_online());

    backend.inject(Fault::Disconnect);
    let error = backend
        .status("INBOX")
        .await
        .expect_err("the connection died");
    let change = supervisor.observe(&error, at(2)).expect("a change");

    assert!(matches!(change, Link::Waiting { attempts: 1, .. }));
    assert!(!supervisor.link().is_online());
}

// ---------------------------------------------------------------------------
// Flapping — the acceptance criterion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_flapping_link_converges_instead_of_thrashing() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());
    let mut now = at(0);
    let mut attempts_at_each_drop = Vec::new();

    // Ten cycles of: connect, immediately lose it.
    for _ in 0..10 {
        // The connection comes up.
        supervisor.poll(&backend, now, MIDPOINT).await;
        assert!(supervisor.link().is_online(), "up at {now}");

        // …and dies a second later, well inside the stability window.
        now += TimeDelta::seconds(1);
        backend.inject(Fault::Disconnect);
        let error = backend.status("INBOX").await.expect_err("dropped");
        let change = supervisor.observe(&error, now).expect("a change");
        attempts_at_each_drop.push(supervisor.attempts());
        now = retry_at(&change);
    }

    assert_eq!(
        attempts_at_each_drop,
        (1..=10).collect::<Vec<u32>>(),
        "a success that did not hold must not clear the count, or the client \
         reconnects as fast as the link can fail"
    );

    // Convergence: the tenth wait is vastly longer than the first, so the
    // client has stopped hammering the server.
    let first = policy().delay(1, MIDPOINT);
    let tenth = policy().delay(10, MIDPOINT);
    assert!(
        tenth > first * 20,
        "the schedule did not converge: {first:?} then {tenth:?}"
    );
    assert!(tenth <= policy().ceiling);
}

#[tokio::test]
async fn a_connection_that_holds_clears_the_count() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());

    backend.inject(Fault::Io("network is unreachable".to_owned()));
    supervisor.poll(&backend, at(0), MIDPOINT).await;
    backend.inject(Fault::Io("network is unreachable".to_owned()));
    supervisor.poll(&backend, at(10), MIDPOINT).await;
    assert_eq!(supervisor.attempts(), 2);

    supervisor.poll(&backend, at(20), MIDPOINT).await;
    assert!(supervisor.link().is_online());
    assert_eq!(
        supervisor.attempts(),
        2,
        "still provisional: it has not held yet"
    );

    // The window is measured from when it came up, which was at(20).
    supervisor.poll(&backend, at(49), MIDPOINT).await;
    assert_eq!(supervisor.attempts(), 2, "one second short");

    supervisor.poll(&backend, at(50), MIDPOINT).await;
    assert_eq!(
        supervisor.attempts(),
        0,
        "thirty seconds up is a recovery, so the next outage starts fresh"
    );
}

// ---------------------------------------------------------------------------
// Never spin on a hard failure — the acceptance criterion
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_auth_failure_stops_and_asks_the_user() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());

    backend.inject(Fault::AuthFailed);
    let change = supervisor.poll(&backend, at(0), MIDPOINT).await;

    let Some(Link::Blocked(blocker)) = change else {
        panic!("expected Blocked, got {change:?}");
    };
    assert!(blocker.needs_credentials());
    assert!(!blocker.reason().is_empty());

    // And it does not try again, however long anyone waits.
    let before = backend.calls();
    for minute in 1..30 {
        assert_eq!(
            supervisor.poll(&backend, at(minute * 60), MIDPOINT).await,
            None
        );
    }
    assert_eq!(
        backend.calls(),
        before,
        "a refused credential must not be retried: that is how an account gets \
         locked, and waiting cannot make a wrong password right"
    );
}

#[tokio::test]
async fn a_new_password_starts_over() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());
    backend.inject(Fault::AuthFailed);
    supervisor.poll(&backend, at(0), MIDPOINT).await;

    let change = supervisor.retry_now(at(60)).expect("a change");
    assert!(matches!(change, Link::Waiting { attempts: 0, .. }));
    assert_eq!(
        supervisor.attempts(),
        0,
        "a fresh start, not a continuation"
    );

    assert_eq!(
        supervisor.poll(&backend, at(60), MIDPOINT).await,
        Some(Link::Online { since: at(60) })
    );
}

#[tokio::test]
async fn a_failure_that_retrying_cannot_fix_also_stops() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());

    backend.inject(Fault::Rejected("this server is closed".to_owned()));
    let change = supervisor.poll(&backend, at(0), MIDPOINT).await;

    let Some(Link::Blocked(blocker)) = change else {
        panic!("expected Blocked, got {change:?}");
    };
    assert!(
        !blocker.needs_credentials(),
        "the password is fine; something else is not"
    );
    assert!(matches!(blocker, Blocker::Unrecoverable(_)));
}

#[tokio::test]
async fn a_server_that_asks_us_to_wait_is_obeyed() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());

    backend.inject(Fault::RateLimited(Some(std::time::Duration::from_secs(
        600,
    ))));
    let change = supervisor
        .poll(&backend, at(0), MIDPOINT)
        .await
        .expect("a change");

    assert_eq!(
        retry_at(&change),
        at(600),
        "ten minutes, because that is what the server asked for — our own \
         backoff would have come back in under a second"
    );
}

// ---------------------------------------------------------------------------
// The network itself
// ---------------------------------------------------------------------------

#[tokio::test]
async fn losing_the_network_parks_the_link_without_spending_attempts() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());
    supervisor.poll(&backend, at(0), MIDPOINT).await;

    assert_eq!(
        supervisor.set_network(NetworkState::Down, at(1)),
        Some(Link::Offline)
    );

    let before = backend.calls();
    for minute in 1..10 {
        assert_eq!(
            supervisor.poll(&backend, at(minute * 60), MIDPOINT).await,
            None
        );
    }
    assert_eq!(
        backend.calls(),
        before,
        "there is nothing to reach, so nothing is attempted"
    );
    assert_eq!(
        supervisor.attempts(),
        0,
        "and no attempt is burned on a network that is not there"
    );
}

#[tokio::test]
async fn the_network_coming_back_reconnects_at_once() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());

    // A long backoff is already in force…
    for second in 0..6 {
        backend.inject(Fault::Io("network is unreachable".to_owned()));
        supervisor.poll(&backend, at(second * 60), MIDPOINT).await;
    }
    supervisor.set_network(NetworkState::Down, at(400));

    // …and then the lid opens.
    let change = supervisor.set_network(NetworkState::Up, at(500));
    assert!(matches!(change, Some(Link::Waiting { .. })));
    assert_eq!(
        retry_at(&change.unwrap()),
        at(500),
        "waiting out a backoff measured against a network that no longer \
         exists is what makes waking a laptop feel slow"
    );

    assert_eq!(
        supervisor.poll(&backend, at(500), MIDPOINT).await,
        Some(Link::Online { since: at(500) })
    );
}

#[tokio::test]
async fn the_link_coming_up_collapses_a_backoff_that_never_saw_it_go_down() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());

    // Six failures, so a long backoff is in force.
    for minute in 0..6 {
        backend.inject(Fault::Io("network is unreachable".to_owned()));
        supervisor.poll(&backend, at(minute * 60), MIDPOINT).await;
    }
    assert_eq!(supervisor.attempts(), 6);
    let due = retry_at(supervisor.link());
    assert!(due > at(310), "the sixth wait is still outstanding at 310s");

    // NetworkManager reports connectivity *without* ever having reported a
    // loss: a move from "connected, no internet" to "connected", or simply the
    // first signal after the app started while the link was already down.
    // Nobody said Offline, so nothing has collapsed the wait.
    let change = supervisor
        .set_network(NetworkState::Up, at(310))
        .expect("a change");

    assert_eq!(
        retry_at(&change),
        at(310),
        "the operating system just said the link is back; waiting out a delay \
         measured against the one that was not there is exactly the lag this \
         signal exists to remove"
    );
    assert_eq!(
        supervisor.attempts(),
        6,
        "a link that is up is not a server that is reachable, so this collapses \
         the wait rather than assuming success — if the attempt fails, the \
         backoff carries on rather than starting again at one second"
    );

    backend.inject(Fault::Io("connection refused".to_owned()));
    supervisor.poll(&backend, at(310), MIDPOINT).await;
    assert_eq!(supervisor.attempts(), 7);
}

#[tokio::test]
async fn a_link_up_signal_with_no_wait_left_to_collapse_reports_nothing() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());
    backend.inject(Fault::Io("network is unreachable".to_owned()));
    let change = supervisor
        .poll(&backend, at(0), MIDPOINT)
        .await
        .expect("a change");
    let due = retry_at(&change);

    assert_eq!(
        supervisor.set_network(NetworkState::Up, due + TimeDelta::seconds(1)),
        None,
        "the attempt was already due, so there is nothing to bring forward and \
         nothing to tell the status line"
    );
}

#[tokio::test]
async fn a_link_up_signal_does_not_disturb_a_healthy_connection() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());
    supervisor.poll(&backend, at(0), MIDPOINT).await;

    assert_eq!(
        supervisor.set_network(NetworkState::Up, at(1)),
        None,
        "an already-connected client has nothing to learn from being told the \
         network works"
    );
    assert_eq!(supervisor.link(), &Link::Online { since: at(0) });
}

#[tokio::test]
async fn losing_track_of_the_network_is_not_news_that_it_came_back() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());
    backend.inject(Fault::Io("network is unreachable".to_owned()));
    let change = supervisor
        .poll(&backend, at(0), MIDPOINT)
        .await
        .expect("a change");
    let due = retry_at(&change);

    // NetworkManager stopped, or was never there after all.
    assert_eq!(supervisor.set_network(NetworkState::Unknown, at(1)), None);

    assert_eq!(
        retry_at(supervisor.link()),
        due,
        "not knowing is not evidence: only a positive link-up signal is worth \
         collapsing a backoff for"
    );
}

#[tokio::test]
async fn the_network_returning_does_not_unblock_a_refused_password() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());
    backend.inject(Fault::AuthFailed);
    supervisor.poll(&backend, at(0), MIDPOINT).await;

    assert_eq!(supervisor.set_network(NetworkState::Down, at(1)), None);
    assert_eq!(supervisor.set_network(NetworkState::Up, at(2)), None);

    assert!(
        matches!(supervisor.link(), Link::Blocked(_)),
        "a wrong password is still wrong on a different network"
    );
}

#[tokio::test]
async fn a_network_nobody_reported_on_is_still_worth_trying() {
    let backend = backend();
    let mut supervisor = Supervisor::new(policy());

    // Never told about NetworkManager at all — the case on a machine that does
    // not run it, and the case before the first signal arrives.
    assert_eq!(
        supervisor.poll(&backend, at(0), MIDPOINT).await,
        Some(Link::Online { since: at(0) }),
        "refusing to try because NetworkManager could not be found would be \
         worse than trying and failing"
    );
}

// ---------------------------------------------------------------------------
// A revoked grant, all the way to the user — ADR 0006 Q5, #194
// ---------------------------------------------------------------------------

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_account::auth::TokenSource;
use postio_account::imap::{ConnectionPool, ImapBackend, PoolConfig, RustlsConnector};
use postio_account::secret::{AccountKey, Password, SecretError};
use postio_account::test_server::{TestMailbox, TestServer};
use postio_model::AuthMethod;

/// A source whose grant is gone: every token it can mint is refused, and
/// invalidating only makes it mint a different refused one.
///
/// The shape that matters. A source handing back the *same* token twice is
/// already covered — the retry is skipped outright — and it would hide the
/// question being asked here, which is what happens when the retry really
/// runs and really fails.
#[derive(Debug)]
struct RevokedGrant {
    minted: AtomicUsize,
}

#[async_trait::async_trait]
impl TokenSource for RevokedGrant {
    async fn access_token(&self, _account: &AccountKey) -> Result<Password, SecretError> {
        let n = self.minted.fetch_add(1, Ordering::SeqCst);
        Ok(Password::new(format!("revoked-token-{n}")))
    }

    async fn invalidate(&self, _account: &AccountKey) {}
}

fn authenticate_attempts(server: &TestServer) -> usize {
    server
        .commands()
        .iter()
        .filter(|line| line.to_ascii_uppercase().contains("AUTHENTICATE"))
        .count()
}

/// The last criterion of #194, and the one no single layer can state: a grant
/// the provider has revoked costs exactly one retry, blocks the link, and
/// then stops — no timer brings it back.
///
/// Composed rather than duplicated, which is the point. The pool decides
/// *one retry*; the supervisor decides *stop asking*; neither knows about the
/// other. What this asserts is that the two together do not add up to a
/// client hammering a token endpoint that has already said no.
#[tokio::test]
async fn a_revoked_grant_reaches_attention_after_exactly_one_retry() {
    let server = TestServer::builder()
        .capabilities(["IMAP4rev1", "SASL-IR", "AUTH=OAUTHBEARER"])
        .access_token("the-token-this-account-no-longer-has")
        .mailbox(TestMailbox::new("INBOX"))
        .start()
        .await;
    let tokens = Arc::new(RevokedGrant {
        minted: AtomicUsize::new(0),
    });
    let backend = ImapBackend::over(Arc::new(ConnectionPool::with_token_source(
        server.settings().with_auth(AuthMethod::OAuth2),
        AccountKey::new(server.account()),
        Arc::clone(&tokens) as Arc<dyn TokenSource>,
        Arc::new(RustlsConnector::new().expect("a connector")),
        PoolConfig::default(),
    )));
    let mut supervisor = Supervisor::new(policy());

    let change = supervisor.poll(&backend, at(0), MIDPOINT).await;

    assert!(
        matches!(change, Some(Link::Blocked(Blocker::Authentication(_)))),
        "a revoked grant is the user's to resolve, not the backoff's: {change:?}"
    );
    assert_eq!(
        authenticate_attempts(&server),
        2,
        "one attempt and one retry — never a third"
    );

    // An hour later, and again an hour after that. `Link::Blocked` is where
    // the schedule stops: a client that kept trying would be asking a
    // provider that has already said no, at whatever cadence the backoff had
    // reached.
    let minted = tokens.minted.load(Ordering::SeqCst);
    for hour in 1..=2 {
        assert_eq!(
            supervisor.poll(&backend, at(hour * 3_600), MIDPOINT).await,
            None,
            "nothing is due, because nothing is scheduled"
        );
    }
    assert_eq!(
        authenticate_attempts(&server),
        2,
        "and no timer put another attempt on the wire"
    );
    assert_eq!(
        tokens.minted.load(Ordering::SeqCst),
        minted,
        "nor asked the source to mint another token"
    );

    // And it comes back the moment the user has done something about it,
    // which is what makes blocking safe rather than terminal.
    assert!(matches!(
        supervisor.retry_now(at(7_200)),
        Some(Link::Waiting { .. })
    ));
}
