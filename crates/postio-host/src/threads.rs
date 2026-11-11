//! A thread as a window or an expansion reads it (spec 013, `Req::Thread`).

use postio_model::listing::StoreError;
use postio_model::{ThreadId, ThreadView};
use postio_storage::Store;
use postio_storage::repository::{LabelRepository, ThreadRepository};

/// Thread `id`, or `None` when it is gone.
///
/// Four statements however long the thread is: its subject (a rowid seek),
/// its members with their lines (`ThreadRepository::members`), its unsent
/// reply, and its labels. The labels come from
/// `LabelRepository::for_threads`, the read Focus's list pages already use
/// for a page of threads, asked here for one. Mute arrives with US5, so
/// `muted` is `None` for now.
pub async fn view(database: &Store, id: ThreadId) -> Result<Option<ThreadView>, StoreError> {
    let connection = database.read().await.map_err(StoreError::from)?;
    let threads = ThreadRepository::new(&connection);
    let Some(subject) = threads.subject(id).await.map_err(StoreError::from)? else {
        return Ok(None);
    };
    let members = threads.members(id).await.map_err(StoreError::from)?;
    let draft = threads.draft(id).await.map_err(StoreError::from)?;
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
        muted: None,
        labels,
    }))
}
