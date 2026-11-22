//! The settings window joined to the store's host through `postio-client` (ADR 0041, ADR 0043): the
//! presenters both desktop
//! apps feed [`crate::settings::SettingsPanel`] with. The app answers some things itself, as an
//! [`Outside`]: the connection
//! test and token-expiry line (they dial out or read the keyring in its process), a removal's undo
//! toast, role-mapping commands.

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

/// Shared between [`accounts`], which adds and removes membership as a rebuild runs, and whatever
/// reads it, such as a search
/// raising a corpus caveat (#981).
pub type Reindexing = Rc<RefCell<HashSet<AccountId>>>;

pub type Later<T> = Pin<Box<dyn Future<Output = T>>>;

#[derive(Debug, Clone)]
pub struct Reached {
    pub incoming: Result<(), String>,
    pub outgoing: Result<(), String>,
    pub trouble: Option<postio_ui::account::Trouble>,
}

pub trait Outside {
    /// The one thing in the window that leaves the machine, and only because somebody pressed "Test
    /// connection" (#980).
    fn test_connection(&self, account: Account) -> Later<Reached>;

    fn token_expiries(
        &self,
        accounts: Vec<(AccountId, String)>,
    ) -> Later<Vec<(AccountId, Option<SystemTime>)>>;

    fn update_credential(&self, account: AccountId, saved: Box<dyn Fn()>);

    fn offer_undo(&self, description: &str, undo: Box<dyn Fn()>);

    fn run(&self, command: postio_core::Command);

    fn attachments_eager(&self) -> bool;
}
