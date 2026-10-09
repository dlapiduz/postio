//! `[keys]` reloaded, and the key map (research R2, slice 11; screen 20).
//!
//! Moved from `postio-gtk`'s window: `set_keymap`, which rebuilt the
//! resolver and handed the keymap to every widget that spells a key, and the
//! CheatSheet toggle. The controller holds the one resolver a frontend
//! presses keys through, built for Focus's commands from the keymap in
//! force, and every key it spells -- the bar's keycaps, a picker's numbers,
//! a banner's button, an empty page's shortcuts -- reads that same keymap.
//! [`crate::Input::Keymap`] changes all of them at once.
//!
//! `?` opens the key map over whatever is up and puts it on the stack; `?`
//! or Back closes it. While it is up the keyboard is its, as GTK's dialog
//! close rule had it: every other key is swallowed rather than acting on the
//! list behind it.

use postio_core::{CommandId, Frontend, Keymap};
use postio_ui::keymap::{Chord, KeyContext, Outcome, Resolver};

use crate::feed::Step;
use crate::{FocusController, Intent, SurfaceKind};

/// The resolver, built from the keymap in force when a key first needs it.
#[derive(Debug, Default)]
pub(crate) struct Keys {
    resolver: Option<Resolver>,
}

/// The resolver Focus presses keys through, for `keymap`; what could not be
/// bound is logged, as GTK's window did.
fn resolver(keymap: &Keymap) -> Resolver {
    let (resolver, problems) = Resolver::from_commands_for(keymap, Frontend::Focus);
    for problem in &problems {
        tracing::warn!(%problem, "a key binding was not honoured");
    }
    resolver
}

impl FocusController {
    /// Resolve one key press in `context` against the keymap in force:
    /// the one resolver both frontends press keys through, rebuilt when
    /// [`crate::Input::Keymap`] says the keys changed. Does not run what it
    /// resolves.
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

    /// Take `keymap` as the keys in force: the resolver is rebuilt on the
    /// next key, a sequence half-typed under the old keys is dropped, and
    /// whatever spells a key that is on screen is said again.
    pub(crate) fn set_keymap(&mut self, keymap: Keymap) -> Vec<Step> {
        self.bar.set_keymap(keymap);
        self.keys.resolver = None;
        let mut steps = self.redraw_bar();
        steps.extend(self.redraw_picker());
        steps.extend(self.said_again());
        steps
    }

    /// `?`: open the key map, over whatever is up.
    pub(crate) fn open_key_map(&mut self) -> Vec<Step> {
        let mut steps = self
            .surfaces
            .opened(SurfaceKind::KeyMap, self.policy.caps.stacking);
        steps.push(Step::Show(Intent::OpenKeyMap));
        steps
    }
}

/// Whether `id` is the key map's to answer while it is up: every key but
/// Quit, which the app answers wherever the keyboard is.
pub(crate) fn key_map_takes(id: CommandId) -> bool {
    id != CommandId::Quit
}
