//! The open message: opening a row, stepping through the list and the
//! thread, the raw source, the menus, and the read clock.
//!
//! The drawing is `view::open`; this is what the keys and the clicks do.

use postio_model::MessageId;
use postio_model::ids::AttachmentId;
use postio_ui::keymap::Outcome;
use postio_ui::terminal::SafeText;

use super::{App, Effect, Focus};
use crate::view::open as draw;

/// The message's source, being shown in place of the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raw {
    /// Whose source.
    pub message: MessageId,
    /// What came off the wire, once it has.
    pub text: Option<SafeText>,
    /// Where the message was scrolled to, for when this closes.
    pub back_to: usize,
}

/// What choosing a row of a menu does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    /// Run a command, as its key does.
    Command(&'static str),
    /// Open a link with the system's opener.
    Link(String),
    /// Open a part of the message.
    Part(MessageId, AttachmentId),
}

/// One row of a menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// Its words.
    pub words: String,
    /// What follows them: a key, a size, where a link goes.
    pub detail: String,
    /// What choosing it does.
    pub action: MenuAction,
}

/// A small framed list over the open message: its links and attachments, or
/// the verbs its action row folded away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    /// What it is for.
    pub title: String,
    /// Its rows.
    pub items: Vec<MenuItem>,
    /// The row the keyboard is on.
    pub at: usize,
}

/// What the open message holds besides the message.
#[derive(Debug, Default)]
pub(super) struct Open {
    pub raw: Option<Raw>,
    pub menu: Option<Menu>,
    /// Which read clock is running; a clock for an older one does nothing.
    pub dwell: u64,
    /// The message whose marker was dismissed while it was open.
    pub dismissed: Option<MessageId>,
    /// The find field, while it is open.
    pub find: Option<super::find::Find>,
}

impl App {
    /// The source being shown in place of the message.
    pub fn raw(&self) -> Option<&Raw> {
        self.open.raw.as_ref()
    }

    /// The menu over the open message.
    pub fn menu(&self) -> Option<&Menu> {
        self.open.menu.as_ref()
    }

    /// The row the open message stands for: its own, when it was opened for
    /// itself, else the list row it was opened from.
    pub fn row_of_open(&self) -> Option<&crate::row::Row> {
        let reading = self.reading.as_ref()?;
        match &reading.own {
            Some(own) => Some(&own.row),
            None => self.row(reading.row),
        }
    }

    /// The message verbs aim at while one is open for itself.
    pub(super) fn own_aim(&self) -> Option<MessageId> {
        self.reading
            .as_ref()
            .filter(|reading| reading.own.is_some())
            .map(|reading| reading.row)
    }

    /// Open `row`'s message for itself, over whatever the window shows: its
    /// verbs aim at it, the list's cursor and marks stay where they are, and
    /// closing it returns the keyboard to `back`.
    pub(super) fn open_for_itself(&mut self, row: crate::row::Row, back: Focus) -> Vec<Effect> {
        let message = row.id;
        self.reading = Some(crate::conversation::Reading {
            row: message,
            members: vec![crate::conversation::Member::from_row(&row)],
            current: 0,
            own: Some(crate::conversation::Own { row, back }),
        });
        self.focus = Focus::Reader;
        self.reader_top = 0;
        self.open = Open {
            dwell: self.open.dwell,
            ..Open::default()
        };
        self.armed_link = None;
        let mut effects = vec![Effect::ReadBody(message)];
        effects.extend(self.arm_dwell());
        effects.push(Effect::Redraw);
        effects
    }

    /// Whether the action card is drawn for `member`: the marker is the
    /// conversation's, so it belongs to the message the row stands for, and
    /// goes once dismissed.
    pub fn marker_shown(
        &self,
        row: &crate::row::Row,
        member: &crate::conversation::Member,
    ) -> bool {
        row.marker.is_some() && row.id == member.id && self.open.dismissed != Some(member.id)
    }

    /// Whether the open message is a draft whose Edit is offered: one on
    /// its way or stopped.
    pub(super) fn open_draft_offers_edit(&self) -> bool {
        self.row_of_open().is_some_and(|row| {
            postio_ui::focus_dialog::send_verbs(row.send_state)
                .is_some_and(|verbs| verbs.contains(&postio_core::CommandId::OpenMessage))
        })
    }

