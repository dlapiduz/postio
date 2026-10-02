//! The first-run wizard, across the boundary (canvas 09).
//!
//! An address goes in and the servers come back -- found, or not, and said
//! so -- and `Connect` signs in before anything is written. The looking-up
//! and the proof are the shared onboarding's (`postio_session::onboarding`),
//! the same steps the desktop's first run takes; what is tested here is that
//! the Mac reaches them, and that a refused login leaves nothing behind.
//!
//! Nothing here dials: discovery goes through a network that answers
//! nothing, and the proof signs in to a `MockBackend`.

use std::sync::Arc;

use postio_account::backend::{Fault, MockBackend};
use postio_account::discovery::{
    AutoconfigEndpoint, CancelToken, DiscoveryAutoconfig, DiscoverySrvReport, DiscoveryTransport,
    TransportError,
};
use postio_ffi::{NewAccountFfi, SecurityFfi, ServerFfi, Session, SessionOptions};
use postio_storage::repository::AccountRepository;
use postio_storage::test_support;

/// A network where nothing answers. What is found is the preset table's,
/// which is data shipped with Postio and reaches no server.
struct Silent;

#[async_trait::async_trait]
impl DiscoveryTransport for Silent {
    async fn autoconfig(
        &self,
        _endpoint: AutoconfigEndpoint<'_>,
        _cancel: &CancelToken,
    ) -> Result<DiscoveryAutoconfig, TransportError> {
        Err(TransportError::new("nothing answers in this test"))
    }

    async fn srv(
        &self,
        _domain: &str,
        _cancel: &CancelToken,
    ) -> Result<DiscoverySrvReport, TransportError> {
        Err(TransportError::new("nothing answers in this test"))
    }

    async fn mx(
        &self,
        _domain: &str,
        _cancel: &CancelToken,
    ) -> Result<Vec<String>, TransportError> {
        Err(TransportError::new("nothing answers in this test"))
    }
}

/// A session whose onboarding reaches no network, signing in to `server`.
async fn a_session(server: MockBackend) -> (Arc<Session>, postio_storage::Store) {
    let database = test_support::memory().await;
    let secrets = Arc::new(postio_account::secret::MemorySecretStore::default());
    let session = Session::open(
        SessionOptions::in_memory_with(database.clone())
            .with_secrets(secrets)
            .with_discovery_for_test(Arc::new(Silent))
            .with_mail_for_test(postio_session::MailOverride {
                backend: Arc::new(server),
                smtp: Arc::new(postio_smtp::transport::ScriptedConnector::new(
                    postio_smtp::transport::SmtpScript::new("220 ready"),
                )),
            }),
    )
    .expect("a session over the store");
    (session, database)
}

/// An address at whichever provider the preset table ships first -- read out
/// of the table, because a test naming a provider is the mistake the code is
/// forbidden to make.
fn an_address_at_a_known_provider() -> (String, String) {
    let preset = postio_account::discovery::presets()
        .first()
        .expect("the preset table ships at least one provider");
    let domain = preset
        .domains()
        .first()
        .expect("a preset claims at least one domain")
        .clone();
    (format!("someone@{domain}"), domain)
}

async fn accounts(database: &postio_storage::Store) -> Vec<postio_model::Account> {
    let connection = database.connect().await.expect("a connection");
    AccountRepository::new(&connection)
        .list()
        .await
        .expect("a list")
}

fn server(host: &str, port: u16) -> ServerFfi {
    ServerFfi {
        host: host.to_owned(),
        port,
        security: SecurityFfi::Tls,
    }
}

fn ada(password: &str) -> NewAccountFfi {
    NewAccountFfi {
        address: "ada@ostwald.invalid".to_owned(),
        name: "Ada Ostwald".to_owned(),
        password: password.to_owned(),
        login: "ada@ostwald.invalid".to_owned(),
        imap: server("imap.ostwald.invalid", 993),
        smtp: server("smtp.ostwald.invalid", 465),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_known_provider_is_found_and_the_card_says_where_it_connects() {
    let (session, _) = a_session(MockBackend::new()).await;
    let (address, domain) = an_address_at_a_known_provider();

    let found = session.discover_account(address).await;

    assert!(found.found, "the preset table is an authoritative answer");
    assert_eq!(found.heading, format!("Found settings for {domain}"));
    assert!(
        !found.imap.host.is_empty(),
        "and it knows where to read mail"
    );
    assert!(
        found
            .imap_line
            .contains(&format!("{}:{}", found.imap.host, found.imap.port)),
        "the card's line is the server as the canvas writes it: {}",
        found.imap_line
    );
    assert!(!found.smtp_line.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_address_nothing_answers_for_is_a_guess_to_check_never_a_finding() {
    // A custom domain publishing nothing is the ordinary case. The desktop
    // prefills the common names rather than five empty boxes (postio-69),
    // and may, because of what it presents them *as*: a guess, in an open
    // form, never the "Found settings" card.
    let (session, _) = a_session(MockBackend::new()).await;

    let found = session
        .discover_account("ada@ostwald.invalid".to_owned())
        .await;

    assert!(!found.found, "a guess is not a finding");
    assert!(
        found.heading.contains("ostwald.invalid") && !found.heading.starts_with("Found"),
        "the card names the domain and does not claim to have found it: {}",
        found.heading
    );
    assert_eq!(
        found.login, "ada@ostwald.invalid",
        "the login is the address"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_signs_in_and_only_then_writes_the_account() {
    let (session, database) = a_session(MockBackend::new()).await;

    let complaint = session.connect_account(ada("hunter2")).await;

    assert_eq!(complaint, None);
    let written = accounts(&database).await;
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].address.address, "ada@ostwald.invalid");
    assert_eq!(written[0].incoming.host, "imap.ostwald.invalid");
    assert_eq!(written[0].display_name, "Ada Ostwald", "the name is kept");
    assert!(
        !format!("{:?}", written[0]).contains("hunter2"),
        "the password is in the keyring, never the row"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_login_says_so_and_leaves_no_account_behind() {
    // The whole reason Connect proves first: an account written before the
    // server said no is one that fails at every sync from then on.
    let refusing = MockBackend::new();
    refusing.fail_all(Fault::AuthFailed);
    let (session, database) = a_session(refusing).await;

    let complaint = session
        .connect_account(ada("wrong"))
        .await
        .expect("a refusal");

    assert!(!complaint.is_empty());
    assert!(accounts(&database).await.is_empty(), "nothing was written");
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_with_no_server_is_refused_without_dialling() {
    let (session, database) = a_session(MockBackend::new()).await;
    let mut account = ada("hunter2");
    account.imap.host = "  ".to_owned();

    let complaint = session.connect_account(account).await.expect("a refusal");

    assert!(
        complaint.contains("server names — it will not guess"),
        "said in one sentence, with no run of spaces in it: {complaint}"
    );
    assert!(!complaint.contains("  "), "{complaint:?}");
    assert!(accounts(&database).await.is_empty());
}

#[test]
fn the_sync_window_choices_are_the_shared_three_with_their_estimates() {
    let choices = postio_ffi::sync_window_choices();
    let labels: Vec<_> = choices.iter().map(|choice| choice.label.as_str()).collect();
    assert_eq!(labels, ["Last 30 days", "Last year", "Everything"]);
    assert!(choices.iter().all(|choice| !choice.estimate.is_empty()));
    assert_eq!(
        choices.iter().filter(|choice| choice.recommended).count(),
        1,
        "one is the default, and it is the field's own"
    );
}
