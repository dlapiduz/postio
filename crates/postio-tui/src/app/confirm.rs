//! The controller's questions: Return says yes, Escape and a click outside say no.

use crossterm::event::KeyEvent;
use postio_focus::{Confirm, Input as Told, Intent};
use postio_ui::keymap::Outcome;

use super::{App, Effect};
use crate::surface::Part;

impl App {
    pub fn confirm(&self) -> Option<&Confirm> {
        self.surfaces.confirm.as_ref()
    }

    pub(super) fn show_confirm_intent(
        &mut self,
        intent: Intent,
    ) -> Result<Vec<Effect>, Box<Intent>> {
        match intent {
            Intent::Confirm(question) => self.surfaces.confirm = Some(question),
            other => return Err(Box::new(other)),
        }
        Ok(vec![Effect::Redraw])
    }

    pub(super) fn confirm_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Outcome::Command(id) = self.keys.press(key, self.key_context(), false) else {
            return Vec::new();
        };
        match id.as_str() {
            "open_message" => self.answer_confirm(true),
            "back" => self.answer_confirm(false),
            _ => Vec::new(),
        }
    }

    pub(super) fn confirm_click(&mut self, part: Part) -> Vec<Effect> {
        self.answer_confirm(part == Part::ConfirmYes)
    }

    fn answer_confirm(&mut self, yes: bool) -> Vec<Effect> {
        let Some(question) = self.surfaces.confirm.take() else {
            return Vec::new();
        };
        let mut effects = if yes {
            self.tell(Told::Confirmed(question.token))
        } else {
            Vec::new()
        };
        effects.push(Effect::Redraw);
        effects
    }
}