    /// What a click on row `line` of the open message's column stands for.
    pub(super) fn line_at(&self, line: usize) -> crate::conversation::At {
        let (outer, inner, _) = self.open_geometry();
        let column = crate::layout::column_width(outer).min(inner);
        draw::document(
            self,
            &crate::theme::Theme::plain(),
            column,
            chrono::Local::now(),
        )
        .lines
        .get(line)
        .map_or(crate::conversation::At::Nothing, |line| line.at.clone())
    }

    /// Whether a to-do offers Task: a vault is configured.
    pub fn captures(&self) -> bool {
        self.features.capture
    }

    /// "Message 5 of 312 · thread of 6", for the open message; nothing for
    /// one opened for itself.
    pub fn position_line(&self) -> String {
        let Some(reading) = self.reading.as_ref() else {
            return String::new();
        };
        // A message opened for itself is no place in the list.
        if reading.own.is_some() {
            return String::new();
        }
        let row = self.row(reading.row);
        let latest = reading.current + 1 >= reading.members.len();
        postio_ui::focus_dialog::position_line(
            self.cursor as usize,
            self.list.total() as usize,
            row.map_or(1, |row| row.count),
            reading.current,
            latest,
            row.and_then(|row| row.send_state),
        )
    }

    /// The pane the open message sits in beside the list, when it is placed
    /// there: `[focus] reading` says so and the terminal is wide enough.
    pub fn pane(&self) -> Option<u16> {
        // Filtered has the whole body: a message opened from it is in the
        // frame, over it.
        if self.features.reading != postio_config::Reading::Pane || self.surfaces.filtered.is_some()
        {
            return None;
        }
        let list = self.window().list;
        crate::layout::split_pane(list).map(|(_, pane)| pane.width)
    }

    /// The keymap in force.
    pub fn keymap(&self) -> &postio_core::Keymap {
        self.keys.keymap()
    }

    /// `F8`: open messages beside the list, or over it again. The choice is
    /// written to `config.toml` and said in the words the desktop uses.
    pub(super) fn toggle_reading_pane(&mut self) -> Vec<Effect> {
        use postio_config::Reading;
        let next = match self.features.reading {
            Reading::Dialog => Reading::Pane,
            Reading::Pane => Reading::Dialog,
        };
        self.features.reading = next;
        let said = postio_ui::focus_target::reading_placement(
            next == Reading::Pane,
            self.pane().is_some(),
        );
        let mut effects = vec![Effect::SetReading(next)];
        effects.extend(self.say(said));
        effects
    }

    /// The width the open message is laid out in, the width of what scrolls,
    /// and how many rows the column shows at once.
    pub(super) fn open_geometry(&self) -> (u16, u16, u16) {
        let find = self.find_rows();
        if let Some(pane) = self.pane() {
            let list = self.window().list;
            return (pane, pane - 1, list.height.saturating_sub(4 + find));
        }
        let area = ratatui::layout::Rect::new(0, 0, self.size.0, self.size.1);
        let frame = crate::layout::open_frame(area);
        (
            frame.width,
            frame.width.saturating_sub(2),
            frame.height.saturating_sub(2 + 4 + find),
        )
    }

    /// How many rows the open message's column has, and how many it shows
    /// at once.
    pub(super) fn open_extent(&self) -> (usize, usize) {
        let (outer, inner, height) = self.open_geometry();
        let length = match &self.open.raw {
            Some(raw) => raw.text.as_ref().map_or(1, |text| {
                draw::raw_lines(text, usize::from(inner).saturating_sub(2).max(1)).len()
            }),
            None => {
                let column = crate::layout::column_width(outer).min(inner);
                draw::document(
                    self,
                    &crate::theme::Theme::plain(),
                    column,
                    chrono::Local::now(),
                )
                .lines
                .len()
            }
        };
        (length, usize::from(height))
    }

    /// Scroll the open message by `lines`, never past its last row.
    pub(super) fn scroll_open_lines(&mut self, lines: isize) {
        let (length, shown) = self.open_extent();
        let last = length.saturating_sub(shown.min(length));
        self.reader_top = self.reader_top.saturating_add_signed(lines).min(last);
    }

