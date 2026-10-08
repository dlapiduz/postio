//! The surfaces over the list, and the open message's keys (research R2,
//! slices 5 and 6; contract invariants 5-7).
//!
//! Moved from `postio-gtk`'s window: `key_context`, the dialog close rule,
//! and `reading_key`. Each frontend says what it opened and closed
//! ([`SurfaceKind`]); the controller keeps the stack, answers which key
//! context is in force, and routes a command by the surface on top -- the
//! open message's own verbs to it, Back to whatever is on top, and the rest
//! on to the list's table (#1754). With `stacking` off (the Mac, M4), one
//! secondary window replaces another.

use postio_core::CommandId;
use postio_model::MessageId;
use postio_ui::keymap::KeyContext;

use crate::Intent;
use crate::cursor::{RowFacts, Rows};
use crate::feed::Step;

/// Something a frontend shows over the list, which takes the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SurfaceKind {
    /// One message, in a dialog, a window of its own, or beside the list.
    Message,
    /// A digest's window.
    Digest,
    /// The composer.
    Composer,
    /// The capture sheet.
    Capture,
    /// Settings.
    Settings,
    /// The key map.
    KeyMap,
    /// Any other dialog: a rule, raw source, the open chooser.
    Dialog,
    /// The command bar.
    Bar,
    /// The Filtered view.
    Filtered,
    /// A picker anchored to a row.
    Picker,
    /// A row's menu.
    RowMenu,
}

impl SurfaceKind {
    /// Whether it is a secondary window on the Mac: one at a time (M4).
    fn is_window(self) -> bool {
        matches!(
            self,
            SurfaceKind::Message
                | SurfaceKind::Digest
                | SurfaceKind::Composer
                | SurfaceKind::Capture
        )
    }

    /// The key context it puts in force.
    fn context(self) -> KeyContext {
        match self {
            SurfaceKind::Message => KeyContext::Reader,
            SurfaceKind::Digest => KeyContext::Digest,
            SurfaceKind::Composer => KeyContext::Composer,
            SurfaceKind::Capture => KeyContext::Capture,
            SurfaceKind::Bar => KeyContext::Search,
            SurfaceKind::Filtered => KeyContext::Filtered,
            SurfaceKind::Picker => KeyContext::Picker,
            // Their keys are the list's context, and only the ones that
            // close them are the window's (GTK's dialog close rule).
            SurfaceKind::Settings
            | SurfaceKind::KeyMap
            | SurfaceKind::Dialog
            | SurfaceKind::RowMenu => KeyContext::List,
        }
    }

    /// Whether Back closes it from the controller. The composer's Back is
    /// its own (it keeps the draft), and a menu's is its own. The bar's
    /// closes it, as GTK's window did, and a picker's, as GTK's picker did.
    fn back_closes(self) -> bool {
        matches!(
            self,
            SurfaceKind::Bar
                | SurfaceKind::Picker
                | SurfaceKind::Digest
                | SurfaceKind::Capture
                | SurfaceKind::Settings
                | SurfaceKind::KeyMap
                | SurfaceKind::Dialog
                | SurfaceKind::Filtered
        )
    }
}

/// What the open message does for a key, as its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReaderVerb {
    /// Close the More menu; the message stays open.
    CloseMore,
    /// Close find; the message stays open.
    CloseFind,
    /// Find in the message.
    FindInMessage,
    /// The next match.
    FindNext,
    /// The previous match.
    FindPrevious,
    /// App colours or the original, for this message.
    SwitchTreatment,
    /// The message's raw source, in place of its content (M4).
    ViewSource,
    /// The More menu.
    ShowMore,
    /// Step through the conversation, by this many messages.
    StepThread(i32),
}

/// A removal under the open message, waiting for the list to let it go.
#[derive(Debug, Clone, Copy)]
struct Stepping {
    /// Where its row was.
    index: u32,
    /// The message removed.
    message: MessageId,
    /// Landings waited so far.
    waited: u8,
}

