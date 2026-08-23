//! The connection pool: bounds, fairness, and replacing dead connections.
//!
//! Every session here is opened against a recorded transcript, so the pool's
//! real behaviour — how many connections it opens, which waiter it serves
//! first, when it throws one away — is observable without a server.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use postio_imap::backend::{BackendError, Capability};
use postio_imap::imap::{
    ConnectionPool, ConnectionSettings, ImapScript, PoolConfig, Priority, ScriptedConnector,
};
use postio_imap::secret::{AccountKey, MemorySecretStore, Password, SecretStore};

const ACCOUNT: &str = "someone@example.com";

/// A pool over a scripted iCloud server, plus the connector so a test can ask
/// how many connections were actually opened.
async fn pool_with(
    config: PoolConfig,
    connector: ScriptedConnector,
) -> (ConnectionPool, ScriptedConnector) {
    let store = MemorySecretStore::new();
    let key = AccountKey::new(ACCOUNT);
    store
        .store(&key, &Password::new("app-specific-password"))
        .await
        .expect("seed the keyring");

    let pool = ConnectionPool::new(
        ConnectionSettings::icloud(ACCOUNT),
        key,
        Arc::new(store),
        Arc::new(connector.clone()),
        config,
    );
    (pool, connector)
}

async fn pool(config: PoolConfig) -> (ConnectionPool, ScriptedConnector) {
    pool_with(config, ScriptedConnector::icloud()).await
}

// ---------------------------------------------------------------------------
// Bounds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nothing_is_connected_until_something_is_asked_for() {
    let (pool, connector) = pool(PoolConfig::default()).await;

    assert_eq!(pool.stats().opened, 0);
    assert!(connector.log().tls.is_empty());
    assert!(pool.capabilities().is_none());
    assert!(pool.dispatch().is_none());
}

#[tokio::test]
async fn the_first_connection_settles_what_the_server_can_do() {
    let (pool, _) = pool(PoolConfig::default()).await;

    pool.execute(Priority::Interactive, async |_| Ok(()))
        .await
        .unwrap();

    let dispatch = pool
        .dispatch()
        .expect("capabilities after the first connect");
    assert!(dispatch.supports(Capability::QResync));
    assert_eq!(
        dispatch.resync_strategy(),
        postio_imap::imap::ResyncStrategy::QResync
    );
}

