use postio_ui::scope::FooterView;

use crate::cursor::Rows;
use crate::surfaces::SurfaceKind;
use crate::{Effect, FocusController, Intent, thread_expand};

// The list's and the window's keys type letters in their fields or do
// nothing there; their own keys are on their own controls.
const OWN_KEYS: [SurfaceKind; 7] = [
    SurfaceKind::Composer,
    SurfaceKind::Settings,
    SurfaceKind::Capture,
    SurfaceKind::Bar,
    SurfaceKind::Picker,
    SurfaceKind::Outline,
    SurfaceKind::ThreadMenu,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Owner {
    #[default]
    List,
    Window,
    Nobody,
}

#[derive(Debug, Clone)]
pub(crate) struct Lone {
    pub(crate) message: postio_model::MessageId,
    pub(crate) footer: FooterView,
}

#[derive(Debug, Default)]
pub(crate) struct Footer {
    owner: Owner,
    said: Option<FooterView>,
    pub(crate) lone: Option<Lone>,
}

impl FocusController {
    fn footer_owner(&self) -> (Owner, Option<FooterView>) {
        if OWN_KEYS.iter().any(|kind| self.surfaces.has(*kind)) || self.bar.places_shown() {
            return (Owner::Nobody, Some(FooterView::default()));
        }
        match self.window_footer() {
            Some(footer) => (Owner::Window, Some(footer)),
            None => (Owner::List, None),
        }
    }

    fn window_footer(&self) -> Option<FooterView> {
        if !self.surfaces.has(SurfaceKind::Message) {
            return None;
        }
        // A removal steps the window on without a walk of its own; the last
        // walk's footer would name the message just removed.
        let shown = self.surfaces.shown()?;
        let walked = self
            .walk
            .as_ref()
            .filter(|walk| walk.shown == shown)
            .and_then(|_| self.walk_footer());
        walked.or_else(|| {
            let lone = self.footer.lone.as_ref()?;
            (lone.message == shown).then(|| lone.footer.clone())
        })
    }

    fn list_footer(&self, raw: &dyn Rows) -> Option<FooterView> {
        let keymap = self.keymap();
        if let Some(filtered) = &self.filtered {
            return filtered.footer_at(filtered.focused()? as u32, keymap);
        }
        let position = self.cursor.position()?;
        thread_expand::footer_at(self.expansion.as_ref(), position, raw, keymap)
    }

    pub(crate) fn settle_footer(&mut self, effects: &mut Vec<Effect>, raw: &dyn Rows) {
        let (owner, own) = self.footer_owner();
        let was = std::mem::replace(&mut self.footer.owner, owner);
        let Some(own) = own else {
            self.footer.said = None;
            let said = effects
                .iter()
                .any(|effect| matches!(effect, Effect::Show(Intent::Footer(_))));
            if was != Owner::List
                && !said
                && let Some(footer) = self.list_footer(raw)
            {
                effects.push(Effect::Show(Intent::Footer(footer)));
            }
            return;
        };
        let before = effects.len();
        effects.retain(|effect| !matches!(effect, Effect::Show(Intent::Footer(_))));
        let dropped = effects.len() != before;
        if dropped || was != owner || self.footer.said.as_ref() != Some(&own) {
            self.footer.said = Some(own.clone());
            effects.push(Effect::Show(Intent::Footer(own)));
        }
    }
}
