use postio_model::{EmailAddress, MessageId};

use crate::error::Result;
use crate::repository::messages::address_id;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    OpenPgp,
    X509,
}

impl KeyKind {
    fn as_str(self) -> &'static str {
        match self {
            KeyKind::OpenPgp => "openpgp",
            KeyKind::X509 => "x509",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Autocrypt,
    Attachment,
    File,
    Gnupg,
}

impl KeySource {
    fn as_str(self) -> &'static str {
        match self {
            KeySource::Autocrypt => "autocrypt",
            KeySource::Attachment => "attachment",
            KeySource::File => "file",
            KeySource::Gnupg => "gnupg",
        }
    }

    fn parse(text: &str) -> Option<KeySource> {
        [
            KeySource::Autocrypt,
            KeySource::Attachment,
            KeySource::File,
            KeySource::Gnupg,
        ]
        .into_iter()
        .find(|source| source.as_str() == text)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NewKey<'a> {
    pub fingerprint: &'a str,
    pub kind: KeyKind,
    pub keyblock: &'a [u8],
    pub primary_uid: Option<&'a str>,
    pub revoked: bool,
    pub addresses: &'a [EmailAddress],
    pub source: KeySource,
    pub message: Option<MessageId>,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredKey {
    pub fingerprint: String,
    pub keyblock: Vec<u8>,
    pub primary_uid: Option<String>,
    pub revoked: bool,
    pub first_seen: i64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictRow {
    pub format: String,
    pub scope: String,
    pub status: String,
    pub reason: Option<String>,
    pub fingerprint: Option<String>,
    pub checked_at: i64,
}

#[derive(Debug)]
pub struct KeyRepository<'a> {
    connection: &'a Connection,
}

impl<'a> KeyRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub async fn record(&self, key: &NewKey<'_>) -> Result<()> {
        let key = *key;
        let addresses = key.addresses.to_vec();
        sql::in_scope(self.connection, |transaction| async move {
            sql::execute(
                &transaction,
                "INSERT INTO public_keys
                     (fingerprint, kind, keyblock, primary_uid, revoked, first_seen, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT (fingerprint) DO UPDATE SET
                     keyblock = excluded.keyblock,
                     primary_uid = excluded.primary_uid,
                     revoked = max(revoked, excluded.revoked),
                     last_seen = max(last_seen, excluded.last_seen)",
                bind![
                    key.fingerprint,
                    key.kind.as_str(),
                    key.keyblock.to_vec(),
                    key.primary_uid.map(str::to_owned),
                    i64::from(key.revoked),
                    key.at
                ],
            )
            .await?;
            sql::execute(
                &transaction,
                "INSERT INTO public_key_sources (fingerprint, source, message_id, seen_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (fingerprint, source) DO UPDATE SET
                     message_id = coalesce(excluded.message_id, message_id),
                     seen_at = max(seen_at, excluded.seen_at)",
                bind![
                    key.fingerprint,
                    key.source.as_str(),
                    key.message.map(MessageId::get),
                    key.at
                ],
            )
            .await?;
            for address in &addresses {
                let id = address_id(&transaction, address).await?;
                sql::execute(
                    &transaction,
                    "INSERT INTO public_key_addresses (fingerprint, address_id) VALUES (?1, ?2)
                     ON CONFLICT DO NOTHING",
                    bind![key.fingerprint, id],
                )
                .await?;
            }
            Ok(())
        })
        .await
    }

    pub async fn for_address(&self, address: &EmailAddress) -> Result<Vec<StoredKey>> {
        sql::all(
            self.connection,
            "SELECT k.fingerprint, k.keyblock, k.primary_uid, k.revoked, k.first_seen, k.last_seen
               FROM addresses a
               JOIN public_key_addresses p ON p.address_id = a.id
               JOIN public_keys k ON k.fingerprint = p.fingerprint
              WHERE a.address_normalized = ?1
              ORDER BY k.first_seen, k.fingerprint
              LIMIT 64",
            bind![address.normalized()],
            |row| {
                Ok(StoredKey {
                    fingerprint: row.text(0)?,
                    keyblock: row.col(1)?,
                    primary_uid: row.opt_text(2)?,
                    revoked: row.flag(3)?,
                    first_seen: row.int(4)?,
                    last_seen: row.int(5)?,
                })
            },
        )
        .await
    }

