//! The conversation rail, for a frontend that draws it (#1576, #1595).
//!
//! Every rule is `postio_ui::reader::rail`'s: the ladder of presentations by
//! window width, the rows, and the one state machine that a row being chosen,
//! `J`/`K`, and the observer reporting what is on screen all move the mark
//! through -- "so keyboard nav and scroll-derived marking can never
//! disagree". This crosses them rather than restating any.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use postio_ui::reader::rail::{Effect, Presentation, Rail, Settle};

/// How the rail is drawn at a window width. See [`rail_presentation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RailPresentationFfi {
    /// A column of numbers and senders.
    Full,
    /// A narrower column of numbers and initials.
    Narrow,
    /// No column: a counter in the header opens the same index.
    Popover,
}

/// Which presentation a window of `width` gets for a conversation of
/// `messages`, or `None` for no rail at all -- one message has nothing to
/// index, and `hidden` is the reader's own `⇧I` (FR-047).
///
/// A window width, not a pane's: the ladder's numbers are window widths.
#[uniffi::export]
pub fn rail_presentation(width: i32, messages: u32, hidden: bool) -> Option<RailPresentationFfi> {
    postio_ui::reader::rail::presentation(width, messages as usize, hidden).map(|step| match step {
        Presentation::Full => RailPresentationFfi::Full,
        Presentation::Narrow => RailPresentationFfi::Narrow,
        Presentation::Popover => RailPresentationFfi::Popover,
    })
}

/// What one rail row says.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RailRowFfi {
    /// One-based, as a person counts.
    pub position: u32,
    /// Who wrote it.
    pub sender: String,
    /// Their initials, for the narrow column.
    pub initials: String,
    /// When it arrived -- for the row's accessible name (FR-046), which says
    /// what the terse visible row leaves out.
    pub when: String,
    /// Its length in lines, when it is long enough to be worth saying.
    pub length: Option<u32>,
}

impl From<postio_ui::reader::rail::Row> for RailRowFfi {
    fn from(row: postio_ui::reader::rail::Row) -> Self {
        Self {
            position: row.position as u32,
            sender: row.sender,
            initials: row.initials,
            when: row.when,
            length: row.length,
        }
    }
}

/// What the frontend must do after asking the rail to move its mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct RailEffectFfi {
    /// Whether the mark moved -- repaint the rail only when it did.
    pub mark_moved: bool,
    /// When the pane must be scrolled to the new mark, the scroll's token:
    /// hand it back to [`RailFfi::settled`] when the scroll finishes, and
    /// the observer is listened to again.
    pub scroll: Option<u64>,
}

/// One window's rail: which message is marked, and whether the observer is
/// being listened to.
///
/// `postio_ui::reader::rail::Rail`, held behind the boundary. Its settle
/// tokens are opaque on purpose -- a late one for a superseded scroll must
/// not end a newer scroll's suppression -- so they cross as numbers this
/// object maps back.
#[derive(uniffi::Object)]
pub struct RailFfi {
    state: Mutex<RailState>,
}

struct RailState {
    rail: Rail,
    scrolls: HashMap<u64, Settle>,
    next: u64,
}

impl RailState {
    fn effect(&mut self, effect: Effect) -> RailEffectFfi {
        match effect {
            Effect::Nothing => RailEffectFfi {
                mark_moved: false,
                scroll: None,
            },
            Effect::Mark => RailEffectFfi {
                mark_moved: true,
                scroll: None,
            },
            Effect::MarkAndScroll(settle) => {
                self.next += 1;
                self.scrolls.insert(self.next, settle);
                RailEffectFfi {
                    mark_moved: true,
                    scroll: Some(self.next),
                }
            }
        }
    }
}

#[uniffi::export]
impl RailFfi {
    /// A rail for a conversation of `count` messages, nothing marked yet.
    #[uniffi::constructor]
    pub fn new(count: u32) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(RailState {
                rail: Rail::new(count as usize),
                scrolls: HashMap::new(),
                next: 0,
            }),
        })
    }

    /// A new conversation is being shown: nothing marked, nothing in flight.
    pub fn set_conversation(&self, count: u32) {
        let mut state = self.state.lock().expect("rail lock");
        state.rail.set_conversation(count as usize);
        state.scrolls.clear();
    }

    /// A row was chosen. The pane follows.
    pub fn activate(&self, index: u32) -> RailEffectFfi {
        let mut state = self.state.lock().expect("rail lock");
        let effect = state.rail.activate(index as usize);
        state.effect(effect)
    }

    /// `J`: the next message, stopping at the last.
    pub fn next_message(&self) -> RailEffectFfi {
        let mut state = self.state.lock().expect("rail lock");
        let effect = state.rail.next_message();
        state.effect(effect)
    }

    /// `K`: the previous message, stopping at the first.
    pub fn previous_message(&self) -> RailEffectFfi {
        let mut state = self.state.lock().expect("rail lock");
        let effect = state.rail.previous_message();
        state.effect(effect)
    }

    /// The observer reported what is on screen. The pane does not follow.
    pub fn observed(&self, index: Option<u32>) -> RailEffectFfi {
        let mut state = self.state.lock().expect("rail lock");
        let effect = state.rail.observed(index.map(|index| index as usize));
        state.effect(effect)
    }

    /// The scroll named by `scroll` finished. A token for a scroll that has
    /// since been superseded changes nothing.
    pub fn settled(&self, scroll: u64) {
        let mut state = self.state.lock().expect("rail lock");
        if let Some(settle) = state.scrolls.remove(&scroll) {
            state.rail.settled(settle);
        }
    }

    /// Which message is marked.
    pub fn marked(&self) -> Option<u32> {
        let state = self.state.lock().expect("rail lock");
        state.rail.marked().map(|index| index as u32)
    }
}
