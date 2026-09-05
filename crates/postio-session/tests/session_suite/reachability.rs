//! "Test connection" answers for each server separately, and says what the
//! server said (#980).
//!
//! The button on an account's detail view asks whether the settings that
//! account **already has** work. That is a different question from
//! onboarding's, and its whole value is in the answer being specific: which
//! of the two servers refused, and in its own words, so the person reading it
//! knows which field to edit.
//!
//! Nothing here reaches the network. The connectors are arguments to
//! `test_connection` for exactly that reason — the same shape
//! `postio_app::onboarding::probe` took after #282, where the only way to
//! reach the code was to dial out and so none of it was covered.

use std::sync::Arc;

use postio_account::imap::{ImapScript, ScriptedConnector as ImapScripted};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_model::account::{AuthMethod, ServerConfig, TransportSecurity};
use postio_model::{Account, EmailAddress};
use postio_session::reachability::{Reachability, test_connection};
use postio_smtp::transport::{ScriptedConnector as SmtpScripted, SmtpScript};

fn account() -> Account {
    let mut account = Account::new("Test", EmailAddress::new(Some("Ada"), "ada@example.com"));
    account.auth = AuthMethod::Password;
    account.incoming = ServerConfig {
        host: "imap.example.com".to_owned(),
        port: 993,
        security: TransportSecurity::Tls,
        username: "ada@example.com".to_owned(),
    };
    account.outgoing = ServerConfig {
        host: "smtp.example.com".to_owned(),
        port: 465,
        security: TransportSecurity::Tls,
        username: "ada@example.com".to_owned(),
    };
    account
}

async fn store_with_password(account: &Account) -> Arc<dyn SecretStore> {
    let store = MemorySecretStore::new();
    store
        .store(
            &AccountKey::new(account.address.address.clone()),
            &Password::new("hunter2"),
        )
        .await
        .expect("an unlocked store accepts a password");
    Arc::new(store)
}

/// A server that completes the handshake, for each protocol. Scripted rather
/// than real: these connectors are public precisely so crates above can test
/// their connection handling without a socket, and no default-suite test here
/// may touch the network.
fn working_imap() -> ImapScripted {
    ImapScripted::new(
        ImapScript::new("* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN] ready")
            .on("AUTHENTICATE", "{tag} OK authenticated")
            .on("LOGIN", "{tag} OK authenticated")
            .on("CAPABILITY", "* CAPABILITY IMAP4rev1\n{tag} OK done"),
    )
}

fn working_smtp() -> SmtpScripted {
    SmtpScripted::new(
        SmtpScript::new("220 mail.example.com ESMTP ready")
            .on("EHLO", "250-mail.example.com\r\n250 AUTH PLAIN")
            .on("AUTH PLAIN", "235 2.7.0 authentication successful"),
    )
}

/// A server that cannot be reached at all, saying so in its own words.
fn unreachable_imap(reason: &str) -> ImapScripted {
    ImapScripted::new(ImapScript::new("* OK ready")).failing_tls(reason)
}

fn unreachable_smtp(reason: &str) -> SmtpScripted {
    SmtpScripted::new(SmtpScript::new("220 ready")).failing_tls(reason)
}

#[tokio::test]
async fn settings_that_work_are_reported_as_reached() {
    // The end state, and what the button exists to confirm.
    let account = account();
    let secrets = store_with_password(&account).await;

    let found = test_connection(&account, &secrets, &working_imap(), &working_smtp()).await;

    assert_eq!(found.incoming, Reachability::Reached, "{found:?}");
    assert_eq!(found.outgoing, Reachability::Reached, "{found:?}");
}

#[tokio::test]
async fn one_server_working_and_the_other_not_is_two_answers() {
    // What a typo actually produces, and the reason this is not a single
    // bool: the half that works is the half to stop editing, and only a
    // per-protocol answer says which.
    let account = account();
    let secrets = store_with_password(&account).await;

    let found = test_connection(
        &account,
        &secrets,
        &working_imap(),
        &unreachable_smtp("smtp.example.com is not listening on 465"),
    )
    .await;

    assert_eq!(found.incoming, Reachability::Reached, "{found:?}");
    assert!(
        reason(&found.outgoing).contains("not listening on 465"),
        "the outgoing answer has to carry the outgoing server's words: {found:?}"
    );
}

fn reason(outcome: &Reachability) -> String {
    match outcome {
        Reachability::Reached => panic!("expected a refusal"),
        Reachability::Refused { reason } => reason.clone(),
    }
}

#[tokio::test]
async fn each_server_is_reported_in_its_own_words() {
    // The half that makes the button worth pressing. Both servers are tried
    // whatever the first one does, and each answer carries the message that
    // belongs to it -- "it does not work" sends somebody to two screens of
    // settings with nothing to go on.
    let account = account();
    let secrets = store_with_password(&account).await;
    let incoming = unreachable_imap("imap.example.com refused the connection");
    let outgoing = unreachable_smtp("smtp.example.com is not listening on 465");

    let found = test_connection(&account, &secrets, &incoming, &outgoing).await;

    assert!(
        reason(&found.incoming).contains("imap.example.com refused"),
        "the incoming answer has to be about the incoming server: {:?}",
        found.incoming
    );
    assert!(
        reason(&found.outgoing).contains("not listening on 465"),
        "and the outgoing one about the outgoing server: {:?}",
        found.outgoing
    );
}

#[tokio::test]
async fn a_credential_that_cannot_be_read_is_reported_without_the_address() {
    // A locked keyring is the ordinary first-run-after-reboot failure, and it
    // is not a server problem -- so the reason has to say so rather than
    // blaming the host. `SecretError` names the account, and this string goes
    // to a log as well as to the screen, so the local part is redacted on the
    // way (CLAUDE.md: logs never carry message content, and an address is
    // the user's).
    let account = account();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::locked());
    // Working servers on purpose: the refusal has to be about the keyring,
    // and a broken server would let this pass for the wrong reason.
    let found = test_connection(&account, &secrets, &working_imap(), &working_smtp()).await;

    let incoming = reason(&found.incoming);
    assert!(
        !incoming.contains("ada@"),
        "the local part reached a string that is also logged: {incoming}"
    );
    assert!(
        !incoming.is_empty(),
        "a refusal with no reason is the silent spinner #980 is about"
    );
}

#[tokio::test]
async fn the_two_answers_are_independent() {
    // The case that decides the shape of the type: one server working and the
    // other not is the common outcome of a typo, and it is the outcome the
    // user most needs told apart. A single bool could not say it.
    let account = account();
    let secrets = store_with_password(&account).await;

    let found = test_connection(
        &account,
        &secrets,
        &unreachable_imap("incoming is down"),
        &unreachable_smtp("outgoing is down"),
    )
    .await;

    assert_ne!(
        reason(&found.incoming),
        reason(&found.outgoing),
        "both answers came out the same, so one of them is not being asked"
    );
}
