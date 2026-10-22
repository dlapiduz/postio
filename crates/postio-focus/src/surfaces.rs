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
    /// The known-sender popover, anchored to a sender's name.
    ContactPopover,
    /// The Contacts view, in the list's place.
    Contacts,
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
            SurfaceKind::ContactPopover => KeyContext::ContactPopover,
            SurfaceKind::Contacts => KeyContext::Contacts,
            // Their keys are the list's context, and only the ones that
            // close them are the window's (GTK's dialog close rule).
            SurfaceKind::Settings
            | SurfaceKind::KeyMap
            | SurfaceKind::Dialog
            | SurfaceKind::RowMenu => KeyContext::List,
        }
    }

    /// Whether Back closes it from the controller. The composer's closes it
    /// keeping the draft (slice 7); a menu's is its own. The bar's
    /// closes it, as GTK's window did, and a picker's, as GTK's picker did.
    fn back_closes(self) -> bool {
        matches!(
            self,
            SurfaceKind::Bar
                | SurfaceKind::Composer
                | SurfaceKind::Picker
                | SurfaceKind::Digest
                | SurfaceKind::Capture
                | SurfaceKind::Settings
                | SurfaceKind::KeyMap
                | SurfaceKind::Dialog
                | SurfaceKind::Filtered
                | SurfaceKind::ContactPopover
                | SurfaceKind::Contacts
        )
    }
}

/// Who runs a command while the surface on top is up, for a frontend that
/// keeps the widgets of some surfaces itself (GTK, until it adopts the
/// composer, the bar and the digest and capture windows). The Mac holds
/// those in the controller and asks [`crate::FocusController::answers`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Route {
    /// The controller's: say [`crate::Input::Command`] and draw what comes
    /// back.
    Controller,
    /// The surface's own widgets run it; the frontend knows how.
    Surface(SurfaceKind),
    /// Nobody's rule: the widget under the keyboard takes the key, and the
    /// list behind does not.
    Toolkit,
    /// The list's table, as with nothing over it.
    List,
}

/// Where a message is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Host {
    /// Its own surface: the message window on the Mac, the reading dialog
    /// or the pane beside the list on Linux.
    Own,
    /// The digest's window, in place of the digest's page: a reference's
    /// email on both platforms, and on the Mac a message from the digest's
    /// list too (M4).
    Digest,
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

/// Which list the open message came from: what `j`/`k` in it walk, and
/// what a verb on it reads its conversation from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Origin {
    /// The list, or the bar's hits.
    #[default]
    List,
    /// Filtered.
    Filtered,
    /// A digest's plain list, read over it (Linux stacks).
    Digest,
}

/// A removal under the open message, waiting for the list to let it go.
#[derive(Debug, Clone, Copy)]
struct Stepping {
    /// Where its row was.
    index: u32,
    /// The message removed.
    message: MessageId,
    /// The timer that gives up on it.
    token: u64,
}

/// How long a removed row is waited for before the step gives up: a failed
/// archive leaves the row, and the message stays open on it. What the step
/// waits for is the list letting the row go; this is only the end of the
/// wait, set once, never a poll.
pub(crate) const STEP_GIVE_UP: std::time::Duration = std::time::Duration::from_secs(5);

/// The bit that marks a timer as the open message's, so its tokens never
/// meet the composer's autosave, which counts up from one.
const READER_TIMER: u64 = 1 << 63;

