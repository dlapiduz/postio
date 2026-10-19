//! A server that goes quiet in every way a server can, against the real pool.
//!
//! The pool is bounded, so a connection that is never given up on is a slot
//! lost for good, and a connection that is given back in a state nobody
//! checked is a corrupted exchange for whoever draws it next. Each case here
//! asserts the three things a person sees: the operation fails promptly with
//! a timeout, the connection is discarded rather than parked, and the next
//! operation succeeds on a fresh connection.
//!
//! The clock is tokio's paused one and the server is reached through memory
//! ([`TestServer::connector`]), so a sixty-second deadline costs milliseconds
//! and the assertions are about exact virtual time. Over a socket the paused
//! clock misfires (docs/gotchas.md).

use std::sync::Arc;
use std::time::Duration;

use postio_account::backend::{AppendMessage, BackendError, BodyPart, UidSet, VecSink};
use postio_account::cancel::CancelToken;
use postio_account::imap::{
    ConnectionPool, PoolConfig, Priority, append, fetch_headers, fetch_part, idle,
};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_account::test_server::{Fault, Quirk, TestMailbox, TestServer};
use postio_model::{RemoteId, UidValidity};
use tokio::time::Instant;

/// How long a command may go without a byte from the server.
const SILENCE: Duration = Duration::from_secs(60);

/// A bound that only a hang can reach. Twice the deadline: if an operation is
/// still running then, the deadline did not apply to it.
const HANG: Duration = Duration::from_secs(120);

const SEEDED: [&str; 3] = ["plain-text-simple", "attachment-pdf", "html-newsletter"];

async fn server() -> TestServer {
    TestServer::builder()
        .mailbox(
            TestMailbox::new("INBOX")
                .uid_validity(UidValidity::new(4_242))
                .corpus(SEEDED),
        )
        .mailbox(TestMailbox::new("Archive"))
        .start()
        .await
}

/// A pool whose connections run through memory, `capacity` bytes at a time.
async fn pool(server: &TestServer, capacity: usize) -> ConnectionPool {
    let store = MemorySecretStore::new();
    let key = AccountKey::new(server.account());
    store
        .store(&key, &Password::new(server.password()))
        .await
        .expect("seed the keyring");

    ConnectionPool::new(
        server.settings(),
        key,
        Arc::new(store),
        Arc::new(server.connector(capacity)),
        PoolConfig {
            command_timeout: SILENCE,
            ..PoolConfig::default()
        },
    )
}

