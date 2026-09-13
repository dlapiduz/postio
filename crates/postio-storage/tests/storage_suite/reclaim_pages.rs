//! Handing freed pages back to the filesystem (#381, and what became of it).
//!
//! Deleting a message frees pages *inside* the file and returns nothing to the
//! filesystem. #381 answered that with `auto_vacuum = INCREMENTAL`, stepped a
//! little at a time on the housekeeping worker, and this engine has neither
//! the pragma nor the step — see
//! `docs/notes/2026-09-13-what-the-engine-swap-could-not-keep.md`.
//!
//! What it does have is a full `VACUUM`, and these are the two halves of using
//! it responsibly: it reclaims, and it is only attempted when the reclaim is
//! worth what it costs.

use postio_storage::test_support;

fn bytes(store: &test_support::TempStore) -> u64 {
    std::fs::metadata(store.directory().join("postio.db"))
        .map(|meta| meta.len())
        .unwrap_or(0)
}

/// Delete nine messages in ten, which is what a `UIDVALIDITY` reset or a
/// cleared archive does to a store.
async fn a_store_full_of_holes(messages: usize) -> test_support::TempStore {
    let store = test_support::temp().await;
    postio_storage::seed::seed_large(&store, 7, messages).await;
    let connection = store.connect().await.expect("checkout");
    connection
        .execute("DELETE FROM messages WHERE id % 10 != 0", ())
        .await
        .expect("delete");
    drop(connection);
    store.truncate_log().await.expect("checkpoint");
    store
}

#[tokio::test(flavor = "multi_thread")]
async fn reclaiming_returns_the_holes_to_the_filesystem_and_keeps_the_mail() {
    let store = a_store_full_of_holes(20_000).await;
    let before = bytes(&store);

    let reclaimed = store.reclaim_free_pages().await.expect("reclaim");
    let after = bytes(&store);

    assert!(
        reclaimed > 0,
        "the reclaim reported nothing; the store went from {before} to {after} bytes"
    );
    assert!(
        after < before / 2,
        "a store with nine tenths of its rows deleted came back at {after} bytes \
         from {before} -- the pages are still in the file"
    );

    // And the tenth that was left is still there and still readable.
    let connection = store.connect().await.expect("checkout");
    let survived = postio_storage::sql::scalar(&connection, "SELECT count(*) FROM messages", ())
        .await
        .expect("count");
    let subjects = postio_storage::sql::scalar(
        &connection,
        "SELECT count(*) FROM messages WHERE subject IS NOT NULL",
        (),
    )
    .await
    .expect("count");
    assert_eq!(survived, 2_000, "the reclaim lost mail");
    assert_eq!(subjects, survived, "the reclaim lost what the rows held");
}

/// The policy itself is `store::reclaim_policy`, six cases at the magnitudes
/// that matter and no store at all. What needs one is the wiring: that the
/// pragmas it reads are the pragmas this engine answers, and that a reclaim
/// actually moves the numbers the policy is reading.
#[tokio::test(flavor = "multi_thread")]
async fn the_free_space_a_store_reports_is_the_space_a_reclaim_gives_back() {
    let store = a_store_full_of_holes(20_000).await;

    let free = store.free_bytes().await.expect("ask for the holes");
    assert!(
        free > 0,
        "a store with nine tenths of its rows deleted reported no free pages, \
         so `is_worth_reclaiming` is reading a pragma this engine does not \
         answer and will say no for ever"
    );

    let reclaimed = store.reclaim_free_pages().await.expect("reclaim");
    assert!(
        reclaimed >= free / 2,
        "the store held {free} bytes of holes and the reclaim gave back \
         {reclaimed}; the two numbers are meant to be about the same thing"
    );

    // And it stops asking. A worker that reclaimed every pass would rewrite
    // the database on every start.
    assert_eq!(
        store.free_bytes().await.expect("ask again"),
        0,
        "a store still reports holes immediately after being reclaimed"
    );
}
