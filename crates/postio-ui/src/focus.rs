//! Where the keyboard goes: Tab's walk across the panes, and the way back
//! out of a surface it went into.
//!
//! The classic app's window owned both — a match table for the pane cycle and
//! four `before_*` cells remembering the context to restore when the
//! keyboard leaves the folder list, the parts panel or a settings list —
//! and the macOS app re-derived the cycle in its own `Pane.next()`. Both
//! are keyboard policy with no toolkit in them, and both are here now so
//! there is one answer to "what does Tab do" and one to "where does Esc
//! put the keyboard back".

use postio_core::Context;

/// Where the keyboard was before it went into a nested surface — the folder
/// list, the parts panel, a list in settings — so leaving puts it back where
/// it was rather than guessing.
///
/// One record per surface, because surfaces nest: the keyboard can go from
/// the list into the parts panel and from there into the folders, and
/// leaving each has to restore the one before it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Returns {
    /// `(surface, the context that had the keyboard before it)`.
    entries: Vec<(Context, Context)>,
}

impl Returns {
    /// The keyboard is going into `surface` from `current`.
    ///
    /// Returns the context to set — `surface` — or `None` when the keyboard
    /// is already there, in which case nothing is recorded: a repeated
    /// entry (a focus controller firing for a child widget, say) must not
    /// overwrite the real way back with `surface` itself.
    pub fn enter(&mut self, surface: Context, current: Context) -> Option<Context> {
        if current == surface {
            return None;
        }
        self.entries.retain(|(recorded, _)| *recorded != surface);
        self.entries.push((surface, current));
        Some(surface)
    }

    /// The keyboard is leaving `surface` while `current` owns it.
    ///
    /// Returns the context to restore — where the keyboard was before it
    /// went in, or the list when nothing recorded it — or `None` when the
    /// keyboard was not in `surface` to begin with, so a leave event for a
    /// surface that never took the keyboard changes nothing.
    pub fn leave(&mut self, surface: Context, current: Context) -> Option<Context> {
        if current != surface {
            return None;
        }
        let recorded = self
            .entries
            .iter()
            .position(|(recorded, _)| *recorded == surface)
            .map(|index| self.entries.remove(index).1);
        Some(recorded.unwrap_or(Context::List))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entering_a_surface_records_the_way_back() {
        let mut returns = Returns::default();
        assert_eq!(
            returns.enter(Context::Sidebar, Context::Reader),
            Some(Context::Sidebar)
        );
        assert_eq!(
            returns.leave(Context::Sidebar, Context::Sidebar),
            Some(Context::Reader),
            "Esc puts the keyboard back where it was, not in the list by default"
        );
    }

    #[test]
    fn entering_the_surface_you_are_already_in_changes_nothing() {
        let mut returns = Returns::default();
        returns.enter(Context::Sidebar, Context::Reader);
        assert_eq!(
            returns.enter(Context::Sidebar, Context::Sidebar),
            None,
            "a focus controller firing for a child widget is not a second entry"
        );
        assert_eq!(
            returns.leave(Context::Sidebar, Context::Sidebar),
            Some(Context::Reader),
            "and it must not have overwritten the real way back"
        );
    }

    #[test]
    fn leaving_a_surface_the_keyboard_is_not_in_changes_nothing() {
        let mut returns = Returns::default();
        returns.enter(Context::Sidebar, Context::Reader);
        assert_eq!(returns.leave(Context::Parts, Context::Sidebar), None);
        assert_eq!(
            returns.leave(Context::Sidebar, Context::List),
            None,
            "the keyboard has already moved on; a late leave event is not a restore"
        );
    }

    #[test]
    fn leaving_a_surface_nothing_recorded_falls_back_to_the_list() {
        let mut returns = Returns::default();
        assert_eq!(
            returns.leave(Context::Parts, Context::Parts),
            Some(Context::List)
        );
    }

    #[test]
    fn nested_surfaces_unwind_one_at_a_time() {
        let mut returns = Returns::default();
        returns.enter(Context::Parts, Context::List);
        returns.enter(Context::Sidebar, Context::Parts);
        assert_eq!(
            returns.leave(Context::Sidebar, Context::Sidebar),
            Some(Context::Parts)
        );
        assert_eq!(
            returns.leave(Context::Parts, Context::Parts),
            Some(Context::List)
        );
    }

    #[test]
    fn re_entering_a_surface_from_somewhere_else_records_the_newer_way_back() {
        let mut returns = Returns::default();
        returns.enter(Context::Sidebar, Context::Reader);
        returns.leave(Context::Sidebar, Context::Sidebar);
        returns.enter(Context::Sidebar, Context::List);
        assert_eq!(
            returns.leave(Context::Sidebar, Context::Sidebar),
            Some(Context::List)
        );
    }
}
