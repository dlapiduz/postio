//! The store's upkeep: the passes that catch the store up with itself once
//! it is open, and the storage ceiling.
//!
//! The store's owner's work, moved here from the desktop app (ADR 0041), so
//! it runs in whichever app has the store open: the desktop app or the
//! terminal. None of it is anything a person waits on, and none of it
//! reaches the network.

use postio_session::Wiring;

/// Start every catch-up pass over `wiring`'s store: the body indexer (a
/// catch-up pass now, then one batched write after each burst of
/// `BodyLoaded` on the wiring's hub), the header repair and index catch-up,
/// and the disk reclaim.
///
/// Every body reaches the search index through the indexer and nothing else
/// -- neither the store nor the fetch writes the row -- see
/// `postio_session::spawn_body_indexer`. A store opened with no account, or
/// with the network down, still has bodies on disk and still becomes
/// searchable.
pub fn spawn_idle_passes(wiring: &Wiring) {
    postio_session::spawn_body_indexer(
        wiring.database.clone(),
        wiring.events.subscribe("indexer"),
        &wiring.runtime,
    );
    repair_the_header_blocks(wiring);
    catch_up_the_header_index(wiring);
    reclaim_disk(wiring);
}

/// Bring the store under `max_bytes`, off the caller's thread: `[storage]
/// max_bytes` changed while Postio runs (#929). Lowering it evicts without a
/// restart; raising it needs nothing beyond the next pass reading the new
/// number.
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

/// Rebuild the header blocks of mail that arrived before there was anywhere
/// to put them, out of the way.
///
/// `messages.body_headers` has been NULL on every row in every store since the
/// schema was written, because both backfill paths passed `headers: None` on
/// purpose (#884). New mail carries its block now; this is the mail already
/// here, and without it `header:` would answer "no such mail" across a mailbox
/// somebody has been using for a year — which reads as the feature being
/// broken rather than as an index catching up.
///
/// `spawn_blocking` and once per start, the same two reasons
/// [`postio_session::spawn_body_indexer`] has: it is a blob read and a header parse per
/// message, synchronous from beginning to end, and nothing on screen is
/// waiting for it. After the index catch-up rather than before, for the same
/// reason that one goes first — somebody is waiting to search their mail, and
/// this only makes a *later* search sharper.
///
/// Every pass after the first costs one query that finds nothing.
pub fn repair_the_header_blocks(wiring: &Wiring) {
    let (database, blobs) = (wiring.database.clone(), wiring.blobs.clone());
    wiring.runtime.spawn(async move {
        if let Err(error) = postio_session::repair_header_blocks(&database, &blobs).await {
            // Recoverable, like every other idle pass here: `header:` is a
            // little less complete until the next start tries again, and a
            // mail client that would not open over it would be trading the
            // wrong thing.
            tracing::warn!(%error, "could not rebuild the stored header blocks");
        }
    });
}

/// Index the header blocks that are already on this machine, out of the way.
///
/// `header:` matches `message_headers`, which is derived from
/// `messages.body_headers` (ADR 0025 Q2) — so on every store that exists, and
/// after every bump of the headers schema half, there is a mailbox's worth of
/// stored blocks with no rows. The fetch path indexes the mail that arrives
/// from now on; this is the mail that is already here.
///
/// After [`repair_the_header_blocks`] rather than before, and the order is
/// load-bearing rather than tidy: that pass is what *writes* the blocks this
/// one reads, so on a store that predates #884 running them the other way
/// round would leave this pass with almost nothing to index and everything to
/// do again next start. Both are spawned, so the ordering is a head start
/// rather than a guarantee — and a message either pass misses is picked up by
/// whichever of them runs next, which is what makes that acceptable.
///
/// `spawn_blocking` and once per start, the same two reasons
/// [`postio_session::spawn_body_indexer`] has: it is synchronous SQLite that
/// decompresses and parses a block per message, and nothing on screen waits
/// for it. Every pass after the first costs one query that finds nothing.
pub fn catch_up_the_header_index(wiring: &Wiring) {
    let database = wiring.database.clone();
    wiring.runtime.spawn(async move {
        if let Err(error) = postio_session::index_local_headers(&database).await {
            // Recoverable, like every other idle pass here: `header:` is a
            // little less complete until the next start tries again.
            tracing::warn!(%error, "could not index the header blocks already on disk");
        }
    });
}

