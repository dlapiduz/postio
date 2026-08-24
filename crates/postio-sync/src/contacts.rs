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
use postio_storage::repository::ContactRepository;
use rusqlite::Connection;

use crate::drain::Result;

/// Records a sighting of every correspondent on `message`, except addresses
/// belonging to `account` itself.
///
/// A correspondent list where the account's own address is the top hit is
/// noise: it turns up whenever the account is cc'd on its own thread, and as
/// the sender of everything filed in Sent. `record_message` has no way to
/// know which address is "ours" — it only sees one message at a time — so
/// the exclusion happens here, against every address `account` can send as.
pub(crate) fn record(connection: &Connection, account: &Account, message: &Message) -> Result<()> {
    let is_own = |address: &EmailAddress| {
        let normalized = address.normalized();
        account.address.normalized() == normalized
            || account
                .identities
                .iter()
                .any(|identity| identity.address.normalized() == normalized)
    };

    let mut trimmed = message.clone();
    trimmed.from.retain(|address| !is_own(address));
    if trimmed.sender.as_ref().is_some_and(&is_own) {
        trimmed.sender = None;
    }
    trimmed.reply_to.retain(|address| !is_own(address));
    trimmed.to.retain(|address| !is_own(address));
    trimmed.cc.retain(|address| !is_own(address));
    trimmed.bcc.retain(|address| !is_own(address));

    ContactRepository::new(connection).record_message(&trimmed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::Identity;
    use postio_storage::repository::MessageRepository;
    use postio_storage::test_support;

    fn message(
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
            .expect("create message");
        message
    }

    #[test]
    fn every_real_correspondent_is_recorded() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        let (account, mailbox) = test_support::account_with_inbox(&connection);
        let message = message(&connection, &account, mailbox);

        record(&connection, &account, &message).expect("record");

        let contacts = ContactRepository::new(&connection)
            .list(Some(account.id))
            .expect("list");
        let addresses: Vec<String> = contacts.iter().map(|c| c.address.normalized()).collect();
        assert!(
            addresses.contains(&"ada@example.com".to_string()),
            "{addresses:?}"
        );
        assert!(
            addresses.contains(&"bob@example.com".to_string()),
            "{addresses:?}"
        );
        assert!(
            addresses.contains(&"carol@example.com".to_string()),
            "{addresses:?}"
        );
    }

    #[test]
    fn the_accounts_own_address_is_never_recorded_as_a_correspondent() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        let (account, mailbox) = test_support::account_with_inbox(&connection);
        let message = message(&connection, &account, mailbox);

        record(&connection, &account, &message).expect("record");

        let contacts = ContactRepository::new(&connection)
            .list(Some(account.id))
            .expect("list");
        assert!(
            contacts
                .iter()
                .all(|contact| contact.address.normalized() != account.address.normalized()),
            "the account's own address must not show up in its own correspondent list: {contacts:?}"
        );
    }

    #[test]
    fn a_send_from_identity_is_also_excluded_as_a_correspondent() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        let (mut account, mailbox) = test_support::account_with_inbox(&connection);
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
            .expect("create message");

        record(&connection, &account, &message).expect("record");

        let contacts = ContactRepository::new(&connection)
            .list(Some(account.id))
            .expect("list");
        assert_eq!(
            contacts.len(),
            1,
            "only bob, not the identity address: {contacts:?}"
        );
        assert_eq!(contacts[0].address.normalized(), "bob@example.com");
    }
}