#[tokio::test]
async fn a_parked_connection_is_reused_rather_than_reopened() {
    let (pool, connector) = pool(PoolConfig::default()).await;

    for _ in 0..5 {
        pool.execute(Priority::Interactive, async |_| Ok(()))
            .await
            .unwrap();
    }

    assert_eq!(pool.stats().opened, 1);
    assert_eq!(connector.log().tls.len(), 1);
    assert_eq!(pool.stats().idle, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_pool_never_exceeds_its_limit_under_concurrent_load() {
    let config = PoolConfig {
        max_connections: 3,
        dedicate_watch_connection: false,
        acquire_timeout: Duration::from_secs(10),
        ..PoolConfig::default()
    };
    let (pool, connector) = pool(config).await;
    let pool = Arc::new(pool);

    let live = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    let mut tasks = Vec::new();
    for _ in 0..12 {
        let pool = Arc::clone(&pool);
        let live = Arc::clone(&live);
        let peak = Arc::clone(&peak);
        tasks.push(tokio::spawn(async move {
            pool.execute(Priority::Background, async |_| {
                let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(25)).await;
                live.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
            .await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }

    assert!(
        peak.load(Ordering::SeqCst) <= 3,
        "{} connections were in use at once",
        peak.load(Ordering::SeqCst)
    );
    assert!(
        connector.log().tls.len() <= 3,
        "the pool opened {} connections for a limit of 3",
        connector.log().tls.len()
    );
    assert_eq!(pool.stats().in_use, 0);
    assert_eq!(pool.stats().capacity, 3);
}

#[tokio::test]
async fn waiting_too_long_for_a_slot_is_a_timeout_and_a_retryable_one() {
    let config = PoolConfig {
        max_connections: 1,
        dedicate_watch_connection: false,
        acquire_timeout: Duration::from_millis(50),
        ..PoolConfig::default()
    };
    let (pool, _) = pool(config).await;

    let held = pool.acquire(Priority::Interactive).await.unwrap();

    let error = pool.acquire(Priority::Background).await.unwrap_err();
    assert!(matches!(error, BackendError::TimedOut { .. }));
    assert!(error.is_transient());

    // The slot was not lost when the waiter gave up.
    drop(held);
    assert!(pool.acquire(Priority::Interactive).await.is_ok());
}

// ---------------------------------------------------------------------------
// Fairness
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_interactive_waiter_is_served_before_a_background_one_that_queued_first() {
    // A backfill must not make opening a thread wait behind it.
    let config = PoolConfig {
        max_connections: 1,
        dedicate_watch_connection: false,
        acquire_timeout: Duration::from_secs(5),
        ..PoolConfig::default()
    };
    let (pool, _) = pool(config).await;
    let pool = Arc::new(pool);

    let held = pool.acquire(Priority::Interactive).await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let background = {
        let pool = Arc::clone(&pool);
        let tx = tx.clone();
        tokio::spawn(async move {
            let _guard = pool.acquire(Priority::Background).await.unwrap();
            tx.send("background").unwrap();
        })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;

    let interactive = {
        let pool = Arc::clone(&pool);
        tokio::spawn(async move {
            let _guard = pool.acquire(Priority::Interactive).await.unwrap();
            tx.send("interactive").unwrap();
        })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(pool.stats().waiting, 2);

    drop(held);
    interactive.await.unwrap();
    background.await.unwrap();

    let mut served = Vec::new();
    while let Ok(who) = rx.try_recv() {
        served.push(who);
    }
    assert_eq!(served, ["interactive", "background"]);
}

#[tokio::test]
async fn the_watcher_has_a_connection_of_its_own() {
    // IDLE parks a connection for minutes. If it competed for a general slot,
    // watching the inbox would cost the slot a fetch needs.
    let config = PoolConfig {
        max_connections: 2,
        dedicate_watch_connection: true,
        acquire_timeout: Duration::from_millis(100),
        ..PoolConfig::default()
    };
    let (pool, connector) = pool(config).await;

    let _command = pool.acquire(Priority::Interactive).await.unwrap();

    // The only general slot is taken, and the watcher is still served.
    let watched = pool
        .watch(async |session| Ok(session.endpoint().to_owned()))
        .await;

    assert_eq!(watched.unwrap(), "imap.mail.me.com:993");
    assert_eq!(connector.log().tls.len(), 2);
}

#[tokio::test]
async fn a_one_connection_budget_lets_the_watcher_share_rather_than_deadlock() {
    let config = PoolConfig {
        max_connections: 1,
        dedicate_watch_connection: true,
        acquire_timeout: Duration::from_millis(100),
        ..PoolConfig::default()
    };
    let (pool, _) = pool(config).await;

    assert!(pool.watch(async |_| Ok(())).await.is_ok());
    assert_eq!(pool.stats().opened, 1);
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_dead_connection_is_thrown_away_and_replaced() {
    // The transcript answers the two handshake commands and then goes silent,
    // so the first command after the handshake sees the connection drop.
    let (pool, connector) = pool_with(
        PoolConfig::default(),
        ScriptedConnector::icloud().closing_after(2),
    )
    .await;

    let error = pool
        .execute(Priority::Interactive, async |session| {
            session.refresh_capabilities().await.map(|_| ())
        })
        .await
        .unwrap_err();
    assert!(error.is_transient());
    assert_eq!(pool.stats().idle, 0, "a dead connection was parked");

    // The next caller gets a fresh connection rather than the corpse.
    pool.execute(Priority::Interactive, async |_| Ok(()))
        .await
        .unwrap();

    assert_eq!(pool.stats().opened, 2);
    assert_eq!(connector.log().tls.len(), 2);
}

#[tokio::test]
async fn a_connection_parked_too_long_is_closed_rather_than_reused() {
    let config = PoolConfig {
        idle_timeout: Duration::from_millis(20),
        ..PoolConfig::default()
    };
    let (pool, connector) = pool(config).await;

    pool.execute(Priority::Interactive, async |_| Ok(()))
        .await
        .unwrap();
    assert_eq!(pool.stats().idle, 1);

    tokio::time::sleep(Duration::from_millis(40)).await;
    pool.execute(Priority::Interactive, async |_| Ok(()))
        .await
        .unwrap();

    assert_eq!(pool.stats().opened, 2);
    assert_eq!(connector.log().tls.len(), 2);
}

#[tokio::test]
async fn a_successful_operation_leaves_the_connection_in_the_pool() {
    let (pool, _) = pool(PoolConfig::default()).await;

    pool.execute(Priority::Interactive, async |session| {
        Ok(session.capabilities().names().len())
    })
    .await
    .unwrap();

    assert_eq!(pool.stats().idle, 1);
    assert_eq!(pool.stats().in_use, 0);
}

#[tokio::test]
async fn a_caller_can_discard_a_connection_it_left_in_a_strange_state() {
    let (pool, _) = pool(PoolConfig::default()).await;

    {
        let mut connection = pool.acquire(Priority::Interactive).await.unwrap();
        connection.discard();
        assert!(connection.is_discarded());
    }

    assert_eq!(pool.stats().idle, 0);
    assert_eq!(pool.stats().in_use, 0);
}

// ---------------------------------------------------------------------------
// Failure and shutdown
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_rejected_password_surfaces_and_does_not_consume_the_pool() {
    let script = ImapScript::new("* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN] ready")
        .on("AUTHENTICATE", "{tag} NO [AUTHENTICATIONFAILED] no");
    let (pool, _) = pool_with(PoolConfig::default(), ScriptedConnector::new(script)).await;

    let error = pool
        .execute(Priority::Interactive, async |_| Ok(()))
        .await
        .unwrap_err();

    assert!(error.is_authentication_failure());
    assert_eq!(pool.stats().opened, 0);

    // The slot the failed attempt held was given back, so the pool is not
    // permanently one connection smaller after a bad password.
    assert_eq!(pool.stats().in_use, 0);
    assert!(
        pool.execute(Priority::Interactive, async |_| Ok(()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_closed_pool_refuses_new_work_and_drops_what_it_was_holding() {
    let (pool, _) = pool(PoolConfig::default()).await;

    pool.execute(Priority::Interactive, async |_| Ok(()))
        .await
        .unwrap();
    assert_eq!(pool.stats().idle, 1);

    pool.close();

    assert_eq!(pool.stats().idle, 0);
    let error = pool.acquire(Priority::Interactive).await.unwrap_err();
    assert!(matches!(error, BackendError::NotConnected { .. }));
}
