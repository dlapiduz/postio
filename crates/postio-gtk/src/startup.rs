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
    /// The window and its widget tree exist, but nothing is on screen yet.
    Window,
    /// The compositor has shown the window. Pixels, but no mail in them yet.
    ///
    /// New in #1114, and the reason the rest of this list moved: Postio
    /// presents its window *before* it opens its store, so "there is a window
    /// on screen" and "there is a mailbox in it" are two different moments
    /// with, on a real install, tens of seconds between them.
    Shell,
    /// The store key is out of the keyring and the database is open.
    ///
    /// I/O, and on a thread of its own since #1114 — a D-Bus round trip to
    /// the keyring, the store's key derivation, the schema migrations and the
    /// search-index rebuild, none of which the main loop waits for any more.
    /// Separated from [`Window`](Phase::Window) by #790, which found the two
    /// of them sharing one 228 ms phase that `docs/PERFORMANCE.md` then
    /// attributed to GTK.
    Store,
    /// The keyring has answered and the window is about to be pointed at the
    /// store: `postio_app::feed_the_window` has been entered.
    ///
    /// Everything between [`Window`](Phase::Window) and here is the crossing
    /// `postio_app::open_or_onboard` describes — a D-Bus round trip to the
    /// keyring, asked on the runtime and answered back on the main context.
    /// It is the one part of this stretch that is *not* the main thread's
    /// own work, and telling it apart from what follows is the whole reason
    /// it is a phase.
    Account,
    /// The panes are pointed at the store and every gesture has a handler:
    /// `postio_app::feed_the_window` has returned.
    ///
    /// Synchronous main-thread work, all of it, and therefore work the first
    /// frame is waiting on. #1479 split this off because the trace could say
    /// the first frame was 84% of a 1250 ms startup and not say what any of
    /// it was — and the answer turned out to be a store read in here rather
    /// than anything GTK was doing.
    Feeds,
    /// The compositor has shown a frame with the mail in it. This is "usable
    /// UI", and it is what the budget is measured against.
    ///
    /// Not the same as [`Shell`](Phase::Shell) since #1114. The window
    /// arrives first and is fed afterwards, so a start that reached pixels in
    /// 200 ms and mail in twelve seconds took twelve seconds to be usable —
    /// and a budget that closed at the first frame would call it a pass.
    FirstFrame,
}

impl Phase {
    /// Every phase, in the order they occur.
    pub const ALL: [Phase; 9] = [
        Phase::Init,
        Phase::Fonts,
        Phase::Styles,
        Phase::Window,
        Phase::Shell,
        Phase::Store,
        Phase::Account,
        Phase::Feeds,
        Phase::FirstFrame,
    ];

    /// The name used in [`Timeline::report`].
    pub fn label(self) -> &'static str {
        match self {
            Phase::Init => "init",
            Phase::Fonts => "fonts",
            Phase::Styles => "styles",
            Phase::Window => "window",
            Phase::Shell => "shell",
            Phase::Store => "store",
            Phase::Account => "account",
            Phase::Feeds => "feeds",
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
    /// How long the process ran before the timeline started: exec, the
    /// dynamic loader, static initialisers -- when it could be read.
    before_main: Option<Duration>,
    marks: RefCell<[Option<Duration>; Phase::ALL.len()]>,
}

impl Timeline {
    /// Start a timeline now. Call it as the first statement in `main`: the
    /// budget is measured from process start, and this is as close as a Rust
    /// program gets to it without reading `/proc`.
    pub fn start() -> Self {
        Self::start_after(Instant::now(), this_process_age())
    }

    /// Start a timeline from an origin you already have.
    pub fn start_at(origin: Instant) -> Self {
        Self::start_after(origin, None)
    }

    /// Start a timeline at `origin`, knowing the process had already run for
    /// `before_main` by then.
    pub fn start_after(origin: Instant, before_main: Option<Duration>) -> Self {
        Timeline(Rc::new(Inner {
            origin,
            before_main,
            marks: RefCell::new([None; Phase::ALL.len()]),
        }))
    }

    /// Record that `phase` has been reached.
    ///
    /// The first mark for a phase wins, so a retried activation — GTK will
    /// activate a running application again when it is launched a second
    /// time — does not overwrite the startup that was actually measured.
    ///
    /// And it is logged, at info, every launch (#1604): a mark used to record
    /// an instant and say nothing, so the journal of a slow start held two
    /// lines for its first second and the time could not be attributed after
    /// the fact. Numbers and a phase name only, which is all a log may carry.
    pub fn mark(&self, phase: Phase) {
        let elapsed = self.0.origin.elapsed();
        {
            let slot = &mut self.0.marks.borrow_mut()[phase.index()];
            if slot.is_some() {
                return;
            }
            *slot = Some(elapsed);
        }
        let cost = self.cost(phase).unwrap_or_default();
        // Every line carries what ran before `main` -- exec, the loader
        // relocating a few hundred shared objects -- which the timeline's own
        // origin cannot see (#1604); 0 where it could not be read.
        tracing::info!(
            phase = phase.label(),
            at_ms = elapsed.as_millis() as u64,
            cost_ms = cost.as_millis() as u64,
            before_main_ms = self
                .0
                .before_main
                .map_or(0, |before| before.as_millis() as u64),
            "startup phase"
        );
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
                "startup {}{} ({phases}) budget {} — {}",
                millis(total),
                self.0
                    .before_main
                    .map(|before| format!(" after {} before main", millis(before)))
                    .unwrap_or_default(),
                millis(BUDGET),
                if total <= BUDGET { "ok" } else { "OVER" }
            ),
            None => format!("startup incomplete ({phases})"),
        }
    }
}

