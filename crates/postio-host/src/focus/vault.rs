//! The capture sheet's requests: the `[focus.vault]` vault, read and appended to on this computer
//! through `postio-vault`, off the runtime's workers (plain file access).
//! Logs only what happened: a capture's text is the person's mail.

use postio_client::protocol::VaultPicture;
use postio_model::listing::StoreError;
use postio_vault::{Captured, NoteEntry, Project, Task, Vault};

use crate::Inner;

pub(crate) const NO_VAULT: &str =
    "No vault to capture into: name one under [focus.vault] in config.toml";

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

async fn with_vault<T: Send + 'static>(
    inner: &Inner,
    work: impl FnOnce(Vault) -> Result<T, postio_vault::VaultError> + Send + 'static,
) -> Result<T, StoreError> {
    let vault = open(inner)?;
    tokio::task::spawn_blocking(move || work(vault).map_err(failed))
        .await
        .map_err(|error| StoreError::new(format!("The vault could not be used: {error}")))?
}

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
