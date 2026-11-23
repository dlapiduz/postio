//! `[keys]` reloaded, and the key map. The controller holds the one resolver a frontend presses
//! keys through; `Input::Keymap` changes every key it spells at once. While `?` has the key map
//! up the keyboard is its: every other key is swallowed rather than acting on the list behind.

use postio_core::{CommandId, Frontend, Keymap};
use postio_ui::keymap::{Chord, KeyContext, Outcome, Resolver};

use crate::feed::Step;
use crate::{FocusController, Intent, SurfaceKind};

#[derive(Debug, Default)]
pub(crate) struct Keys {
    resolver: Option<Resolver>,
}

fn resolver(keymap: &Keymap) -> Resolver {
    let (resolver, problems) = Resolver::from_commands_for(keymap, Frontend::Focus);
    for problem in &problems {
        tracing::warn!(%problem, "a key binding was not honoured");
    }
    resolver
}

impl FocusController {
    /// Does not run what it resolves.
    pub fn press(
        &mut self,
        chord: &Chord,
        context: KeyContext,
        in_text_entry: bool,
        now: std::time::Instant,
    ) -> Outcome {
        let keymap = self.bar.keymap();
        self.keys
            .resolver
            .get_or_insert_with(|| resolver(keymap))
            .press(chord, context, in_text_entry, now)
    }

    /// A sequence half-typed under the old keys is dropped.
    pub(crate) fn set_keymap(&mut self, keymap: Keymap) -> Vec<Step> {
        self.bar.set_keymap(keymap);
        self.keys.resolver = None;
        let mut steps = self.redraw_bar();
        steps.extend(self.redraw_picker());
        steps.extend(self.said_again());
        steps
    }

    pub(crate) fn open_key_map(&mut self) -> Vec<Step> {
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::KeyMap, self.policy.caps.stacking);
        steps.push(Step::Show(Intent::OpenKeyMap));
        steps
    }
}

pub(crate) fn key_map_takes(id: CommandId) -> bool {
    !matches!(id, CommandId::Quit | CommandId::CloseWindow)
}
