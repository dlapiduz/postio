//! What a queued send costs while the server it is for says nothing (#1216).
//!
//! POSTIO-MEASUREMENT: its output is numbers a person reads, and it costs
//! 10.2 s, so it runs on the nightly timer rather than the merge path

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

/// Longer than the whole test: the reported shape is a connect outstanding, not a thread between
/// retries.
const NO_ANSWER_AFTER: Duration = Duration::from_secs(60);

/// Not an immediate error: the reported scenario is a connect that hangs for its whole timeout and
/// then gives up.
#[derive(Debug, Default)]
struct Silent {
    /// Asserted on: an engine that never tried to send would pass for the wrong reason.
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

#[tokio::test]
#[ignore = "needs a system D-Bus and a live NetworkManager"]
async fn the_networkmanager_listener_costs_no_cpu_either() {
    assert_networkmanager_is_really_there().await;
    measure_a_waiting_send(NetworkSource::NetworkManager).await;
}

/// `follow` returns quietly when the bus or service is absent, which looks the same as a listener
/// that cost nothing; this reads the property `follow` reads.
async fn assert_networkmanager_is_really_there() {
    // Awaited: wrapping it in a runtime of its own would panic.
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
    let state = proxy
        .get_property::<u32>("State")
        .await
        .expect("NetworkManager's State property");
    eprintln!("NetworkManager reports state {state}; the listener has a bus to follow");
}

#[tokio::test]
async fn a_queued_send_to_a_silent_server_costs_no_cpu_while_it_waits() {
    measure_a_waiting_send(NetworkSource::Ignored).await;
}

async fn measure_a_waiting_send(network: NetworkSource) {
    assert_the_clock_can_see_a_burn();

    // Left in, pointed at the test writer so it is silent unless `--nocapture`: the drain's outcome
    // for the send is what an investigation wants.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let database = test_support::memory().await;
    let report = seed_small(&database, 11).await;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let (sink, _events) = event_channel();

    // Queued before the engine starts: an arrival mid-run races the first sync.
    {
        let connection = database.connect().await.expect("checkout");
        let mut account = postio_storage::repository::AccountRepository::new(&connection)
            .get(report.account.id)
            .await
            .expect("read the account")
            .expect("the seeded account");
        // The seed writes no identity, and a send without one fails before reaching the transport.
        if account.identities.is_empty() {
            account
                .identities
                .push(postio_model::account::Identity::new(
                    account.id,
                    account.address.clone(),
                ));
            postio_storage::repository::AccountRepository::new(&connection)
                .update(&mut account)
                .await
                .expect("give the account an identity");
        }
        let mut draft = postio_model::draft::Draft::new(account.id);
        draft.start_as(&account.identities[0]);
        draft.to = vec![postio_model::address::EmailAddress::new(
            None::<String>,
            "grace@example.net",
        )];
        draft.subject = "Analytical engine".to_owned();
        draft.body.text = Some("Notes on the difference engine.".to_owned());
        let draft_id = DraftRepository::new(&connection)
            .save(&mut draft)
            .await
            .expect("save the draft");
        OperationQueueRepository::new(&connection)
            .enqueue(
                account.id,
                OperationTarget::Draft(draft_id),
                &Operation::Send { draft: draft_id },
                chrono::Utc::now(),
            )
            .await
            .expect("enqueue the send");
    }

    // A password the send can authenticate with; without one it fails before the transport.
    let secrets = Arc::new(postio_account::secret::MemorySecretStore::default());
    {
        let connection = database.connect().await.expect("checkout");
        let account = postio_storage::repository::AccountRepository::new(&connection)
            .get(report.account.id)
            .await
            .expect("read the account")
            .expect("the seeded account");
        let key = postio_account::secret::AccountKey::new(&account.address.address);
        // Awaited, not `block_on`ed: this is async under `#[tokio::test]`, and a nested runtime
        // panics (it did every night, unseen on the merge path).
        secrets
            .store(&key, &postio_account::secret::Password::new("pw"))
            .await
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

    // Settle first: measuring across the first sync would measure the sync.
    std::thread::sleep(postio_test_support::scaled(Duration::from_secs(2)));

    let window = postio_test_support::scaled(Duration::from_secs(3));
    let before = cpu_time();
    let started = Instant::now();
    std::thread::sleep(window);
    let burned = cpu_time().saturating_sub(before);
    let elapsed = started.elapsed();

    // The dial started during the settle is still outstanding; without this an engine that gave up
    // on the first error would pass.
    let dialled = smtp.attempts.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        dialled > 0,
        "the engine never dialled SMTP, so this measured an idle engine \
         rather than one waiting on a send"
    );
    eprintln!("{dialled} dial(s), {burned:?} of CPU across {elapsed:?}");

    // A tenth of a core: the engine's real work here is milliseconds and a spin is the whole core.
    let ceiling = elapsed / 10;
    assert!(
        burned < ceiling,
        "the engine burned {burned:?} of CPU in {elapsed:?} while a queued \
         send waited on a server that never answers — {:.0}% of a core. \
         Waiting is not work: something is spinning rather than parking.",
        burned.as_secs_f64() / elapsed.as_secs_f64() * 100.0,
    );
}
