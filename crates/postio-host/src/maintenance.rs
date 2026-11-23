//! The store's upkeep: passes that catch the store up with itself, and the storage ceiling.
//! Nothing here is waited on or reaches the network.

use postio_session::Wiring;

/// Every body reaches the search index through the indexer and nothing else
/// (`postio_session::spawn_body_indexer`), so a store with no account or network still becomes
/// searchable.
pub fn spawn_idle_passes(wiring: &Wiring) {
    postio_session::spawn_body_indexer(
        wiring.database.clone(),
        wiring.events.subscribe("indexer"),
        &wiring.runtime,
    );
    // Only what is already on disk, never a fetch.
    postio_session::spawn_attachment_indexer(
        wiring.database.clone(),
        wiring.blobs.clone(),
        wiring.events.subscribe("attachment-indexer"),
        &wiring.runtime,
    );
    repair_the_header_blocks(wiring);
    catch_up_the_header_index(wiring);
    build_contacts(wiring);
    reclaim_disk(wiring);
    empty_junk_and_trash(wiring);
}

/// The setting counts days, so hours are plenty.
const AUTO_EMPTY_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// Off by default, and then no task starts. Queues like any delete, so the drainer tells the server
/// (#1932).
pub fn empty_junk_and_trash(wiring: &Wiring) {
    let policy = wiring.auto_empty;
    if !policy.is_on() {
        return;
    }
    let database = wiring.database.clone();
    wiring.runtime.spawn(async move {
        loop {
            match postio_session::auto_empty::run(&database, policy, chrono::Utc::now()).await {
                Ok(report) if report.purged > 0 => {
                    tracing::info!(
                        purged = report.purged,
                        more = report.more,
                        "emptied Junk and Trash"
                    );
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "could not empty Junk and Trash"),
            }
            tokio::time::sleep(AUTO_EMPTY_EVERY).await;
        }
    });
}

/// Lowering it evicts without a restart (#929).
pub fn enforce_ceiling(wiring: &Wiring, max_bytes: Option<u64>) {
    let database = wiring.database.clone();
    let blobs = wiring.blobs.clone();
    wiring.runtime.spawn(async move {
        if let Err(error) =
            postio_session::enforce_storage_ceiling(&database, &blobs, max_bytes).await
        {
            tracing::warn!(%error, "could not bring the store under its new ceiling");
        }
    });
}

/// `messages.body_headers` was NULL on every row until #884, so without this `header:` answers "no
/// such mail" across an existing mailbox.
/// `spawn_blocking`, once per start: a synchronous blob read and header parse per message that
/// nothing waits on.
/// Runs after the index catch-up, which a searcher waits on. Later passes cost one query that finds
/// nothing.
pub fn repair_the_header_blocks(wiring: &Wiring) {
    let (database, blobs) = (wiring.database.clone(), wiring.blobs.clone());
    wiring.runtime.spawn(async move {
        if let Err(error) = postio_session::repair_header_blocks(&database, &blobs).await {
            // Recoverable: `header:` is a little less complete until the next start.
            tracing::warn!(%error, "could not rebuild the stored header blocks");
        }
    });
}

/// `header:` matches `message_headers`, derived from `messages.body_headers` (ADR 0025 Q2), so
/// existing stores have blocks with no rows.
/// Runs after [`repair_the_header_blocks`], which writes the blocks this one reads; both are
/// spawned, so the order is a head start, not a guarantee.
/// `spawn_blocking`, once per start; later passes cost one query that finds nothing.
pub fn catch_up_the_header_index(wiring: &Wiring) {
    let database = wiring.database.clone();
    wiring.runtime.spawn(async move {
        if let Err(error) = postio_session::index_local_headers(&database).await {
            tracing::warn!(%error, "could not index the header blocks already on disk");
        }
    });
}

