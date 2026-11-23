use postio_model::{
    Account, AccountId, AuthMethod, EmailAddress, Identity, IdentityId, ServerConfig, Signature,
    SignatureId, TransportSecurity,
};

use super::{from_millis, require_persisted, to_millis, unknown_enum};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

/// An account and its identities are one unit: `update` makes the stored list match the one given;
/// use [`IdentityRepository`] to change one.
#[derive(Debug)]
pub struct AccountRepository<'a> {
    connection: &'a Connection,
}

const ACCOUNT_COLUMNS: &str = "\
id, display_name, address, address_name, incoming_host, incoming_port, incoming_security,
incoming_username, outgoing_host, outgoing_port, outgoing_security, outgoing_username,
auth_method, enabled, created_at, default_signature_id, pending_deletion,
oauth_client_id, oauth_token_url, oauth_authorize_url, oauth_scopes, backend,
backend_location, oauth_refresh_lifetime_days, is_default, max_message_size";

impl<'a> AccountRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// One transaction, so a half-saved identity list never shows a "From" picker missing an
    /// address.
    pub async fn create(&self, account: &mut Account) -> Result<AccountId> {
        sql::in_scope(self.connection, |transaction| async move {
            sql::execute(
                &transaction,
                "INSERT INTO accounts (display_name, address, address_name, incoming_host,
                                   incoming_port, incoming_security, incoming_username,
                                   outgoing_host, outgoing_port, outgoing_security,
                                   outgoing_username, auth_method, enabled, created_at,
                                   default_signature_id, oauth_client_id, oauth_token_url,
                                   oauth_authorize_url, oauth_scopes, backend,
                                   backend_location, oauth_refresh_lifetime_days,
                                   max_message_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                     ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
                bind![
                    account.display_name,
                    account.address.address,
                    account.address.name,
                    account.incoming.host,
                    account.incoming.port,
                    account.incoming.security.as_str(),
                    account.incoming.username,
                    account.outgoing.host,
                    account.outgoing.port,
                    account.outgoing.security.as_str(),
                    account.outgoing.username,
                    account.auth.as_str(),
                    account.enabled,
                    to_millis(account.created_at),
                    optional_signature_id(account.default_signature_id),
                    account.oauth.as_ref().map(|oauth| oauth.client_id.as_str()),
                    account.oauth.as_ref().map(|oauth| oauth.token_url.as_str()),
                    account
                        .oauth
                        .as_ref()
                        .map(|oauth| oauth.authorize_url.as_str()),
                    account.oauth.as_ref().map(|oauth| oauth.scopes.as_str()),
                    account.backend.kind(),
                    backend_location(&account.backend),
                    account
                        .oauth
                        .as_ref()
                        .and_then(|oauth| oauth.refresh_token_lifetime_days),
                    // SQLite integers are signed; a limit overflowing `i64` is not one any provider
                    // has.
                    account.max_message_size.and_then(|n| i64::try_from(n).ok()),
                ],
            )
            .await?;

            let id = AccountId::new(transaction.last_insert_rowid());
            account.id = id;
            super::release_own_address(&transaction, &account.address).await?;
            for (position, identity) in account.identities.iter_mut().enumerate() {
                identity.account_id = id;
                identity.id =
                    IdentityId::new(insert_identity(&transaction, identity, position).await?);
            }

            Ok(id)
        })
        .await
    }

    /// Makes the stored identity list match the value's; survivors keep their ids so drafts
    /// pointing at them survive. Takes `&mut` so new identities get their ids.
    ///
    /// Each identity's `account_id` is set from the account, not trusted.
    ///
    /// # Errors
    ///
    /// [`Error::NotPersisted`] if the account has no id yet; silently doing nothing would lose the
    /// edit.
    pub async fn update(&self, account: &mut Account) -> Result<()> {
        let id = require_persisted(account.id.get(), "account")?;
        let account_id = account.id;
        sql::in_scope(self.connection, |transaction| async move {
            let changed = sql::execute(
                &transaction,
                "UPDATE accounts
                SET display_name = ?2, address = ?3, address_name = ?4,
                    incoming_host = ?5, incoming_port = ?6, incoming_security = ?7,
                    incoming_username = ?8, outgoing_host = ?9, outgoing_port = ?10,
                    outgoing_security = ?11, outgoing_username = ?12, auth_method = ?13,
                    enabled = ?14, created_at = ?15, default_signature_id = ?16,
                    oauth_client_id = ?17, oauth_token_url = ?18,
                    oauth_authorize_url = ?19, oauth_scopes = ?20, backend = ?21,
                    backend_location = ?22,
                    oauth_refresh_lifetime_days = ?23,
                    max_message_size = ?24
              WHERE id = ?1",
                bind![
                    id,
                    account.display_name,
                    account.address.address,
                    account.address.name,
                    account.incoming.host,
                    account.incoming.port,
                    account.incoming.security.as_str(),
                    account.incoming.username,
                    account.outgoing.host,
                    account.outgoing.port,
                    account.outgoing.security.as_str(),
                    account.outgoing.username,
                    account.auth.as_str(),
                    account.enabled,
                    to_millis(account.created_at),
                    optional_signature_id(account.default_signature_id),
                    account.oauth.as_ref().map(|oauth| oauth.client_id.as_str()),
                    account.oauth.as_ref().map(|oauth| oauth.token_url.as_str()),
                    account
                        .oauth
                        .as_ref()
                        .map(|oauth| oauth.authorize_url.as_str()),
                    account.oauth.as_ref().map(|oauth| oauth.scopes.as_str()),
                    account.backend.kind(),
                    backend_location(&account.backend),
                    account
                        .oauth
                        .as_ref()
                        .and_then(|oauth| oauth.refresh_token_lifetime_days),
                    // SQLite integers are signed; a limit overflowing `i64` is not one any provider
                    // has.
                    account.max_message_size.and_then(|n| i64::try_from(n).ok()),
                ],
            )
            .await?;
            if changed == 0 {
                return Err(Error::NotFound {
                    entity: "account",
                    id,
                });
            }

            // Clear the default first: the schema allows only one per account, and
            // moving it between two identities would otherwise collide mid-update.
            sql::execute(
                &transaction,
                "UPDATE identities SET is_default = 0 WHERE account_id = ?1",
                [id],
            )
            .await?;

            let mut kept: Vec<i64> = Vec::with_capacity(account.identities.len());
            for (position, identity) in account.identities.iter_mut().enumerate() {
                identity.account_id = account_id;
                if identity.id.is_assigned() {
                    update_identity(&transaction, identity, position).await?;
                } else {
                    identity.id =
                        IdentityId::new(insert_identity(&transaction, identity, position).await?);
                }
                kept.push(identity.id.get());
            }

            let placeholders = super::messages::placeholders(kept.len(), 2);
            let mut arguments: Vec<turso::Value> = Vec::with_capacity(kept.len() + 1);
            arguments.push(turso::Value::Integer(id));
            arguments.extend(kept.into_iter().map(turso::Value::Integer));
            sql::execute(
                &transaction,
                &format!(
                    "DELETE FROM identities WHERE account_id = ?1 AND id NOT IN ({placeholders})"
                ),
                arguments,
            )
            .await?;

            Ok(())
        })
        .await
    }

    pub async fn get(&self, id: AccountId) -> Result<Option<Account>> {
        let found = sql::first(
            self.connection,
            &format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?1"),
            [id.get()],
            read_account,
        )
        .await?;
        let Some(mut account) = found else {
            return Ok(None);
        };
        account.identities = IdentityRepository::new(self.connection)
            .list_for_account(id)
            .await?;
        account.signatures = SignatureRepository::new(self.connection)
            .list_for_account(id)
            .await?;
        Ok(Some(account))
    }

    pub async fn list(&self) -> Result<Vec<Account>> {
        self.list_where("").await
    }

    /// Excludes accounts marked for removal even before the reap has run: an engine must never
    /// start against a row on its way out.
    pub async fn list_enabled(&self) -> Result<Vec<Account>> {
        self.list_where("WHERE enabled = 1 AND pending_deletion = 0")
            .await
    }

    async fn list_where(&self, filter: &str) -> Result<Vec<Account>> {
        let mut accounts: Vec<Account> = sql::all(
            self.connection,
            &format!("SELECT {ACCOUNT_COLUMNS} FROM accounts {filter} ORDER BY id"),
            (),
            read_account,
        )
        .await?;

        let identities = IdentityRepository::new(self.connection);
        let signatures = SignatureRepository::new(self.connection);
        for account in &mut accounts {
            account.identities = identities.list_for_account(account.id).await?;
            account.signatures = signatures.list_for_account(account.id).await?;
        }
        Ok(accounts)
    }

    /// Cascades in the schema; blobs are swept separately by
    /// [`BlobStore::collect_garbage`](crate::blob::BlobStore::collect_garbage).
    pub async fn delete(&self, id: AccountId) -> Result<bool> {
        let deleted = sql::execute(
            self.connection,
            "DELETE FROM accounts WHERE id = ?1",
            [id.get()],
        )
        .await?;
        Ok(deleted > 0)
    }

    /// A single-column write, not `update`: the settings toggle holds no fresh `Account`, and
    /// `update` could rewrite identities from a stale copy (#464).
    pub async fn set_enabled(&self, id: AccountId, enabled: bool) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "UPDATE accounts SET enabled = ?2 WHERE id = ?1",
            bind![id.get(), enabled],
        )
        .await?;
        Ok(changed > 0)
    }

    /// The account new messages come from when the message does not say; it does not order the
    /// sidebar or choose a reply's from address (#960).
    ///
    /// One transaction: a missing row rolls back rather than leaving no default. No
    /// `clear_default`: the reversal of marking one is marking another, so the command has no undo.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] if no account has that id.
    pub async fn set_default(&self, id: AccountId) -> Result<()> {
        sql::in_scope(self.connection, |transaction| async move {
            sql::execute(&transaction, "UPDATE accounts SET is_default = 0", ()).await?;
            let changed = sql::execute(
                &transaction,
                "UPDATE accounts SET is_default = 1 WHERE id = ?1",
                [id.get()],
            )
            .await?;
            if changed == 0 {
                return Err(Error::NotFound {
                    entity: "account",
                    id: id.get(),
                });
            }
            Ok(())
        })
        .await
    }

    /// Reversible with [`AccountRepository::restore`] until the reap runs, which is what the undo
    /// toast undoes (#464).
    pub async fn mark_pending_deletion(&self, id: AccountId) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "UPDATE accounts SET pending_deletion = 1 WHERE id = ?1",
            [id.get()],
        )
        .await?;
        Ok(changed > 0)
    }

    pub async fn restore(&self, id: AccountId) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "UPDATE accounts SET pending_deletion = 0 WHERE id = ?1",
            [id.get()],
        )
        .await?;
        Ok(changed > 0)
    }

    /// Run once at startup before any engine exists, never live: a crash before the undo toast
    /// expires leaves the row as marked, not half deleted.
    pub async fn reap_pending_deletions(&self) -> Result<Vec<AccountId>> {
        let ids: Vec<AccountId> = sql::all(
            self.connection,
            "SELECT id FROM accounts WHERE pending_deletion = 1",
            (),
            |row| Ok(AccountId::new(row.col(0)?)),
        )
        .await?;

        for id in &ids {
            sql::execute(
                self.connection,
                "DELETE FROM accounts WHERE id = ?1",
                [id.get()],
            )
            .await?;
        }
        Ok(ids)
    }
}

