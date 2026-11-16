//! Thread ▾'s menu and the outline, over the message window (spec 013 US5,
//! FR-019), and the thread verbs they and the keys share: mute, split, and
//! the two "this message only" verbs.
//!
//! The controller decides what each entry does and draws nothing: the menu
//! and the outline are `postio_ui::thread_outline`'s views, handed to the
//! frontend whole each time they change.

use chrono::Local;
use postio_core::{Command, CommandId, MessageTarget};
use postio_model::{ThreadId, ThreadView};
use postio_ui::thread_outline::{self as words, OutlineRow};

use crate::cursor::Rows;
use crate::feed::Step;
use crate::{FocusController, Intent, Request, SurfaceKind, ToastKind};

/// What a frontend says of the outline.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutlineInput {
    /// The filter field holds this text now.
    Typed(String),
    /// A person chip was chosen, or cleared with `None`.
    Person(Option<String>),
    /// Up or down: by this many rows.
    Move(i32),
    /// Return, or a click on a row: show it. `None` is the cursor's.
    Show(Option<usize>),
    /// `/`: the keyboard goes to the field.
    FocusFilter,
}

/// The outline while it is up.
#[derive(Debug)]
struct Outline {
    /// The thread, as read when it opened.
    view: ThreadView,
    /// Every message of it.
    all: Vec<OutlineRow>,
    filter: String,
    person: Option<String>,
    /// Among the rows the filter leaves.
    cursor: usize,
    /// The list row it was opened from, when not from the window.
    from_row: Option<u32>,
}

/// What the controller keeps of the menu and outline.
#[derive(Debug, Default)]
pub(crate) struct ThreadUi {
    outline: Option<Outline>,
}

/// The commands the menu's entries run.
fn is_entry(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::ThreadOutline
            | CommandId::Forward
            | CommandId::ForwardWhat2
            | CommandId::ReplyToLatest
            | CommandId::MuteThread
            | CommandId::SplitThread
            | CommandId::MoveThisMessage
            | CommandId::DeleteThisMessage
            | CommandId::SwitchTreatment
    )
}

impl FocusController {
    pub(crate) fn forget_thread_surfaces(&mut self) {
        if !self.surfaces.has(SurfaceKind::Outline) {
            self.thread_ui.outline = None;
        }
    }

    /// Whether `id` is the menu's or the outline's own while it is on top.
    pub(crate) fn thread_surface_answers(id: CommandId) -> bool {
        is_entry(id)
            || matches!(
                id,
                CommandId::Back
                    | CommandId::ThreadMenu
                    | CommandId::OutlineFirstUnread
                    | CommandId::FirstUnreadInConversation
                    | CommandId::NextMessage
                    | CommandId::PrevMessage
                    | CommandId::OpenMessage
                    | CommandId::Search
            )
    }

    /// A command while the menu or the outline is on top. Each holds the
    /// keyboard: what is not its own does nothing, but Undo and the key map.
    pub(crate) fn thread_surface_command(
        &mut self,
        id: CommandId,
        rows: &dyn Rows,
    ) -> Option<Vec<Step>> {
        if matches!(id, CommandId::Undo | CommandId::CheatSheet) {
            return None;
        }
        match self.surfaces.top()? {
            SurfaceKind::ThreadMenu => match id {
                CommandId::Back | CommandId::ThreadMenu => {
                    Some(self.close_thread_surface(SurfaceKind::ThreadMenu))
                }
                id if is_entry(id) => Some(self.thread_menu_pick(id, rows)),
                _ => Some(Vec::new()),
            },
            SurfaceKind::Outline => Some(match id {
                CommandId::Back | CommandId::ThreadOutline => {
                    self.close_thread_surface(SurfaceKind::Outline)
                }
                CommandId::OutlineFirstUnread | CommandId::FirstUnreadInConversation => {
                    self.outline_input_first_unread()
                }
                CommandId::NextMessage => self.outline_input(OutlineInput::Move(1), rows),
                CommandId::PrevMessage => self.outline_input(OutlineInput::Move(-1), rows),
                CommandId::OpenMessage => self.outline_input(OutlineInput::Show(None), rows),
                CommandId::Search => self.outline_input(OutlineInput::FocusFilter, rows),
                _ => Vec::new(),
            }),
            _ => None,
        }
    }

    fn close_thread_surface(&mut self, kind: SurfaceKind) -> Vec<Step> {
        self.surfaces.dismiss(kind);
        self.thread_ui.outline = None;
        let mut steps = vec![Step::Show(Intent::CloseSurface(kind))];
        if !self.has_surface() {
            steps.push(Step::Show(Intent::KeyboardHome));
        }
        steps
    }

