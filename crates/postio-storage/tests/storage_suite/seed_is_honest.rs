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
async fn assert_counts_are_real(
    database: &postio_storage::Store,
    mailboxes: &[postio_model::Mailbox],
) {
    let connection = database.connect().await.expect("a connection");
    for mailbox in mailboxes {
        let total: u32 = postio_storage::sql::one(
            &connection,
            "SELECT count(*) FROM messages WHERE mailbox_id = ?1 AND deleted_locally = 0",
            [mailbox.id.get()],
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("counting the rows");
        let unread: u32 = postio_storage::sql::one(
            &connection,
            "SELECT count(*) FROM messages
                  WHERE mailbox_id = ?1 AND deleted_locally = 0 AND seen = 0",
            [mailbox.id.get()],
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
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

#[tokio::test]
async fn a_small_seed_leaves_counts_the_triggers_maintained() {
    let database = test_support::memory().await;
    let report = seed_small(&database, 11).await;

    assert!(report.message_count > 0, "seeded nothing to count");
    assert_counts_are_real(&database, &report.mailboxes).await;
}

#[tokio::test]
async fn a_large_seed_leaves_counts_the_triggers_maintained() {
    // Batched inserts, which is the shape a real sync writes in and the one
    // where a trigger that fires per statement rather than per row would be
    // caught.
    let database = test_support::memory().await;
    let report = seed_large(&database, 7, 2_000).await;

    assert!(report.message_count >= 2_000);
    assert_counts_are_real(&database, &report.mailboxes).await;
}

#[tokio::test]
async fn a_seeded_inbox_is_not_empty() {
    // The property the application actually depends on, stated plainly: a
    // seeded store has mail *and says so*. the app's wiring tests
    // assert the window lists it; this asserts the store it lists from is
    // not lying about being empty.
    let database = test_support::memory().await;
    let report = seed_small(&database, 11).await;

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

#[tokio::test]
async fn the_seed_is_the_corpus_the_storyboards_were_pinned_on() {
    // Every storyboard that counts rows or names a message id counts this
    // seed, and the seed is a stream of random draws walked over the corpus
    // in file order: one more fixture re-deals every mailbox after it, and the
    // nightly's catalogue goes red on a number nobody changed (#1869 added
    // `encoded-word-control-chars-in-header` and put one more message in the
    // demo inbox). A fixture that is a hostile-header test rather than mail
    // for a screenshot is left out of the seed (`UNSEEDED` in `seed.rs`); one that
    // belongs in it is a deliberate change, made together with the
    // storyboards' counts.
    let database = test_support::memory().await;
    let report = seed_small(&database, 1).await;

    assert_eq!(
        report.message_count, 79,
        "the small seed filed {} messages; the storyboards are pinned on 79. \
         If you added a corpus fixture, leave it out of the seed with \
         `UNSEEDED` in `seed.rs` unless it should be in the demo, and then re-pin \
         every `rows.count` and message id in `storyboards/`.",
        report.message_count
    );

    // The second account of the "two-accounts" seed walks the corpus too.
    let second =
        postio_storage::seed::seed_extra_account(&database, "Home", "home@example.net", 12).await;
    assert_eq!(
        second.message_count, 79,
        "the extra account filed {} messages; the storyboards are pinned on 79",
        second.message_count
    );
}