#[derive(Debug)]
pub struct IdentityRepository<'a> {
    connection: &'a Connection,
}

const IDENTITY_COLUMNS: &str = "\
id, account_id, display_name, address, address_name, reply_to_address, reply_to_name,
signature_text, signature_html, is_default";

impl<'a> IdentityRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub async fn create(&self, identity: &mut Identity) -> Result<IdentityId> {
        let account_id = require_persisted(identity.account_id.get(), "account")?;
        let position = sql::scalar(
            self.connection,
            "SELECT coalesce(max(position) + 1, 0) FROM identities WHERE account_id = ?1",
            [account_id],
        )
        .await?;
        let id = insert_identity(self.connection, identity, position as usize).await?;
        identity.id = IdentityId::new(id);
        Ok(identity.id)
    }

    pub async fn update(&self, identity: &Identity) -> Result<()> {
        let id = require_persisted(identity.id.get(), "identity")?;
        let changed = sql::execute(
            self.connection,
            "UPDATE identities
                SET display_name = ?2, address = ?3, address_name = ?4,
                    reply_to_address = ?5, reply_to_name = ?6,
                    signature_text = ?7, signature_html = ?8, is_default = ?9
              WHERE id = ?1",
            bind![
                id,
                identity.display_name,
                identity.address.address,
                identity.address.name,
                identity.reply_to.as_ref().map(|to| to.address.clone()),
                identity.reply_to.as_ref().and_then(|to| to.name.clone()),
                identity.signature.as_ref().map(|s| s.text.clone()),
                identity.signature.as_ref().and_then(|s| s.html.clone()),
                identity.is_default,
            ],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "identity",
                id,
            });
        }
        Ok(())
    }

    pub async fn get(&self, id: IdentityId) -> Result<Option<Identity>> {
        sql::first(
            self.connection,
            &format!("SELECT {IDENTITY_COLUMNS} FROM identities WHERE id = ?1"),
            [id.get()],
            read_identity,
        )
        .await
    }

    /// Every address the person sends as, on every account: what says which domains are their own.
    pub async fn own_addresses(&self) -> Result<Vec<EmailAddress>> {
        sql::all(self.connection, Self::explain_own_addresses(), (), |row| {
            Ok(EmailAddress::new(None::<String>, row.col::<String>(0)?))
        })
        .await
    }

    pub fn explain_own_addresses() -> &'static str {
        "SELECT address FROM accounts UNION SELECT address FROM identities"
    }

    pub async fn list_for_account(&self, account_id: AccountId) -> Result<Vec<Identity>> {
        sql::all(
            self.connection,
            &format!(
                "SELECT {IDENTITY_COLUMNS} FROM identities \
                 WHERE account_id = ?1 ORDER BY position, id"
            ),
            [account_id.get()],
            read_identity,
        )
        .await
    }

    pub async fn set_default(&self, account_id: AccountId, id: IdentityId) -> Result<()> {
        sql::in_scope(self.connection, |transaction| async move {
            sql::execute(
                &transaction,
                "UPDATE identities SET is_default = 0 WHERE account_id = ?1",
                [account_id.get()],
            )
            .await?;
            let changed = sql::execute(
                &transaction,
                "UPDATE identities SET is_default = 1 WHERE id = ?1 AND account_id = ?2",
                [id.get(), account_id.get()],
            )
            .await?;
            if changed == 0 {
                return Err(Error::NotFound {
                    entity: "identity",
                    id: id.get(),
                });
            }
            Ok(())
        })
        .await
    }

    pub async fn delete(&self, id: IdentityId) -> Result<bool> {
        let deleted = sql::execute(
            self.connection,
            "DELETE FROM identities WHERE id = ?1",
            [id.get()],
        )
        .await?;
        Ok(deleted > 0)
    }
}