    pub async fn sources(&self, fingerprint: &str) -> Result<Vec<KeySource>> {
        let names: Vec<String> = sql::all(
            self.connection,
            "SELECT source FROM public_key_sources WHERE fingerprint = ?1 ORDER BY source LIMIT 8",
            bind![fingerprint],
            |row| row.text(0),
        )
        .await?;
        Ok(names
            .iter()
            .filter_map(|name| KeySource::parse(name))
            .collect())
    }

    pub async fn accept(&self, address: &EmailAddress, fingerprint: &str, at: i64) -> Result<()> {
        let contact = crate::repository::ContactRepository::new(self.connection)
            .person_for_email(address)
            .await?;
        let id = address_id(self.connection, address).await?;
        sql::execute(
            self.connection,
            "INSERT INTO contact_accepted_keys (contact_id, address_id, fingerprint, accepted_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (contact_id, address_id) DO UPDATE SET
                 fingerprint = excluded.fingerprint, accepted_at = excluded.accepted_at",
            bind![contact.get(), id, fingerprint, at],
        )
        .await?;
        Ok(())
    }

    pub async fn accepted(&self, address: &EmailAddress) -> Result<Option<(String, i64)>> {
        sql::first(
            self.connection,
            "SELECT c.fingerprint, c.accepted_at
               FROM addresses a
               JOIN contact_accepted_keys c ON c.address_id = a.id
              WHERE a.address_normalized = ?1",
            bind![address.normalized()],
            |row| Ok((row.text(0)?, row.int(1)?)),
        )
        .await
    }

    pub async fn unaccept(&self, address: &EmailAddress) -> Result<()> {
        sql::execute(
            self.connection,
            "DELETE FROM contact_accepted_keys
              WHERE address_id IN (SELECT id FROM addresses WHERE address_normalized = ?1)",
            bind![address.normalized()],
        )
        .await?;
        Ok(())
    }

    pub async fn forget(&self, fingerprint: &str) -> Result<()> {
        sql::execute(
            self.connection,
            "DELETE FROM public_keys WHERE fingerprint = ?1",
            bind![fingerprint],
        )
        .await?;
        Ok(())
    }

    pub async fn drop_source(&self, source: KeySource) -> Result<u64> {
        sql::in_scope(self.connection, |transaction| async move {
            sql::execute(
                &transaction,
                "DELETE FROM public_key_sources WHERE source = ?1",
                bind![source.as_str()],
            )
            .await?;
            sql::execute(
                &transaction,
                "DELETE FROM public_keys
                  WHERE fingerprint NOT IN (SELECT fingerprint FROM public_key_sources)
                    AND fingerprint NOT IN (SELECT fingerprint FROM contact_accepted_keys)",
                (),
            )
            .await
        })
        .await
    }

    pub async fn signed_messages_from(&self, address: &EmailAddress) -> Result<Vec<MessageId>> {
        sql::all(
            self.connection,
            "SELECT v.message_id
               FROM addresses a
               JOIN recipients r ON r.address_id = a.id AND r.kind = 'from'
               JOIN signature_verdicts v ON v.message_id = r.message_id
              WHERE a.address_normalized = ?1
              ORDER BY v.message_id
              LIMIT 500",
            bind![address.normalized()],
            |row| Ok(MessageId::new(row.int(0)?)),
        )
        .await
    }

    pub async fn put_verdict(&self, message: MessageId, row: &VerdictRow) -> Result<()> {
        sql::execute(
            self.connection,
            "INSERT INTO signature_verdicts
                 (message_id, format, scope, status, reason, fingerprint, checked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (message_id) DO UPDATE SET
                 format = excluded.format, scope = excluded.scope, status = excluded.status,
                 reason = excluded.reason, fingerprint = excluded.fingerprint,
                 checked_at = excluded.checked_at",
            bind![
                message.get(),
                row.format.clone(),
                row.scope.clone(),
                row.status.clone(),
                row.reason.clone(),
                row.fingerprint.clone(),
                row.checked_at
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn verdict(&self, message: MessageId) -> Result<Option<VerdictRow>> {
        sql::first(
            self.connection,
            "SELECT format, scope, status, reason, fingerprint, checked_at
               FROM signature_verdicts WHERE message_id = ?1",
            bind![message.get()],
            |row| {
                Ok(VerdictRow {
                    format: row.text(0)?,
                    scope: row.text(1)?,
                    status: row.text(2)?,
                    reason: row.opt_text(3)?,
                    fingerprint: row.opt_text(4)?,
                    checked_at: row.int(5)?,
                })
            },
        )
        .await
    }
}
