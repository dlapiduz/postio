//! The surfaces over the list, and the open message's keys. Each frontend says what it opened and
//! closed; the controller keeps the stack and routes a command by the surface on top (#1754). With
//! `stacking` off (the Mac, M4), one secondary window replaces another.

use postio_core::CommandId;
use postio_model::MessageId;
use postio_ui::keymap::KeyContext;

use crate::Intent;
use crate::cursor::{RowFacts, Rows};
use crate::feed::Step;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SurfaceKind {
    Message,
    Digest,
    Composer,
    Capture,
    Settings,
    KeyMap,
    Dialog,
    Bar,
    Filtered,
    ContactPopover,
    Contacts,
    Calendar,
    Picker,
    RowMenu,
    ThreadMenu,
    Outline,
    Source,
}

impl SurfaceKind {
    fn is_window(self) -> bool {
        matches!(
            self,
            SurfaceKind::Message
                | SurfaceKind::Digest
                | SurfaceKind::Composer
                | SurfaceKind::Capture
        )
    }

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
            SurfaceKind::Calendar => KeyContext::Calendar,
            SurfaceKind::ThreadMenu => KeyContext::Reader,
            SurfaceKind::Outline => KeyContext::Outline,
            SurfaceKind::Source => KeyContext::Source,
            // Their keys are the list's context, and only the ones that
            // close them are the window's (GTK's dialog close rule).
            SurfaceKind::Settings
            | SurfaceKind::KeyMap
            | SurfaceKind::Dialog
            | SurfaceKind::RowMenu => KeyContext::List,
        }
    }

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
                | SurfaceKind::Calendar
                | SurfaceKind::ThreadMenu
                | SurfaceKind::Outline
                | SurfaceKind::Source
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Route {
    Controller,
    Surface(SurfaceKind),
    /// Nobody's rule: the widget under the keyboard takes the key, and the list behind does not.
    Toolkit,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Host {
    Own,
    Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReaderVerb {
    CloseMore,
    CloseFind,
    FindInMessage,
    FindNext,
    FindPrevious,
    /// When there is none, say `Input::MatchEdge`.
    NextMatch,
    /// When there is none, say `Input::MatchEdge`.
    PrevMatch,
    SwitchTreatment,
    ShowMore,
    /// Said only when the controller holds no walk.
    StepThread(i32),
    FirstInThread,
    LatestInThread,
    FirstUnread,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Origin {
    #[default]
    List,
    Filtered,
    Digest,
}

#[derive(Debug, Clone, Copy)]
struct Stepping {
    index: u32,
    message: MessageId,
    token: u64,
}

/// A failed archive leaves the row, and the message stays open on it. What the step waits for is
/// the list letting the row go; this is only the end of the wait, set once, never a poll.
pub(crate) const STEP_GIVE_UP: std::time::Duration = std::time::Duration::from_secs(5);

/// The bit that marks a timer as the open message's, so its tokens never
/// meet the composer's autosave, which counts up from one.
const READER_TIMER: u64 = 1 << 63;

pub(crate) fn is_reader_timer(token: u64) -> bool {
    token & READER_TIMER != 0
}

#[derive(Debug, Default)]
pub(crate) struct Surfaces {
    stack: Vec<SurfaceKind>,
    more_open: bool,
    finding: bool,
    reading: Option<MessageId>,
    stepping: Option<Stepping>,
    timers: u64,
    origin: Origin,
}

impl Surfaces {
    pub(crate) fn top(&self) -> Option<SurfaceKind> {
        self.stack.last().copied()
    }

    pub(crate) fn has(&self, kind: SurfaceKind) -> bool {
        self.stack.contains(&kind)
    }

    /// The frontend's `SurfaceClosed` for it is then nothing.
    pub(crate) fn dismiss(&mut self, kind: SurfaceKind) -> bool {
        self.remove(kind)
    }

    pub(crate) fn key_context(&self) -> KeyContext {
        self.top().map_or(KeyContext::List, SurfaceKind::context)
    }

    pub(crate) fn show(&mut self, message: MessageId) {
        self.reading = Some(message);
        self.stepping = None;
    }

    pub(crate) fn reading(&self) -> Option<MessageId> {
        (self.top() == Some(SurfaceKind::Message))
            .then_some(self.reading)
            .flatten()
    }

    pub(crate) fn shown(&self) -> Option<MessageId> {
        self.has(SurfaceKind::Message)
            .then_some(self.reading)
            .flatten()
    }

    pub(crate) fn answers(&self, id: CommandId) -> Option<bool> {
        let top = self.top()?;
        match top {
            SurfaceKind::Message => (id == CommandId::Back
                || id == CommandId::PrevView
                || id == CommandId::ViewSource
                || reader_verb(id).is_some())
            .then_some(true),
            // They take the keyboard: every key but Quit is theirs, and
            // only their close keys do anything.
            SurfaceKind::KeyMap | SurfaceKind::Dialog => Some(crate::keys::key_map_takes(id)),
            kind if id == CommandId::Back => Some(kind.back_closes()),
            SurfaceKind::Composer
            | SurfaceKind::Bar
            | SurfaceKind::Picker
            | SurfaceKind::RowMenu => Some(false),
            _ => None,
        }
    }

    pub(crate) fn route(&self, id: CommandId) -> Route {
        let Some(top) = self.top() else {
            return Route::List;
        };
        match top {
            SurfaceKind::Message => {
                if matches!(
                    id,
                    CommandId::Back | CommandId::PrevView | CommandId::ViewSource
                ) || reader_verb(id).is_some()
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
            SurfaceKind::ContactPopover
            | SurfaceKind::Contacts
            | SurfaceKind::Calendar
            | SurfaceKind::ThreadMenu
            | SurfaceKind::Outline => Route::Controller,
            SurfaceKind::Digest
            | SurfaceKind::Capture
            | SurfaceKind::Composer
            | SurfaceKind::Picker
            | SurfaceKind::RowMenu => Route::Surface(top),
            SurfaceKind::Bar | SurfaceKind::Filtered => Route::List,
            SurfaceKind::Source if crate::source::Source::owns(id) => Route::Controller,
            SurfaceKind::Source => Route::Toolkit,
        }
    }

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

    pub(crate) fn set_origin(&mut self, origin: Origin) {
        self.origin = origin;
    }

    pub(crate) fn origin(&self) -> Origin {
        self.origin
    }

    pub(crate) fn takes_keyboard(&self) -> bool {
        matches!(self.top(), Some(SurfaceKind::KeyMap | SurfaceKind::Dialog))
    }

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
        Some(vec![Step::Show(Intent::CloseSurface(top))])
    }

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

    pub(crate) fn timed_out(&mut self, token: u64) {
        if self
            .stepping
            .as_ref()
            .is_some_and(|step| step.token == token)
        {
            self.stepping = None;
        }
    }

    pub(crate) fn landed(&mut self, rows: &dyn Rows) -> Option<Landing> {
        let stepping = self.stepping.take()?;
        // The row is still there: the archive has not reached the list, and
        // the next landing is waited for -- until the timer gives up.
        if rows.position_of(stepping.message).is_some() {
            self.stepping = Some(stepping);
            return None;
        }
        let unread_or_message = |at: &u32| rows.facts(*at).is_none_or(|row| !row.digest);
        let from = stepping.index.min(rows.len().saturating_sub(1));
        let next = (from..rows.len())
            .find(unread_or_message)
            .or_else(|| (0..from).rev().find(unread_or_message));
        match next {
            Some(index) => Some(Landing::Open(index)),
            None => {
                self.remove(SurfaceKind::Message);
                Some(Landing::Close)
            }
        }
    }
}

pub(crate) enum Landing {
    Open(u32),
    Close,
}

fn reader_verb(id: CommandId) -> Option<ReaderVerb> {
    Some(match id {
        CommandId::FindInMessage => ReaderVerb::FindInMessage,
        CommandId::FindNext => ReaderVerb::FindNext,
        CommandId::FindPrevious => ReaderVerb::FindPrevious,
        CommandId::SwitchTreatment => ReaderVerb::SwitchTreatment,
        CommandId::MoreActions => ReaderVerb::ShowMore,
        CommandId::NextInConversation => ReaderVerb::StepThread(1),
        CommandId::PrevInConversation => ReaderVerb::StepThread(-1),
        CommandId::FirstInConversation => ReaderVerb::FirstInThread,
        CommandId::LatestInConversation => ReaderVerb::LatestInThread,
        CommandId::FirstUnreadInConversation => ReaderVerb::FirstUnread,
        _ => return None,
    })
}
