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

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gtk::glib;
use postio_gtk::feed::Feeds;
use postio_gtk::window::Window;
use postio_session::Wiring;
use postio_storage::repository::SettingsRepository;

/// The `settings` row this feature owns.
///
/// Global, never per account: ADR 0012 Q6 — a second account joining a
/// running installation must not teach the keyboard again.
const SEEN_KEY: &str = "orientation_seen";

/// Wire the strip to the sync engine, to the store, and to every command
/// the window runs.
pub fn install(window: &Window, wiring: &Wiring, feeds: &Feeds) {
    let state = Rc::new(RefCell::new(Orientation::default()));

    // Has some earlier run already shown it? The answer is in SQLite, so it
    // arrives asynchronously — which is exactly why [`Orientation`] takes
    // its four inputs in any order rather than assuming this one is first.
    let answer = crate::search::ask(&wiring.database, &wiring.runtime, |connection| {
        SettingsRepository::new(connection).get(SEEN_KEY).ok()
    });
    glib::spawn_future_local(glib::clone!(
        #[weak]
        window,
        #[strong]
        wiring,
        #[strong]
        state,
        async move {
            // A read that failed outright is treated as *already seen*. The
            // write is on the same store, so a strip shown against a broken
            // read is one the user could dismiss and meet again tomorrow —
            // worse than one that quietly never appears.
            let seen = !matches!(answer.recv().await, Ok(Some(None)));
            let effect = state.borrow_mut().remembered(seen);
            act(&window, &wiring, effect);
        }
    ));

    // The first successful sync, and every one after it: `Orientation` is
    // what knows the difference, so this needs no "have I already" flag of
    // its own. `last_sync` is `Some` from the moment a list pass completes.
    feeds.folders.connect_status(glib::clone!(
        #[weak]
        window,
        #[strong]
        wiring,
        #[strong]
        state,
        move |status| {
            if status.last_sync.is_some() {
                let effect = state.borrow_mut().synced();
                act(&window, &wiring, effect);
            }
        }
    ));

    // Retiring is the window's to notice and this crate's to write down:
    // ADR 0012 Q6 counts a command run from the keyboard or the palette and
    // counts neither a click nor a modifier press, and the only layer that
    // can tell those apart is the one resolving the keys. It fires once,
    // whether or not the strip was ever on screen.
    window.orientation().connect_retired(glib::clone!(
        #[weak]
        window,
        #[strong]
        wiring,
        #[strong]
        state,
        move || {
            let effect = state.borrow_mut().retire();
            act(&window, &wiring, effect);
        }
    ));
}

/// Carry out what the state machine decided.
fn act(window: &Window, wiring: &Wiring, effect: Effect) {
    match effect {
        Effect::Nothing => {}
        Effect::Show => window.orientation().set_visible(true),
        Effect::Retire => {
            window.orientation().set_visible(false);
            remember(wiring);
        }
    }
}

/// Write down that this installation is done with the orientation.
///
/// The value is when, rather than `"true"`: a row that says only that
/// something happened is a row nobody can ever debug, and the column is a
/// string either way. Spawned rather than awaited — ADR 0012 Q4 asks for a
/// strip that does not block the list, and that includes not blocking it on
/// the way out.
fn remember(wiring: &Wiring) {
    let database = wiring.database.clone();
    wiring.runtime.spawn_blocking(move || {
        let Ok(connection) = database.connection() else {
            return;
        };
        if let Err(error) =
            SettingsRepository::new(&connection).set(SEEN_KEY, &Utc::now().to_rfc3339())
        {
            tracing::warn!(%error, "could not remember that the orientation was seen");
        }
    });
}

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
