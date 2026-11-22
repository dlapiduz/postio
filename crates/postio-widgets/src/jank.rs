//! Frames that miss their budget, attributed to the last action dispatched within
//! [`ATTRIBUTION_WINDOW`].
//! Enable with `POSTIO_LOG=postio_widgets::jank=debug` before the window is built; off, it costs
//! nothing, and retuning
//! `[logging]` does not install it on a running window.

use std::cell::{Cell, RefCell};
use std::fmt::Display;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

pub const FRAME_BUDGET: Duration = Duration::from_millis(16);

pub const HEARTBEAT: Duration = Duration::from_millis(8);

pub const ATTRIBUTION_WINDOW: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Frame,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stall {
    pub kind: Kind,
    pub took: Duration,
    pub after: Option<(String, Duration)>,
}

#[derive(Debug, Default)]
pub struct LastAction(RefCell<Option<(String, Instant)>>);

impl LastAction {
    pub fn note(&self, action: impl Display, at: Instant) {
        *self.0.borrow_mut() = Some((action.to_string(), at));
    }

    /// An action is blamed only within [`ATTRIBUTION_WINDOW`]; an older one would be a guess
    /// dressed as a finding.
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
