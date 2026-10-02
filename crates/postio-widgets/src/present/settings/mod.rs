//! The settings window, joined to the store's host: the presenters both
//! desktop apps feed [`crate::settings::SettingsPanel`] with (ADR 0043;
//! specs/007-postio-focus T233).
//!
//! The panel draws the sections and calls back through its `connect_*`
//! handlers without knowing anything persists them. These are the other
//! half: every read and write is the host's, asked through `postio-client`
//! (ADR 0041), so the window behaves the same in the classic app and in
//! Focus.
//!
//! - [`accounts`]: the account rows, their detail view and signatures, the
//!   connection test, rebuilding an index, removing with undo.
//! - [`credential`]: re-entering an account's credential.
//! - [`egress`] and [`privacy`]: the Privacy section's logs.
//! - [`backfill`]: skipping or resuming a folder's backfill (ADR 0016).
//!
//! # What stays with the app
//!
//! A few things are not the host's to answer and not this crate's to reach:
//! the connection test and the token-expiry line dial out or read the
//! keyring from the app's own process, a removal's undo is the app's toast,
//! and a role mapping is a command the app runs. The app answers them as an
//! [`Outside`].

use std::cell::RefCell;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::time::SystemTime;

use postio_model::Account;
use postio_model::ids::AccountId;

pub mod accounts;
pub mod backfill;
pub mod credential;
pub mod egress;
pub mod privacy;

/// Accounts whose local search index is being rebuilt right now (#981).
///
/// Shared between [`accounts`], which adds and removes membership as a
/// rebuild runs, and whatever in the app reads it -- the classic app's
/// search, which raises a corpus caveat while an account's index is
/// mid-rebuild.
pub type Reindexing = Rc<RefCell<HashSet<AccountId>>>;

/// An answer the app gives later, on the main loop.
pub type Later<T> = Pin<Box<dyn Future<Output = T>>>;

/// What a connection test found: the incoming server's answer, then the
/// outgoing one's, each `Err` with the reason it refused.
pub type Reached = (Result<(), String>, Result<(), String>);

/// What the settings window asks of the app it is drawn in.
pub trait Outside {
    /// Try `account`'s stored settings against its servers (#980).
    ///
    /// The one thing in the window that leaves the machine, and only because
    /// somebody pressed "Test connection".
    fn test_connection(&self, account: Account) -> Later<Reached>;

    /// When each of `accounts` -- an id and its address -- has its stored
    /// sign-in token expire, for the accounts that signed in through
    /// Postio's own OAuth client (#878).
    fn token_expiries(
        &self,
        accounts: Vec<(AccountId, String)>,
    ) -> Later<Vec<(AccountId, Option<SystemTime>)>>;

    /// Re-enter `account`'s credential, over the app's window; `saved` runs
    /// once the new one is written.
    fn update_credential(&self, account: AccountId, saved: Box<dyn Fn()>);

    /// Offer to undo something just done: `description` on the app's toast,
    /// whose button runs `undo`.
    fn offer_undo(&self, description: &str, undo: Box<dyn Fn()>);

    /// Run `command` as the app runs any other: a mailbox role mapping is
    /// one (ADR 0035).
    fn run(&self, command: postio_core::Command);

    /// Whether `[sync] attachment_fetch` is eager, which the account rows'
    /// weights say.
    fn attachments_eager(&self) -> bool;
}
