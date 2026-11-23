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

fn unreachable_imap(reason: &str) -> ImapScripted {
    ImapScripted::new(ImapScript::new("* OK ready")).failing_tls(reason)
}

fn unreachable_smtp(reason: &str) -> SmtpScripted {
    SmtpScripted::new(SmtpScript::new("220 ready")).failing_tls(reason)
}

#[tokio::test]
async fn settings_that_work_are_reported_as_reached() {
    let account = account();
    let secrets = store_with_password(&account).await;

    let found = test_connection(&account, &secrets, &working_imap(), &working_smtp()).await;

    assert_eq!(found.incoming, Reachability::Reached, "{found:?}");
    assert_eq!(found.outgoing, Reachability::Reached, "{found:?}");
}

#[tokio::test]
async fn one_server_working_and_the_other_not_is_two_answers() {
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
        Reachability::Refused { reason, .. } => reason.clone(),
    }
}

#[tokio::test]
async fn each_server_is_reported_in_its_own_words() {
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
    let account = account();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::locked());
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

#[tokio::test]
async fn a_password_the_server_refuses_says_the_credential_was_refused() {
    let account = account();
    let secrets = store_with_password(&account).await;
    let refusing = ImapScripted::new(
        ImapScript::new("* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN] ready")
            .on(
                "AUTHENTICATE",
                "{tag} NO [AUTHENTICATIONFAILED] invalid credentials",
            )
            .on(
                "LOGIN",
                "{tag} NO [AUTHENTICATIONFAILED] invalid credentials",
            ),
    );

    let found = test_connection(&account, &secrets, &refusing, &working_smtp()).await;

    assert_eq!(
        found.trouble(),
        Some(postio_ui::account::Trouble::Rejected),
        "{found:?}"
    );
}

#[tokio::test]
async fn nothing_in_the_keyring_says_the_credential_is_missing() {
    let account = account();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecretStore::new());

    let found = test_connection(&account, &secrets, &working_imap(), &working_smtp()).await;

    assert_eq!(
        found.trouble(),
        Some(postio_ui::account::Trouble::Missing),
        "{found:?}"
    );
}

#[tokio::test]
async fn a_server_that_is_not_there_is_no_credential_trouble() {
    let account = account();
    let secrets = store_with_password(&account).await;

    let found = test_connection(
        &account,
        &secrets,
        &unreachable_imap("connection refused"),
        &unreachable_smtp("connection refused"),
    )
    .await;

    assert_eq!(found.trouble(), None, "{found:?}");
}
