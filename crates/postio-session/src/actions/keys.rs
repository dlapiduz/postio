use postio_core::dispatch::CommandError;
use postio_core::undo::UndoKind;
use postio_core::{Command, KeptKey};
use postio_model::{EmailAddress, MessageId};
use postio_storage::repository::{KeyKind, KeyRepository, KeySource, MessageRepository, NewKey};

use super::{Actions, Applied, store_failure};

impl Actions {
    pub(super) async fn accept_key(
        &self,
        message: Option<MessageId>,
        accepted: bool,
        kept: Option<&str>,
    ) -> Result<Applied, CommandError> {
        let (connection, _permit) = self.connect().await?;
        let (row, signer) = signed(&connection, message).await?;
        let from = sender(&row)?;
        let keys = KeyRepository::new(&connection);
        let (kind, inverse) = if accepted {
            let before = keys.accepted(&from).await.map_err(store_failure)?;
            keys.accept(&from, &signer, chrono::Utc::now().timestamp())
                .await
                .map_err(store_failure)?;
            (
                UndoKind::AcceptKey,
                Command::AcceptKey {
                    message: Some(row.id),
                    accepted: false,
                    kept: before.map(|(key, _)| key),
                },
            )
        } else {
            match kept {
                Some(before) => keys
                    .accept(&from, before, chrono::Utc::now().timestamp())
                    .await
                    .map_err(store_failure)?,
                None => keys.unaccept(&from).await.map_err(store_failure)?,
            }
            (
                UndoKind::UnacceptKey,
                Command::AcceptKey {
                    message: Some(row.id),
                    accepted: true,
                    kept: None,
                },
            )
        };
        Ok(Applied::changed_one(kind, row.account_id, row.id, inverse))
    }

    pub(super) async fn forget_key(
        &self,
        message: Option<MessageId>,
        forgotten: bool,
        kept: Option<&KeptKey>,
    ) -> Result<Applied, CommandError> {
        let (connection, permit) = self.connect().await?;
        let row = MessageRepository::new(&connection)
            .get(message.ok_or_else(|| CommandError::rejected("Pick a signed message"))?)
            .await
            .map_err(store_failure)?
            .ok_or_else(|| CommandError::rejected("That message is no longer here"))?;
        let from = sender(&row)?;
        let keys = KeyRepository::new(&connection);
        let (kind, inverse) = if forgotten {
            let (_, signer) = signed(&connection, message).await?;
            let (key, addresses) = keys
                .key(&signer)
                .await
                .map_err(store_failure)?
                .ok_or_else(|| CommandError::rejected("Postio holds no key for this signature"))?;
            let sources = keys
                .sources(&signer)
                .await
                .map_err(store_failure)?
                .into_iter()
                .map(|source| source.as_str().to_owned())
                .collect();
            keys.forget(&signer).await.map_err(store_failure)?;
            (
                UndoKind::ForgetKey,
                Command::ForgetKey {
                    message: Some(row.id),
                    forgotten: false,
                    kept: Some(KeptKey {
                        fingerprint: key.fingerprint,
                        keyblock: key.keyblock,
                        primary_uid: key.primary_uid,
                        sources,
                        addresses,
                        first_seen: key.first_seen,
                    }),
                },
            )
        } else {
            let kept =
                kept.ok_or_else(|| CommandError::rejected("There is no key to bring back"))?;
            let addresses: Vec<EmailAddress> = kept
                .addresses
                .iter()
                .map(|address| EmailAddress::new(None::<String>, address.as_str()))
                .collect();
            for source in kept
                .sources
                .iter()
                .filter_map(|name| KeySource::parse(name))
            {
                keys.record(&NewKey {
                    fingerprint: &kept.fingerprint,
                    kind: KeyKind::OpenPgp,
                    keyblock: &kept.keyblock,
                    primary_uid: kept.primary_uid.as_deref(),
                    revoked: false,
                    addresses: &addresses,
                    source,
                    message: None,
                    at: kept.first_seen,
                })
                .await
                .map_err(store_failure)?;
            }
            (
                UndoKind::RestoreKey,
                Command::ForgetKey {
                    message: Some(row.id),
                    forgotten: true,
                    kept: None,
                },
            )
        };
        drop(permit);
        let again = keys
            .signed_messages_from(&from)
            .await
            .map_err(store_failure)?;
        drop(connection);
        if let Some(blobs) = &self.blobs {
            crate::signature::check_messages(&self.database, blobs, &again)
                .await
                .map_err(store_failure)?;
        }
        Ok(Applied::changed_one(kind, row.account_id, row.id, inverse))
    }
}

async fn signed(
    connection: &postio_storage::Checkout,
    message: Option<MessageId>,
) -> Result<(postio_model::Message, String), CommandError> {
    let message = message.ok_or_else(|| CommandError::rejected("Pick a signed message"))?;
    let row = MessageRepository::new(connection)
        .get(message)
        .await
        .map_err(store_failure)?
        .ok_or_else(|| CommandError::rejected("That message is no longer here"))?;
    let signer = KeyRepository::new(connection)
        .verdict(message)
        .await
        .map_err(store_failure)?
        .and_then(|verdict| verdict.fingerprint)
        .ok_or_else(|| CommandError::rejected("This message names no signing key"))?;
    Ok((row, signer))
}

fn sender(row: &postio_model::Message) -> Result<EmailAddress, CommandError> {
    row.from
        .first()
        .cloned()
        .ok_or_else(|| CommandError::rejected("That message has no sender"))
}
