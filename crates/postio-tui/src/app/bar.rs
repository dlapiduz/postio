//! The command bar and the search results, driven by the controller.

use crossterm::event::{KeyCode, KeyEvent};
use postio_core::CommandId;
use postio_focus::{Input as Told, Intent, ResultRow, SurfaceKind};
use postio_ui::keymap::{KeyContext, Outcome};

use super::{App, Effect, Focus};
use crate::bar::{Bar, Shown};
use crate::results::Results;

/// The commands the results' controller answers that this terminal draws no
/// surface for: Quick Look, the filter popovers and a result's thread.
const NOT_DRAWN: [CommandId; 9] = [
    CommandId::QuickLook,
    CommandId::FilterFrom,
    CommandId::FilterTo,
    CommandId::FilterDate,
    CommandId::FilterAnywhere,
    CommandId::FilterLabel,
    CommandId::StepRangeBack,
    CommandId::StepRangeForward,
    CommandId::ExpandThread,
];

impl App {
    pub(super) fn show_bar_intent(&mut self, intent: &Intent) -> Option<Vec<Effect>> {
        match intent {
            Intent::OpenBar { text, .. } => {
                if self.bar.is_none() {
                    let from = match self.focus {
                        Focus::Palette | Focus::Bar => Focus::List,
                        focus => focus,
                    };
                    self.bar = Some(Bar::open(from));
                    self.focus = Focus::Bar;
                }
                if let Some(bar) = self.bar.as_mut() {
                    bar.set_text(text);
                }
            }
            Intent::BarLines(view) => {
                if let Some(bar) = self.bar.as_mut() {
                    bar.show_lines(view.clone());
                }
            }
            Intent::Dropdown(view) => {
                if let Some(bar) = self.bar.as_mut() {
                    bar.show_dropdown(view.clone());
                }
            }
            Intent::CloseSurface(SurfaceKind::Bar) => {
                if let Some(bar) = self.bar.take() {
                    self.focus = bar.from();
                }
            }
            Intent::ShowResults(view) => {
                let mut shown = Results::new((**view).clone());
                if let Some(before) = self.results.take() {
                    shown.query = before.query;
                    shown.none = before.none;
                    shown.top = before.top;
                    shown.cursor = view.cursor.unwrap_or(before.cursor);
                }
                self.results = Some(shown);
                self.reveal_result();
                return Some(self.results_fetches());
            }
            Intent::ResultsPage { .. } => {}
            Intent::ResultsCursor(position) => {
                if let Some(results) = self.results.as_mut() {
                    results.cursor = *position;
                }
                self.reveal_result();
                return Some(self.results_fetches());
            }
            Intent::Query(view) => {
                if let Some(results) = self.results.as_mut() {
                    results.query = Some(view.clone());
                }
            }
            Intent::Relaxations(none) => {
                if let Some(results) = self.results.as_mut() {
                    results.none = none.as_deref().cloned();
                }
            }
            Intent::LeaveResults => self.results = None,
            Intent::SavePopover(Some(view)) => {
                return Some(self.tell(Told::SaveSearchAs {
                    name: view.name.clone(),
                    pin: view.pin,
                    notify: view.notify,
                    rolling: view.rolling,
                }));
            }
            Intent::SaveSearch(save) => {
                let path = crate::config_file::path();
                let saved = postio_focus::save_search(path.as_deref(), save);
                return Some(self.tell(Told::SearchSaved(saved)));
            }
            Intent::OpenMessage { message, .. } if self.results.is_some() => {
                return Some(self.open_result(*message));
            }
            Intent::Popover(_)
            | Intent::QuickLook(_)
            | Intent::ResultSplice(_)
            | Intent::ResultChildFocus(_)
            | Intent::MatchBanner(_)
            | Intent::FileCopy(_)
            | Intent::SavePopover(None) => {}
            _ => return None,
        }
        Some(vec![Effect::Redraw])
    }

    pub fn bar(&self) -> Option<&Bar> {
        self.bar.as_ref()
    }

    pub fn bar_typed(&self) -> Option<&str> {
        self.bar.as_ref().map(Bar::typed)
    }

    pub fn results(&self) -> Option<&Results> {
        self.results.as_ref()
    }

    pub fn result_row(&self, position: u64) -> Option<ResultRow> {
        self.ctl.result_row(position)
    }

    pub fn result_file(&self, position: u64) -> Option<postio_focus::FileCard> {
        self.ctl.result_file(position)
    }

    pub fn result_person(&self, position: u64) -> Option<postio_focus::PersonRow> {
        self.ctl.result_person(position)
    }

    fn result_room(&self) -> u64 {
        let list = self.window().list;
        u64::from(list.height.saturating_sub(crate::view::results::HEADER + 1)).max(1)
    }

    fn reveal_result(&mut self) {
        let room = self.result_room();
        if let Some(results) = self.results.as_mut() {
            results.reveal(room);
        }
    }

