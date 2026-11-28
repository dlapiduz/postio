//! The key-map sheet: the controller says when it opens, and a close that
//! starts here goes to it as Back.

use crossterm::event::KeyEvent;
use postio_focus::{Input as Told, Intent, SurfaceKind};

use super::{App, Effect, Focus};

impl App {
    pub(super) fn show_sheet_intent(&mut self, intent: &Intent) -> Option<Vec<Effect>> {
        Some(match intent {
            Intent::OpenKeyMap => self.open_keys(),
            Intent::CloseSurface(SurfaceKind::KeyMap) => {
                if let Some(sheet) = self.sheet.take() {
                    self.focus = sheet.from();
                }
                vec![Effect::Redraw]
            }
            _ => return None,
        })
    }

    pub(crate) fn open_keys(&mut self) -> Vec<Effect> {
        let from = match (&self.sheet, self.focus) {
            (Some(open), _) => open.from(),
            (None, focus) => focus,
        };
        self.sheet = Some(crate::sheet::Sheet::open(from));
        self.focus = Focus::Keys;
        vec![Effect::Redraw]
    }

    pub(super) fn keys_key(&mut self, key: &KeyEvent) -> Vec<Effect> {
        let (max, page) = self.sheet_limits();
        let Some(sheet) = self.sheet.as_mut() else {
            return Vec::new();
        };
        match sheet.key(key, &mut self.keys, page, max) {
            crate::sheet::Step::Stay => vec![Effect::Redraw],
            crate::sheet::Step::Close => self.close_keys(),
        }
    }

    pub(super) fn close_keys(&mut self) -> Vec<Effect> {
        // A sheet another surface opened for itself is not on the controller's stack.
        if self.ctl.top_surface() == Some(SurfaceKind::KeyMap) {
            return self.tell(Told::Command(postio_core::CommandId::Back));
        }
        if let Some(sheet) = self.sheet.take() {
            self.focus = sheet.from();
        }
        vec![Effect::Redraw]
    }
}
