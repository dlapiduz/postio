//! The connector reports every connection to the egress sink (#151).
//!
//! The privacy claim — nothing leaves this machine that the user did not
//! ask for — is proven with a log rather than asserted, and the log is only
//! as complete as the seams that feed it. This transport is the seam every
//! IMAP connection passes through, so a connector holding a sink must
//! report the successful connect and the failed one alike — a log that only
//! ever showed successes would hide exactly the traffic a user most wants
//! to see.
//!
//! Loopback only: the "server" is this crate's own `TestServer` on
//! 127.0.0.1, so nothing here touches the network.

use std::sync::{Arc, Mutex};

use postio_imap::imap::{ImapConnector, RustlsConnector};
use postio_imap::test_server::TestServer;
use postio_model::egress::{EgressEvent, EgressOutcome, EgressSink, EgressSubsystem};

#[derive(Default)]
struct Recorded(Mutex<Vec<EgressEvent>>);

impl EgressSink for Recorded {
    fn record(&self, event: EgressEvent) {
        self.0.lock().expect("no poisoned test sink").push(event);
    }
}

#[tokio::test]
async fn every_connection_reaches_the_sink_success_and_failure_alike() {
    let server = TestServer::builder().start().await;
    let sink = Arc::new(Recorded::default());
    let connector = RustlsConnector::new()
        .expect("a connector")
        .with_egress(sink.clone());

    let settings = server.settings();
    let connected = connector
        .connect_tcp(&settings.host, settings.port)
        .await;
    assert!(connected.is_ok(), "the loopback server accepts");

    // A port nothing listens on: the failure the log must also carry.
    let refused = connector.connect_tcp("127.0.0.1", 1).await;
    assert!(refused.is_err(), "nothing listens on port 1");

    let events = sink.0.lock().expect("no poisoned test sink").clone();
    assert_eq!(events.len(), 2, "one row per attempt, success or not");
    assert_eq!(events[0].subsystem, EgressSubsystem::Imap);
    assert_eq!(events[0].host, settings.host);
    assert_eq!(events[0].port, settings.port);
    assert_eq!(events[0].outcome, EgressOutcome::Connected);
    assert_eq!(events[1].port, 1);
    assert_eq!(events[1].outcome, EgressOutcome::Failed);
    assert_eq!(
        events[1].account, None,
        "the transport does not know the account; the wiring's per-account \
         sink adds it"
    );
}
