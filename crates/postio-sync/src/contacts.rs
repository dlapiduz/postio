//! Recording contacts as messages sync in.
//!
//! `ContactRepository::record_message` existed, was tested, and had no
//! caller: nothing wrote the `contacts` table, so @ in the finder and the
//! composer's recipient completion always listed nobody, however much mail
//! the account held (`postio-66j`, one of the eight `postio-bl2` instances —
//! a capability fully implemented one layer down and never called from the
//! one above).
//!
//! [`record`] is what [`crate::initial::enumerate`] and
//! `crate::resync::incremental` call for every message a sync pass
//! genuinely inserts. "Genuinely inserts" matters: a `Coverage::Everything`
//! re-enumeration re-fetches messages that are already known (its whole
//! point is to refresh what an untrustworthy incremental pull might have
//! missed), and recording a sighting for those again on every such pass
//! would inflate `times_seen` without a new message ever having arrived.
//! Both call sites already compute the set of UIDs known before the pass
//! started, to decide what to fetch — this reuses that same set to decide
//! what to record, so a message is counted exactly once, on the pass that
//! first wrote it.

use postio_model::{Account, EmailAddress, Message};
use postio_storage::Connection;
use postio_storage::repository::ContactRepository;

use crate::drain::Result;

/// Records a sighting of every correspondent on `message`, except addresses
/// belonging to `account` itself.
///
/// A correspondent list where the account's own address is the top hit is
/// noise: it turns up whenever the account is cc'd on its own thread, and as
/// the sender of everything filed in Sent. The same addresses are what make a
/// message the user's own, and so what marks its recipients as written to
/// (specs/005-contacts R3) — which is why they are handed down rather than
/// stripped here: stripping the sender first would leave nothing to tell a
/// sent message from a received one.
pub(crate) async fn record(
    connection: &Connection,
    account: &Account,
    message: &Message,
) -> Result<()> {
    ContactRepository::new(connection)
        .record_message(message, &own_addresses(account))
        .await?;
    Ok(())
}

/// Every address `account` sends as: its own and each identity's.
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

    /// Every address the store now attributes to a person, normalised.
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
        // specs/005-contacts R3: "written to" is decided by the From header,
        // whichever of the account's addresses sent it -- a message the user
        // sent as an identity is as much theirs as one sent as the account.
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
}