/// Give back the disk nothing is using any more, out of the way.
///
/// # Why this had to be added rather than fixed
///
/// `BlobStore::collect_garbage` and `BlobStore::purge_temporary` were both
/// written, tested and documented, and neither had a production caller (#416).
/// The blob store's own module docs describe garbage collection as *the*
/// mechanism that keeps blobs from leaking — "a sweep cannot drift out of sync
/// with the data" — and `MessageRepository::delete` removes a message's row
/// without touching its blobs precisely because that sweep is supposed to
/// follow.
///
/// With nothing calling it, **deleting mail freed nothing, ever**, and a
/// `UIDVALIDITY` reset — which wipes and re-syncs an entire mailbox — orphaned
/// every blob in it at once, permanently.
///
/// # Two sweeps, two different costs
///
/// The debris purge is one `read_dir` of a directory that is empty in the
/// ordinary case, so it runs first and inline. Garbage collection walks the
/// whole blob tree, which on a backfilled archive is a great many files, so it
/// goes on a worker for the same reason [`postio_session::spawn_body_indexer`] does: a
/// mail client that will not draw until it has counted its own files has
/// traded the wrong thing.
///
/// `BLOB_GRACE_PERIOD`, never `Duration::ZERO`. A blob is written before the
/// row that references it is committed, so inside that window a healthy blob
/// looks exactly like an orphan, and a sweep without the grace period deletes
/// the body of a message that is mid-fetch.
///
/// Once per start, not on a timer. Orphans are produced by deletes, moves and
/// resyncs — none of which happen fast enough to be worth a schedule, and all
/// of which will still be there next time.
///
/// # And the third sweep
///
/// `BlobStore::evict_to_fit` was the one #416 left behind, on the argument
/// that it is opt-in and so less urgent than the two that leaked
/// unconditionally. It stayed uncalled, which meant `[storage] max_bytes` was
/// a setting that parsed and did nothing (#862). It runs last here, after the
/// two sweeps that take only what nothing wants: there is no sense evicting a
/// blob somebody would have to refetch when an orphan of the same size was
/// about to go for free.
pub fn reclaim_disk(wiring: &Wiring) {
    let (database, blobs) = (wiring.database.clone(), wiring.blobs.clone());
    let ceiling = wiring.storage_ceiling;
    wiring.runtime.spawn(async move {
        if let Err(error) = postio_session::purge_fetch_debris(&blobs) {
            tracing::warn!(%error, "could not remove debris from unfinished fetches");
        }
        // Dragged-out mail, which is a privacy question rather than only a
        // disk one: these are full plaintext copies of messages, outside the
        // blob store, in a directory nothing audits (#278). The grace period
        // is the guard -- see `reclaim_drag_exports` -- not a tuning knob.
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
            // Recoverable, and the same judgement the body index makes: a mail
            // client that could not tidy up still reads mail, and the next
            // start tries again.
            tracing::warn!(%error, "could not reclaim blobs nothing references");
        }
        // Settled operations past their retention: the queue's own sweep,
        // which existed and was never run until now.
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
        // Last, and only when somebody has set a ceiling: this is the sweep
        // that costs a refetch, so it takes what the free sweeps left.
        if let Err(error) =
            postio_session::enforce_storage_ceiling(&database, &blobs, ceiling).await
        {
            tracing::warn!(%error, "could not bring the store under its ceiling");
        }
        // The database's own pages, when there are enough of them to be worth
        // it. #381 converted the store to `auto_vacuum = INCREMENTAL` and
        // stepped it here, because deleting a message frees pages inside the
        // file and hands nothing back to the filesystem -- which a store
        // holding a whole mailbox replica cannot afford, since a
        // `UIDVALIDITY` reset wipes and re-syncs an entire folder from one
        // server-side event.
        //
        // This engine has no `auto_vacuum` toggle and no incremental step. It
        // has a full `VACUUM`, which is a different proposition: it rewrites
        // the whole database and **blocks every writer while it does**, at
        // roughly 25 MiB/s. So it is not stepped, it is *decided* --
        // `is_worth_reclaiming` says no unless the holes are both large in
        // themselves and a real share of the file, because freed pages are
        // reused and a store plateaus rather than creeping upward. See
        // `Store::reclaim_free_pages` for what that costs and why an
        // interrupted one is safe.
        //
        // Here rather than anywhere else for the same reason the log
        // truncation below is: off the startup path and off every
        // interaction.
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
        // The write-ahead log is the other half that *can* be reclaimed. #1175
        // bounded it with `journal_size_limit`, which this engine does not
        // have; `wal_checkpoint(TRUNCATE)` is the mechanism it does, and
        // here is where it belongs -- off the startup path and off every
        // interaction, which is what #1175's own 676 MB WAL was about.
        match database.truncate_log().await {
            Ok(0) => {}
            Ok(bytes) => tracing::info!(bytes, "truncated the write-ahead log"),
            Err(error) => tracing::warn!(%error, "could not truncate the log"),
        }
    });
}