async fn insert_identity(
    connection: &Connection,
    identity: &Identity,
    position: usize,
) -> Result<i64> {
    super::release_own_address(connection, &identity.address).await?;
    sql::execute(
        connection,
        "INSERT INTO identities (account_id, display_name, address, address_name,
                                 reply_to_address, reply_to_name, signature_text,
                                 signature_html, is_default, position)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        bind![
            identity.account_id.get(),
            identity.display_name,
            identity.address.address,
            identity.address.name,
            identity.reply_to.as_ref().map(|to| to.address.clone()),
            identity.reply_to.as_ref().and_then(|to| to.name.clone()),
            identity.signature.as_ref().map(|s| s.text.clone()),
            identity.signature.as_ref().and_then(|s| s.html.clone()),
            identity.is_default,
            position as i64,
        ],
    )
    .await?;
    Ok(connection.last_insert_rowid())
}

async fn update_identity(
    connection: &Connection,
    identity: &Identity,
    position: usize,
) -> Result<()> {
    super::release_own_address(connection, &identity.address).await?;
    sql::execute(
        connection,
        "UPDATE identities
            SET account_id = ?2, display_name = ?3, address = ?4, address_name = ?5,
                reply_to_address = ?6, reply_to_name = ?7, signature_text = ?8,
                signature_html = ?9, is_default = ?10, position = ?11
          WHERE id = ?1",
        bind![
            identity.id.get(),
            identity.account_id.get(),
            identity.display_name,
            identity.address.address,
            identity.address.name,
            identity.reply_to.as_ref().map(|to| to.address.clone()),
            identity.reply_to.as_ref().and_then(|to| to.name.clone()),
            identity.signature.as_ref().map(|s| s.text.clone()),
            identity.signature.as_ref().and_then(|s| s.html.clone()),
            identity.is_default,
            position as i64,
        ],
    )
    .await?;
    Ok(())
}

