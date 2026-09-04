//! ADR 0012 Q4-Q6: showing the first-run keyboard orientation once, after
//! the first successful sync, and remembering that it has been shown.
//!
//! [`postio_gtk::orientation`] owns the strip's shape and its text. This is
//! the half that needs the store — *has this been seen before*, and *when
//! did it first make sense to show at all* — which the view layer may not
//! ask, because it may not link `rusqlite`.
//!
//! # The decision is a state machine, not a pile of `if`s
//!
//! Four things arrive in any order: what the store remembers, the first
//! sync completing, the user's first command, and the dismiss button. The
//! order genuinely varies — a store read is asynchronous and a user can
//! press `j` while it is in flight — so the ordering rules live in
//! [`Orientation`], which has no widget and no database in it and is
//! therefore provable in a unit test rather than only through a window.

/// What the application should do about the strip, right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Nothing has changed that the strip cares about.
    Nothing,
    /// Put it on screen.
    Show,
    /// Take it off screen if it is on, and write down that it is done with.
    ///
    /// One `Retire` is emitted per store, ever: the write behind it happens
    /// once rather than on every command the user runs afterwards.
    Retire,
}

/// When the first-run orientation shows, and when it is over for good.
///
/// ADR 0012 Q6 decides two things this encodes. The trigger that retires it
/// is a *dispatched command*, not a key event — a modifier press or typing
/// into search is not evidence anybody knows about the keyboard system. And
/// a command that arrives before the strip was ever shown retires it all
/// the same: that user has already demonstrated the thing it was going to
/// teach them, so they should never see it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Orientation {
    /// The store has answered whether this was seen before.
    answered: bool,
    /// There is nothing left to show: the store had seen it, or the user
    /// has since demonstrated they know.
    over: bool,
    /// A sync has completed in this session.
    synced: bool,
    /// It is on screen.
    showing: bool,
}

impl Orientation {
    /// What the store remembers: `true` if some earlier run showed it.
    pub fn remembered(&mut self, seen: bool) -> Effect {
        self.answered = true;
        self.over |= seen;
        self.appear()
    }

    /// A sync completed — the first moment there is mail to navigate.
    pub fn synced(&mut self) -> Effect {
        self.synced = true;
        self.appear()
    }

    /// The user dismissed it, or ran their first command. Either way they
    /// are done with it and so is every later run.
    pub fn retire(&mut self) -> Effect {
        if self.over {
            return Effect::Nothing;
        }
        self.over = true;
        self.showing = false;
        Effect::Retire
    }

    /// Show it, if everything it was waiting for has now happened.
    ///
    /// Both callers go through this rather than deciding for themselves,
    /// because either of them can be the last to arrive: the store read is
    /// asynchronous, so a small mailbox can finish syncing before it
    /// answers, and a large one after.
    fn appear(&mut self) -> Effect {
        if self.over || self.showing || !self.answered || !self.synced {
            return Effect::Nothing;
        }
        self.showing = true;
        Effect::Show
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_waits_for_the_first_sync_before_appearing() {
        let mut orientation = Orientation::default();
        assert_eq!(
            orientation.remembered(false),
            Effect::Nothing,
            "before a sync there is no mail to navigate, so `j` refers to nothing"
        );
        assert_eq!(orientation.synced(), Effect::Show);
    }

    #[test]
    fn a_store_that_has_already_seen_it_never_shows_it_again() {
        let mut orientation = Orientation::default();
        assert_eq!(orientation.remembered(true), Effect::Nothing);
        assert_eq!(
            orientation.synced(),
            Effect::Nothing,
            "shown once, ever -- across runs, not merely within one"
        );
    }

    #[test]
    fn a_sync_that_lands_before_the_store_answers_still_shows_it() {
        let mut orientation = Orientation::default();
        assert_eq!(
            orientation.synced(),
            Effect::Nothing,
            "nothing is shown on a guess: the store has not answered yet"
        );
        assert_eq!(orientation.remembered(false), Effect::Show);
    }

    #[test]
    fn a_command_before_it_ever_appeared_retires_it_for_good() {
        let mut orientation = Orientation::default();
        orientation.remembered(false);
        assert_eq!(
            orientation.retire(),
            Effect::Retire,
            "ADR 0012 Q6: someone who pressed a key first has already \
             demonstrated what this was going to teach them"
        );
        assert_eq!(orientation.synced(), Effect::Nothing);
    }

    #[test]
    fn it_is_written_down_once_however_many_commands_follow() {
        let mut orientation = Orientation::default();
        orientation.remembered(false);
        assert_eq!(orientation.synced(), Effect::Show);
        assert_eq!(orientation.retire(), Effect::Retire);
        assert_eq!(
            orientation.retire(),
            Effect::Nothing,
            "every later command would otherwise be another write"
        );
    }

    #[test]
    fn a_second_sync_does_not_bring_it_back() {
        let mut orientation = Orientation::default();
        orientation.remembered(false);
        assert_eq!(orientation.synced(), Effect::Show);
        assert_eq!(
            orientation.synced(),
            Effect::Nothing,
            "the status feed reports every pass; the strip is already up"
        );
    }
}