    /// Where the message on screen is among its thread's members.
    fn shown_at(&self) -> Option<usize> {
        let walk = self.walk.as_ref()?;
        walk.view
            .members
            .iter()
            .position(|member| member.id == walk.shown)
    }

    /// A thread command with the message window on top, or `None` when it
    /// is not one of these or the window holds no walk to act on.
    pub(crate) fn thread_window_command(
        &mut self,
        id: CommandId,
        rows: &dyn Rows,
    ) -> Option<Vec<Step>> {
        match id {
            CommandId::ThreadMenu => {
                let at = self.shown_at()?;
                let walk = self.walk.as_ref()?;
                let menu = words::menu(self.keymap(), walk.view.muted.is_some(), at > 0);
                let mut steps = self
                    .surfaces
                    .opened(SurfaceKind::ThreadMenu, self.policy.caps.stacking);
                steps.push(Step::Show(Intent::ThreadMenu(Box::new(menu))));
                Some(steps)
            }
            CommandId::ThreadOutline => {
                let at = self.shown_at()?;
                let view = self.walk.as_ref()?.view.clone();
                Some(self.open_outline(view, at, None))
            }
            CommandId::MuteThread => {
                let walk = self.walk.as_mut()?;
                let muted = walk.view.muted.is_none();
                walk.view.muted = muted.then_some(0);
                let thread = walk.view.thread;
                Some(mute_steps(thread, muted))
            }
            CommandId::SplitThread => {
                let at = self.shown_at()?;
                let shown = self.walk.as_ref()?.shown;
                if at == 0 {
                    return Some(vec![Step::Show(Intent::Toast {
                        text: "The first message starts the thread, so it cannot be split off"
                            .to_owned(),
                        kind: ToastKind::Notice,
                    })]);
                }
                Some(vec![Step::Ask(Request::Post(Command::SplitThread {
                    message: Some(shown),
                    rejoin: None,
                }))])
            }
            // This message alone, whatever the thread is (spec 013 D2).
            CommandId::MoveThisMessage => {
                let shown = self.surfaces.reading()?;
                Some(self.picker_on_shown(shown))
            }
            CommandId::DeleteThisMessage => {
                let shown = self.surfaces.reading()?;
                let mut steps = vec![Step::Ask(Request::Send {
                    command: Command::default_for(CommandId::Delete),
                    aims: vec![MessageTarget::Messages(vec![shown])],
                    everything: None,
                })];
                // The window leaves the message at once (spec 013, edge
                // cases); a window with no walk has no thread to stay in.
                steps.extend(self.walk_deleted_shown());
                Some(steps)
            }
            _ => {
                let _ = rows;
                None
            }
        }
    }

    /// `m` or `alt+t` on the list's cursor row.
    pub(crate) fn thread_list_command(&mut self, id: CommandId, rows: &dyn Rows) -> Vec<Step> {
        let Some(position) = self.cursor.position() else {
            return Vec::new();
        };
        match id {
            CommandId::MuteThread => {
                let Some(thread) = rows
                    .facts(position)
                    .and_then(|f| f.threads.first().copied())
                else {
                    return Vec::new();
                };
                let muted_now = match rows.row(position) {
                    Some(row) => row
                        .as_conversation()
                        .is_some_and(|row| row.summary.muted.is_some()),
                    None => self
                        .expansion
                        .as_ref()
                        .is_some_and(|expansion| expansion.view.muted.is_some()),
                };
                mute_steps(thread, !muted_now)
            }
            CommandId::ThreadOutline => {
                // The outline of a thread already read: the one expanded
                // under the cursor.
                let Some(expansion) = self.expansion.as_ref() else {
                    return Vec::new();
                };
                let (view, anchor) = (expansion.view.clone(), expansion.anchor);
                let Some(row) = rows.position_of(anchor) else {
                    return Vec::new();
                };
                self.open_outline(view, 0, Some(row))
            }
            _ => Vec::new(),
        }
    }

    /// A click on the bar's "... N more" chip: the outline, with the ring
    /// on the first member the chip stands for.
    pub(crate) fn more_chip_clicked(&mut self, from: u32) -> Vec<Step> {
        let (Some(at), Some(walk)) = (self.shown_at(), self.walk.as_ref()) else {
            return Vec::new();
        };
        let view = walk.view.clone();
        let last = view.members.len().saturating_sub(1);
        let mut steps = self.open_outline(view, at, None);
        if let Some(outline) = self.thread_ui.outline.as_mut() {
            outline.cursor = (from as usize).min(last);
        }
        // Drawn once more with the ring where the chip pointed.
        steps.retain(|step| !matches!(step, Step::Show(Intent::Outline(_))));
        steps.extend(self.draw_outline());
        steps
    }

