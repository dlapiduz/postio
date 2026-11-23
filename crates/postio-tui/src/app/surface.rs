//! What a key, a click or the wheel does on one of Focus's surfaces: the
//! part says which surface it belongs to, and the controller decides.

use postio_core::CommandId;
use postio_focus::{Input as Told, Intent};

use super::{App, Effect, Focus};
use crate::surface::Part;

impl App {
    pub(super) fn surface_click(&mut self, part: Part, index: usize) -> Vec<Effect> {
        match part {
            Part::FilteredTab | Part::FilteredRow => self.filtered_click(part, index),
            Part::ConfirmCancel | Part::ConfirmYes => self.confirm_click(part),
            Part::DigestTab | Part::DigestRow | Part::DigestReference => {
                self.digest_click(part, index)
            }
            Part::RuleRow | Part::RemoveCancel | Part::RemoveConfirm => {
                self.rules_click(part, index)
            }
            Part::RuleField | Part::RuleCancel | Part::RuleCreate => self.rule_click(part, index),
            Part::CaptureMode
            | Part::CapturePick
            | Part::CaptureProject
            | Part::CaptureText
            | Part::CaptureCancel => self.capture_click(part, index),
        }
    }

    pub(super) fn show_surface_intent(
        &mut self,
        intent: Intent,
    ) -> Result<Vec<Effect>, Box<Intent>> {
        let intent = match self.show_filtered_intent(intent) {
            Err(intent) => *intent,
            done => return done,
        };
        let intent = match self.show_digest_intent(intent) {
            Err(intent) => *intent,
            done => return done,
        };
        let intent = match self.show_rule_intent(intent) {
            Err(intent) => *intent,
            done => return done,
        };
        let intent = match self.show_capture_intent(intent) {
            Err(intent) => *intent,
            done => return done,
        };
        self.show_confirm_intent(intent)
    }

    pub(super) fn surface_wheel(&mut self, lines: isize) -> bool {
        if self.surfaces.digest.is_some() {
            self.digest_wheel(lines);
            return true;
        }
        if self.surfaces.rules.is_some() {
            let height = self.rules_height();
            if let Some(list) = self.surfaces.rules.as_mut() {
                list.scroll(lines, height);
            }
            return true;
        }
        if self.surfaces.filtered.is_some() {
            self.filtered_wheel(lines);
            return true;
        }
        false
    }

    pub(super) fn surface_command(&mut self, id: &str) -> Vec<Effect> {
        if matches!(id, "undo" | "cheat_sheet") {
            return self.global(id);
        }
        let Ok(command) = id.parse::<CommandId>() else {
            return Vec::new();
        };
        // Filtered draws no conversation opened inside it.
        if matches!(command, CommandId::ExpandThread | CommandId::CollapseThread) {
            return Vec::new();
        }
        if self.focus == Focus::Filtered && id.starts_with("go_to_") && id != "go_to_filtered" {
            let mut effects = self.tell(Told::Command(CommandId::GoToInbox));
            let inbox_open = matches!(self.scope, Some(postio_model::ListScope::Focus(_)));
            if id != "go_to_inbox" || !inbox_open {
                effects.extend(self.command(id));
            }
            return effects;
        }
        self.tell(Told::Command(command))
    }

    pub(super) fn global(&mut self, id: &str) -> Vec<Effect> {
        match id {
            "cheat_sheet" => self.open_keys(),
            _ => self.send(id),
        }
    }
}
