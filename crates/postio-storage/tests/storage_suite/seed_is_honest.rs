//! The fixture must not supply what the application is supposed to produce.
//!
//! `postio-bl2`: eight capabilities were found fully implemented, tested and
//! never called, and the reason none of the tests caught any of it is that
//! every fixture below the composition root answered — by hand — the question
//! the layer above was supposed to answer.
//!
//! [`seed`](postio_storage::seed) was one of the two. It called
//! `MailboxRepository::recount_account` after inserting, so a seeded store
//! came out with correct cached counts. A real one did not, because nothing
//! maintained them: the message list drew rows from every fixture in the
//! project and nothing at all from a live account with 81,716 messages in it,
//! and no test could tell the difference. One line in a helper, and it made a
//! green suite compatible with an application that listed nothing.
//!
//! The counts now come from migration 0003's triggers — the same mechanism a
//! real sync goes through — so a seeded store is listable only if the
//! production path works. This file is what keeps it that way: if `seed` ever
//! grows a shortcut again, or the triggers stop maintaining what they claim
//! to, one of these fails.

use postio_model::MailboxRole;
use postio_storage::seed::{seed_large, seed_small};
use postio_storage::test_support;

/// Every folder's cached counts, against the rows actually in it.
fn assert_counts_are_real(
    database: &postio_storage::Database,
    mailboxes: &[postio_model::Mailbox],
) {
    let connection = database.connection().expect("a connection");
    for mailbox in mailboxes {
        let total: u32 = connection
            .query_row(
                "SELECT count(*) FROM messages WHERE mailbox_id = ?1 AND deleted_locally = 0",
                [mailbox.id.get()],
                |row| row.get(0),
            )
            .expect("counting the rows");
        let unread: u32 = connection
            .query_row(
                "SELECT count(*) FROM messages
                  WHERE mailbox_id = ?1 AND deleted_locally = 0 AND seen = 0",
                [mailbox.id.get()],
                |row| row.get(0),
            )
            .expect("counting the unread rows");

        assert_eq!(
            mailbox.counts.total, total,
            "{}: cached total {} but {total} rows are in it. The seed no longer \
             recounts on purpose — if this is wrong, the triggers in migration \
             0003 are wrong, and a live account is drawing an empty list right \
             now. Do not repair it by recounting in the fixture.",
            mailbox.path, mailbox.counts.total
        );
        assert_eq!(
            mailbox.counts.unread, unread,
            "{}: cached unread {} but {unread} rows are unread",
            mailbox.path, mailbox.counts.unread
        );
    }
}

#[test]
fn a_small_seed_leaves_counts_the_triggers_maintained() {
    let database = test_support::memory();
    let report = seed_small(&database, 11);

    assert!(report.message_count > 0, "seeded nothing to count");
    assert_counts_are_real(&database, &report.mailboxes);
}

#[test]
fn a_large_seed_leaves_counts_the_triggers_maintained() {
    // Batched inserts, which is the shape a real sync writes in and the one
    // where a trigger that fires per statement rather than per row would be
    // caught.
    let database = test_support::memory();
    let report = seed_large(&database, 7, 2_000);

    assert!(report.message_count >= 2_000);
    assert_counts_are_real(&database, &report.mailboxes);
}

#[test]
fn a_seeded_inbox_is_not_empty() {
    // The property the application actually depends on, stated plainly: a
    // seeded store has mail *and says so*. `postio-app`'s `tests/wiring.rs`
    // asserts the window lists it; this asserts the store it lists from is
    // not lying about being empty.
    let database = test_support::memory();
    let report = seed_small(&database, 11);

    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed makes an inbox");

    assert!(
        inbox.counts.total > 0,
        "the inbox reports {} messages. A store that says it is empty is one \
         the list will draw as empty however much mail is in it — that was the \
         live bug.",
        inbox.counts.total
    );
}