/// Separate from [`IdentityRepository`]: a signature belongs to the account, and the composer
/// chooses one per draft (#12).
#[derive(Debug)]
pub struct SignatureRepository<'a> {
    connection: &'a Connection,
}

const SIGNATURE_COLUMNS: &str = "id, name, text, html";

impl<'a> SignatureRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub async fn create(
        &self,
        account_id: AccountId,
        signature: &mut Signature,
    ) -> Result<SignatureId> {
        let account = require_persisted(account_id.get(), "account")?;
        let position = sql::scalar(
            self.connection,
            "SELECT coalesce(max(position) + 1, 0) FROM signatures WHERE account_id = ?1",
            [account],
        )
        .await?;
        sql::execute(
            self.connection,
            "INSERT INTO signatures (account_id, name, text, html, position)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            bind![
                account,
                signature.name,
                signature.text,
                signature.html,
                position
            ],
        )
        .await?;
        signature.id = SignatureId::new(self.connection.last_insert_rowid());
        Ok(signature.id)
    }

    pub async fn update(&self, signature: &Signature) -> Result<()> {
        let id = require_persisted(signature.id.get(), "signature")?;
        let changed = sql::execute(
            self.connection,
            "UPDATE signatures SET name = ?2, text = ?3, html = ?4 WHERE id = ?1",
            bind![id, signature.name, signature.text, signature.html],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "signature",
                id,
            });
        }
        Ok(())
    }

    pub async fn delete(&self, id: SignatureId) -> Result<bool> {
        let deleted = sql::execute(
            self.connection,
            "DELETE FROM signatures WHERE id = ?1",
            [id.get()],
        )
        .await?;
        Ok(deleted > 0)
    }

    pub async fn list_for_account(&self, account_id: AccountId) -> Result<Vec<Signature>> {
        sql::all(
            self.connection,
            &format!(
                "SELECT {SIGNATURE_COLUMNS} FROM signatures \
                  WHERE account_id = ?1 ORDER BY position, id"
            ),
            [account_id.get()],
            |row| {
                Ok(Signature {
                    id: SignatureId::new(row.col(0)?),
                    name: row.col(1)?,
                    text: row.col(2)?,
                    html: row.col(3)?,
                })
            },
        )
        .await
    }
}

