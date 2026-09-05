//! Reclaiming disk: the sweeps that had no production caller (#416).
//!
//! `collect_garbage` and `purge_temporary` were written, tested and
//! documented, and nothing in a running Postio ever called either. So a
//! deleted message's bytes stayed on disk for ever, and a mailbox whose
//! `UIDVALIDITY` reset orphaned every blob in it with no way to get the
//! space back.
//!
//! What is in the blob store is the raw `.eml` and the attachment payloads;
//! since ADR 0020 the body text is a column on the row and goes when the row
//! goes, without a sweep. So these drive the raw source, which is the blob
//! every synced message has.

use std::time::Duration;

use postio_storage::BlobStore;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// A store, a blob directory beside it, and one message holding one blob —
/// its raw RFC 5322 source.
fn store_with_a_message() -> (
    test_support::TempDatabase,
    BlobStore,
    postio_model::MessageId,
) {
    let database = test_support::temp();
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let blob = blobs
        .put(b"the raw source of a message somebody will delete")
        .expect("put");
    let mut message = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    message.server.uid = Some(postio_model::Uid::new(1));
    message.server.uid_validity = Some(postio_model::UidValidity::new(1));
    message.raw_blob_id = Some(blob);
    let messages = MessageRepository::new(&connection);
    let id = messages.create(&mut message).expect("create");
    drop(connection);
    (database, blobs, id)
}

