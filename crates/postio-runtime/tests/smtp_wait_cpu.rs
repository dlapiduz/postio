//! What a queued send costs while the server it is for says nothing (#1216).
//!
//! The report: `postio-sync` burned 99.4 s of CPU in one thread over a
//! thirteen-minute session, and the burn ended exactly when a queued send
//! against an unreachable SMTP host exhausted its retries — 8 attempts of 30 s
//! each, and a silent gap in the journal of exactly 240 s. That is a
//! correlation and the issue says so; nothing named the loop.
//!
//! This is the measurement the correlation was standing in for. A real engine,
//! a real queued `Operation::Send`, and a connector that answers nothing —
//! which is what an unreachable host is, minus the network. If the wait costs
//! CPU, it costs it here.
//!
//! POSTIO-MEASUREMENT: its output is numbers a person reads, and it costs
//! 10.2 s, so it runs on the nightly timer rather than the merge path
//! (#1450). `.config/nextest.toml`'s `profile.default` filter is what holds
//! it back; run it with
//!
//! ```text
//! cargo nextest run --profile nightly -p postio-runtime -E 'binary(smtp_wait_cpu)'
//! ```
//!
//! # Its own binary, and why
//!
//! It reads this **process's** CPU time (`postio_test_support::cpu`), so anything
//! else running in the process is measured too. `runtime_suite` is one binary
//! over libtest's thread pool, where a neighbour compiling a regex would land
//! in this reading as a spin. `shutdown.rs` and `logging_privacy.rs` are out
//! of that binary for the same class of reason, stated in its module docs: a
//! case that needs its own process does not belong in it.
//!
//! Nothing here touches the network. The IMAP side is `MockBackend`; the SMTP
//! side is [`Silent`], which sleeps and then answers the way a connect timeout
//! does.

use std::sync::Arc;
use std::time::{Duration, Instant};

use postio_account::backend::{MockBackend, MockMailbox};
use postio_account::secret::SecretStore;
use postio_core::bridge::event_channel;
use postio_model::operation::{Operation, OperationTarget};
use postio_runtime::engine::{Engine, EngineParts, NetworkSource, SystemClock};
use postio_smtp::transport::{SmtpConnector, SmtpStream, TransportError};
use postio_storage::repository::{DraftRepository, OperationQueueRepository};
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};
use postio_test_support::cpu::{assert_the_clock_can_see_a_burn, cpu_time};

/// How long a connect to a host that never answers takes to give up.
///
/// Longer than the whole test, deliberately. The reported shape is 8 attempts
/// of 30 s **back to back** — the journal's silent gap is exactly 240 s — so
/// what was being measured there was a thread with a connect outstanding, not
/// a thread between retries. A short timeout here would put most of the
/// measurement window in the gap between attempts, where an idle engine is
/// idle for a reason that has nothing to do with the bug.
const NO_ANSWER_AFTER: Duration = Duration::from_secs(60);

