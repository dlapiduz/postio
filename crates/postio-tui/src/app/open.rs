//! The open message as the controller shows it, and what the terminal adds: its source, its
//! menus, the read clock and the paint held for its reads.

use postio_focus::{Input as Told, Intent, ReaderVerb, SurfaceKind};
use postio_model::MessageId;
use postio_model::ids::AttachmentId;
use postio_ui::keymap::Outcome;
use postio_ui::terminal::SafeText;
use postio_ui::thread_bar::{BannerView, MatchBannerView, ThreadBarView};

use super::{App, Effect, Focus};
use crate::conversation::{Member, Own, Reading};
use crate::view::open as draw;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raw {
    pub message: MessageId,
    pub text: Option<SafeText>,
    pub back_to: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    Command(&'static str),
    Link(String),
    Part(MessageId, AttachmentId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub words: String,
    pub detail: String,
    pub action: MenuAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub title: String,
    pub items: Vec<MenuItem>,
    pub at: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Thread {
    pub bar: Option<ThreadBarView>,
    pub kicker: String,
    pub older: Option<BannerView>,
    pub new_reply: Option<BannerView>,
    pub reply_to: Option<String>,
    pub matches: Option<MatchBannerView>,
}

#[derive(Debug, Default)]
pub(super) struct Open {
    pub raw: Option<Raw>,
    pub menu: Option<Menu>,
    /// A clock for an older generation does nothing.
    pub dwell: u64,
    pub dismissed: Option<MessageId>,
    pub find: Option<super::find::Find>,
    /// An older deadline does nothing.
    pub held: Option<u64>,
    pub holds: u64,
    pub place: (u32, u32),
    pub thread: Thread,
}

/// A body read within one frame (16 ms) is painted once with it; a slow read is painted without it.
pub(super) const SETTLE: std::time::Duration = std::time::Duration::from_millis(16);

impl App {
    pub fn holds_paint(&self) -> bool {
        self.reading.is_some() && self.open.held.is_some()
    }

    /// One deadline from the first read, however many follow it.
    pub(super) fn hold_paint(&mut self, effects: &[Effect]) -> Option<Effect> {
        if self.reading.is_none() {
            self.open.held = None;
            return None;
        }
        let reads = effects
            .iter()
            .any(|effect| matches!(effect, Effect::ReadBody(_)));
        if self.open.held.is_some() || !reads {
            return None;
        }
        self.open.holds += 1;
        self.open.held = Some(self.open.holds);
        Some(Effect::Settle {
            generation: self.open.holds,
            after: SETTLE,
        })
    }

    pub(super) fn release_paint(&mut self) {
        self.open.held = None;
    }

    pub(super) fn settled(&mut self, generation: u64) -> Vec<Effect> {
        if self.open.held != Some(generation) {
            return Vec::new();
        }
        self.open.held = None;
        vec![Effect::Redraw]
    }

    pub fn raw(&self) -> Option<&Raw> {
        self.open.raw.as_ref()
    }

    pub fn menu(&self) -> Option<&Menu> {
        self.open.menu.as_ref()
    }

    pub fn thread(&self) -> &Thread {
        &self.open.thread
    }

    pub fn row_of_open(&self) -> Option<&crate::row::Row> {
        let reading = self.reading.as_ref()?;
        match &reading.own {
            Some(own) => Some(&own.row),
            None => self.row(reading.row),
        }
    }

    pub(super) fn own_aim(&self) -> Option<MessageId> {
        self.reading
            .as_ref()
            .filter(|reading| reading.own.is_some())
            .map(|reading| reading.row)
    }

    /// Over whatever the window shows: its verbs aim at it, the list's cursor and marks stay, and
    /// closing it returns the keyboard to `back`.
    pub(super) fn open_for_itself(&mut self, row: crate::row::Row, back: Focus) -> Vec<Effect> {
        let message = row.id;
        let reading = Reading::of(&row);
        self.reading = Some(Reading {
            own: Some(Own { row, back }),
            ..reading
        });
        self.focus = Focus::Reader;
        self.reader_top = 0;
        self.open = Open {
            dwell: self.open.dwell,
            holds: self.open.holds,
            ..Open::default()
        };
        self.armed_link = None;
        let mut effects = vec![Effect::ReadBody(message)];
        effects.extend(self.arm_dwell());
        effects.push(Effect::Redraw);
        effects
    }

    /// The marker is the conversation's, so it belongs to the message its row stands for.
    pub fn marker_shown(&self, row: &crate::row::Row, member: &Member) -> bool {
        row.marker.is_some() && row.id == member.id && self.open.dismissed != Some(member.id)
    }

    pub(super) fn open_draft_offers_edit(&self) -> bool {
        self.row_of_open().is_some_and(|row| {
            postio_ui::focus_dialog::send_verbs(row.send_state)
                .is_some_and(|verbs| verbs.contains(&postio_core::CommandId::OpenMessage))
        })
    }

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

    pub fn captures(&self) -> bool {
        self.features.capture
    }

    /// Nothing for a message opened for itself.
    pub fn position_line(&self) -> String {
        let Some(reading) = self
            .reading
            .as_ref()
            .filter(|reading| reading.own.is_none())
        else {
            return String::new();
        };
        let (index, total) = self.open.place;
        postio_ui::focus_dialog::position_line(
            index as usize,
            total as usize,
            1,
            0,
            true,
            self.row(reading.row).and_then(|row| row.send_state),
        )
    }

    pub fn pane(&self) -> Option<u16> {
        // Filtered has the whole body: a message opened from it is in the frame, over it.
        if self.features.reading != postio_config::Reading::Pane || self.surfaces.filtered.is_some()
        {
            return None;
        }
        let list = self.window().list;
        crate::layout::split_pane(list).map(|(_, pane)| pane.width)
    }

    pub fn keymap(&self) -> &postio_core::Keymap {
        self.keys.keymap()
    }

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

    pub(super) fn scroll_open_lines(&mut self, lines: isize) {
        let (length, shown) = self.open_extent();
        let last = length.saturating_sub(shown.min(length));
        self.reader_top = self.reader_top.saturating_add_signed(lines).min(last);
    }

    /// Two rows of the last screenful stay for the eye.
    pub(super) fn scroll_reader(&mut self, pages: isize) {
        let (_, shown) = self.open_extent();
        let step = isize::try_from(shown.saturating_sub(2).max(1)).unwrap_or(1);
        self.scroll_open_lines(step * pages);
    }

    /// A step through its thread keeps the row it was opened from, and what the controller said
    /// of that thread until it says it again.
    fn show_message(&mut self, message: MessageId, place: (u32, u32)) -> Vec<Effect> {
        self.focus = Focus::Reader;
        self.open.place = place;
        let row = self.cursor_message().unwrap_or(message);
        let kept = self
            .reading
            .as_ref()
            .filter(|reading| reading.own.is_none() && reading.row == row);
        if kept.is_some_and(|reading| reading.shown.id == message) {
            return vec![Effect::Redraw];
        }
        let thread = match kept {
            Some(_) => std::mem::take(&mut self.open.thread),
            None => Thread::default(),
        };
        let shown = match self.row(message) {
            Some(found) if found.id == message => Member::from_row(found),
            _ => Member::new(message),
        };
        self.reading = Some(Reading {
            row,
            shown,
            own: None,
        });
        self.reader_top = 0;
        self.open = Open {
            dwell: self.open.dwell,
            holds: self.open.holds,
            place,
            thread,
            ..Open::default()
        };
        self.armed_link = None;
        let mut effects = vec![Effect::ReadBody(message)];
        effects.extend(self.arm_dwell());
        effects.push(Effect::Redraw);
        effects
    }

    /// A message open on no cursor row (from Filtered, a digest, a search) is what the
    /// controller's verbs and pickers aim at, so it hears which one is on screen.
    pub(super) fn report_shown(&mut self) -> Vec<Effect> {
        let shown = self.reading.as_ref().map(|reading| reading.shown.id);
        if self.told_shown == shown {
            return Vec::new();
        }
        self.told_shown = shown;
        self.tell(Told::Shown(shown))
    }

    pub(super) fn follow_cursor(&mut self) -> Vec<Effect> {
        if self.focus != Focus::Reader || self.own_aim().is_some() {
            return Vec::new();
        }
        self.tell(Told::Follow)
    }

    pub(super) fn close_message(&mut self) -> Vec<Effect> {
        let back = self
            .reading
            .as_ref()
            .and_then(|reading| reading.own.as_ref())
            .map(|own| own.back);
        self.focus = back.unwrap_or(Focus::List);
        self.reading = None;
        self.reader_top = 0;
        self.armed_link = None;
        self.open = Open {
            dwell: self.open.dwell + 1,
            holds: self.open.holds,
            ..Open::default()
        };
        let mut effects = Vec::new();
        // The controller opened it over Filtered, and keeps it there until told.
        if back == Some(Focus::Filtered) {
            effects.extend(self.tell(postio_focus::Input::SurfaceClosed(
                postio_focus::SurfaceKind::Message,
            )));
        }
        effects.push(Effect::Redraw);
        effects
    }

    fn close_more(&mut self) -> Vec<Effect> {
        if self.open.menu.take().is_some() {
            return vec![Effect::Redraw];
        }
        if let Some(raw) = self.open.raw.take() {
            self.reader_top = raw.back_to;
        }
        vec![Effect::Redraw]
    }

    pub(super) fn back_from_own(&mut self) -> Vec<Effect> {
        if self.open.menu.is_some() || self.open.raw.is_some() {
            return self.close_more();
        }
        self.close_message()
    }

    pub(super) fn the_readers(command: postio_core::CommandId) -> bool {
        use postio_core::CommandId as C;
        matches!(
            command,
            C::NextMessage
                | C::PrevMessage
                | C::FirstMessage
                | C::LastMessage
                | C::Back
                | C::PrevView
                | C::DismissMarker
                | C::OpenMessage
                | C::Reply
                | C::ReplyAll
                | C::Forward
                | C::ReplyToLatest
                | C::CancelSend
                | C::RetrySend
                | C::MarkSent
                | C::FindInMessage
                | C::FindNext
                | C::FindPrevious
                | C::SwitchTreatment
                | C::ViewSource
                | C::MoreActions
                | C::NextInConversation
                | C::PrevInConversation
                | C::FirstInConversation
                | C::LatestInConversation
                | C::FirstUnreadInConversation
        ) || postio_ui::focus_target::dispatch(command).is_some()
    }

    /// What Back would close goes first, so the controller closes it before the message.
    pub(super) fn tell_reader(&mut self, command: postio_core::CommandId) -> Vec<Effect> {
        use postio_core::CommandId as C;
        match command {
            C::ToggleRead => self.cancel_dwell(),
            C::DismissMarker => {
                self.open.dismissed = self.reading.as_ref().map(|reading| reading.shown.id);
            }
            _ => {}
        }
        let mut effects = self.tell(Told::ReaderState {
            more_open: self.open.menu.is_some() || self.open.raw.is_some(),
            finding: self.open.find.is_some(),
        });
        effects.extend(self.tell(Told::Command(command)));
        effects
    }

    pub(super) fn chip_clicked(&mut self, index: usize) -> Vec<Effect> {
        match u32::try_from(index) {
            Ok(index) => self.tell(Told::ThreadBarChipClicked(index)),
            Err(_) => Vec::new(),
        }
    }

    pub(super) fn show_reader_intent(&mut self, intent: Intent) -> Vec<Effect> {
        if let Intent::OpenMessage {
            message,
            index,
            total,
            host: postio_focus::Host::Own,
        } = intent
        {
            return self.show_message(message, (index, total));
        }
        match intent {
            Intent::CloseSurface(SurfaceKind::Message) => return self.close_message(),
            Intent::Reader(verb) => return self.reader_verb(verb),
            _ if self.reading.is_none() => return Vec::new(),
            _ => {}
        }
        let thread = &mut self.open.thread;
        match intent {
            Intent::ThreadBar(bar) => thread.bar = Some(bar),
            Intent::Kicker(kicker) => thread.kicker = kicker,
            Intent::OlderBanner(banner) => thread.older = banner,
            Intent::NewReply(banner) => thread.new_reply = banner,
            Intent::ReplyTo(text) => thread.reply_to = text,
            Intent::MatchBanner(banner) => thread.matches = banner,
            _ => return Vec::new(),
        }
        vec![Effect::Redraw]
    }

    fn reader_verb(&mut self, verb: ReaderVerb) -> Vec<Effect> {
        match verb {
            ReaderVerb::CloseMore => self.close_more(),
            ReaderVerb::CloseFind => self.close_find(),
            ReaderVerb::FindInMessage => self.open_find(),
            ReaderVerb::FindNext => self.find_step(true),
            ReaderVerb::FindPrevious => self.find_step(false),
            ReaderVerb::NextMatch => self.match_step(true),
            ReaderVerb::PrevMatch => self.match_step(false),
            ReaderVerb::SwitchTreatment => self.view_original(),
            ReaderVerb::ViewSource => self.view_source(),
            ReaderVerb::ShowMore => self.more_actions(),
            // The rest are said only for a message with no thread to step through.
            _ => Vec::new(),
        }
    }

    pub(super) fn addressed(
        &mut self,
        message: MessageId,
        row: &postio_model::Message,
        warning: Option<&str>,
    ) -> Vec<Effect> {
        match self.reading.as_mut().map(|reading| &mut reading.shown) {
            Some(shown) if shown.id == message => {
                shown.address(row, warning);
                vec![Effect::Redraw]
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn arm_dwell(&mut self) -> Vec<Effect> {
        self.open.dwell += 1;
        let shown = self.reading.as_ref().map(|reading| reading.shown.id.get());
        match postio_ui::dwell::on_cursor(shown) {
            postio_ui::dwell::Arm::Start { message, after } => vec![Effect::ArmDwell {
                generation: self.open.dwell,
                message: MessageId::new(message),
                after,
            }],
            postio_ui::dwell::Arm::Cancel => Vec::new(),
        }
    }

    pub(super) fn cancel_dwell(&mut self) {
        self.open.dwell += 1;
    }

    pub(super) fn dwelt(&mut self, generation: u64, message: MessageId) -> Vec<Effect> {
        let shown = self
            .reading
            .as_ref()
            .filter(|_| self.focus == Focus::Reader)
            .map(|reading| reading.shown.id);
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

    pub(super) fn view_source(&mut self) -> Vec<Effect> {
        let Some(message) = self
            .reading
            .as_ref()
            .map(|reading| reading.shown.id)
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

    pub(super) fn offer_choices(&mut self) -> Vec<Effect> {
        let Some(member) = self.reading.as_ref().map(|reading| &reading.shown) else {
            return Vec::new();
        };
        let mut items: Vec<MenuItem> = Vec::new();
        for link in member.body.iter().flat_map(|body| &body.links) {
            let action = MenuAction::Link(link.as_str().to_owned());
            if items.iter().any(|item| item.action == action) {
                continue;
            }
            items.push(MenuItem {
                words: link.as_str().to_owned(),
                detail: "link".to_owned(),
                action,
            });
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

    /// A notice, not a menu: ignoring it or Esc keeps everything, and its button is one command
    /// on exactly the found messages, so one undo (#1952).
    pub(super) fn confirm_unsubscribed(
        &mut self,
        name: &str,
        found: Option<postio_client::protocol::ListMailFound>,
    ) -> Vec<Effect> {
        use postio_core::{Command, MessageTarget};
        use postio_ui::unsubscribe::{SweepScope, sweep_notice};
        let action = self.features.after_unsubscribe;
        let notice = found.as_ref().and_then(|found| {
            sweep_notice(
                &SweepScope::ListId(name.to_owned()),
                found.ids.len(),
                found.more,
                action,
            )
        });
        let (Some(notice), Some(found)) = (notice, found) else {
            return self.say(&postio_ui::focus_target::unsubscribed(name));
        };
        let target = MessageTarget::Messages(found.ids);
        let command = match notice.action {
            postio_config::AfterUnsubscribe::Archive => Command::Archive { target },
            postio_config::AfterUnsubscribe::Delete => Command::Delete { target },
        };
        let effects = self.say(&notice.text);
        self.notice_offer = Some((notice.button, command));
        effects
    }

    #[cfg(test)]
    pub(crate) fn confirm_unsubscribed_for_test(&mut self, name: &str, ids: &[i64]) {
        self.confirm_unsubscribed(
            name,
            Some(postio_client::protocol::ListMailFound {
                name: name.to_owned(),
                ids: ids.iter().copied().map(MessageId::new).collect(),
                more: false,
            }),
        );
    }

    pub(super) fn take_up_offer(&mut self) -> Vec<Effect> {
        match self.notice_offer.take() {
            Some((_, command)) => vec![Effect::Send(command), Effect::Redraw],
            None => Vec::new(),
        }
    }

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
}