    /// Scroll by `pages` screenfuls, keeping two rows for the eye.
    pub(super) fn scroll_reader(&mut self, pages: isize) {
        let (_, shown) = self.open_extent();
        let step = isize::try_from(shown.saturating_sub(2).max(1)).unwrap_or(1);
        self.scroll_open_lines(step * pages);
    }

    /// Open the row under the cursor in the open message: its body is read
    /// now, its conversation when it has one, and the read clock starts.
    pub(super) fn open_at_cursor(&mut self) -> Vec<Effect> {
        let Some(message) = self.cursor_message() else {
            return Vec::new();
        };
        // The row already open is open.
        if self.focus == Focus::Reader
            && self.reading.as_ref().map(|reading| reading.row) == Some(message)
        {
            return vec![Effect::Redraw];
        }
        let mut effects = self.open_reading(message);
        self.focus = Focus::Reader;
        self.reader_top = 0;
        self.open = Open {
            dwell: self.open.dwell,
            ..Open::default()
        };
        self.armed_link = None;
        effects.extend(self.arm_dwell());
        effects.push(Effect::Redraw);
        effects
    }

    /// `j` and `k` in the open message: the next or previous row opens in
    /// its place, the list's cursor with it. A digest is not a message and
    /// is passed over.
    pub(super) fn step_open(&mut self, by: i32) -> Vec<Effect> {
        // One opened for itself steps through where it came from.
        if let Some(own) = self
            .reading
            .as_ref()
            .and_then(|reading| reading.own.as_ref())
        {
            return match own.back {
                Focus::Filtered => self.step_filtered_open(by as isize),
                _ => Vec::new(),
            };
        }
        let last = self.list.total().saturating_sub(1);
        let mut at = self.cursor;
        loop {
            let next = at.saturating_add_signed(by).min(last);
            if next == at {
                return Vec::new();
            }
            at = next;
            if self
                .row_at(at)
                .is_none_or(|row| row.kind != crate::row::Kind::Digest)
            {
                break;
            }
        }
        self.move_to(at);
        self.open_at_cursor()
    }

    /// The cursor's row is not the one open, though a message is: open it.
    /// What follows a list that moved on under an open message -- the one
    /// archived from it, or a row that was not here yet when `j` was
    /// pressed.
    pub(super) fn follow_cursor(&mut self) -> Vec<Effect> {
        if self.focus != Focus::Reader || self.own_aim().is_some() {
            return Vec::new();
        }
        let Some(open) = self.reading.as_ref().map(|reading| reading.row) else {
            return Vec::new();
        };
        match self.cursor_message() {
            Some(message) if message != open => {
                if self
                    .row_at(self.cursor)
                    .is_some_and(|row| row.kind == crate::row::Kind::Digest)
                {
                    return self.close_message();
                }
                self.open_at_cursor()
            }
            None if self.scope.is_some() && self.list.total() == 0 => self.close_message(),
            _ => Vec::new(),
        }
    }

    /// Close the message: the list is as it was, the cursor and the
    /// selection with it.
    pub(super) fn close_message(&mut self) -> Vec<Effect> {
        self.focus = self
            .reading
            .as_ref()
            .and_then(|reading| reading.own.as_ref())
            .map_or(Focus::List, |own| own.back);
        self.reading = None;
        self.reader_top = 0;
        self.armed_link = None;
        self.open = Open {
            dwell: self.open.dwell + 1,
            ..Open::default()
        };
        vec![Effect::Redraw]
    }

    /// Escape: the find field first, then a menu, then the source, then the
    /// message.
    pub(super) fn back_from_message(&mut self) -> Vec<Effect> {
        if self.open.find.is_some() {
            return self.close_find();
        }
        if self.open.menu.take().is_some() {
            return vec![Effect::Redraw];
        }
        if let Some(raw) = self.open.raw.take() {
            self.reader_top = raw.back_to;
            return vec![Effect::Redraw];
        }
        self.close_message()
    }

