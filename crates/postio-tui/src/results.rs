//! The search results over the controller's `ResultsView`: what it last said and the scroll.

use postio_focus::{NoResultsView, QueryView, ResultsView};

#[derive(Debug)]
pub struct Results {
    pub view: ResultsView,
    pub query: Option<QueryView>,
    pub none: Option<NoResultsView>,
    pub cursor: u64,
    pub top: u64,
}

impl Results {
    pub fn new(view: ResultsView) -> Results {
        Results {
            cursor: view.cursor.unwrap_or(0),
            view,
            query: None,
            none: None,
            top: 0,
        }
    }

    pub fn reveal(&mut self, room: u64) {
        let room = room.max(1);
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + room {
            self.top = self.cursor + 1 - room;
        }
    }
}
