//! The capture sheet's requests (spec 007 US15, FR-180, FR-181): the vault
//! `[focus.vault]` names, read and appended to on this computer through
//! `postio-vault`, off the runtime's worker threads since it is plain file
//! access.
//!
//! Nothing here is logged but what happened: a capture's text is the
//! person's mail, and logs never carry it.

use postio_client::protocol::VaultPicture;
use postio_model::listing::StoreError;
use postio_vault::{Captured, NoteEntry, Project, Task, Vault};

use crate::Inner;

/// What is said when no vault is configured.
pub(crate) const NO_VAULT: &str =
    "No vault to capture into: name one under [focus.vault] in config.toml";

/// The vault `[focus.vault]` names, opened.
fn open(inner: &Inner) -> Result<Vault, StoreError> {
    let section = inner
        .wiring
        .focus
        .config()
        .and_then(|config| config.vault)
        .ok_or_else(|| StoreError::new(NO_VAULT))?;
    let root = section.root().ok_or_else(|| StoreError::new(NO_VAULT))?;
    Vault::open(
        root,
        Some(&section.tasks_note()),
        section.projects().as_deref(),
    )
    .map_err(failed)
}

fn failed(error: postio_vault::VaultError) -> StoreError {
    StoreError::new(format!("The vault could not be used: {error}"))
}

/// Run `work` on the vault, on a thread that may block.
async fn with_vault<T: Send + 'static>(
    inner: &Inner,
    work: impl FnOnce(Vault) -> Result<T, postio_vault::VaultError> + Send + 'static,
) -> Result<T, StoreError> {
    let vault = open(inner)?;
    tokio::task::spawn_blocking(move || work(vault).map_err(failed))
        .await
        .map_err(|error| StoreError::new(format!("The vault could not be used: {error}")))?
}

/// The projects, the one suggested for `subject`, and the captured tasks.
pub(crate) async fn picture(inner: &Inner, subject: String) -> Result<VaultPicture, StoreError> {
    let tasks_note = inner
        .wiring
        .focus
        .config()
        .and_then(|config| config.vault)
        .map(|section| section.tasks_note())
        .unwrap_or_default();
    with_vault(inner, move |vault| {
        Ok(VaultPicture {
            projects: vault.projects()?,
            suggestion: vault.suggest(&subject)?,
            tasks_note,
            tasks: vault.tasks()?,
        })
    })
    .await
}

/// Append `task` to `project`'s note, or the tasks note.
pub(crate) async fn capture_task(
    inner: &Inner,
    project: Option<Project>,
    task: Task,
) -> Result<Captured, StoreError> {
    let captured = with_vault(inner, move |vault| {
        vault.append_task(project.as_ref(), &task)
    })
    .await;
    if captured.is_ok() {
        tracing::info!("a task was captured into the vault");
    }
    captured
}

/// Append `entry` to `note`.
pub(crate) async fn capture_note(
    inner: &Inner,
    note: std::path::PathBuf,
    entry: NoteEntry,
) -> Result<Captured, StoreError> {
    let captured = with_vault(inner, move |vault| vault.append_note(&note, &entry)).await;
    if captured.is_ok() {
        tracing::info!("a note was captured into the vault");
    }
    captured
}