    /// Start the read clock on the message shown, stopping any other. The
    /// delay and the rule are `postio_ui::dwell`'s.
    pub(super) fn arm_dwell(&mut self) -> Vec<Effect> {
        self.open.dwell += 1;
        let shown = self
            .reading
            .as_ref()
            .and_then(|reading| reading.members.get(reading.current))
            .map(|member| member.id.get());
        match postio_ui::dwell::on_cursor(shown) {
            postio_ui::dwell::Arm::Start { message, after } => vec![Effect::ArmDwell {
                generation: self.open.dwell,
                message: MessageId::new(message),
                after,
            }],
            postio_ui::dwell::Arm::Cancel => Vec::new(),
        }
    }

    /// Stop the read clock: the person set the read state themselves, or
    /// the message closed.
    pub(super) fn cancel_dwell(&mut self) {
        self.open.dwell += 1;
    }

    /// The clock of `generation` ran out on `message`: it counts as read if
    /// it is still the one on screen.
    pub(super) fn dwelt(&mut self, generation: u64, message: MessageId) -> Vec<Effect> {
        let shown = self
            .reading
            .as_ref()
            .filter(|_| self.focus == Focus::Reader)
            .and_then(|reading| reading.members.get(reading.current))
            .map(|member| member.id);
        if generation != self.open.dwell || shown != Some(message) {
            return Vec::new();
        }
        if !self.row_of_open().is_some_and(|row| row.unread) {
            return Vec::new();
        }
        vec![Effect::Send(postio_core::Command::MarkReadOnDwell {
            message,
        })]
    }

    /// `[` and `]`: another message of the conversation. Its body is read
    /// the first time it is shown, and it starts its own read clock.
    pub(super) fn walk_conversation(&mut self, step: isize) -> Vec<Effect> {
        let Some(reading) = self.reading.as_mut() else {
            return Vec::new();
        };
        let Some(next) =
            postio_ui::focus_dialog::step_thread(reading.current, reading.members.len(), step)
        else {
            return Vec::new();
        };
        reading.current = next;
        let mut effects = vec![Effect::Redraw];
        if let Some(member) = reading.members.get_mut(next)
            && member.body.is_none()
            && !member.asked
        {
            member.asked = true;
            effects.push(Effect::ReadBody(member.id));
        }
        self.reader_top = 0;
        self.open.raw = None;
        effects.extend(self.arm_dwell());
        effects
    }

    /// `v`: the message as it came off the wire, in place of the message.
    pub(super) fn view_source(&mut self) -> Vec<Effect> {
        let Some(message) = self
            .reading
            .as_ref()
            .and_then(|reading| reading.members.get(reading.current))
            .map(|member| member.id)
            .or_else(|| self.cursor_message())
        else {
            return Vec::new();
        };
        self.open.raw = Some(Raw {
            message,
            text: None,
            back_to: self.reader_top,
        });
        self.reader_top = 0;
        vec![Effect::ReadSource(message), Effect::Redraw]
    }

    /// The source arrived.
    pub(super) fn source_read(
        &mut self,
        message: MessageId,
        read: Result<Vec<u8>, String>,
    ) -> Vec<Effect> {
        if self.open.raw.as_ref().map(|raw| raw.message) != Some(message) {
            return Vec::new();
        }
        match read {
            Ok(bytes) => {
                if let Some(raw) = self.open.raw.as_mut() {
                    raw.text = Some(SafeText::new(&String::from_utf8_lossy(&bytes)));
                }
                vec![Effect::Redraw]
            }
            Err(reason) => {
                self.open.raw = None;
                self.say(&reason)
            }
        }
    }

    /// `o`: the message's links and its attachments, to choose one to open.
    pub(super) fn offer_choices(&mut self) -> Vec<Effect> {
        let Some(reading) = self.reading.as_ref() else {
            return Vec::new();
        };
        let Some(member) = reading.members.get(reading.current) else {
            return self.say(postio_ui::focus_target::NOTHING_TO_OPEN);
        };
        let mut items: Vec<MenuItem> = Vec::new();
        if let Some(body) = &member.body {
            for link in &body.links {
                if items
                    .iter()
                    .any(|item| item.action == MenuAction::Link(link.as_str().to_owned()))
                {
                    continue;
                }
                items.push(MenuItem {
                    words: link.as_str().to_owned(),
                    detail: "link".to_owned(),
                    action: MenuAction::Link(link.as_str().to_owned()),
                });
            }
        }
        for part in member.attachments() {
            let name = SafeText::new(part.filename.as_deref().unwrap_or(&part.mime_type));
            items.push(MenuItem {
                words: name.as_str().to_owned(),
                detail: postio_ui::format::human_size(part.size),
                action: MenuAction::Part(member.id, part.id),
            });
        }
        if items.is_empty() {
            return self.say(postio_ui::focus_target::NOTHING_TO_OPEN);
        }
        self.open.menu = Some(Menu {
            title: "Open attachment or link".to_owned(),
            items,
            at: 0,
        });
        vec![Effect::Redraw]
    }

