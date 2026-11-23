//! The pickers, driven by the controller: its intents become the box, and
//! keys and clicks go back as inputs.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use postio_core::CommandId;
use postio_focus::{Input as Told, Intent, PickerField, PickerKind, SurfaceKind};
use postio_ui::keymap::{KeyContext, Outcome};
use postio_ui::pickers as rules;
use tui_input::backend::crossterm::EventHandler;

use super::{App, Effect, Focus};
use crate::pickers::Picker;

/// Rows the wheel moves the keyboard by.
const WHEEL: usize = 3;

impl App {
    pub(super) fn show_picker_intent(&mut self, intent: &Intent) -> Option<Vec<Effect>> {
        match intent {
            Intent::OpenPicker(view) => {
                let from = match self.focus {
                    Focus::Picker => Focus::List,
                    focus => focus,
                };
                self.picker = Some(Picker::opened(view.clone(), from));
                self.focus = Focus::Picker;
            }
            Intent::PickerRows(view) => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.show(view.clone());
                }
            }
            Intent::PickerField => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.into_field();
                }
            }
            Intent::CloseSurface(SurfaceKind::Picker) => {
                if let Some(picker) = self.picker.take() {
                    self.focus = picker.from();
                }
            }
            Intent::DraftReminder { at } => {
                let Some(composer) = self.composer.as_mut() else {
                    return Some(vec![Effect::Redraw]);
                };
                composer.set_remind_at(Some(*at));
                return Some(self.edited());
            }
            _ => return None,
        }
        Some(vec![Effect::Redraw])
    }

    pub fn picker(&self) -> Option<&Picker> {
        self.picker.as_ref()
    }

    pub(super) fn open_picker(&mut self, command: CommandId) -> Vec<Effect> {
        self.tell(Told::Command(command))
    }

    pub fn open_remind_picker_for_draft(&mut self, target: &str) -> Vec<Effect> {
        let said = self.ctl.open_draft_reminder(target);
        let mut effects = self.carry_out(said);
        effects.push(Effect::Redraw);
        effects
    }

    pub(super) fn picker_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Some(picker) = self.picker.as_mut() else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Down => picker.step(true, 1),
            KeyCode::Up => picker.step(false, 1),
            _ => {
                let filter_empty =
                    picker.view().field == PickerField::Filter && picker.typed().is_empty();
                let bare = (key.modifiers - KeyModifiers::SHIFT).is_empty();
                let digit_or_space =
                    matches!(key.code, KeyCode::Char(c) if c.is_ascii_digit() || c == ' ');
                let typing =
                    rules::is_typing(picker.in_field(), filter_empty, bare, digit_or_space);
                if let Outcome::Command(id) = self.keys.press(key, KeyContext::Picker, typing) {
                    return match id.parse::<CommandId>() {
                        Ok(command) => self.picker_command(command),
                        Err(_) => Vec::new(),
                    };
                }
                if !picker.in_field() {
                    return vec![Effect::Redraw];
                }
                // Enter in a field confirms what is typed.
                if key.code == KeyCode::Enter {
                    return self.picker_command(CommandId::PickerConfirm);
                }
                let before = picker.typed().to_owned();
                picker
                    .field_mut()
                    .handle_event(&crossterm::event::Event::Key(*key));
                if picker.typed() != before {
                    let text = picker.typed().to_owned();
                    return self.tell(Told::PickerTyped { text });
                }
            }
        }
        vec![Effect::Redraw]
    }

    pub(super) fn picker_command(&mut self, command: CommandId) -> Vec<Effect> {
        let Some(picker) = self.picker.as_ref() else {
            return Vec::new();
        };
        let token = picker.chosen_row().map(|row| row.token);
        let dated = matches!(picker.kind(), PickerKind::Snooze | PickerKind::Remind);
        let typed = !picker.typed().trim().is_empty();
        let input = match command {
            CommandId::PickerToggle => token.map(Told::PickerToggle),
            CommandId::PickerConfirm if dated && picker.in_field() && typed => {
                Some(Told::Command(command))
            }
            CommandId::PickerConfirm if picker.kind() == PickerKind::Label => {
                Some(Told::Command(command))
            }
            CommandId::PickerConfirm => token.map(Told::PickerChoose),
            _ => Some(Told::Command(command)),
        };
        let mut effects = input.map_or_else(Vec::new, |input| self.tell(input));
        effects.push(Effect::Redraw);
        effects
    }

    pub(super) fn picker_click(&mut self, index: usize) -> Vec<Effect> {
        let token = self
            .picker
            .as_ref()
            .and_then(|picker| picker.row(index))
            .map(|row| row.token);
        token.map_or_else(Vec::new, |token| self.tell(Told::PickerChoose(token)))
    }

    pub(super) fn picker_wheel(&mut self, down: bool) -> Vec<Effect> {
        if let Some(picker) = self.picker.as_mut() {
            picker.step(down, WHEEL);
        }
        vec![Effect::Redraw]
    }
}