async fn headers(pool: &ConnectionPool) -> Result<usize, BackendError> {
    fetch_headers(
        pool,
        "INBOX",
        &UidSet::all(),
        None,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .map(|found| found.len())
}

async fn body(pool: &ConnectionPool) -> Result<Vec<u8>, BackendError> {
    let mut sink = VecSink::new();
    fetch_part(
        pool,
        "INBOX",
        &RemoteId::new("4242:2"),
        &BodyPart::Whole,
        &mut sink,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await?;
    Ok(sink.as_slice().to_vec())
}

/// Runs `operation` and requires it to fail with a timeout of `SILENCE`,
/// reached after `SILENCE` of virtual time and not much more.
async fn times_out<T: std::fmt::Debug>(
    operation: impl Future<Output = Result<T, BackendError>>,
    lead_in: Duration,
) {
    let started = Instant::now();
    let outcome = tokio::time::timeout(HANG, operation)
        .await
        .expect("the operation hung past twice the deadline");
    let error = outcome.expect_err("a silent server cannot have succeeded");

    assert!(
        matches!(error, BackendError::TimedOut { after, .. } if after == SILENCE),
        "{error}"
    );
    assert!(error.is_transient(), "the caller has to be able to retry");
    let took = started.elapsed();
    assert!(
        took >= lead_in + SILENCE && took < lead_in + SILENCE + Duration::from_secs(1),
        "gave up after {took:?}, not after the {SILENCE:?} of silence"
    );
}

/// The poisoned connection was thrown away and a fresh one does the work.
async fn recovers(pool: &ConnectionPool) {
    let stats = pool.stats();
    assert_eq!(
        stats.idle, 0,
        "the stalled connection went back to the pool"
    );
    assert_eq!(
        stats.in_use, 0,
        "the stalled connection is still checked out"
    );
    let opened = stats.opened;

    assert_eq!(headers(pool).await.expect("a fresh connection works"), 3);
    assert_eq!(
        pool.stats().opened,
        opened + 1,
        "the next operation reused a connection instead of opening one"
    );
}

#[tokio::test(start_paused = true)]
async fn a_command_the_server_never_answers_times_out_and_is_replaced() {
    let server = server().await;
    let pool = pool(&server, 64 * 1024).await;
    server.inject(Fault::Stall {
        during: "FETCH".to_owned(),
    });

    times_out(headers(&pool), Duration::ZERO).await;

    recovers(&pool).await;
}

#[tokio::test(start_paused = true)]
async fn a_body_the_server_never_starts_times_out_and_is_replaced() {
    let server = server().await;
    let pool = pool(&server, 64 * 1024).await;
    server.inject(Fault::Stall {
        during: "FETCH".to_owned(),
    });

    times_out(body(&pool), Duration::ZERO).await;

    recovers(&pool).await;
}

#[tokio::test(start_paused = true)]
async fn a_server_that_stalls_mid_literal_times_out_and_is_replaced() {
    let server = server().await;
    let pool = pool(&server, 64 * 1024).await;
    server.inject(Fault::StallMidLiteral {
        during: "FETCH".to_owned(),
    });

    times_out(headers(&pool), Duration::ZERO).await;

    recovers(&pool).await;
}

#[tokio::test(start_paused = true)]
async fn a_streamed_body_that_stalls_mid_literal_times_out_and_is_replaced() {
    let server = server().await;
    let pool = pool(&server, 64 * 1024).await;
    server.inject(Fault::StallMidLiteral {
        during: "FETCH".to_owned(),
    });

    times_out(body(&pool), Duration::ZERO).await;

    recovers(&pool).await;
}

#[tokio::test(start_paused = true)]
async fn an_idle_that_is_never_acknowledged_after_done_times_out_and_is_replaced() {
    let server = server().await;
    let pool = pool(&server, 64 * 1024).await;
    server.inject(Fault::IgnoreDone);

    // The watcher is told to stop a second in. `DONE` goes out, and the
    // tagged completion that should answer it never comes.
    let stop = CancelToken::new();
    let wake = stop.clone();
    let operation = async {
        let (result, ()) = tokio::join!(
            idle(&pool, "INBOX", Duration::from_secs(3_600), &stop),
            async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                wake.cancel();
            }
        );
        result
    };
    times_out(operation, Duration::from_secs(1)).await;

    let stats = pool.stats();
    assert_eq!(stats.idle, 0, "the wedged watch connection was parked");
    let opened = stats.opened;

    // The watch lane is usable again, on a connection of its own.
    let events = tokio::time::timeout(
        HANG,
        idle(&pool, "INBOX", Duration::from_secs(2), &CancelToken::new()),
    )
    .await
    .expect("the next watch hung")
    .expect("the next watch works");
    assert!(events.is_empty());
    assert_eq!(pool.stats().opened, opened + 1);
}

#[tokio::test(start_paused = true)]
async fn a_server_that_stops_reading_a_write_times_out_and_is_replaced() {
    let server = server().await;
    // A pipe that holds four kilobytes, and a message that cannot fit.
    let pool = pool(&server, 4 * 1024).await;
    server.quirk(Quirk::DeafToLiterals);
    let message = AppendMessage::new(vec![b'x'; 256 * 1024]);

    times_out(
        async { append(&pool, "Archive", &message, Priority::Interactive).await },
        Duration::ZERO,
    )
    .await;

    // The server is still deaf to literals, and a fetch carries none.
    recovers(&pool).await;
}

#[tokio::test(start_paused = true)]
async fn an_operation_dropped_mid_read_does_not_return_its_connection() {
    let server = server().await;
    let pool = pool(&server, 64 * 1024).await;
    server.inject(Fault::Stall {
        during: "FETCH".to_owned(),
    });

    // The caller gives up first: a user closed the view, a job was
    // superseded. The future is dropped with the response half read.
    let abandoned = tokio::time::timeout(Duration::from_secs(1), headers(&pool)).await;
    assert!(abandoned.is_err(), "the stalled fetch cannot have finished");

    recovers(&pool).await;
}

#[tokio::test(start_paused = true)]
async fn a_watch_dropped_mid_idle_does_not_return_its_connection() {
    let server = server().await;
    let pool = pool(&server, 64 * 1024).await;

    let abandoned = tokio::time::timeout(
        Duration::from_secs(1),
        idle(
            &pool,
            "INBOX",
            Duration::from_secs(3_600),
            &CancelToken::new(),
        ),
    )
    .await;
    assert!(abandoned.is_err(), "nothing arrived, so the idle runs on");

    let stats = pool.stats();
    assert_eq!(stats.idle, 0, "a connection still inside IDLE was parked");
    let opened = stats.opened;

    let events = tokio::time::timeout(
        HANG,
        idle(&pool, "INBOX", Duration::from_secs(2), &CancelToken::new()),
    )
    .await
    .expect("the next watch hung on the abandoned connection")
    .expect("the next watch works");
    assert!(events.is_empty());
    assert_eq!(pool.stats().opened, opened + 1);
}
