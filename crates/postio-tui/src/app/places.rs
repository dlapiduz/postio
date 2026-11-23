//! The go-to-folder box, driven by the controller: it lists the places and
//! goes to the one chosen.

use crossterm::event::KeyEvent;
use postio_focus::{Input as Told, Intent};

use super::{App, Effect, Focus};
use crate::folders::{Folders, Step};

impl App {
    pub(super) fn show_places_intent(&mut self, intent: &Intent) -> Option<Vec<Effect>> {
        match intent {
            Intent::OpenPlaces => {
                let from = match (&self.places_box, self.focus) {
                    (Some(open), _) => open.from(),
                    (None, Focus::Palette) => Focus::List,
                    (None, focus) => focus,
                };
                self.places_box = Some(Folders::open(from, self.ctl.places("")));
                self.focus = Focus::Folders;
                let mut effects = self.tell(Told::PlacesShown(true));
                effects.push(Effect::Redraw);
                Some(effects)
            }
            Intent::PlacesChanged => {
                self.relist_places();
                Some(vec![Effect::Redraw])
            }
            // A label or a saved search is a search.
            Intent::OpenBar { text, .. } => Some(self.open_bar(text)),
            _ => None,
        }
    }

    fn relist_places(&mut self) {
        if let Some(open) = self.places_box.as_mut() {
            let entries = self.ctl.places(open.filter());
            open.show(entries);
        }
    }

    pub fn folders(&self) -> Option<&Folders> {
        self.places_box.as_ref()
    }

    pub(super) fn open_folders(&mut self) -> Vec<Effect> {
        self.tell(Told::Command(postio_core::CommandId::GoToFolders))
    }

    pub(super) fn folders_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let Some(open) = self.places_box.as_mut() else {
            return Vec::new();
        };
        let step = open.key(key, &mut self.keys);
        self.folders_step(step)
    }

    pub(super) fn folders_click(&mut self, index: usize) -> Vec<Effect> {
        let step = self.places_box.as_ref().map(|open| open.go(index));
        step.map_or_else(Vec::new, |step| self.folders_step(step))
    }

    pub(super) fn folders_wheel(&mut self, down: bool) -> Vec<Effect> {
        if let Some(open) = self.places_box.as_mut() {
            open.wheel(down);
        }
        vec![Effect::Redraw]
    }

    fn folders_step(&mut self, step: Step) -> Vec<Effect> {
        match step {
            Step::Stay => vec![Effect::Redraw],
            Step::Filter => {
                self.relist_places();
                vec![Effect::Redraw]
            }
            Step::Close => self.close_folders(),
            Step::Go(token) => {
                let mut effects = self.close_folders();
                effects.extend(self.tell(Told::OpenPlace(token)));
                effects
            }
            Step::Run(command) => {
                let mut effects = self.close_folders();
                effects.extend(self.command(command.as_str()));
                effects
            }
        }
    }

    fn close_folders(&mut self) -> Vec<Effect> {
        if let Some(open) = self.places_box.take() {
            self.focus = open.from();
        }
        let mut effects = self.tell(Told::PlacesShown(false));
        effects.push(Effect::Redraw);
        effects
    }
}