/// A connector for a host that is not there.
///
/// Not an immediate error: an immediate failure is a different scenario, and
/// the reported one is a connect that hangs for its whole timeout and then
/// gives up. `TransportError::Connect` is what `transport::tcp`'s own
/// `tokio::time::timeout` turns that into, and it is the error the journal
/// line in #1216 was printed from.
#[derive(Debug, Default)]
struct Silent {
    /// How many times the engine has dialled.
    ///
    /// Asserted on, because a measurement of an engine that never tried to
    /// send would pass for the wrong reason and prove nothing: no password,
    /// no SMTP settings, an operation abandoned on its first error, and the
    /// CPU reading is of an idle engine with nothing to wait for.
    attempts: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl SmtpConnector for Silent {
    async fn connect_tcp(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn SmtpStream>, TransportError> {
        self.connect_tls(host, port).await
    }

    async fn connect_tls(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn SmtpStream>, TransportError> {
        self.attempts
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tokio::time::sleep(NO_ANSWER_AFTER).await;
        Err(TransportError::Connect {
            host: host.to_owned(),
            port,
            reason: format!("no answer within {NO_ANSWER_AFTER:?}"),
        })
    }
}

/// Drive one future to completion on a runtime of its own.
///
/// The test itself is synchronous — it sleeps and reads `/proc` — and the two
/// setup calls that are async do not need a runtime shared with anything.
fn futures_lite_block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(future)
}

/// The same measurement with the NetworkManager listener running (#1216).
///
/// `#[ignore]`: it needs a system D-Bus and a live NetworkManager, which CI
/// has neither of. `follow` returns immediately without them, so an
/// unattended run would report an idle engine and prove nothing — the same
/// vacuous pass this file's first version had.
///
/// Why it exists: every other engine test uses `NetworkSource::Ignored`, so
/// nothing spawns `network::follow`, and the listener is the only task that
/// shares the engine's current-thread runtime. While the engine is parked
/// inside `drain`'s await on a 30 s connect, it is the only thing that *can*
/// consume that thread — which makes it the last candidate standing for
/// #1216's burn once the send itself is measured at zero.
///
/// Run it by hand during an investigation:
///
/// ```text
/// cargo test -p postio-runtime --test smtp_wait_cpu -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs a system D-Bus and a live NetworkManager"]
fn the_networkmanager_listener_costs_no_cpu_either() {
    assert_networkmanager_is_really_there();
    measure_a_waiting_send(NetworkSource::NetworkManager);
}

/// The listener has something to listen to.
///
/// `follow` returns quietly when the bus or the service is absent — which is
/// correct, and is also indistinguishable from a listener that ran and cost
/// nothing. This file has already had one green test that measured an engine
/// doing nothing at all; the same reading twice would be a coincidence worth
/// refusing to rely on. Reads the property `follow` reads, from the process
/// that will run it.
fn assert_networkmanager_is_really_there() {
    let state = futures_lite_block_on(async {
        let connection = zbus::Connection::system()
            .await
            .expect("a system bus (this case is #[ignore]d because CI has none)");
        let proxy = zbus::Proxy::new(
            &connection,
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
        )
        .await
        .expect("NetworkManager on the bus");
        proxy
            .get_property::<u32>("State")
            .await
            .expect("NetworkManager's State property")
    });
    eprintln!("NetworkManager reports state {state}; the listener has a bus to follow");
}

#[test]
fn a_queued_send_to_a_silent_server_costs_no_cpu_while_it_waits() {
    measure_a_waiting_send(NetworkSource::Ignored);
}

/// The measurement both cases share: an engine with a send it cannot deliver,
/// and what it costs to sit there.
fn measure_a_waiting_send(network: NetworkSource) {
    assert_the_clock_can_see_a_burn();

    // Left in, and pointed at the test writer so it is silent unless someone
    // asks. The next person to look at #1216 will want to see the drain's
    // outcome for the send, and finding out that it needs a subscriber is
    // half an hour of the investigation. `--nocapture` shows it.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let database = test_support::memory();
    let report = seed_small(&database, 11);
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = event_channel();

    // The send, queued before the engine starts: an operation that arrives
    // mid-run races the first sync, and what is under test is the waiting,
    // not the arrival.
    {
        let connection = database.connection().expect("checkout");
        let mut account = postio_storage::repository::AccountRepository::new(&connection)
            .get(report.account.id)
            .expect("read the account")
            .expect("the seeded account");
        // The seed writes no identity, and a send without one fails before
        // it reaches the transport: "the account has no identity to send as".
        if account.identities.is_empty() {
            account
                .identities
                .push(postio_model::account::Identity::new(
                    account.id,
                    account.address.clone(),
                ));
            postio_storage::repository::AccountRepository::new(&connection)
                .update(&mut account)
                .expect("give the account an identity");
        }
        let mut draft = postio_model::draft::Draft::new(account.id);
        draft.use_identity(&account.identities[0]);
        draft.to = vec![postio_model::address::EmailAddress::new(
            None::<String>,
            "grace@example.net",
        )];
        draft.subject = "Analytical engine".to_owned();
        draft.body.text = Some("Notes on the difference engine.".to_owned());
        let draft_id = DraftRepository::new(&connection)
            .save(&mut draft)
            .expect("save the draft");
        OperationQueueRepository::new(&connection)
            .enqueue(
                account.id,
                OperationTarget::Draft(draft_id),
                &Operation::Send { draft: draft_id },
                chrono::Utc::now(),
            )
            .expect("enqueue the send");
    }

    // A password the send can actually authenticate with. Without one the
    // operation fails before it reaches the transport, which is a different
    // scenario and measures nothing about waiting.
    let secrets = Arc::new(postio_account::secret::MemorySecretStore::default());
    {
        let connection = database.connection().expect("checkout");
        let account = postio_storage::repository::AccountRepository::new(&connection)
            .get(report.account.id)
            .expect("read the account")
            .expect("the seeded account");
        let key = postio_account::secret::AccountKey::new(&account.address.address);
        futures_lite_block_on(secrets.store(&key, &postio_account::secret::Password::new("pw")))
            .expect("store the password");
    }

    let smtp = Arc::new(Silent::default());
    let backend = Arc::new(
        MockBackend::builder()
            .mailbox(MockMailbox::new("INBOX"))
            .mailbox(MockMailbox::new("Sent"))
            .build(),
    );
    let _engine = Engine::spawn(EngineParts {
        account: report.account.id,
        database: database.clone(),
        blobs,
        backend: backend.clone(),
        smtp: smtp.clone(),
        tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(
            secrets.clone(),
        )),
        events: sink,
        retry: Default::default(),
        backfill: Default::default(),
        reconnect: Default::default(),
        watch: Default::default(),
        network,
        mailbox_roles: Default::default(),
        clock: Arc::new(SystemClock),
    })
    .expect("the engine starts");

    // Let the first sync happen and settle. Measuring across it would measure
    // the sync, which is work the engine is supposed to be doing.
    std::thread::sleep(postio_test_support::scaled(Duration::from_secs(2)));

    let window = postio_test_support::scaled(Duration::from_secs(3));
    let before = cpu_time();
    let started = Instant::now();
    std::thread::sleep(window);
    let burned = cpu_time().saturating_sub(before);
    let elapsed = started.elapsed();

    // The measurement is of an engine that is actually waiting on a send.
    // Without this the test passes for an engine that gave up on the first
    // error, or never had the settings to try.
    // The dial started during the settle above and is still outstanding: the
    // connector has not answered and will not for another minute. Without
    // this the test passes for an engine that gave up on the first error, or
    // never had the settings to try.
    let dialled = smtp.attempts.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        dialled > 0,
        "the engine never dialled SMTP, so this measured an idle engine \
         rather than one waiting on a send"
    );
    eprintln!("{dialled} dial(s), {burned:?} of CPU across {elapsed:?}");

    // A tenth of one core. The engine has real work in this window — a 5 s
    // supervisor tick, a queue poll every 500 ms, and a send attempt every
    // time the backoff comes due — and all of it together is milliseconds.
    // A spin is the whole core, so the margin between the two answers is an
    // order of magnitude and the threshold does not have to be delicate.
    let ceiling = elapsed / 10;
    assert!(
        burned < ceiling,
        "the engine burned {burned:?} of CPU in {elapsed:?} while a queued \
         send waited on a server that never answers — {:.0}% of a core. \
         Waiting is not work: something is spinning rather than parking.",
        burned.as_secs_f64() / elapsed.as_secs_f64() * 100.0,
    );
}
