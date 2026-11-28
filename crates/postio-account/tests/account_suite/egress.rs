use std::sync::{Arc, Mutex};

use postio_account::imap::{ImapConnector, RustlsConnector};
use postio_account::test_server::TestServer;
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
        .with_egress(sink.clone(), postio_model::egress::EgressSubsystem::Imap);

    let settings = server.settings();
    let connected = connector.connect_tcp(&settings.host, settings.port).await;
    assert!(connected.is_ok(), "the loopback server accepts");

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