/// How long ago this process started, from `/proc/self/stat`'s start time
/// and `/proc/uptime`: `None` where either cannot be read or parsed.
pub fn process_age(stat: &str, uptime: &str) -> Option<Duration> {
    // Fields are counted after the *last* `)`: the command name inside the
    // brackets may itself hold spaces and brackets. The start time is field
    // 22 of the line, the 20th after the name, in clock ticks -- which the
    // kernel reports to user space at 100 a second on every Linux, whatever
    // the internal tick rate.
    let after_name = &stat[stat.rfind(')')? + 1..];
    let started_ticks: u64 = after_name.split_whitespace().nth(19)?.parse().ok()?;
    let up: f64 = uptime.split_whitespace().next()?.parse().ok()?;
    let up_ms = (up * 1000.0).round() as u64;
    let started_ms = started_ticks.checked_mul(10)?;
    up_ms.checked_sub(started_ms).map(Duration::from_millis)
}

/// [`process_age`] of this process, now.
fn this_process_age() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let uptime = std::fs::read_to_string("/proc/uptime").ok()?;
    process_age(&stat, &uptime)
}

/// Run `f` once, on the first frame after `widget` reaches the screen.
///
/// The hook is a tick callback added when the widget is mapped and removed
/// again as soon as it runs, so it costs one frame's worth of bookkeeping and
/// then nothing at all. It fires within a frame of the moment the compositor
/// first shows the window — as close to "the user can see it" as the frame
/// clock can say.
pub fn on_first_frame<W: IsA<gtk::Widget>>(widget: &W, f: impl Fn() + 'static) {
    let pending = Rc::new(RefCell::new(Some(f)));
    // **A widget that is already mapped never emits `map` again.** The
    // measurement path registers this while building the window, so it never
    // met that; a caller that hangs real work off the first frame does --
    // `activate` handlers run in registration order, and the one that
    // presents the window runs first. Hooked that way, the work would simply
    // never run, and the failure is silent: no account opens and nothing says
    // why.
    fn arm<F: Fn() + 'static>(widget: &gtk::Widget, pending: &Rc<RefCell<Option<F>>>) {
        let Some(f) = pending.borrow_mut().take() else {
            return;
        };
        widget.add_tick_callback(move |_, _| {
            f();
            glib::ControlFlow::Break
        });
    }
    if widget.is_mapped() {
        arm(widget.as_ref(), &pending);
        return;
    }
    widget.connect_map(move |widget| arm(widget.as_ref(), &pending));
}