/// Resumable: an address with a person is not in the walk, so a pass cut off by quitting leaves the
/// rest for the next start.
/// One short transaction per batch, with a pause so sync can write.
pub fn build_contacts(wiring: &Wiring) {
    let database = wiring.database.clone();
    wiring.runtime.spawn(async move {
        if let Err(error) = crate::contacts::catch_up(&database, 500, |_| {}).await {
            tracing::warn!(%error, "could not build contacts from the mail already here");
        }
        if let Err(error) = crate::contacts::detect_kinds(&database, 200).await {
            tracing::warn!(%error, "could not sort contacts by kind");
        }
        if let Err(error) = crate::contacts::save_people_written_to(&database).await {
            tracing::warn!(%error, "could not save the people written to");
        }
    });
}

/// `BLOB_GRACE_PERIOD`, never `Duration::ZERO`: a blob is written before the row that references it
/// commits, so a healthy blob inside that window looks like an orphan. The cheap debris purge runs
/// inline; the whole-tree garbage collection goes on a worker. `evict_to_fit` runs last, so it
/// never evicts a blob that an orphan of the same size could replace for free (#862). Once per
/// start.
pub fn reclaim_disk(wiring: &Wiring) {
    let (database, blobs) = (wiring.database.clone(), wiring.blobs.clone());
    let ceiling = wiring.storage_ceiling;
    wiring.runtime.spawn(async move {
        if let Err(error) = postio_session::purge_fetch_debris(&blobs) {
            tracing::warn!(%error, "could not remove debris from unfinished fetches");
        }
        // Dragged-out mail is full plaintext outside the blob store (#278); the grace period is the
        // guard, not a tuning knob.
        match postio_session::reclaim_drag_exports(
            &postio_session::paths::export_dir(),
            postio_session::DRAG_EXPORT_GRACE_PERIOD,
        ) {
            Ok(0) => {}
            Ok(removed) => tracing::debug!(removed, "reclaimed dragged-out exports"),
            Err(error) => tracing::warn!(%error, "could not reclaim dragged-out exports"),
        }
        if let Err(error) = postio_session::reclaim_orphaned_blobs(
            &database,
            &blobs,
            postio_session::BLOB_GRACE_PERIOD,
        )
        .await
        {
            // Recoverable: the next start tries again.
            tracing::warn!(%error, "could not reclaim blobs nothing references");
        }
        match postio_session::prune_settled_operations(
            &database,
            postio_session::OPERATION_RETENTION,
            chrono::Utc::now(),
        )
        .await
        {
            Ok(0) => {}
            Ok(removed) => tracing::debug!(removed, "pruned settled operations"),
            Err(error) => tracing::warn!(%error, "could not prune settled operations: {error}"),
        }
        // Last, and only with a ceiling set: this sweep costs a refetch, so it takes what the free
        // sweeps left.
        if let Err(error) =
            postio_session::enforce_storage_ceiling(&database, &blobs, ceiling).await
        {
            tracing::warn!(%error, "could not bring the store under its ceiling");
        }
        // A full `VACUUM` rewrites the whole database and blocks every writer (about 25 MiB/s), so
        // it is decided, not stepped:
        // `is_worth_reclaiming` says no unless the holes are large and a real share of the file
        // (see `Store::reclaim_free_pages`).
        // Here, off the startup path and every interaction.
        match database.is_worth_reclaiming().await {
            Ok(true) => match database.reclaim_free_pages().await {
                Ok(bytes) => tracing::info!(bytes, "reclaimed free database pages"),
                Err(error) => tracing::warn!(%error, "could not reclaim free pages: {error}"),
            },
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, "could not ask the store about free pages: {error}");
            }
        }
        // `wal_checkpoint(TRUNCATE)` reclaims the write-ahead log (#1175); here, off the startup
        // path and every interaction.
        match database.truncate_log().await {
            Ok(0) => {}
            Ok(bytes) => tracing::info!(bytes, "truncated the write-ahead log"),
            Err(error) => tracing::warn!(%error, "could not truncate the log"),
        }
    });
}
