//! Who the person wrote to, as their mail is filed (spec 007, T075).
//!
//! `postio_storage`'s `CorrespondentRepository` counts each message the
//! person sent once, for every address it went to. Sent mail reaches the
//! store two ways and this is the sync half: a pass over a Sent folder
//! counts the rows it *inserted*, which is exactly the mail this store did
//! not have -- sent from another client, or before this one existed. A copy
//! this client's own send filed (and counted) is matched by its Message-ID
//! and updated, not inserted, so it is not counted a second time; neither
//! is anything a re-enumeration re-reads. The send half is
//! [`crate::send`]'s.

use postio_model::{AccountId, MailboxRole};
use postio_storage::Connection;
use postio_storage::repository::{CorrespondentRepository, UpsertReport};

use crate::drain::Result;

/// Counts what `upsert` inserted into a mailbox of `role`, when that is the
/// account's Sent: two statements for the unit, whatever it held, and
/// nothing at all for any other folder.
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
