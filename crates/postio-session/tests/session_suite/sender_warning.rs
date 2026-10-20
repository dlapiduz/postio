//! The sender-authenticity line a reading carries (#1928): the account's
//! provider decides which `Authentication-Results` is believed.

use chrono::Utc;
use postio_model::{Account, EmailAddress, Message};
use postio_session::reading::sender_warning;
use postio_storage::repository::AccountRepository;
use postio_storage::test_support;

const FAILED_AT_GOOGLE: &str = "Authentication-Results: mx.google.com;\r\n \
    dkim=fail header.d=example.org;\r\n \
    spf=fail smtp.mailfrom=ada@example.org;\r\n \
    dmarc=fail header.from=example.org\r\n\
    From: Ada <ada@example.org>\r\n";

async fn account_at(connection: &postio_storage::Checkout, host: &str) -> Account {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(None::<String>, "grace@example.net"),
    );
    account.incoming.host = host.to_owned();
    AccountRepository::new(connection)
        .create(&mut account)
        .await
        .expect("an account");
    account
}

fn from(name: Option<&str>, address: &str, account: &Account) -> Message {
    let mut message = Message::new(account.id, postio_model::MailboxId::new(1), Utc::now());
    message.from = vec![EmailAddress::new(name, address)];
    message
}

#[tokio::test]
async fn a_failure_the_accounts_provider_stamped_warns() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let account = account_at(&connection, "imap.gmail.com").await;
    let row = from(Some("Ada"), "ada@example.org", &account);
    let said = sender_warning(&connection, &row, Some(FAILED_AT_GOOGLE)).await;
    assert!(said.is_some_and(|said| said.contains("example.org")));
}

#[tokio::test]
async fn the_same_header_on_an_account_elsewhere_is_nobodys_word() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let account = account_at(&connection, "imap.example.org").await;
    let row = from(Some("Ada"), "ada@example.org", &account);
    assert_eq!(
        sender_warning(&connection, &row, Some(FAILED_AT_GOOGLE)).await,
        None
    );
}

#[tokio::test]
async fn a_name_that_is_another_address_warns_with_no_header_at_all() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let account = account_at(&connection, "imap.example.org").await;
    let row = from(Some("billing@bank.example"), "x@other.example", &account);
    assert!(sender_warning(&connection, &row, None).await.is_some());
}

#[tokio::test]
async fn mail_the_user_sent_is_never_judged() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let account = account_at(&connection, "imap.gmail.com").await;
    let row = from(Some("billing@bank.example"), "grace@example.net", &account);
    assert_eq!(sender_warning(&connection, &row, None).await, None);
}
