//! A thread as a window or an expansion reads it (spec 013, `Req::Thread`).

use postio_model::listing::StoreError;
use postio_model::{ThreadId, ThreadView};
use postio_storage::Store;
use postio_storage::repository::{LabelRepository, ThreadRepository};

/// Five statements however long the thread is; the mute is a join through the thread's claims that
/// reads only the mutes.
pub async fn view(database: &Store, id: ThreadId) -> Result<Option<ThreadView>, StoreError> {
    let connection = database.read().await.map_err(StoreError::from)?;
    let threads = ThreadRepository::new(&connection);
    let Some(subject) = threads.subject(id).await.map_err(StoreError::from)? else {
        return Ok(None);
    };
    let members = threads.members(id).await.map_err(StoreError::from)?;
    let draft = threads.draft(id).await.map_err(StoreError::from)?;
    let muted = threads
        .muted(&[id])
        .await
        .map_err(StoreError::from)?
        .get(&id)
        .copied();
    let labels = LabelRepository::new(&connection)
        .for_threads(&[id])
        .await
        .map_err(StoreError::from)?
        .into_iter()
        .map(|(_, label)| label)
        .collect();
    tracing::debug!(thread = id.get(), members = members.len(), "thread read");
    Ok(Some(ThreadView {
        thread: id,
        subject,
        members,
        draft,
        muted,
        labels,
    }))
}

pub async fn muted_of(
    database: &Store,
    threads: &[ThreadId],
) -> Result<Vec<(ThreadId, u32)>, StoreError> {
    let connection = database.read().await.map_err(StoreError::from)?;
    let muted = ThreadRepository::new(&connection)
        .muted(threads)
        .await
        .map_err(StoreError::from)?;
    Ok(muted.into_iter().collect())
}