/// Landings to wait for a removed row to leave before giving up: a failed
/// archive leaves the row, and the message stays open on it.
const STEP_PATIENCE: u8 = 4;

/// The surfaces open over the list, bottom first.
#[derive(Debug, Default)]
pub(crate) struct Surfaces {
    stack: Vec<SurfaceKind>,
    /// The open message's More menu is up.
    more_open: bool,
    /// The open message's find is up.
    finding: bool,
    /// The message on screen, when one is.
    reading: Option<MessageId>,
    stepping: Option<Stepping>,
}

impl Surfaces {
    pub(crate) fn top(&self) -> Option<SurfaceKind> {
        self.stack.last().copied()
    }

    /// Whether `kind` is open, anywhere in the stack.
    pub(crate) fn has(&self, kind: SurfaceKind) -> bool {
        self.stack.contains(&kind)
    }

    /// Take `kind` off the stack now, as the controller closes it; whether
    /// it was there. The frontend's `SurfaceClosed` for it is then nothing.
    pub(crate) fn dismiss(&mut self, kind: SurfaceKind) -> bool {
        self.remove(kind)
    }

    pub(crate) fn key_context(&self) -> KeyContext {
        self.top().map_or(KeyContext::List, SurfaceKind::context)
    }

    /// The message on screen, when the open message is on top.
    pub(crate) fn reading(&self) -> Option<MessageId> {
        (self.top() == Some(SurfaceKind::Message))
            .then_some(self.reading)
            .flatten()
    }

    /// Whether `id` is the top surface's to answer, `None` when the surface
    /// has no say and the list's rules apply.
    pub(crate) fn answers(&self, id: CommandId) -> Option<bool> {
        let top = self.top()?;
        match top {
            SurfaceKind::Message => {
                (id == CommandId::Back || reader_verb(id).is_some()).then_some(true)
            }
            SurfaceKind::KeyMap | SurfaceKind::Dialog
                if matches!(id, CommandId::Back | CommandId::CheatSheet) =>
            {
                Some(true)
            }
            kind if id == CommandId::Back => Some(kind.back_closes()),
            // The composer, the bar, a picker and a menu take their own keys.
            SurfaceKind::Composer
            | SurfaceKind::Bar
            | SurfaceKind::Picker
            | SurfaceKind::RowMenu => Some(false),
            _ => None,
        }
    }

    /// A surface opened. With `stacking` off, a secondary window replaces
    /// the one open.
    pub(crate) fn opened(&mut self, kind: SurfaceKind, stacking: bool) -> Vec<Step> {
        let mut steps = Vec::new();
        if !stacking && kind.is_window() {
            let others: Vec<SurfaceKind> = self
                .stack
                .iter()
                .copied()
                .filter(|open| open.is_window() && *open != kind)
                .collect();
            for other in others {
                self.remove(other);
                steps.push(Step::Show(Intent::CloseSurface(other)));
            }
        }
        if self.top() != Some(kind) {
            self.stack.push(kind);
        }
        steps
    }

    /// A surface closed, however it was: the keyboard goes home once
    /// nothing is left over the list.
    pub(crate) fn closed(&mut self, kind: SurfaceKind) -> Vec<Step> {
        if !self.remove(kind) {
            return Vec::new();
        }
        if self.stack.is_empty() {
            vec![Step::Show(Intent::KeyboardHome)]
        } else {
            Vec::new()
        }
    }

    /// The open message's More menu and find, as the frontend has them.
    pub(crate) fn reader_state(&mut self, more_open: bool, finding: bool) {
        self.more_open = more_open;
        self.finding = finding;
    }

    fn remove(&mut self, kind: SurfaceKind) -> bool {
        let Some(at) = self.stack.iter().rposition(|open| *open == kind) else {
            return false;
        };
        self.stack.remove(at);
        if kind == SurfaceKind::Message {
            self.more_open = false;
            self.finding = false;
            self.reading = None;
            self.stepping = None;
        }
        true
    }

