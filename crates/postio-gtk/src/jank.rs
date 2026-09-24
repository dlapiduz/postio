//! Frames that miss their budget, and what was asked for just before.
//!
//! CLAUDE.md's budget is 16 ms per interaction, and the counts that gate a
//! pull request are the *causes* of that budget -- statements, rows, scans --
//! because a shared runner cannot time anything. What they cannot say is
//! which keystroke, on this machine, with this mailbox, took 40 ms. This can:
//!
//! ```text
//! POSTIO_LOG=postio_gtk::jank=debug cargo run -p postio-app
//! ```
//!
//! Two kinds of stall, because they are caught in different places:
//!
//! - **A frame**: the frame clock's `update` to its `after-paint`, which is
//!   animation ticks, layout and snapshot -- what GTK itself spent drawing.
//! - **The main loop**: a heartbeat that expects to run every
//!   [`HEARTBEAT`] and notes how late it was. A key handler that reads the
//!   store synchronously blocks here, between frames, where the frame clock
//!   never sees it.
//!
//! Each is attributed to the last action dispatched within
//! [`ATTRIBUTION_WINDOW`], by name -- which is what makes a line in the log
//! something a person can act on.
//!
//! Off unless that target is enabled when the window is built, and then it
//! costs nothing: no heartbeat wakes an idle machine, no signal handler is
//! connected. Retuning `[logging]` live does not install it on a running
//! window; restart with the filter set.

use std::cell::{Cell, RefCell};
use std::fmt::Display;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

/// What one frame, or one turn of the main loop, may take.
pub const FRAME_BUDGET: Duration = Duration::from_millis(16);

/// How often the main-loop heartbeat expects to run.
pub const HEARTBEAT: Duration = Duration::from_millis(8);

/// How long after an action a stall is still blamed on it.
///
/// Long enough to cover an action's own frame and the repaint it causes, and
/// short enough that a stall a second later is not pinned on a keystroke that
/// had long finished.
pub const ATTRIBUTION_WINDOW: Duration = Duration::from_millis(500);

/// Where a stall was seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Drawing: the frame clock's update to after-paint.
    Frame,
    /// Between frames: a handler held the main loop.
    MainLoop,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Frame => "frame",
            Kind::MainLoop => "main loop",
        }
    }
}

/// One stall over budget, and what it is blamed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stall {
    /// Where it was seen.
    pub kind: Kind,
    /// How long it took.
    pub took: Duration,
    /// The last action dispatched before it, if one was recent enough to
    /// blame, and how long before the stall ended it was.
    pub after: Option<(String, Duration)>,
}

/// The action last dispatched on this thread, and when.
#[derive(Debug, Default)]
pub struct LastAction(RefCell<Option<(String, Instant)>>);

impl LastAction {
    /// Note that `action` was just dispatched.
    pub fn note(&self, action: impl Display, at: Instant) {
        *self.0.borrow_mut() = Some((action.to_string(), at));
    }

    /// Whether `took`, ending at `now`, is a stall -- and if so, what it is
    /// blamed on.
    ///
    /// `None` when it was inside [`FRAME_BUDGET`]. An action is blamed only if
    /// it came within [`ATTRIBUTION_WINDOW`] of `now`; an older one would be
    /// a guess dressed as a finding.
    pub fn judge(&self, kind: Kind, took: Duration, now: Instant) -> Option<Stall> {
        if took <= FRAME_BUDGET {
            return None;
        }
        let after = self.0.borrow().as_ref().and_then(|(action, at)| {
            let since = now.checked_duration_since(*at)?;
            (since <= ATTRIBUTION_WINDOW).then(|| (action.clone(), since))
        });
        Some(Stall { kind, took, after })
    }
}

thread_local! {
    static LAST: LastAction = LastAction::default();
    static PROBING: Cell<bool> = const { Cell::new(false) };
}

/// Note that `action` was just dispatched, for a stall that follows it.
///
/// Free when the probe is off: nothing is formatted.
pub fn note_action(action: impl Display) {
    if PROBING.with(Cell::get) {
        LAST.with(|last| last.note(action, Instant::now()));
    }
}

fn report(kind: Kind, took: Duration) {
    let Some(stall) = LAST.with(|last| last.judge(kind, took, Instant::now())) else {
        return;
    };
    let took_ms = stall.took.as_secs_f64() * 1000.0;
    match &stall.after {
        Some((action, since)) => tracing::debug!(
            kind = stall.kind.as_str(),
            took_ms,
            action = action.as_str(),
            after_ms = since.as_secs_f64() * 1000.0,
            "over the frame budget"
        ),
        None => tracing::debug!(kind = stall.kind.as_str(), took_ms, "over the frame budget"),
    }
}

/// Watch `widget`'s frames and the main loop, if `postio_gtk::jank` is
/// enabled at debug.
pub fn install<W: IsA<gtk::Widget>>(widget: &W) {
    if !tracing::enabled!(tracing::Level::DEBUG) {
        return;
    }
    PROBING.with(|probing| probing.set(true));

    let began: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
    let watch = move |widget: &gtk::Widget| {
        let Some(clock) = widget.frame_clock() else {
            return;
        };
        let start = began.clone();
        clock.connect_update(move |_| start.set(Some(Instant::now())));
        let end = began.clone();
        clock.connect_after_paint(move |_| {
            if let Some(start) = end.take() {
                report(Kind::Frame, start.elapsed());
            }
        });
    };
    if widget.is_realized() {
        watch(widget.as_ref());
    } else {
        // Once: a frame clock belongs to the surface, which a realize makes.
        let done = Cell::new(false);
        widget.connect_realize(move |widget| {
            if !done.replace(true) {
                watch(widget.upcast_ref());
            }
        });
    }

    let last = Cell::new(Instant::now());
    glib::timeout_add_local(HEARTBEAT, move || {
        let now = Instant::now();
        let late = now
            .duration_since(last.replace(now))
            .saturating_sub(HEARTBEAT);
        report(Kind::MainLoop, late);
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_frame_inside_the_budget_is_not_a_stall() {
        let last = LastAction::default();
        assert_eq!(last.judge(Kind::Frame, ms(16), Instant::now()), None);
    }

    #[test]
    fn a_stall_is_blamed_on_the_action_just_before_it() {
        let last = LastAction::default();
        let pressed = Instant::now();
        last.note("archive", pressed);
        let stall = last
            .judge(Kind::MainLoop, ms(40), pressed + ms(45))
            .expect("40 ms is over budget");
        assert_eq!(stall.kind, Kind::MainLoop);
        assert_eq!(stall.took, ms(40));
        assert_eq!(stall.after, Some(("archive".to_owned(), ms(45))));
    }

    #[test]
    fn an_old_action_is_not_blamed() {
        let last = LastAction::default();
        let pressed = Instant::now();
        last.note("archive", pressed);
        let stall = last
            .judge(Kind::Frame, ms(30), pressed + ATTRIBUTION_WINDOW + ms(1))
            .expect("30 ms is over budget");
        assert_eq!(stall.after, None, "a stall long after is not the key's");
    }

    #[test]
    fn a_stall_with_no_action_at_all_is_still_reported() {
        let last = LastAction::default();
        let stall = last.judge(Kind::Frame, ms(20), Instant::now());
        assert_eq!(
            stall.map(|stall| stall.after),
            Some(None),
            "an unprompted slow frame is the one nobody would otherwise see"
        );
    }
}