    /// `.`: the verbs the action row folded away, as a menu. Nothing when
    /// nothing is folded.
    pub(super) fn more_actions(&mut self) -> Vec<Effect> {
        let (outer, inner, _) = self.open_geometry();
        if self.reading.is_none() || !draw::folded(self, outer, inner) {
            return Vec::new();
        }
        let items = postio_ui::focus_dialog::FOLDED
            .iter()
            .filter_map(|command| {
                let verb = postio_ui::focus_dialog::OPEN_TOOLBAR
                    .iter()
                    .find(|verb| verb.command == *command)?;
                Some(MenuItem {
                    words: verb.label.to_owned(),
                    detail: self.hint(*command).unwrap_or_default(),
                    action: MenuAction::Command(command.as_str()),
                })
            })
            .collect();
        self.open.menu = Some(Menu {
            title: "More".to_owned(),
            items,
            at: 0,
        });
        vec![Effect::Redraw]
    }

    /// A key while a menu is over the message.
    pub(super) fn menu_key(&mut self, key: &crossterm::event::KeyEvent) -> Vec<Effect> {
        use crossterm::event::KeyCode;
        let Some(menu) = self.open.menu.as_mut() else {
            return Vec::new();
        };
        let last = menu.items.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => {
                self.open.menu = None;
            }
            KeyCode::Down | KeyCode::Char('j') => menu.at = (menu.at + 1).min(last),
            KeyCode::Up | KeyCode::Char('k') => menu.at = menu.at.saturating_sub(1),
            KeyCode::Enter => {
                let at = menu.at;
                return self.choose(at);
            }
            // A row's own key, as its words show it, chooses that row.
            _ => {
                let Outcome::Command(id) = self.keys.press(key, self.key_context(), false) else {
                    return Vec::new();
                };
                let chosen = self.open.menu.as_ref().and_then(|menu| {
                    menu.items.iter().position(
                        |item| matches!(item.action, MenuAction::Command(own) if own == id),
                    )
                });
                return match chosen {
                    Some(at) => self.choose(at),
                    None => Vec::new(),
                };
            }
        }
        vec![Effect::Redraw]
    }

    /// Choose row `at` of the menu: it closes, and its row does what it says.
    pub(super) fn choose(&mut self, at: usize) -> Vec<Effect> {
        let Some(item) = self
            .open
            .menu
            .take()
            .and_then(|menu| menu.items.into_iter().nth(at))
        else {
            return Vec::new();
        };
        match item.action {
            MenuAction::Command(id) => self.command(id),
            MenuAction::Link(target) => {
                let mut effects = self.say(&format!("Opening {target}"));
                effects.push(Effect::OpenLink(target));
                effects
            }
            MenuAction::Part(message, attachment) => vec![
                Effect::SavePart {
                    message,
                    attachment,
                    to: None,
                },
                Effect::Redraw,
            ],
        }
    }

    /// `-` on the open message: its marker is a wrong one, taken off now and
    /// from the store, with one undo.
    pub(super) fn dismiss_open_marker(&mut self) -> Vec<Effect> {
        let Some(message) = self
            .reading
            .as_ref()
            .and_then(|reading| reading.members.get(reading.current))
            .map(|member| member.id)
        else {
            return Vec::new();
        };
        self.open.dismissed = Some(message);
        vec![
            Effect::Send(postio_core::Command::DismissMarker {
                target: postio_core::MessageTarget::Messages(vec![message]),
                dismissed: true,
            }),
            Effect::Redraw,
        ]
    }
}