/// Whether `token` is a timer the open message set.
pub(crate) fn is_reader_timer(token: u64) -> bool {
    token & READER_TIMER != 0
}

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
    /// Step-past timers handed out so far: a token is never reused.
    timers: u64,
    /// Which list the open message came from.
    origin: Origin,
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
                (id == CommandId::Back || id == CommandId::PrevView || reader_verb(id).is_some())
                    .then_some(true)
            }
            // They take the keyboard: every key but Quit is theirs, and
            // only their close keys do anything.
            SurfaceKind::KeyMap | SurfaceKind::Dialog => Some(crate::keys::key_map_takes(id)),
            kind if id == CommandId::Back => Some(kind.back_closes()),
            // The composer, the bar, a picker and a menu take their own keys.
            SurfaceKind::Composer
            | SurfaceKind::Bar
            | SurfaceKind::Picker
            | SurfaceKind::RowMenu => Some(false),
            _ => None,
        }
    }

    /// Who runs `id` with this stack: the table over the surfaces.
    pub(crate) fn route(&self, id: CommandId) -> Route {
        let Some(top) = self.top() else {
            return Route::List;
        };
        match top {
            SurfaceKind::Message => {
                if matches!(id, CommandId::Back | CommandId::PrevView) || reader_verb(id).is_some()
                {
                    Route::Controller
                } else {
                    Route::List
                }
            }
            SurfaceKind::KeyMap | SurfaceKind::Dialog => {
                if matches!(id, CommandId::Back | CommandId::CheatSheet) {
                    Route::Controller
                } else {
                    Route::Toolkit
                }
            }
            SurfaceKind::Settings => {
                if matches!(id, CommandId::Back | CommandId::Settings) {
                    Route::Controller
                } else {
                    Route::Surface(top)
                }
            }
            SurfaceKind::ContactPopover | SurfaceKind::Contacts => Route::Controller,
            SurfaceKind::Digest
            | SurfaceKind::Capture
            | SurfaceKind::Composer
            | SurfaceKind::Picker
            | SurfaceKind::RowMenu => Route::Surface(top),
            SurfaceKind::Bar | SurfaceKind::Filtered => Route::List,
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
            self.origin = Origin::List;
        }
        true
    }

    /// The open message came from `origin`'s list.
    pub(crate) fn set_origin(&mut self, origin: Origin) {
        self.origin = origin;
    }

    /// Which list the open message came from.
    pub(crate) fn origin(&self) -> Origin {
        self.origin
    }

    /// Whether the surface on top takes the keyboard whole: the key map,
    /// and a dialog.
    pub(crate) fn takes_keyboard(&self) -> bool {
        matches!(self.top(), Some(SurfaceKind::KeyMap | SurfaceKind::Dialog))
    }

    /// Back, or the key map's own key, on a surface that closes on it.
    pub(crate) fn close_top(&mut self, id: CommandId) -> Option<Vec<Step>> {
        let top = self.top()?;
        let closes = match top {
            SurfaceKind::KeyMap | SurfaceKind::Dialog => {
                matches!(id, CommandId::Back | CommandId::CheatSheet)
            }
            SurfaceKind::Message => false,
            // Settings toggles on its own key, as it opens on it.
            SurfaceKind::Settings => matches!(id, CommandId::Back | CommandId::Settings),
            kind => id == CommandId::Back && kind.back_closes(),
        };
        if !closes {
            return None;
        }
        // The key map and a dialog come off here, as the bar does: the
        // controller closed them, so the Mac says nothing back (an echo
        // could land after a reopening and close the new one). Left on the
        // stack to wait for that echo, they kept the keyboard, and every
        // key after `?` `?` or Escape was swallowed.
        if matches!(top, SurfaceKind::KeyMap | SurfaceKind::Dialog) {
            self.remove(top);
            let mut steps = vec![Step::Show(Intent::CloseSurface(top))];
            if self.stack.is_empty() {
                steps.push(Step::Show(Intent::KeyboardHome));
            }
            return Some(steps);
        }
        // The frontend closes it and says so; the stack follows then.
        Some(vec![Step::Show(Intent::CloseSurface(top))])
    }

    /// Open `row`, at `index` of `total`, to be read. (A draft not yet on
    /// its way is written instead: `FocusController::write`.)
    pub(crate) fn open(
        &mut self,
        row: &RowFacts,
        index: u32,
        total: u32,
        stacking: bool,
    ) -> Vec<Step> {
        let mut steps = self.opened(SurfaceKind::Message, stacking);
        self.reading = Some(row.id);
        self.origin = Origin::List;
        // Another message shown by anyone -- a key pressed while an archive
        // settled -- is where the reader is now: a step past the old one
        // does not drag it back.
        self.stepping = None;
        steps.push(Step::Show(Intent::OpenMessage {
            message: row.id,
            index,
            total,
            host: Host::Own,
        }));
        steps
    }

    /// The open message's own key, or `None` when it is not one of its.
    pub(crate) fn reader_key(&mut self, id: CommandId) -> Option<Vec<Step>> {
        // Gmail's `u` goes back to the list from wherever the message is:
        // it closes the message, with no rung for More or find first.
        if id == CommandId::PrevView {
            self.remove(SurfaceKind::Message);
            let mut steps = vec![Step::Show(Intent::CloseSurface(SurfaceKind::Message))];
            if self.stack.is_empty() {
                steps.push(Step::Show(Intent::KeyboardHome));
            }
            return Some(steps);
        }
        if id == CommandId::Back {
            let step = if self.more_open {
                Step::Show(Intent::Reader(ReaderVerb::CloseMore))
            } else if self.finding {
                Step::Show(Intent::Reader(ReaderVerb::CloseFind))
            } else {
                self.remove(SurfaceKind::Message);
                let mut steps = vec![Step::Show(Intent::CloseSurface(SurfaceKind::Message))];
                // Over Filtered or a digest, the keyboard goes back to it.
                if self.stack.is_empty() {
                    steps.push(Step::Show(Intent::KeyboardHome));
                }
                return Some(steps);
            };
            return Some(vec![step]);
        }
        let verb = reader_verb(id)?;
        Some(vec![Step::Show(Intent::Reader(verb))])
    }

    /// The open message's row is being removed: once the list lets it go,
    /// show what took its place. The timer it returns gives up on the wait
    /// ([`Self::timed_out`]).
    pub(crate) fn step_past(&mut self, index: u32, message: MessageId) -> Step {
        self.timers += 1;
        let token = READER_TIMER | self.timers;
        self.stepping = Some(Stepping {
            index,
            message,
            token,
        });
        Step::Timer {
            token,
            after: STEP_GIVE_UP,
        }
    }

    /// A timer of the open message's ran out: the step past it was waiting
    /// on, when it is still the one waiting, gives up and the message stays
    /// as it is.
    pub(crate) fn timed_out(&mut self, token: u64) {
        if self
            .stepping
            .as_ref()
            .is_some_and(|step| step.token == token)
        {
            self.stepping = None;
        }
    }

    /// The list landed. Where a removed message's row has gone, the row now
    /// in its place, or the one above when it was the last, is the one to
    /// open; with nothing left, the message closes. What the cursor does is
    /// the caller's: it is handed the position.
    pub(crate) fn landed(&mut self, rows: &dyn Rows) -> Option<Landing> {
        let stepping = self.stepping.take()?;
        // The row is still there: the archive has not reached the list, and
        // the next landing is waited for -- until the timer gives up.
        if rows.position_of(stepping.message).is_some() {
            self.stepping = Some(stepping);
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
