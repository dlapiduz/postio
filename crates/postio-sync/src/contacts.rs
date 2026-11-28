//! Records contacts for each message a sync pass genuinely inserts, so @ and recipient completion
//! have people to list.
//! Called with the set of UIDs known before the pass, so a `Coverage::Everything` re-enumeration
//! does not inflate `times_seen`.

use postio_model::{Account, EmailAddress, Message};
use postio_storage::Connection;
use postio_storage::repository::ContactRepository;

use crate::drain::Result;

/// Records a sighting of every correspondent on `message` except the account's own addresses.
/// Own addresses are handed down, not stripped first: they are what marks a message as the user's
/// own (specs/005-contacts R3).
pub(crate) async fn record(
    connection: &Connection,
    account: &Account,
    message: &Message,
) -> Result<()> {
    let own = own_addresses(account);
    let contacts = ContactRepository::new(connection);
    contacts.record_message(message, &own).await?;
    // A reply from an address other than the one written to is evidence they are one person
    // (specs/005-contacts R11).
    contacts.record_reply(message, &own).await?;
    Ok(())
}

fn own_addresses(account: &Account) -> Vec<EmailAddress> {
    std::iter::once(account.address.clone())
        .chain(
            account
                .identities
                .iter()
                .map(|identity| identity.address.clone()),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::Identity;
    use postio_storage::repository::MessageRepository;
    use postio_storage::test_support;

    async fn message(
        connection: &Connection,
        account: &Account,
        mailbox: postio_model::MailboxId,
    ) -> Message {
        let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
        message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        message.to = vec![
            account.address.clone(),
            EmailAddress::new(Some("Bob"), "bob@example.com"),
        ];
        message.cc = vec![EmailAddress::new(Some("Carol"), "carol@example.com")];
        MessageRepository::new(connection)
            .create(&mut message)
            .await
            .expect("create message");
        message
    }

    async fn known(connection: &Connection) -> Vec<String> {
        ContactRepository::new(connection)
            .people(1_000)
            .await
            .expect("people")
            .iter()
            .flat_map(|person| &person.addresses)
            .map(|owned| owned.address.normalized())
            .collect()
    }

    #[tokio::test]
    async fn every_real_correspondent_is_recorded() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;
        let message = message(&connection, &account, mailbox).await;

        record(&connection, &account, &message)
            .await
            .expect("record");

        let addresses = known(&connection).await;
        for expected in ["ada@example.com", "bob@example.com", "carol@example.com"] {
            assert!(addresses.contains(&expected.to_string()), "{addresses:?}");
        }
    }

    #[tokio::test]
    async fn the_accounts_own_address_is_never_recorded_as_a_correspondent() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;
        let message = message(&connection, &account, mailbox).await;

        record(&connection, &account, &message)
            .await
            .expect("record");

        assert!(
            !known(&connection)
                .await
                .contains(&account.address.normalized()),
            "the account's own address must not show up among its correspondents"
        );
    }

    #[tokio::test]
    async fn a_send_from_identity_is_also_excluded_as_a_correspondent() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (mut account, mailbox) = test_support::account_with_inbox(&connection).await;
        account.identities.push(Identity::new(
            account.id,
            EmailAddress::new(Some("Ada at Work"), "ada.work@example.com"),
        ));

        let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
        message.from = vec![EmailAddress::new(Some("Bob"), "bob@example.com")];
        message.to = vec![EmailAddress::new(
            Some("Ada at Work"),
            "ada.work@example.com",
        )];
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create message");

        record(&connection, &account, &message)
            .await
            .expect("record");

        assert_eq!(
            known(&connection).await,
            ["bob@example.com"],
            "only bob, not the identity address"
        );
    }

    #[tokio::test]
    async fn mail_sent_from_an_identity_marks_its_recipients_written_to() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (mut account, mailbox) = test_support::account_with_inbox(&connection).await;
        account.identities.push(Identity::new(
            account.id,
            EmailAddress::new(Some("Ada at Work"), "ada.work@example.com"),
        ));

        let mut sent = Message::new(account.id, mailbox, chrono::Utc::now());
        sent.from = vec![EmailAddress::new(
            Some("Ada at Work"),
            "Ada.Work@example.com",
        )];
        sent.to = vec![EmailAddress::new(Some("Bob"), "bob@example.com")];
        MessageRepository::new(&connection)
            .create(&mut sent)
            .await
            .expect("create message");

        record(&connection, &account, &sent).await.expect("record");

        let bob = ContactRepository::new(&connection)
            .by_address("bob@example.com")
            .await
            .expect("lookup")
            .expect("bob");
        assert_eq!(bob.written, 1, "the user wrote to bob");
    }

    async fn exchange(
        to: &str,
        reply_from: &str,
        parent_from_user: bool,
    ) -> Vec<postio_model::JoinSuggestion> {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;
        let mut sent = Message::new(account.id, mailbox, chrono::Utc::now());
        sent.rfc_message_id = Some(postio_model::ids::RfcMessageId::new(
            "question@example.test",
        ));
        sent.from = vec![if parent_from_user {
            account.address.clone()
        } else {
            EmailAddress::new(Some("Grace"), "grace@example.org")
        }];
        sent.to = vec![EmailAddress::new(Some("Ada"), to)];
        MessageRepository::new(&connection)
            .create(&mut sent)
            .await
            .expect("sent");
        record(&connection, &account, &sent).await.expect("record");
        let mut reply = Message::new(account.id, mailbox, chrono::Utc::now());
        reply.rfc_message_id = Some(postio_model::ids::RfcMessageId::new("answer@example.test"));
        reply.in_reply_to = Some(postio_model::ids::RfcMessageId::new(
            "question@example.test",
        ));
        reply.from = vec![EmailAddress::new(Some("A. L."), reply_from)];
        reply.to = vec![account.address.clone()];
        MessageRepository::new(&connection)
            .create(&mut reply)
            .await
            .expect("reply");
        record(&connection, &account, &reply).await.expect("record");
        ContactRepository::new(&connection)
            .suggestions(10)
            .await
            .expect("suggestions")
    }

    #[tokio::test]
    async fn an_answer_from_another_address_is_evidence() {
        let found = exchange("ada@work.example", "ada@home.example", true).await;
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(
            found[0].reason,
            postio_model::SuggestionReason::Replied { replies: 1 }
        );
    }

    #[tokio::test]
    async fn an_answer_from_the_same_address_is_not() {
        assert!(
            exchange("ada@work.example", "ada@work.example", true)
                .await
                .is_empty()
        );
    }

    #[tokio::test]
    async fn an_answer_to_someone_elses_message_is_not() {
        assert!(
            exchange("ada@work.example", "ada@home.example", false)
                .await
                .is_empty()
        );
    }
}
