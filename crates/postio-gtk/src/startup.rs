//! Startup instrumentation.
//!
//! CLAUDE.md budgets startup to **under 500 ms** from process start to a
//! usable UI on a populated database. A budget nobody measures is a wish, so
//! the app records where its startup time goes and hands the numbers to
//! whoever asks: a [`Timeline`] is created as the first statement in `main`,
//! marked at each [`Phase`], and read back by the bench harness (E1.10).
//!
//! Two environment variables make it usable from a shell or a benchmark
//! without a debugger attached:
//!
//! * `POSTIO_STARTUP_TRACE=1` prints [`Timeline::report`] to stderr once the
//!   first frame is on screen.
//! * `POSTIO_STARTUP_EXIT=1` quits the application at that same moment, so
//!   `hyperfine 'POSTIO_STARTUP_EXIT=1 postio'` measures exactly the interval
//!   the budget is written against — process start to first frame.
//!
//! Nothing here allocates on a hot path or touches a clock the UI depends on:
//! it is a handful of [`Instant`]s.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

/// The startup budget from CLAUDE.md: process start to a usable UI.
pub const BUDGET: Duration = Duration::from_millis(500);

/// Set to `1` to print the timeline to stderr once the first frame is up.
pub const TRACE_ENV: &str = "POSTIO_STARTUP_TRACE";

/// Set to `1` to quit as soon as the first frame is up, for benchmarking.
pub const EXIT_ENV: &str = "POSTIO_STARTUP_EXIT";

/// The milestones between `main` and a window the user can act on.
///
/// The order of the variants is the order they happen in, and
/// [`Timeline::report`] leans on that.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    /// GTK and libadwaita are up and a display is open.
    Init,
    /// The embedded font faces are registered — before the first widget.
    Fonts,
    /// The generated tokens are installed on the display.
    Styles,
    /// The store key is out of the keyring and the database is open.
    ///
    /// Blocking I/O, and the only phase that is: a D-Bus round trip to the
    /// keyring and SQLCipher's key derivation, both before the main loop
    /// starts. Separated from [`Window`](Phase::Window) by #790, which found
    /// the two of them sharing one 228 ms phase that `docs/PERFORMANCE.md`
    /// then attributed to GTK. They want telling apart: this half is I/O and
    /// could be moved off the main thread, and the other half cannot.
    Store,
    /// The window and its widget tree exist, but nothing is on screen yet.
    Window,
    /// The compositor has shown the first frame. This is "usable UI".
    FirstFrame,
}

impl Phase {
    /// Every phase, in the order they occur.
    pub const ALL: [Phase; 6] = [
        Phase::Init,
        Phase::Fonts,
        Phase::Styles,
        Phase::Store,
        Phase::Window,
        Phase::FirstFrame,
    ];

    /// The name used in [`Timeline::report`].
    pub fn label(self) -> &'static str {
        match self {
            Phase::Init => "init",
            Phase::Fonts => "fonts",
            Phase::Styles => "styles",
            Phase::Store => "store",
            Phase::Window => "window",
            Phase::FirstFrame => "first frame",
        }
    }

    fn index(self) -> usize {
        Phase::ALL.iter().position(|p| *p == self).unwrap()
    }
}

/// When each [`Phase`] was reached, relative to when the timeline started.
///
/// Cheap to clone — the closures that mark it are scattered across the
/// application's `startup`, `activate` and tick callbacks, and they all share
/// one timeline.
#[derive(Clone)]
pub struct Timeline(Rc<Inner>);

struct Inner {
    origin: Instant,
    marks: RefCell<[Option<Duration>; Phase::ALL.len()]>,
}

impl Timeline {
    /// Start a timeline now. Call it as the first statement in `main`: the
    /// budget is measured from process start, and this is as close as a Rust
    /// program gets to it without reading `/proc`.
    pub fn start() -> Self {
        Self::start_at(Instant::now())
    }