fn read_account(row: &Row) -> Result<Account> {
    let incoming_security: String = row.col(6)?;
    let outgoing_security: String = row.col(10)?;
    let auth: String = row.col(12)?;

    Ok(Account {
        id: AccountId::new(row.col(0)?),
        display_name: row.col(1)?,
        address: EmailAddress::new(row.col::<Option<String>>(3)?, row.col::<String>(2)?),
        incoming: ServerConfig {
            host: row.col(4)?,
            port: row.col(5)?,
            security: parse_security(&incoming_security, "accounts.incoming_security")?,
            username: row.col(7)?,
        },
        outgoing: ServerConfig {
            host: row.col(8)?,
            port: row.col(9)?,
            security: parse_security(&outgoing_security, "accounts.outgoing_security")?,
            username: row.col(11)?,
        },
        auth: AuthMethod::from_name(&auth)
            .ok_or_else(|| unknown_enum("accounts.auth_method", auth))?,
        enabled: row.col(13)?,
        identities: Vec::new(),
        signatures: Vec::new(),
        default_signature_id: row.col::<Option<i64>>(15)?.map(SignatureId::new),
        created_at: from_millis(row.col(14)?),
        pending_deletion: row.col(16)?,
        is_default: row.col(24)?,
        // A negative ceiling is a corrupt row; reading it as "no limit" cannot refuse mail the
        // provider would take.
        max_message_size: row
            .col::<Option<i64>>(25)?
            .and_then(|n| u64::try_from(n).ok()),
        oauth: match (
            row.col::<Option<String>>(17)?,
            row.col::<Option<String>>(18)?,
        ) {
            (Some(client_id), Some(token_url)) => Some(postio_model::account::OAuthConfig {
                client_id,
                token_url,
                authorize_url: row.col::<Option<String>>(19)?.unwrap_or_default(),
                scopes: row.col::<Option<String>>(20)?.unwrap_or_default(),
                refresh_token_lifetime_days: row.col::<Option<u32>>(23)?,
            }),
            _ => None,
        },
        backend: match (
            row.col::<String>(21)?.as_str(),
            row.col::<Option<String>>(22)?,
        ) {
            // A jmap row without its session URL cannot dial; fall back to IMAP, since the incoming
            // server is stored either way.
            ("jmap", Some(session_url)) => postio_model::account::Backend::Jmap { session_url },
            ("gmail", _) => postio_model::account::Backend::Gmail,
            // A maildir row without its root has no incoming server to fall back to: it stays a
            // maildir and fails at connect.
            ("maildir", root) => postio_model::account::Backend::Maildir {
                root: root.unwrap_or_default(),
            },
            _ => postio_model::account::Backend::Imap,
        },
    })
}