    fn open_outline(
        &mut self,
        view: ThreadView,
        current: usize,
        from_row: Option<u32>,
    ) -> Vec<Step> {
        let now = self.pickers.clock.unwrap_or_else(Local::now);
        let all = words::rows_of(&view, current, &[], self.keymap(), now);
        let outline = Outline {
            view,
            all,
            filter: String::new(),
            person: None,
            cursor: current,
            from_row,
        };
        self.thread_ui.outline = Some(outline);
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::Outline, self.policy.caps.stacking);
        steps.extend(self.draw_outline());
        steps
    }

    fn draw_outline(&self) -> Vec<Step> {
        let Some(outline) = self.thread_ui.outline.as_ref() else {
            return Vec::new();
        };
        let view = words::outline(
            &outline.all,
            &outline.filter,
            outline.person.as_deref(),
            outline.cursor,
            self.keymap(),
        );
        vec![Step::Show(Intent::Outline(Box::new(view)))]
    }

    fn outline_input_first_unread(&mut self) -> Vec<Step> {
        let Some(outline) = self.thread_ui.outline.as_mut() else {
            return Vec::new();
        };
        let shown = words::filter_rows(&outline.all, &outline.filter, outline.person.as_deref());
        if let Some(at) = words::first_unread(&shown) {
            outline.cursor = at;
        }
        self.draw_outline()
    }

    /// A pick from the menu, by key or click: it closes, then runs.
    pub(crate) fn thread_menu_pick(&mut self, id: CommandId, rows: &dyn Rows) -> Vec<Step> {
        if self.surfaces.top() != Some(SurfaceKind::ThreadMenu) || !is_entry(id) {
            return Vec::new();
        }
        let mut steps = self.close_thread_surface(SurfaceKind::ThreadMenu);
        // "Forward the whole thread" is a forward that opens on that card.
        let whole = id == CommandId::ForwardWhat2;
        let id = if whole { CommandId::Forward } else { id };
        self.compose
            .forward_as(whole.then_some(postio_model::ForwardForm::Thread));
        let ran = self
            .thread_window_command(id, rows)
            .or_else(|| self.surfaces.reader_key(id))
            .or_else(|| self.compose_verb(id, rows));
        self.compose.forward_as(None);
        steps.extend(ran.unwrap_or_default());
        steps
    }

    /// The outline's field, chips, arrows and Return.
    pub(crate) fn outline_input(&mut self, input: OutlineInput, rows: &dyn Rows) -> Vec<Step> {
        if self.surfaces.top() != Some(SurfaceKind::Outline) {
            return Vec::new();
        }
        let Some(outline) = self.thread_ui.outline.as_mut() else {
            return Vec::new();
        };
        match input {
            OutlineInput::FocusFilter => return vec![Step::Show(Intent::OutlineFocusFilter)],
            OutlineInput::Typed(text) => {
                outline.filter = text;
                outline.cursor = 0;
            }
            OutlineInput::Person(person) => {
                outline.person = person;
                outline.cursor = 0;
            }
            OutlineInput::Move(by) => {
                let shown =
                    words::filter_rows(&outline.all, &outline.filter, outline.person.as_deref());
                let last = shown.len().saturating_sub(1) as i64;
                outline.cursor = (outline.cursor as i64 + i64::from(by)).clamp(0, last) as usize;
            }
            OutlineInput::Show(at) => return self.outline_show(at, rows),
        }
        self.draw_outline()
    }

    /// Show the message of the outline's row `at` (the cursor's when
    /// `None`), and put the outline away.
    fn outline_show(&mut self, at: Option<usize>, rows: &dyn Rows) -> Vec<Step> {
        let Some(outline) = self.thread_ui.outline.as_ref() else {
            return Vec::new();
        };
        let shown = words::filter_rows(&outline.all, &outline.filter, outline.person.as_deref());
        let Some(row) = shown.get(at.unwrap_or(outline.cursor)) else {
            return Vec::new();
        };
        let (index, message) = (row.index, row.message);
        let (view, from_row) = (outline.view.clone(), outline.from_row);
        let mut steps = self.close_thread_surface(SurfaceKind::Outline);
        match from_row {
            // From the window: its walk shows that member.
            None => steps.extend(self.walk_show(index)),
            // From the list: open the message as a child's Return does.
            Some(position) => {
                self.walk_seed = Some(view);
                steps.extend(self.open_target(position, message, rows));
            }
        }
        steps
    }
}

/// Mute or unmute `thread`: the engine's own command, one undo.
fn mute_steps(thread: ThreadId, muted: bool) -> Vec<Step> {
    vec![Step::Ask(Request::Post(Command::MuteThread {
        thread: Some(thread),
        muted,
    }))]
}