    /// Start a timeline from an origin you already have.
    pub fn start_at(origin: Instant) -> Self {
        Timeline(Rc::new(Inner {
            origin,
            marks: RefCell::new([None; Phase::ALL.len()]),
        }))
    }

    /// Record that `phase` has been reached.
    ///
    /// The first mark for a phase wins, so a retried activation — GTK will
    /// activate a running application again when it is launched a second
    /// time — does not overwrite the startup that was actually measured.
    pub fn mark(&self, phase: Phase) {
        let elapsed = self.0.origin.elapsed();
        let slot = &mut self.0.marks.borrow_mut()[phase.index()];
        if slot.is_none() {
            *slot = Some(elapsed);
        }
    }

    /// How long after the start `phase` was reached, if it has been.
    pub fn at(&self, phase: Phase) -> Option<Duration> {
        self.0.marks.borrow()[phase.index()]
    }

    /// How long `phase` itself took: the gap from the previous phase that was
    /// marked, or from the start if it is the first.
    pub fn cost(&self, phase: Phase) -> Option<Duration> {
        let at = self.at(phase)?;
        let previous = Phase::ALL
            .iter()
            .take(phase.index())
            .filter_map(|p| self.at(*p))
            .next_back()
            .unwrap_or_default();
        Some(at.saturating_sub(previous))
    }

    /// Start to usable UI: the whole point of the exercise.
    pub fn total(&self) -> Option<Duration> {
        self.at(Phase::FirstFrame)
    }

    /// Whether startup came in under [`BUDGET`]. `None` until the first frame.
    pub fn within_budget(&self) -> Option<bool> {
        Some(self.total()? <= BUDGET)
    }

    /// One line, meant for a terminal and for grepping out of a bench log.
    pub fn report(&self) -> String {
        let phases: Vec<String> = Phase::ALL
            .iter()
            .filter_map(|p| Some(format!("{} {}", p.label(), millis(self.cost(*p)?))))
            .collect();
        let phases = if phases.is_empty() {
            "nothing marked".to_string()
        } else {
            phases.join(" · ")
        };

        match self.total() {
            Some(total) => format!(
                "startup {} ({phases}) budget {} — {}",
                millis(total),
                millis(BUDGET),
                if total <= BUDGET { "ok" } else { "OVER" }
            ),
            None => format!("startup incomplete ({phases})"),
        }
    }
}

/// Run `f` once, on the first frame after `widget` reaches the screen.
///
/// The hook is a tick callback added when the widget is mapped and removed
/// again as soon as it runs, so it costs one frame's worth of bookkeeping and
/// then nothing at all. It fires within a frame of the moment the compositor
/// first shows the window — as close to "the user can see it" as the frame
/// clock can say.
pub fn on_first_frame<W: IsA<gtk::Widget>>(widget: &W, f: impl Fn() + 'static) {
    let pending = RefCell::new(Some(f));
    widget.connect_map(move |widget| {
        let Some(f) = pending.borrow_mut().take() else {
            return;
        };
        widget.add_tick_callback(move |_, _| {
            f();
            glib::ControlFlow::Break
        });
    });
}

fn millis(d: Duration) -> String {
    format!("{:.1}ms", d.as_secs_f64() * 1000.0)
}

