//! Counts the Sent-folder rows a pass inserted (mail sent from another client); this client's own
//! copies are matched by Message-ID and not counted twice.

use postio_model::{AccountId, MailboxRole};
use postio_storage::Connection;
use postio_storage::repository::{CorrespondentRepository, UpsertReport};

use crate::drain::Result;

pub(crate) async fn record(
    connection: &Connection,
    role: MailboxRole,
    account: AccountId,
    upsert: &UpsertReport,
) -> Result<()> {
    if role != MailboxRole::Sent || upsert.inserted_ids.is_empty() {
        return Ok(());
    }
    CorrespondentRepository::new(connection)
        .record_sent(account, &upsert.inserted_ids)
        .await?;
    Ok(())
}