/// Close the timeline once `window` is showing mail, and act on the
/// benchmarking switches documented in this module.
///
/// Called by whoever fed the window, which is the composition root — not by
/// `app::build_with`, which cannot know. Before #1114 the two were the same
/// moment and this lived there; now the window is presented first and fed
/// afterwards, so the frame that closes the budget is the one after the panes
/// were pointed at the store.
///
/// A start that never gets a store never calls this, and the timeline stays
/// open. That is the honest answer — there is no usable UI to have reached —
/// and it is visible rather than silent: [`Timeline::report`] says `startup
/// incomplete` and names the phases that did happen.
pub fn report_usable<W: IsA<gtk::Window>>(window: &W, timeline: &Timeline) {
    let timeline = timeline.clone();
    let window = window.as_ref().clone();
    let quitting = window.clone();
    on_first_frame(&window, move || {
        timeline.mark(Phase::FirstFrame);
        if enabled(TRACE_ENV) {
            // Through tracing rather than straight to stderr, so it is
            // filtered and formatted like everything else. `POSTIO_LOG=off`
            // now silences it, which is the correct reading of `off`; the
            // benchmark path is `POSTIO_STARTUP_EXIT` and does not read this.
            tracing::info!("{}", timeline.report());
        }
        if enabled(EXIT_ENV)
            && let Some(application) = gtk::prelude::GtkWindowExt::application(&quitting)
        {
            application.quit();
        }
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
    fn the_store_is_its_own_phase_and_no_longer_precedes_the_window() {
        // #790: `window` measured 228ms of a 427ms startup and
        // `docs/PERFORMANCE.md` attributed it to GTK's first-realize cost.
        // It cannot be that -- `present()` is called *after* `Phase::Window`
        // is marked, so the shader compile lands in `first frame`. What
        // actually sat in that gap was the blocking keyring read and the
        // store open, which is I/O. It got its own phase so the trace could
        // say which.
        //
        // #1114 then moved that I/O off the main thread and behind the
        // window, which is what this half of the assertion is: the store now
        // opens *after* there are pixels, so a list in the old order would
        // have `report` computing every cost from a phase that had not
        // happened yet.
        assert!(Phase::Window < Phase::Shell);
        assert!(Phase::Shell < Phase::Store);
        assert_eq!(Phase::Store.label(), "store");
        assert_eq!(Phase::Shell.label(), "shell");
        for phase in [Phase::Store, Phase::Shell] {
            assert!(
                Phase::ALL.contains(&phase),
                "a phase nothing reports is a phase nothing measures"
            );
        }
    }

    #[test]
    fn pixels_and_usable_are_two_different_moments() {
        // The distinction #1114 creates, and the one a budget cannot be
        // allowed to blur: a start that put a window on screen in 200ms and
        // mail in it twelve seconds later took twelve seconds to be usable.
        // `total` has to be the second of those or the budget passes every
        // launch it exists to catch.
        let origin = Instant::now();
        let timeline = Timeline::start_at(origin);
        timeline.mark(Phase::Shell);
        assert_eq!(
            timeline.total(),
            None,
            "pixels are not a usable UI, and the budget has no verdict yet"
        );
        assert_eq!(timeline.within_budget(), None);

        timeline.mark(Phase::FirstFrame);
        assert!(timeline.total().is_some());
        assert!(
            Phase::Shell < Phase::FirstFrame,
            "and the window is on screen before the mail is in it"
        );
    }

    #[test]
    fn the_first_frame_gap_is_split_into_what_waits_and_what_works() {
        // #1479: `first frame` measured 1044ms of a 1250ms startup on a real
        // store, and the trace could say which phase and not what. Everything
        // in that gap is local -- `start_syncing` runs *after* it -- so what
        // it wanted was telling apart the three things it holds: a keyring
        // round trip that is not this thread's work, the synchronous main
        // thread work that points the panes at the store, and GTK's own
        // paint. One phase could not, and three can.
        assert!(Phase::Window < Phase::Account);
        assert!(Phase::Account < Phase::Feeds);
        assert!(Phase::Feeds < Phase::FirstFrame);
        assert_eq!(Phase::Account.label(), "account");
        assert_eq!(Phase::Feeds.label(), "feeds");
        assert!(
            Phase::ALL.contains(&Phase::Account) && Phase::ALL.contains(&Phase::Feeds),
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

    /// Log lines written while `body` runs, as text.
    fn logged(body: impl FnOnce()) -> String {
        #[derive(Clone, Default)]
        struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Captured {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().expect("not poisoned").extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
            type Writer = Captured;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }
        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, body);
        let bytes = captured.0.lock().expect("not poisoned").clone();
        String::from_utf8(bytes).expect("utf-8")
    }

    #[test]
    fn a_process_s_age_is_the_uptime_less_its_start() {
        // A name with spaces and a closing bracket in it: the fields are
        // counted from the *last* `)`, or a hostile name shifts them all.
        let stat = "4242 (postio (main) x) S 1 4242 4242 0 -1 4194560 100 0 0 0 \
                    5 3 0 0 20 0 8 0 12345 0 0 18446744073709551615";
        assert_eq!(
            process_age(stat, "124.50 900.00\n"),
            Some(Duration::from_millis(1_050)),
            "started at tick 12345 (123.45 s), up 124.50 s"
        );
        assert_eq!(process_age("nonsense", "124.5 1"), None);
        assert_eq!(
            process_age(stat, "123.00 1"),
            None,
            "a start after now is unreadable"
        );
    }

    #[test]
    fn a_phase_says_what_ran_before_main() {
        // #1604: the ~215 ms of exec and relocation before `main` was outside
        // the timeline, so no launch's journal could say it had happened.
        let timeline = Timeline::start_after(Instant::now(), Some(Duration::from_millis(215)));
        let log = logged(|| {
            timeline.mark(Phase::Init);
            timeline.mark(Phase::Window);
        });
        let lines: Vec<&str> = log
            .lines()
            .filter(|line| line.contains("startup phase"))
            .collect();
        assert!(
            lines[0].contains("before_main_ms=215"),
            "the first phase line does not say what ran before main: {}",
            lines[0]
        );
    }

    #[test]
    fn every_phase_reaches_the_log_with_its_cost() {
        // #1604: the journal of a real launch had two lines in its first
        // second, because a mark recorded an instant and said nothing, and the
        // report was printed only under a debugging switch. A slow start could
        // not be attributed after the fact. Every mark now says which phase,
        // when, and what it cost -- once, the first time, like the mark.
        let timeline = Timeline::start();
        let log = logged(|| {
            timeline.mark(Phase::Init);
            timeline.mark(Phase::Window);
            timeline.mark(Phase::Window);
        });
        let lines: Vec<&str> = log
            .lines()
            .filter(|line| line.contains("startup phase"))
            .collect();
        assert_eq!(
            lines.len(),
            2,
            "one line per phase reached, and a second mark says nothing: {log}"
        );
        assert!(
            lines[1].contains(Phase::Window.label()) && lines[1].contains("cost_ms="),
            "the line names the phase and its cost: {}",
            lines[1]
        );
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
