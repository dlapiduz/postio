//! What a list has sent that "delete what the list sent" could reach (#1931).
//!
//! One read, shared by every frontend's route to it (the host's request and
//! the Mac boundary), so the scope, the page and the exclusions cannot be
//! derived two ways. It only finds ids; archiving or deleting them is an
//! ordinary command, queued and undoable like any other.

use postio_model::ids::MessageId;
use postio_storage::Connection;
use postio_storage::repository::{ListMail, MessageRepository, SweepRepository};
use postio_ui::unsubscribe::{SweepScope, sweep_scope};

/// A page: enough for any real list, bounded for a hostile one.
pub const PAGE: u32 = 5_000;

/// The mail a list left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The `List-Id`, or the sender's address when the message named no list.
    pub name: String,
    /// The messages, newest first.
    pub ids: Vec<MessageId>,
    /// More matched than the page held, so `ids.len()` is a floor.
    pub more: bool,
}

/// Why nothing was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The message is not in the store.
    Gone,
    /// It names neither a list nor a sender.
    NoList,
    /// The store would not answer.
    Store(String),
}

impl Refusal {
    /// The sentence a person is shown.
    pub fn said(&self) -> String {
        match self {
            Self::Gone => "That message is gone.".to_owned(),
            Self::NoList => "This message names no list to clear.".to_owned(),
            Self::Store(error) => error.clone(),
        }
    }
}

/// What the list `message` came from has left in its account.
pub async fn find(connection: &Connection, message: MessageId) -> Result<Found, Refusal> {
    let found = MessageRepository::new(connection)
        .get(message)
        .await
        .map_err(|error| Refusal::Store(error.to_string()))?
        .ok_or(Refusal::Gone)?;
    let scope = sweep_scope(found.list_id.as_deref(), &found.from).ok_or(Refusal::NoList)?;
    let from = match &scope {
        SweepScope::ListId(list) => ListMail::ListId(list),
        SweepScope::Sender(address) => ListMail::Sender(address),
    };
    let page = SweepRepository::new(connection)
        .list_mail(found.account_id, from, PAGE)
        .await
        .map_err(|error| Refusal::Store(error.to_string()))?;
    Ok(Found {
        name: scope.name().to_owned(),
        ids: page.ids,
        more: page.more,
    })
}
