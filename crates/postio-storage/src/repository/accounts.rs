//! Accounts and the identities that send from them.

use postio_model::{
    Account, AccountId, AuthMethod, EmailAddress, Identity, IdentityId, ServerConfig, Signature,
    SignatureId, TransportSecurity,
};
use rusqlite::{Connection, Row, params};

use super::{from_millis, require_persisted, to_millis, unknown_enum};
use crate::error::{Error, Result};

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
jmap_session_url";

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
    pub fn create(&self, account: &mut Account) -> Result<AccountId> {
        let transaction = super::Scope::open(self.connection)?;

        transaction.execute(
            "INSERT INTO accounts (display_name, address, address_name, incoming_host,
                                   incoming_port, incoming_security, incoming_username,
                                   outgoing_host, outgoing_port, outgoing_security,
                                   outgoing_username, auth_method, enabled, created_at,
                                   default_signature_id, oauth_client_id, oauth_token_url,
                                   oauth_authorize_url, oauth_scopes, backend,
                                   jmap_session_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                     ?16, ?17, ?18, ?19, ?20, ?21)",
            params![
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
                match &account.backend {
                    postio_model::account::Backend::Jmap { session_url } =>
                        Some(session_url.as_str()),
                    postio_model::account::Backend::Imap
                    | postio_model::account::Backend::Gmail => None,
                },
            ],
        )?;

        let id = AccountId::new(transaction.last_insert_rowid());
        account.id = id;
        for (position, identity) in account.identities.iter_mut().enumerate() {
            identity.account_id = id;
            identity.id = IdentityId::new(insert_identity(&transaction, identity, position)?);
        }

        transaction.commit()?;
        Ok(id)
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
    pub fn update(&self, account: &mut Account) -> Result<()> {
        let id = require_persisted(account.id.get(), "account")?;
        let account_id = account.id;
        let transaction = super::Scope::open(self.connection)?;

        let changed = transaction.execute(
            "UPDATE accounts
                SET display_name = ?2, address = ?3, address_name = ?4,
                    incoming_host = ?5, incoming_port = ?6, incoming_security = ?7,
                    incoming_username = ?8, outgoing_host = ?9, outgoing_port = ?10,
                    outgoing_security = ?11, outgoing_username = ?12, auth_method = ?13,
                    enabled = ?14, created_at = ?15, default_signature_id = ?16,
                    oauth_client_id = ?17, oauth_token_url = ?18,
                    oauth_authorize_url = ?19, oauth_scopes = ?20, backend = ?21,
                    jmap_session_url = ?22
              WHERE id = ?1",
            params![
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
                match &account.backend {
                    postio_model::account::Backend::Jmap { session_url } =>
                        Some(session_url.as_str()),
                    postio_model::account::Backend::Imap
                    | postio_model::account::Backend::Gmail => None,
                },
            ],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "account",
                id,
            });
        }

        // Clear the default first: the schema allows only one per account, and
        // moving it between two identities would otherwise collide mid-update.
        transaction.execute(
            "UPDATE identities SET is_default = 0 WHERE account_id = ?1",
            [id],
        )?;

        let mut kept: Vec<i64> = Vec::with_capacity(account.identities.len());
        for (position, identity) in account.identities.iter_mut().enumerate() {
            identity.account_id = account_id;
            if identity.id.is_assigned() {
                update_identity(&transaction, identity, position)?;
            } else {
                identity.id = IdentityId::new(insert_identity(&transaction, identity, position)?);
            }
            kept.push(identity.id.get());
        }

        let placeholders = placeholders(kept.len());
        let mut statement = transaction.prepare(&format!(
            "DELETE FROM identities WHERE account_id = ?1 AND id NOT IN ({placeholders})"
        ))?;
        let mut arguments: Vec<i64> = Vec::with_capacity(kept.len() + 1);
        arguments.push(id);
        arguments.extend(kept);
        statement.execute(rusqlite::params_from_iter(arguments))?;
        drop(statement);

        transaction.commit()?;
        Ok(())
    }

    /// One account, with its identities.
    pub fn get(&self, id: AccountId) -> Result<Option<Account>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?1"
        ))?;
        let mut rows = statement.query([id.get()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let mut account = read_account(row)?;
        drop(rows);
        drop(statement);
        account.identities = IdentityRepository::new(self.connection).list_for_account(id)?;
        account.signatures = SignatureRepository::new(self.connection).list_for_account(id)?;
        Ok(Some(account))
    }

    /// Every account, in creation order, with their identities.
    pub fn list(&self) -> Result<Vec<Account>> {
        self.list_where("")
    }

    /// Every account that participates in sync.
    ///
    /// Excludes anything marked for removal even before
    /// [`AccountRepository::reap_pending_deletions`] has actually run --
    /// belt and braces, since the reap is meant to run first regardless, but
    /// an engine should never start against a row on its way out.
    pub fn list_enabled(&self) -> Result<Vec<Account>> {
        self.list_where("WHERE enabled = 1 AND pending_deletion = 0")
    }

    fn list_where(&self, filter: &str) -> Result<Vec<Account>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {ACCOUNT_COLUMNS} FROM accounts {filter} ORDER BY id"
        ))?;
        let rows = statement.query_map([], read_account)?;
        let mut accounts: Vec<Account> = rows.collect::<Result<_, _>>()?;
        drop(statement);

        let identities = IdentityRepository::new(self.connection);
        let signatures = SignatureRepository::new(self.connection);
        for account in &mut accounts {
            account.identities = identities.list_for_account(account.id)?;
            account.signatures = signatures.list_for_account(account.id)?;
        }
        Ok(accounts)
    }

    /// Deletes an account and everything that hangs off it, returning whether
    /// there was one.
    ///
    /// Mailboxes, messages, threads, labels, drafts and queued operations all
    /// cascade in the schema; the blob store is swept separately by
    /// [`BlobStore::collect_garbage`](crate::blob::BlobStore::collect_garbage).
    pub fn delete(&self, id: AccountId) -> Result<bool> {
        let deleted = self
            .connection
            .execute("DELETE FROM accounts WHERE id = ?1", [id.get()])?;
        Ok(deleted > 0)
    }

    /// Flips whether the account participates in sync (#464, ADR 0005 Q6).
    ///
    /// A single-column write rather than [`AccountRepository::update`]: the
    /// caller here is a settings-panel toggle, not code holding a full,
    /// freshly-loaded `Account` with its identity list intact, and routing
    /// through `update` would risk silently rewriting identities from a
    /// stale copy.
    pub fn set_enabled(&self, id: AccountId, enabled: bool) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE accounts SET enabled = ?2 WHERE id = ?1",
            params![id.get(), enabled],
        )?;
        Ok(changed > 0)
    }

    /// Marks the account for removal without deleting anything yet (#464,
    /// ADR 0005 Q6a).
    ///
    /// Reversible with [`AccountRepository::restore`] until
    /// [`AccountRepository::reap_pending_deletions`] actually runs, which is
    /// what gives the undo toast something to undo.
    pub fn mark_pending_deletion(&self, id: AccountId) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE accounts SET pending_deletion = 1 WHERE id = ?1",
            [id.get()],
        )?;
        Ok(changed > 0)
    }

    /// Undoes [`AccountRepository::mark_pending_deletion`].
    pub fn restore(&self, id: AccountId) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE accounts SET pending_deletion = 0 WHERE id = ?1",
            [id.get()],
        )?;
        Ok(changed > 0)
    }

    /// Permanently deletes every account still marked pending, cascading to
    /// everything that hangs off it (#464, ADR 0005 Q6a).
    ///
    /// Called once, at the next startup, before any engine is created — never
    /// live, so a session that crashes before an undo toast expires leaves
    /// the row exactly as `mark_pending_deletion` left it, not half deleted.
    /// Returns which accounts were actually reaped.
    pub fn reap_pending_deletions(&self) -> Result<Vec<AccountId>> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM accounts WHERE pending_deletion = 1")?;
        let ids: Vec<AccountId> = statement
            .query_map([], |row| row.get::<_, i64>(0))?
            .map(|id| id.map(AccountId::new))
            .collect::<rusqlite::Result<_>>()?;
        drop(statement);

        for id in &ids {
            self.connection
                .execute("DELETE FROM accounts WHERE id = ?1", [id.get()])?;
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
    pub fn create(&self, identity: &mut Identity) -> Result<IdentityId> {
        let account_id = require_persisted(identity.account_id.get(), "account")?;
        let position: i64 = self.connection.query_row(
            "SELECT coalesce(max(position) + 1, 0) FROM identities WHERE account_id = ?1",
            [account_id],
            |row| row.get(0),
        )?;
        let id = insert_identity(self.connection, identity, position as usize)?;
        identity.id = IdentityId::new(id);
        Ok(identity.id)
    }

    /// Writes an identity back, leaving its position alone.
    pub fn update(&self, identity: &Identity) -> Result<()> {
        let id = require_persisted(identity.id.get(), "identity")?;
        let changed = self.connection.execute(
            "UPDATE identities
                SET display_name = ?2, address = ?3, address_name = ?4,
                    reply_to_address = ?5, reply_to_name = ?6,
                    signature_text = ?7, signature_html = ?8, is_default = ?9
              WHERE id = ?1",
            params![
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
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "identity",
                id,
            });
        }
        Ok(())
    }

    /// One identity.
    pub fn get(&self, id: IdentityId) -> Result<Option<Identity>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {IDENTITY_COLUMNS} FROM identities WHERE id = ?1"
        ))?;
        let mut rows = statement.query([id.get()])?;
        Ok(rows.next()?.map(read_identity).transpose()?)
    }

    /// An account's identities, in the order the picker shows them.
    pub fn list_for_account(&self, account_id: AccountId) -> Result<Vec<Identity>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {IDENTITY_COLUMNS} FROM identities WHERE account_id = ?1 ORDER BY position, id"
        ))?;
        let rows = statement.query_map([account_id.get()], read_identity)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Makes one identity the account's default, clearing any other.
    pub fn set_default(&self, account_id: AccountId, id: IdentityId) -> Result<()> {
        let transaction = super::Scope::open(self.connection)?;
        transaction.execute(
            "UPDATE identities SET is_default = 0 WHERE account_id = ?1",
            [account_id.get()],
        )?;
        let changed = transaction.execute(
            "UPDATE identities SET is_default = 1 WHERE id = ?1 AND account_id = ?2",
            [id.get(), account_id.get()],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "identity",
                id: id.get(),
            });
        }
        transaction.commit()?;
        Ok(())
    }

    /// Deletes an identity, returning whether there was one.
    pub fn delete(&self, id: IdentityId) -> Result<bool> {
        let deleted = self
            .connection
            .execute("DELETE FROM identities WHERE id = ?1", [id.get()])?;
        Ok(deleted > 0)
    }
}

