use std::sync::{Arc, Mutex};

use postio_model::egress::{EgressEvent, EgressOutcome, EgressSink, EgressSubsystem};
use postio_smtp::transport::{RustlsConnector, SmtpConnector};

#[derive(Default)]
struct Recorded(Mutex<Vec<EgressEvent>>);

impl EgressSink for Recorded {
    fn record(&self, event: EgressEvent) {
        self.0.lock().expect("no poisoned test sink").push(event);
    }
}

#[tokio::test]
async fn every_connection_reaches_the_sink_success_and_failure_alike() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback listener");
    let port = listener.local_addr().expect("an address").port();
    tokio::spawn(async move {
        let _held = listener.accept().await;
        std::future::pending::<()>().await;
    });

    let sink = Arc::new(Recorded::default());
    let connector = RustlsConnector::new()
        .expect("a connector")
        .with_egress(sink.clone(), postio_model::egress::EgressSubsystem::Smtp);

    let connected = connector.connect_tcp("127.0.0.1", port).await;
    assert!(connected.is_ok(), "the loopback listener accepts");
    let refused = connector.connect_tcp("127.0.0.1", 1).await;
    assert!(refused.is_err(), "nothing listens on port 1");

    let events = sink.0.lock().expect("no poisoned test sink").clone();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].subsystem, EgressSubsystem::Smtp);
    assert_eq!(events[0].outcome, EgressOutcome::Connected);
    assert_eq!(events[0].port, port);
    assert_eq!(events[1].outcome, EgressOutcome::Failed);
}
