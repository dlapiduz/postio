//! Whose footer is up (#2218).
//!
//! The footer speaks for the surface that has the keyboard: the cursor's
//! row on the list, or the message shown in the window. The composer,
//! Settings, the capture sheet, the command bar and a row's picker draw
//! none -- their keys are on their own controls, and nothing's scope needs
//! saying there. The rest of the controller says the list's footer as the
//! cursor moves; this decides, once at the end of every input, whether
//! that footer is the one on screen, and says the right one when the owner
//! changes.

use postio_ui::scope::FooterView;

use crate::cursor::Rows;
use crate::surfaces::SurfaceKind;
use crate::{Effect, FocusController, Intent, thread_expand};

/// The surfaces whose keys are on their own controls, with the keyboard in
/// their fields: no footer is drawn behind them, since the list's and the
/// window's keys type letters there. The command bar and a row's picker
/// teach their own keys in their own foot.
const OWN_KEYS: [SurfaceKind; 5] = [
    SurfaceKind::Composer,
    SurfaceKind::Settings,
    SurfaceKind::Capture,
    SurfaceKind::Bar,
    SurfaceKind::Picker,
];

/// Who the footer speaks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Owner {
    /// The list: its cursor's row, or Filtered's focused one.
    #[default]
    List,
    /// The message window, on the message it shows.
    Window,
    /// A surface whose keys are its own controls': no footer is drawn.
    Nobody,
}

/// The footer of a window that walks no thread, said once when it opened:
/// a message alone, or a draft on its way.
#[derive(Debug, Clone)]
pub(crate) struct Lone {
    /// The message shown.
    pub(crate) message: postio_model::MessageId,
    /// What its footer says.
    pub(crate) footer: FooterView,
}

/// What the window's footer and the list's need between inputs.
#[derive(Debug, Default)]
pub(crate) struct Footer {
    /// Who the footer was said for last.
    owner: Owner,
    /// What was said while a surface owned it, so it is said once.
    said: Option<FooterView>,
    /// The window on a message that walks no thread.
    pub(crate) lone: Option<Lone>,
}

impl FocusController {
    /// Who the footer speaks for now.
    fn footer_owner(&self) -> (Owner, Option<FooterView>) {
        if OWN_KEYS.iter().any(|kind| self.surfaces.has(*kind)) {
            return (Owner::Nobody, Some(FooterView::default()));
        }
        match self.window_footer() {
            Some(footer) => (Owner::Window, Some(footer)),
            None => (Owner::List, None),
        }
    }

    /// The footer of the message window, when one is up and knows what it
    /// shows: its walk's, or the one it opened with.
    fn window_footer(&self) -> Option<FooterView> {
        if !self.surfaces.has(SurfaceKind::Message) {
            return None;
        }
        // Only the footer of the message on screen: a removal under the
        // window steps it on to the next row without a walk of its own,
        // and the last walk's footer would name the message just removed.
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

    /// The list's footer: Filtered's focused row, or the cursor's.
    fn list_footer(&self, raw: &dyn Rows) -> Option<FooterView> {
        let keymap = self.keymap();
        if let Some(filtered) = &self.filtered {
            return filtered.footer_at(filtered.focused()? as u32, keymap);
        }
        let position = self.cursor.position()?;
        thread_expand::footer_at(self.expansion.as_ref(), position, raw, keymap)
    }

    /// Make the footer in `effects` the owner's: a surface's own replaces
    /// whatever the list said under it, and when the owner changes, the new
    /// owner's is said.
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
