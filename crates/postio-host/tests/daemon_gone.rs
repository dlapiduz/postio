//! A daemon that goes away while a frontend is connected.
//!
//! The frontend has to learn it -- to say so, and to offer to reconnect --
//! rather than find out one failed call at a time. This stops a host that
//! still has a client, as a crash or a `kill` stops `postio-daemon`, and
//! asserts on what the client can see: its `closed` signal, and what a call
//! answers afterwards.

use std::sync::Arc;
use std::time::Duration;

use postio_client::Disconnected;
use postio_client::protocol::ClientKind;
use postio_client::socket::{Endpoint, connect};
use postio_host::Host;
use postio_storage::test_support;

/// How long anything here may take to happen before it has not.
const PATIENCE: Duration = Duration::from_secs(10);

/// A host over an empty store, serving `endpoint` until `stop` is sent.
fn a_daemon(
    endpoint: &Endpoint,
) -> (
    tokio::sync::oneshot::Sender<()>,
    std::thread::JoinHandle<()>,
    tempfile::TempDir,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let database = rt.block_on(test_support::memory());
    let blobs_dir = tempfile::tempdir().expect("a blob directory");
    let blobs =
        postio_storage::BlobStore::open(blobs_dir.path().to_path_buf(), &test_support::blob_keys())
            .expect("a blob store");
    let host = Arc::new(Host::start(database, blobs, |wiring| wiring).expect("a host"));
    let listener = postio_host::serve::bind(endpoint).expect("the socket binds");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = std::thread::spawn(move || {
        host.serve_until(listener, Duration::from_secs(30), async move {
            let _ = stopped.await;
        });
        // As `postio-daemon` does once serving returns: the host stops,
        // and goes.
        host.stop();
        drop(host);
    });
    (stop, serving, blobs_dir)
}

#[test]
fn a_client_hears_its_daemon_go_and_every_call_after_says_so() {
    let runtime_dir = tempfile::tempdir().expect("a runtime directory");
    let endpoint = Endpoint::at(runtime_dir.path().join("postio"));
    let (stop, serving, _blobs) = a_daemon(&endpoint);
    let client = connect(&endpoint, ClientKind::Tui).expect("connects");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");

    // Connected, it is not closed.
    let early = rt.block_on(async {
        tokio::time::timeout(Duration::from_millis(300), client.closed()).await
    });
    assert!(early.is_err(), "closed while the daemon is serving");
    rt.block_on(client.accounts()).expect("the daemon answers");

    stop.send(()).expect("the daemon is serving");
    serving.join().expect("the daemon stopped");

    let heard = rt.block_on(async { tokio::time::timeout(PATIENCE, client.closed()).await });
    assert!(heard.is_ok(), "the client never heard its daemon go");
    let error = rt
        .block_on(client.accounts())
        .expect_err("nobody is there to answer");
    assert_eq!(error.message(), Disconnected.to_string());
}

#[test]
fn a_reconnected_client_reaches_the_new_daemon_through_every_clone() {
    let runtime_dir = tempfile::tempdir().expect("a runtime directory");
    let endpoint = Endpoint::at(runtime_dir.path().join("postio"));
    let (stop, serving, _blobs) = a_daemon(&endpoint);
    let client = connect(&endpoint, ClientKind::Tui).expect("connects");
    // What a surface kept from before: a clone.
    let kept = client.clone();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let reconnected = client.reconnected();

    stop.send(()).expect("the daemon is serving");
    serving.join().expect("the daemon stopped");
    rt.block_on(async { tokio::time::timeout(PATIENCE, client.closed()).await })
        .expect("closed");

    let (stop, serving, _blobs_again) = a_daemon(&endpoint);
    let fresh = connect(&endpoint, ClientKind::Tui).expect("connects to the new one");
    client.reconnect(&fresh);

    assert!(reconnected.has_changed().expect("the client is alive"));
    rt.block_on(kept.accounts())
        .expect("the clone reaches the new daemon");
    let still = rt
        .block_on(async { tokio::time::timeout(Duration::from_millis(300), kept.closed()).await });
    assert!(still.is_err(), "the new connection is not closed");

    drop((client, kept, fresh));
    stop.send(()).expect("the daemon is serving");
    serving.join().expect("the daemon stopped");
}
