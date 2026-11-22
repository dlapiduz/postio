//! A fixture must not supply what the application produces: `seed` once called `recount_account`,
//! so
//! seeded stores listed while a real account's counts were never maintained (`postio-bl2`). Counts
//! now
//! come from migration 0003's triggers, so a seeded store lists only if the production path works.

use postio_model::MailboxRole;
use postio_storage::seed::{seed_large, seed_small};
use postio_storage::test_support;

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
    let database = test_support::memory().await;
    let report = seed_large(&database, 7, 2_000).await;

    assert!(report.message_count >= 2_000);
    assert_counts_are_real(&database, &report.mailboxes).await;
}

#[tokio::test]
async fn a_seeded_inbox_is_not_empty() {
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
    // The storyboards count rows and name message ids on this seed, a stream of random draws over
    // the
    // corpus in file order: one more fixture re-deals every mailbox after it (#1869).
    // Hostile-header
    // fixtures go in `UNSEEDED` in `seed.rs`.
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

    let second =
        postio_storage::seed::seed_extra_account(&database, "Home", "home@example.net", 12).await;
    assert_eq!(
        second.message_count, 79,
        "the extra account filed {} messages; the storyboards are pinned on 79",
        second.message_count
    );
}
