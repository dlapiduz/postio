//! What a list has sent that "delete what the list sent" could reach (#1931): one read shared by
//! every frontend. It only finds ids; archiving or deleting is an ordinary command.

use postio_model::ids::MessageId;
use postio_storage::Connection;
use postio_storage::repository::{ListMail, MessageRepository, SweepRepository};
use postio_ui::unsubscribe::{SweepScope, sweep_scope};

pub const PAGE: u32 = 5_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub ids: Vec<MessageId>,
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Gone,
    NoList,
    Store(String),
}

impl Refusal {
    pub fn said(&self) -> String {
        match self {
            Self::Gone => "That message is gone.".to_owned(),
            Self::NoList => "This message names no list to clear.".to_owned(),
            Self::Store(error) => error.clone(),
        }
    }
}

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