fn insert_identity(connection: &Connection, identity: &Identity, position: usize) -> Result<i64> {
    connection.execute(
        "INSERT INTO identities (account_id, display_name, address, address_name,
                                 reply_to_address, reply_to_name, signature_text,
                                 signature_html, is_default, position)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
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
    )?;
    Ok(connection.last_insert_rowid())
}

fn update_identity(connection: &Connection, identity: &Identity, position: usize) -> Result<()> {
    connection.execute(
        "UPDATE identities
            SET account_id = ?2, display_name = ?3, address = ?4, address_name = ?5,
                reply_to_address = ?6, reply_to_name = ?7, signature_text = ?8,
                signature_html = ?9, is_default = ?10, position = ?11
          WHERE id = ?1",
        params![
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
    )?;
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
    pub fn create(&self, account_id: AccountId, signature: &mut Signature) -> Result<SignatureId> {
        let account = require_persisted(account_id.get(), "account")?;
        let position: i64 = self.connection.query_row(
            "SELECT coalesce(max(position) + 1, 0) FROM signatures WHERE account_id = ?1",
            [account],
            |row| row.get(0),
        )?;
        self.connection.execute(
            "INSERT INTO signatures (account_id, name, text, html, position)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                account,
                signature.name,
                signature.text,
                signature.html,
                position
            ],
        )?;
        signature.id = SignatureId::new(self.connection.last_insert_rowid());
        Ok(signature.id)
    }

    /// Writes a signature back, leaving its position alone.
    pub fn update(&self, signature: &Signature) -> Result<()> {
        let id = require_persisted(signature.id.get(), "signature")?;
        let changed = self.connection.execute(
            "UPDATE signatures SET name = ?2, text = ?3, html = ?4 WHERE id = ?1",
            params![id, signature.name, signature.text, signature.html],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "signature",
                id,
            });
        }
        Ok(())
    }

    /// Deletes a signature, returning whether there was one.
    pub fn delete(&self, id: SignatureId) -> Result<bool> {
        let deleted = self
            .connection
            .execute("DELETE FROM signatures WHERE id = ?1", [id.get()])?;
        Ok(deleted > 0)
    }

    /// One account's signatures, in picker order.
    pub fn list_for_account(&self, account_id: AccountId) -> Result<Vec<Signature>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {SIGNATURE_COLUMNS} FROM signatures
              WHERE account_id = ?1 ORDER BY position, id"
        ))?;
        let rows = statement.query_map([account_id.get()], |row| {
            Ok(Signature {
                id: SignatureId::new(row.get(0)?),
                name: row.get(1)?,
                text: row.get(2)?,
                html: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

fn read_account(row: &Row<'_>) -> rusqlite::Result<Account> {
    let incoming_security: String = row.get(6)?;
    let outgoing_security: String = row.get(10)?;
    let auth: String = row.get(12)?;

    Ok(Account {
        id: AccountId::new(row.get(0)?),
        display_name: row.get(1)?,
        address: EmailAddress::new(row.get::<_, Option<String>>(3)?, row.get::<_, String>(2)?),
        incoming: ServerConfig {
            host: row.get(4)?,
            port: row.get(5)?,
            security: parse_security(&incoming_security, "accounts.incoming_security")?,
            username: row.get(7)?,
        },
        outgoing: ServerConfig {
            host: row.get(8)?,
            port: row.get(9)?,
            security: parse_security(&outgoing_security, "accounts.outgoing_security")?,
            username: row.get(11)?,
        },
        auth: AuthMethod::from_name(&auth)
            .ok_or_else(|| to_sqlite(unknown_enum("accounts.auth_method", auth)))?,
        enabled: row.get(13)?,
        identities: Vec::new(),
        signatures: Vec::new(),
        default_signature_id: row.get::<_, Option<i64>>(15)?.map(SignatureId::new),
        created_at: from_millis(row.get(14)?),
        pending_deletion: row.get(16)?,
        oauth: match (
            row.get::<_, Option<String>>(17)?,
            row.get::<_, Option<String>>(18)?,
        ) {
            (Some(client_id), Some(token_url)) => Some(postio_model::account::OAuthConfig {
                client_id,
                token_url,
                authorize_url: row.get::<_, Option<String>>(19)?.unwrap_or_default(),
                scopes: row.get::<_, Option<String>>(20)?.unwrap_or_default(),
            }),
            _ => None,
        },
        backend: match (
            row.get::<_, String>(21)?.as_str(),
            row.get::<_, Option<String>>(22)?,
        ) {
            // A jmap row that lost its session URL cannot dial anything;
            // falling back to IMAP keeps the account working rather than
            // dead — the incoming server is stored either way.
            ("jmap", Some(session_url)) => postio_model::account::Backend::Jmap { session_url },
            ("gmail", _) => postio_model::account::Backend::Gmail,
            _ => postio_model::account::Backend::Imap,
        },
    })
}

fn read_identity(row: &Row<'_>) -> rusqlite::Result<Identity> {
    let reply_to_address: Option<String> = row.get(5)?;
    let signature_text: Option<String> = row.get(7)?;

    Ok(Identity {
        id: IdentityId::new(row.get(0)?),
        account_id: AccountId::new(row.get(1)?),
        display_name: row.get(2)?,
        address: EmailAddress::new(row.get::<_, Option<String>>(4)?, row.get::<_, String>(3)?),
        reply_to: reply_to_address
            .map(|address| {
                Ok::<_, rusqlite::Error>(EmailAddress::new(
                    row.get::<_, Option<String>>(6)?,
                    address,
                ))
            })
            .transpose()?,
        signature: signature_text
            .map(|text| {
                Ok::<_, rusqlite::Error>(Signature {
                    // The identity's own signature is not a row in the named
                    // set — it is what this identity signs with unless the
                    // draft says otherwise (migration 0009).
                    id: SignatureId::UNASSIGNED,
                    name: String::new(),
                    text,
                    html: row.get(8)?,
                })
            })
            .transpose()?,
        is_default: row.get(9)?,
    })
}

fn parse_security(value: &str, column: &'static str) -> rusqlite::Result<TransportSecurity> {
    TransportSecurity::from_name(value).ok_or_else(|| to_sqlite(unknown_enum(column, value)))
}

/// Wraps one of our errors so it can travel back out of a rusqlite row mapper.
///
/// `query_map` insists on `rusqlite::Error`; `FromSqlConversionFailure` is the
/// variant meant for "this column held something I cannot turn into the type
/// asked for", which is exactly the case.
fn to_sqlite(error: Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
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
