//! Accounts and the identities that send from them.

use postio_model::{
    Account, AccountId, AuthMethod, EmailAddress, Identity, IdentityId, ServerConfig, Signature,
    SignatureId, TransportSecurity,
};

use super::{from_millis, require_persisted, to_millis, unknown_enum};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

/// Reads and writes [`Account`] rows, together with their identities.
///
/// An account and its identities are one unit: [`AccountRepository::get`]
/// returns the identities loaded, and [`AccountRepository::update`] makes the
/// stored list match the one it is handed. Use [`IdentityRepository`] to change
/// one identity without rewriting the account.
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
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Inserts `account` and every identity on it, assigning their ids.
    ///
    /// The account row and its identities are written in one transaction: an
    /// account whose identity list was half saved would show a "From" picker
    /// missing the address the user just typed.
    pub async fn create(&self, account: &mut Account) -> Result<AccountId> {
        sql::in_scope(self.connection, |transaction| async move {
            transaction
                .execute(
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
                        // SQLite integers are signed, and a limit large enough to
                        // overflow `i64` is not a limit any provider has.
                        account.max_message_size.and_then(|n| i64::try_from(n).ok()),
                    ],
                )
                .await?;

            let id = AccountId::new(transaction.last_insert_rowid());
            account.id = id;
            for (position, identity) in account.identities.iter_mut().enumerate() {
                identity.account_id = id;
                identity.id =
                    IdentityId::new(insert_identity(&transaction, identity, position).await?);
            }

            Ok(id)
        })
        .await
    }

    /// Writes `account` back, making its identity list authoritative.
    ///
    /// Identities present in the value are inserted or updated; identities the
    /// database still has and the value does not are deleted. An identity that
    /// survives keeps its id, so a draft that points at it survives too. Takes
    /// `&mut` for the same reason [`AccountRepository::create`] does: a newly
    /// added identity gets its id written back.
    ///
    /// The account owns the list, so each identity's `account_id` is set from
    /// the account rather than trusted — an identity built by the settings UI
    /// has not been told which account it is about to belong to.
    ///
    /// # Errors
    ///
    /// [`Error::NotPersisted`] if the account has no id yet — that is a
    /// [`AccountRepository::create`], and silently doing nothing would lose the
    /// user's edit.
    pub async fn update(&self, account: &mut Account) -> Result<()> {
        let id = require_persisted(account.id.get(), "account")?;
        let account_id = account.id;
        sql::in_scope(self.connection, |transaction| async move {
            let changed = transaction
                .execute(
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
                        // SQLite integers are signed, and a limit large enough to
                        // overflow `i64` is not a limit any provider has.
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
            transaction
                .execute(
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

            let placeholders = placeholders(kept.len());
            let mut arguments: Vec<turso::Value> = Vec::with_capacity(kept.len() + 1);
            arguments.push(turso::Value::Integer(id));
            arguments.extend(kept.into_iter().map(turso::Value::Integer));
            transaction
            .execute(
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

    /// One account, with its identities.
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

    /// Every account, in creation order, with their identities.
    pub async fn list(&self) -> Result<Vec<Account>> {
        self.list_where("").await
    }

    /// Every account that participates in sync.
    ///
    /// Excludes anything marked for removal even before
    /// [`AccountRepository::reap_pending_deletions`] has actually run --
    /// belt and braces, since the reap is meant to run first regardless, but
    /// an engine should never start against a row on its way out.
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

    /// Deletes an account and everything that hangs off it, returning whether
    /// there was one.
    ///
    /// Mailboxes, messages, threads, labels, drafts and queued operations all
    /// cascade in the schema; the blob store is swept separately by
    /// [`BlobStore::collect_garbage`](crate::blob::BlobStore::collect_garbage).
    pub async fn delete(&self, id: AccountId) -> Result<bool> {
        let deleted = self
            .connection
            .execute("DELETE FROM accounts WHERE id = ?1", [id.get()])
            .await?;
        Ok(deleted > 0)
    }

    /// Flips whether the account participates in sync (#464, ADR 0005 Q6).
    ///
    /// A single-column write rather than [`AccountRepository::update`]: the
    /// caller here is a settings-panel toggle, not code holding a full,
    /// freshly-loaded `Account` with its identity list intact, and routing
    /// through `update` would risk silently rewriting identities from a
    /// stale copy.
    pub async fn set_enabled(&self, id: AccountId, enabled: bool) -> Result<bool> {
        let changed = self
            .connection
            .execute(
                "UPDATE accounts SET enabled = ?2 WHERE id = ?1",
                bind![id.get(), enabled],
            )
            .await?;
        Ok(changed > 0)
    }

    /// Makes one account the default, clearing any other (#960).
    ///
    /// The account new messages come from when the message itself does not
    /// say — and nothing else. It does not order the sidebar, prioritise
    /// sync, or decide a reply's from address, which
    /// [`postio_model::reply`] settles from the message being replied to.
    ///
    /// Both statements are in one transaction, and a caller naming a row that
    /// is gone rolls the whole thing back rather than leaving the store with
    /// no default at all: "clear every marker" landing without "set this one"
    /// is the failure this shape exists to prevent.
    ///
    /// The same shape as [`IdentityRepository::set_default`] one level down.
    /// There is deliberately no `clear_default`: the reversal of marking an
    /// account is marking another, which is why the command carries no undo.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] if no account has that id.
    pub async fn set_default(&self, id: AccountId) -> Result<()> {
        sql::in_scope(self.connection, |transaction| async move {
            transaction
                .execute("UPDATE accounts SET is_default = 0", ())
                .await?;
            let changed = transaction
                .execute(
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

    /// Marks the account for removal without deleting anything yet (#464,
    /// ADR 0005 Q6a).
    ///
    /// Reversible with [`AccountRepository::restore`] until
    /// [`AccountRepository::reap_pending_deletions`] actually runs, which is
    /// what gives the undo toast something to undo.
    pub async fn mark_pending_deletion(&self, id: AccountId) -> Result<bool> {
        let changed = self
            .connection
            .execute(
                "UPDATE accounts SET pending_deletion = 1 WHERE id = ?1",
                [id.get()],
            )
            .await?;
        Ok(changed > 0)
    }

    /// Undoes [`AccountRepository::mark_pending_deletion`].
    pub async fn restore(&self, id: AccountId) -> Result<bool> {
        let changed = self
            .connection
            .execute(
                "UPDATE accounts SET pending_deletion = 0 WHERE id = ?1",
                [id.get()],
            )
            .await?;
        Ok(changed > 0)
    }

    /// Permanently deletes every account still marked pending, cascading to
    /// everything that hangs off it (#464, ADR 0005 Q6a).
    ///
    /// Called once, at the next startup, before any engine is created — never
    /// live, so a session that crashes before an undo toast expires leaves
    /// the row exactly as `mark_pending_deletion` left it, not half deleted.
    /// Returns which accounts were actually reaped.
    pub async fn reap_pending_deletions(&self) -> Result<Vec<AccountId>> {
        let ids: Vec<AccountId> = sql::all(
            self.connection,
            "SELECT id FROM accounts WHERE pending_deletion = 1",
            (),
            |row| Ok(AccountId::new(row.col(0)?)),
        )
        .await?;

        for id in &ids {
            self.connection
                .execute("DELETE FROM accounts WHERE id = ?1", [id.get()])
                .await?;
        }
        Ok(ids)
    }
}

/// Reads and writes individual [`Identity`] rows.
#[derive(Debug)]
pub struct IdentityRepository<'a> {
    connection: &'a Connection,
}

const IDENTITY_COLUMNS: &str = "\
id, account_id, display_name, address, address_name, reply_to_address, reply_to_name,
signature_text, signature_html, is_default";

impl<'a> IdentityRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Inserts an identity at the end of its account's list, assigning its id.
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

    /// Writes an identity back, leaving its position alone.
    pub async fn update(&self, identity: &Identity) -> Result<()> {
        let id = require_persisted(identity.id.get(), "identity")?;
        let changed = self
            .connection
            .execute(
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

    /// One identity.
    pub async fn get(&self, id: IdentityId) -> Result<Option<Identity>> {
        sql::first(
            self.connection,
            &format!("SELECT {IDENTITY_COLUMNS} FROM identities WHERE id = ?1"),
            [id.get()],
            read_identity,
        )
        .await
    }

    /// An account's identities, in the order the picker shows them.
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

    /// Makes one identity the account's default, clearing any other.
    pub async fn set_default(&self, account_id: AccountId, id: IdentityId) -> Result<()> {
        sql::in_scope(self.connection, |transaction| async move {
            transaction
                .execute(
                    "UPDATE identities SET is_default = 0 WHERE account_id = ?1",
                    [account_id.get()],
                )
                .await?;
            let changed = transaction
                .execute(
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

    /// Deletes an identity, returning whether there was one.
    pub async fn delete(&self, id: IdentityId) -> Result<bool> {
        let deleted = self
            .connection
            .execute("DELETE FROM identities WHERE id = ?1", [id.get()])
            .await?;
        Ok(deleted > 0)
    }
}

async fn insert_identity(
    connection: &Connection,
    identity: &Identity,
    position: usize,
) -> Result<i64> {
    connection
        .execute(
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
    connection
        .execute(
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

/// Reads and writes the account's named [`Signature`] set (#12).
///
/// Separate from [`IdentityRepository`] because a signature is no longer a
/// property of one identity: it belongs to the account, and which one a
/// message signs with is a decision the composer makes per draft.
#[derive(Debug)]
pub struct SignatureRepository<'a> {
    connection: &'a Connection,
}

const SIGNATURE_COLUMNS: &str = "id, name, text, html";

impl<'a> SignatureRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Inserts a signature at the end of `account_id`'s list, assigning its id.
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
        self.connection
            .execute(
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

    /// Writes a signature back, leaving its position alone.
    pub async fn update(&self, signature: &Signature) -> Result<()> {
        let id = require_persisted(signature.id.get(), "signature")?;
        let changed = self
            .connection
            .execute(
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

    /// Deletes a signature, returning whether there was one.
    pub async fn delete(&self, id: SignatureId) -> Result<bool> {
        let deleted = self
            .connection
            .execute("DELETE FROM signatures WHERE id = ?1", [id.get()])
            .await?;
        Ok(deleted > 0)
    }

    /// One account's signatures, in picker order.
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
        // A negative ceiling is not a small one, it is a corrupt row --
        // reading it as "no limit" is the reading that cannot refuse mail
        // the provider would have taken.
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
            // A jmap row that lost its session URL cannot dial anything;
            // falling back to IMAP keeps the account working rather than
            // dead — the incoming server is stored either way.
            ("jmap", Some(session_url)) => postio_model::account::Backend::Jmap { session_url },
            ("gmail", _) => postio_model::account::Backend::Gmail,
            // A maildir row that lost its root has nothing to read: unlike
            // the jmap case there is no incoming server to fall back to, so
            // it stays a maildir and fails at connect, where the message
            // says which directory is missing.
            ("maildir", root) => postio_model::account::Backend::Maildir {
                root: root.unwrap_or_default(),
            },
            _ => postio_model::account::Backend::Imap,
        },
    })
}

/// The one place a backend's location goes into the row.
///
/// A JMAP session URL and a maildir root are the same fact — where this
/// account lives — so they share one column (#1278). It is only meaningful
/// beside `backend`, the column that names the protocol family, which is why
/// the read side matches on the pair rather than on the location alone.
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
                    // The identity's own signature is not a row in the named
                    // set — it is what this identity signs with unless the
                    // draft says otherwise (migration 0009).
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

/// `?1, ?2, ...` for `count` parameters, offset by one for the leading id.
fn placeholders(count: usize) -> String {
    (0..count)
        .map(|index| format!("?{}", index + 2))
        .collect::<Vec<_>>()
        .join(", ")
}