    /// Back, or the key map's own key, on a surface that closes on it.
    pub(crate) fn close_top(&mut self, id: CommandId) -> Option<Vec<Step>> {
        let top = self.top()?;
        let closes = match top {
            SurfaceKind::KeyMap | SurfaceKind::Dialog => {
                matches!(id, CommandId::Back | CommandId::CheatSheet)
            }
            SurfaceKind::Message => false,
            kind => id == CommandId::Back && kind.back_closes(),
        };
        if !closes {
            return None;
        }
        // The frontend closes it and says so; the stack follows then.
        Some(vec![Step::Show(Intent::CloseSurface(top))])
    }

    /// Open `row`, at `index` of `total`: to be read, or -- a draft not yet
    /// on its way -- to be written.
    pub(crate) fn open(
        &mut self,
        row: &RowFacts,
        index: u32,
        total: u32,
        stacking: bool,
    ) -> Vec<Step> {
        if row.writes {
            let mut steps = Vec::new();
            if self.remove(SurfaceKind::Message) {
                steps.push(Step::Show(Intent::CloseSurface(SurfaceKind::Message)));
            }
            steps.push(Step::Show(Intent::OpenDraft { message: row.id }));
            return steps;
        }
        let mut steps = self.opened(SurfaceKind::Message, stacking);
        self.reading = Some(row.id);
        steps.push(Step::Show(Intent::OpenMessage {
            message: row.id,
            index,
            total,
        }));
        steps
    }

    /// The open message's own key, or `None` when it is not one of its.
    pub(crate) fn reader_key(&mut self, id: CommandId) -> Option<Vec<Step>> {
        if id == CommandId::Back {
            let step = if self.more_open {
                Step::Show(Intent::Reader(ReaderVerb::CloseMore))
            } else if self.finding {
                Step::Show(Intent::Reader(ReaderVerb::CloseFind))
            } else {
                self.remove(SurfaceKind::Message);
                return Some(vec![
                    Step::Show(Intent::CloseSurface(SurfaceKind::Message)),
                    Step::Show(Intent::KeyboardHome),
                ]);
            };
            return Some(vec![step]);
        }
        let verb = reader_verb(id)?;
        Some(vec![Step::Show(Intent::Reader(verb))])
    }

    /// The open message's row is being removed: once the list lets it go,
    /// show what took its place.
    pub(crate) fn step_past(&mut self, index: u32, message: MessageId) {
        self.stepping = Some(Stepping {
            index,
            message,
            waited: 0,
        });
    }

    /// The list landed. Where a removed message's row has gone, the row now
    /// in its place, or the one above when it was the last, is the one to
    /// open; with nothing left, the message closes. What the cursor does is
    /// the caller's: it is handed the position.
    pub(crate) fn landed(&mut self, rows: &dyn Rows) -> Option<Landing> {
        let mut stepping = self.stepping.take()?;
        if rows.position_of(stepping.message).is_some() {
            stepping.waited += 1;
            if stepping.waited < STEP_PATIENCE {
                self.stepping = Some(stepping);
            }
            return None;
        }
        if rows.is_empty() {
            self.remove(SurfaceKind::Message);
            return Some(Landing::Close);
        }
        let index = stepping.index.min(rows.len() - 1);
        Some(Landing::Open(index))
    }
}

/// What a landing does to the open message.
pub(crate) enum Landing {
    /// Show the row at this position.
    Open(u32),
    /// Nothing is left to read.
    Close,
}

/// The open message's own verb for `id`.
fn reader_verb(id: CommandId) -> Option<ReaderVerb> {
    Some(match id {
        CommandId::FindInMessage => ReaderVerb::FindInMessage,
        CommandId::FindNext => ReaderVerb::FindNext,
        CommandId::FindPrevious => ReaderVerb::FindPrevious,
        CommandId::SwitchTreatment => ReaderVerb::SwitchTreatment,
        CommandId::ViewSource => ReaderVerb::ViewSource,
        CommandId::MoreActions => ReaderVerb::ShowMore,
        CommandId::NextInConversation => ReaderVerb::StepThread(1),
        CommandId::PrevInConversation => ReaderVerb::StepThread(-1),
        _ => return None,
    })
}