    pub(super) fn results_fetches(&mut self) -> Vec<Effect> {
        let Some(results) = self.results.as_ref() else {
            return vec![Effect::Redraw];
        };
        let (top, rows) = (results.top, results.view.rows);
        let end = (top + self.result_room()).min(rows);
        let mut effects = Vec::new();
        for position in top..end {
            if self.ctl.result_row(position).is_none() {
                let said = self.ctl.results_wanted(position);
                effects.extend(self.carry_out(said));
            }
        }
        effects.push(Effect::Redraw);
        effects
    }

    pub(super) fn in_results_command(&self, command: CommandId) -> bool {
        self.results.is_some()
            && self.focus == Focus::List
            && !NOT_DRAWN.contains(&command)
            && self.ctl.answers(command)
    }

    pub(super) fn results_context(&self) -> Option<KeyContext> {
        (self.results.is_some() && self.focus == Focus::List).then_some(KeyContext::Results)
    }

    pub(super) fn bar_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Some(bar) = self.bar.as_mut() else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Down | KeyCode::Up => {
                let by = if key.code == KeyCode::Down { 1 } else { -1 };
                let token = bar.step(by);
                let dropdown = matches!(bar.shown(), Shown::Dropdown(_));
                return match token {
                    Some(token) if dropdown => self.tell(Told::SearchHighlighted(token)),
                    _ => vec![Effect::Redraw],
                };
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                return match bar.selected() {
                    Some(token) => self.tell(Told::BarRun(token)),
                    None => self.tell(Told::Command(CommandId::ShowAllResults)),
                };
            }
            KeyCode::Tab if key.modifiers.is_empty() => return self.tell(Told::BarTab),
            _ => {}
        }
        match self.keys.press(key, KeyContext::Search, true) {
            Outcome::Command(id) => {
                return match id.parse::<CommandId>() {
                    Ok(command) => self.tell(Told::Command(command)),
                    Err(_) => Vec::new(),
                };
            }
            Outcome::Pending(_) => return Vec::new(),
            Outcome::Unhandled => {}
        }
        let Some(bar) = self.bar.as_mut() else {
            return Vec::new();
        };
        if bar.type_in(key) {
            let text = bar.typed().to_owned();
            return self.tell(Told::Typed { text });
        }
        vec![Effect::Redraw]
    }

    pub(super) fn bar_click(&mut self, index: usize) -> Vec<Effect> {
        let token = self
            .bar
            .as_ref()
            .and_then(|bar| bar.stops().get(index).copied());
        token.map_or_else(Vec::new, |token| self.tell(Told::BarRun(token)))
    }

    pub(super) fn bar_saved(&mut self, index: usize) -> Vec<Effect> {
        let command = postio_ui::command_bar::SAVED.get(index).copied();
        command.map_or_else(Vec::new, |command| self.tell(Told::Command(command)))
    }

    pub(super) fn bar_run(&mut self) -> Vec<Effect> {
        let token = self.bar.as_ref().and_then(Bar::selected);
        token.map_or_else(Vec::new, |token| self.tell(Told::BarRun(token)))
    }

    pub(super) fn bar_wheel(&mut self, down: bool) -> Vec<Effect> {
        if let Some(bar) = self.bar.as_mut() {
            bar.step(if down { 3 } else { -3 });
        }
        vec![Effect::Redraw]
    }

    pub(super) fn result_click(&mut self, position: u64) -> Vec<Effect> {
        self.tell(Told::ResultsPoint(position))
    }

    pub(super) fn result_tab(&mut self, index: usize) -> Vec<Effect> {
        let tab = self
            .results
            .as_ref()
            .and_then(|results| results.view.tabs.get(index))
            .map(|tab| tab.tab);
        tab.map_or_else(Vec::new, |tab| self.tell(Told::ResultsTab(tab)))
    }

    pub(super) fn result_chip(&mut self, token: u32) -> Vec<Effect> {
        self.tell(Told::SearchEdit(postio_focus::TermEdit::Remove { token }))
    }

    pub(super) fn result_wheel(&mut self, down: bool) -> Vec<Effect> {
        let room = self.result_room();
        if let Some(results) = self.results.as_mut() {
            let last = results.view.rows.saturating_sub(room);
            results.top = if down {
                (results.top + 3).min(last)
            } else {
                results.top.saturating_sub(3)
            };
        }
        self.results_fetches()
    }

    fn open_result(&mut self, message: postio_model::MessageId) -> Vec<Effect> {
        let found = self
            .results
            .as_ref()
            .and_then(|results| self.ctl.result_row(results.cursor))
            .filter(|row| row.message == message);
        let Some(found) = found else {
            return Vec::new();
        };
        let text = |text: &str| postio_ui::terminal::SafeText::new(text);
        let row = crate::row::Row {
            id: message,
            thread: found.thread,
            is_thread: false,
            kind: crate::row::Kind::Message,
            from: text(&found.sender),
            address: found.from_address.clone(),
            subject: text(
                &found
                    .subject
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
            ),
            preview: text(
                &found
                    .passage
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>(),
            ),
            when: found.at,
            unread: false,
            attachment: found.attachments,
            count: 1,
            marker: None,
            labels: Vec::new(),
            send_state: None,
        };
        self.open_for_itself(row, Focus::List)
    }
}