/// Whether an environment variable is switched on.
///
/// Deliberately strict: only `1` counts, so `POSTIO_STARTUP_EXIT=0` in a shell
/// profile does not quit the application out from under someone.
pub fn enabled(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| v == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_is_its_own_phase_between_styles_and_the_window() {
        // #790: `window` measured 228ms of a 427ms startup and
        // `docs/PERFORMANCE.md` attributed it to GTK's first-realize cost.
        // It cannot be that -- `present()` is called *after* `Phase::Window`
        // is marked, so the shader compile lands in `first frame`. What
        // actually sits in that gap is the blocking keyring read and the
        // SQLCipher store open, which is I/O and can move off the main
        // thread in a way a shader compile never can. It gets its own phase
        // so the trace says which.
        assert!(Phase::Styles < Phase::Store);
        assert!(Phase::Store < Phase::Window);
        assert_eq!(Phase::Store.label(), "store");
        assert!(
            Phase::ALL.contains(&Phase::Store),
            "a phase nothing reports is a phase nothing measures"
        );
    }

    #[test]
    fn phases_are_ordered_the_way_startup_happens() {
        let mut sorted = Phase::ALL;
        sorted.sort();
        assert_eq!(sorted, Phase::ALL, "Phase::ALL should already be in order");
        assert!(Phase::Init < Phase::FirstFrame);
    }

    #[test]
    fn marks_are_cumulative_and_monotonic() {
        let timeline = Timeline::start();
        timeline.mark(Phase::Init);
        timeline.mark(Phase::Fonts);
        timeline.mark(Phase::FirstFrame);

        let init = timeline.at(Phase::Init).expect("init was marked");
        let fonts = timeline.at(Phase::Fonts).expect("fonts was marked");
        let frame = timeline.total().expect("the first frame was marked");

        assert!(init <= fonts, "marks measure from the same origin");
        assert!(fonts <= frame);
        assert_eq!(timeline.at(Phase::Styles), None, "styles was never marked");
    }

    #[test]
    fn the_first_mark_for_a_phase_wins() {
        let timeline = Timeline::start();
        timeline.mark(Phase::Window);
        let first = timeline.at(Phase::Window).unwrap();
        std::thread::sleep(Duration::from_millis(2));
        timeline.mark(Phase::Window);
        assert_eq!(
            timeline.at(Phase::Window),
            Some(first),
            "a second activation must not overwrite the measured startup"
        );
    }

    #[test]
    fn cost_is_the_gap_from_the_previous_marked_phase() {
        let origin = Instant::now();
        let timeline = Timeline::start_at(origin);
        timeline.mark(Phase::Init);
        std::thread::sleep(Duration::from_millis(5));
        // Styles, not Fonts: the gap should be measured from whatever was
        // marked last, not from a phase that never happened.
        timeline.mark(Phase::Styles);

        let init = timeline.cost(Phase::Init).unwrap();
        let styles = timeline.cost(Phase::Styles).unwrap();
        assert_eq!(timeline.cost(Phase::Fonts), None);
        assert!(styles >= Duration::from_millis(4), "got {styles:?}");
        assert_eq!(
            timeline.at(Phase::Styles).unwrap(),
            init + styles,
            "the costs should add up to the cumulative mark"
        );
    }

    #[test]
    fn the_budget_verdict_needs_a_first_frame() {
        let timeline = Timeline::start();
        assert_eq!(timeline.within_budget(), None);
        assert!(timeline.report().starts_with("startup incomplete"));

        timeline.mark(Phase::FirstFrame);
        assert_eq!(timeline.within_budget(), Some(true));

        let report = timeline.report();
        assert!(report.contains("first frame"), "{report}");
        assert!(report.contains("budget 500.0ms"), "{report}");
        assert!(report.ends_with("ok"), "{report}");
    }

    #[test]
    fn a_slow_startup_reports_over_budget() {
        let origin = Instant::now() - (BUDGET + Duration::from_millis(1));
        let timeline = Timeline::start_at(origin);
        timeline.mark(Phase::FirstFrame);
        assert_eq!(timeline.within_budget(), Some(false));
        assert!(timeline.report().ends_with("OVER"), "{}", timeline.report());
    }

    #[test]
    fn only_one_switches_a_trace_on() {
        // SAFETY: single-threaded test, and the variable is read nowhere else
        // in this process.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("POSTIO_TEST_SWITCH", "0")
        };
        assert!(!enabled("POSTIO_TEST_SWITCH"));
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("POSTIO_TEST_SWITCH", "1")
        };
        assert!(enabled("POSTIO_TEST_SWITCH"));
        #[allow(unsafe_code)]
        unsafe {
            std::env::remove_var("POSTIO_TEST_SWITCH")
        };
        assert!(!enabled("POSTIO_TEST_SWITCH"));
    }
}