/// A JMAP session URL and a maildir root share one column (#1278); it is only meaningful beside
/// `backend`, so the read side matches on the pair.
fn backend_location(backend: &postio_model::account::Backend) -> Option<&str> {
    match backend {
        postio_model::account::Backend::Jmap { session_url } => Some(session_url.as_str()),
        postio_model::account::Backend::Maildir { root } => Some(root.as_str()),
        postio_model::account::Backend::Imap | postio_model::account::Backend::Gmail => None,
    }
}

fn read_identity(row: &Row) -> Result<Identity> {
    let reply_to_address: Option<String> = row.col(5)?;
    let signature_text: Option<String> = row.col(7)?;

    Ok(Identity {
        id: IdentityId::new(row.col(0)?),
        account_id: AccountId::new(row.col(1)?),
        display_name: row.col(2)?,
        address: EmailAddress::new(row.col::<Option<String>>(4)?, row.col::<String>(3)?),
        reply_to: reply_to_address
            .map(|address| {
                Ok::<_, Error>(EmailAddress::new(row.col::<Option<String>>(6)?, address))
            })
            .transpose()?,
        signature: signature_text
            .map(|text| {
                Ok::<_, Error>(Signature {
                    // The identity's own signature is not a row in the named set (migration 0009).
                    id: SignatureId::UNASSIGNED,
                    name: String::new(),
                    text,
                    html: row.col(8)?,
                })
            })
            .transpose()?,
        is_default: row.col(9)?,
    })
}

fn parse_security(value: &str, column: &'static str) -> Result<TransportSecurity> {
    TransportSecurity::from_name(value).ok_or_else(|| unknown_enum(column, value))
}

fn optional_signature_id(id: Option<SignatureId>) -> Option<i64> {
    id.filter(|id| id.is_assigned()).map(SignatureId::get)
}