/// Every regular file under the store's root, excluding its temp directory.
fn blob_files(store: &BlobStore) -> Vec<std::path::PathBuf> {
    fn walk(directory: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(store.root()).expect("read").flatten() {
        let path = entry.path();
        if path == store.temporary_directory() {
            continue;
        }
        if path.is_dir() {
            walk(&path, &mut out);
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn a_deleted_message_s_raw_source_is_reclaimed_by_a_later_sweep() {
    // The headline. `MessageRepository::delete` removes the row and never
    // touches blobs -- deliberately, because the schema delegates that to this
    // sweep. Nothing called the sweep, so deleting mail freed nothing, for
    // ever.
    let (database, blobs, id) = store_with_a_message();
    let connection = database.connection().expect("checkout");
    assert_eq!(blob_files(&blobs).len(), 1);

    MessageRepository::new(&connection)
        .delete(&[id])
        .expect("delete");
    assert_eq!(
        blob_files(&blobs).len(),
        1,
        "deleting the row leaves the bytes, which is the whole reason a sweep exists"
    );

    let report =
        postio_session::reclaim_orphaned_blobs(&database, &blobs, Duration::ZERO).expect("sweep");

    assert_eq!(report.removed, 1);
    assert!(report.bytes_reclaimed > 0);
    assert!(blob_files(&blobs).is_empty());
}

#[test]
fn a_blob_still_referenced_is_never_swept() {
    // The other half, and the one that matters more: a sweep that took a live
    // blob would lose mail that is not refetchable if the server no longer has
    // it. `referenced_blobs` is what stands between this and that.
    let (database, blobs, _id) = store_with_a_message();

    let report =
        postio_session::reclaim_orphaned_blobs(&database, &blobs, Duration::ZERO).expect("sweep");

    assert_eq!(report.removed, 0);
    assert_eq!(blob_files(&blobs).len(), 1);
}

#[test]
fn a_blob_younger_than_the_grace_period_is_left_alone() {
    // `min_age` is load-bearing rather than decoration. A blob written but not
    // yet committed to a row is indistinguishable from an orphan, so a sweep
    // with no grace period would delete the body of a message that was
    // mid-fetch. The default is an hour; production must not pass `ZERO`.
    let database = test_support::temp();
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    blobs
        .put(b"written a moment ago, not committed yet")
        .expect("put");

    let report =
        postio_session::reclaim_orphaned_blobs(&database, &blobs, Duration::from_secs(3600))
            .expect("sweep");

    assert_eq!(report.removed, 0, "too young to be called garbage");
    assert_eq!(blob_files(&blobs).len(), 1);
}

#[test]
fn debris_from_a_torn_off_fetch_is_purged() {
    // A cancelled fetch's writer removes its own temp file, so this is for the
    // case no destructor ran at all: a power cut or a kill -9 mid-fetch leaves
    // a `.part` file nothing will ever finish.
    let database = test_support::temp();
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    std::fs::write(
        blobs.temporary_directory().join("1234-0.part"),
        b"half a message",
    )
    .expect("stage some debris");

    let purged = postio_session::purge_fetch_debris(&blobs).expect("purge");

    assert_eq!(purged, 1);
    assert!(
        std::fs::read_dir(blobs.temporary_directory())
            .expect("read")
            .flatten()
            .next()
            .is_none()
    );
    let _ = database;
}

// ---------------------------------------------------------------------------
// The ceiling: `[storage] max_bytes`, and the third sweep (#862)
// ---------------------------------------------------------------------------

/// A store, a blob directory beside it, and `count` messages — oldest first,
/// one second apart — each holding a raw-source blob of `size` distinct bytes.
///
/// Distinct bytes because the store is content-addressed: two messages filled
/// with the same byte would share one blob, and a test about *which* blob
/// eviction takes would be testing nothing.
fn store_with_messages(
    count: usize,
    size: usize,
) -> (
    test_support::TempDatabase,
    BlobStore,
    Vec<postio_model::BlobId>,
) {
    let database = test_support::temp();
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut written = Vec::new();
    for index in 0..count {
        let blob = blobs
            .put(&vec![b'a' + index as u8; size])
            .expect("put a raw source");
        let received = chrono::TimeZone::timestamp_opt(&chrono::Utc, 1_000 + index as i64, 0)
            .single()
            .expect("a timestamp");
        let mut message = postio_model::Message::new(account.id, inbox, received);
        message.server.uid = Some(postio_model::Uid::new(index as u32 + 1));
        message.server.uid_validity = Some(postio_model::UidValidity::new(1));
        message.raw_blob_id = Some(blob.clone());
        messages.create(&mut message).expect("create");
        written.push(blob);
    }
    drop(connection);
    (database, blobs, written)
}

#[test]
fn a_store_over_its_ceiling_loses_its_least_wanted_blobs() {
    // The sentence #862 is about: `evict_to_fit` had no production caller, so
    // a `max_bytes` in somebody's config.toml did nothing at all. Oldest mail
    // first -- the mirror of a backfill that fetches newest first.
    let (database, blobs, written) = store_with_messages(3, 40_000);
    assert_eq!(blob_files(&blobs).len(), 3);

    // Room for the newest blob and nothing else.
    let budget = blobs.len_of(&written[2]).expect("len") + 16;
    let report = postio_session::enforce_storage_ceiling(&database, &blobs, Some(budget))
        .expect("the pass runs")
        .expect("a ceiling was set, so a pass ran");

    assert_eq!(report.removed, 2, "both of the older two");
    assert!(report.bytes_reclaimed > 0);
    assert!(report.bytes_remaining <= budget);
    assert!(!blobs.contains(&written[0]), "the oldest goes first");
    assert!(!blobs.contains(&written[1]));
    assert!(blobs.contains(&written[2]), "this week's mail stays");
}

#[test]
fn a_store_under_its_ceiling_loses_nothing() {
    // The other half, and the one a user notices: a ceiling they set high
    // enough must never cost them a refetch.
    let (database, blobs, written) = store_with_messages(3, 4_000);

    let report =
        postio_session::enforce_storage_ceiling(&database, &blobs, Some(100 * 1024 * 1024))
            .expect("the pass runs")
            .expect("a ceiling was set, so a pass ran");

    assert_eq!(report.removed, 0);
    assert_eq!(report.bytes_reclaimed, 0);
    assert!(written.iter().all(|blob| blobs.contains(blob)));
    assert_eq!(blob_files(&blobs).len(), 3);
}

#[test]
fn no_ceiling_means_the_pass_does_not_run_at_all() {
    // Unset is the default and the documented answer -- `[storage]`'s module
    // docs say a number here is a promise about somebody else's disk, and
    // Postio does not know how big theirs is. So no ceiling must not be read
    // as a ceiling of zero, which is the reading that would delete a whole
    // store on first start.
    let (database, blobs, written) = store_with_messages(3, 40_000);

    let report =
        postio_session::enforce_storage_ceiling(&database, &blobs, None).expect("the pass runs");

    assert!(report.is_none(), "no ceiling, so nothing to enforce");
    assert!(written.iter().all(|blob| blobs.contains(blob)));
    assert_eq!(blob_files(&blobs).len(), 3);
}
