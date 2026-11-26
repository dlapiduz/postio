//! The terminal as a driver of `postio_focus::FocusController` (ADR 0045).

use postio_config::paths::Platform;
use postio_focus::{Capabilities, Policy, RowFacts, Rows};
use postio_model::{MessageId, ThreadId};
use postio_ui::focus_list::FocusRow;
use postio_ui::list::{ListRow, ListWindow, RowKey};

use crate::row::Row;

pub fn policy() -> Policy {
    Policy {
        platform: Platform::host(),
        caps: Capabilities {
            reading_pane: true,
            stacking: false,
            contacts_view: false,
            calendar: true,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub row: Row,
    pub source: FocusRow,
}

impl Held {
    pub fn of(source: FocusRow) -> Held {
        Held {
            row: Row::from(source.clone()),
            source,
        }
    }
}

impl std::ops::Deref for Held {
    type Target = Row;

    fn deref(&self) -> &Row {
        &self.row
    }
}

impl ListRow for Held {
    fn id(&self) -> Option<MessageId> {
        Some(self.row.id)
    }

    fn thread(&self) -> Option<ThreadId> {
        self.row.thread()
    }

    // A fired reminder is not its conversation's row.
    fn key(&self) -> Option<RowKey> {
        Some(self.source.key())
    }
}

pub struct RowsView<'a>(pub &'a ListWindow<Held>);

impl Rows for RowsView<'_> {
    fn len(&self) -> u32 {
        self.0.total()
    }

    fn facts(&self, position: u32) -> Option<RowFacts> {
        self.0
            .resident_at(position)
            .map(|held| RowFacts::of(&held.source))
    }

    fn position_of(&self, message: MessageId) -> Option<u32> {
        self.0.position_of(message)
    }

    fn position_of_thread(&self, thread: ThreadId) -> Option<u32> {
        self.0.position_of_key(RowKey::Thread(thread))
    }

    fn said(&self, position: u32) -> Option<(String, String)> {
        self.0
            .resident_at(position)
            .filter(|held| held.row.kind != crate::row::Kind::Digest)
            .map(|held| (held.row.from.to_string(), held.row.subject.to_string()))
    }

    fn row(&self, position: u32) -> Option<FocusRow> {
        self.0.resident_at(position).map(|held| held.source.clone())
    }
}
