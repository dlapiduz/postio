//! Startup timeline: marks each `Phase` so the bench harness can read where startup time goes.
//! `POSTIO_STARTUP_TRACE=1` prints the report at first frame; `POSTIO_STARTUP_EXIT=1` quits then,
//! for hyperfine.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;

pub const BUDGET: Duration = Duration::from_millis(500);

pub const TRACE_ENV: &str = "POSTIO_STARTUP_TRACE";

pub const EXIT_ENV: &str = "POSTIO_STARTUP_EXIT";

/// Variant order is occurrence order; `Timeline::report` relies on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Init,
    Fonts,
    Styles,
    Window,
    /// Pixels, but no mail yet: the window is presented before the store opens (#1114).
    Shell,
    /// Runs on its own thread: keyring D-Bus round trip, key derivation, migrations, index rebuild.
    Store,
    /// The keyring has answered. Its D-Bus round trip is not main-thread work, which is why this is
    /// a phase.
    Account,
    /// Synchronous main-thread work that the first frame waits on.
    Feeds,
    /// "Usable UI", where the budget ends: mail is on screen, not just the window (#1114).
    FirstFrame,
}

impl Phase {
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

/// When each `Phase` was reached; clones share one timeline.
#[derive(Clone)]
pub struct Timeline(Rc<Inner>);

struct Inner {
    origin: Instant,
    /// Exec and loader time before `main`; `None` if unreadable.
    before_main: Option<Duration>,
    marks: RefCell<[Option<Duration>; Phase::ALL.len()]>,
}

impl Timeline {
    /// Call as the first statement in `main`: the budget runs from process start.
    pub fn start() -> Self {
        Self::start_after(Instant::now(), this_process_age())
    }

    #[cfg(test)]
    pub fn start_at(origin: Instant) -> Self {
        Self::start_after(origin, None)
    }

    pub fn start_after(origin: Instant, before_main: Option<Duration>) -> Self {
        Timeline(Rc::new(Inner {
            origin,
            before_main,
            marks: RefCell::new([None; Phase::ALL.len()]),
        }))
    }

    /// The first mark per phase wins, so a re-activation does not overwrite it. Each mark is logged
    /// at info.
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
        // Includes pre-`main` time (exec, loader) that the origin cannot see; 0 if unreadable
        // (#1604).
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

    pub fn at(&self, phase: Phase) -> Option<Duration> {
        self.0.marks.borrow()[phase.index()]
    }

    /// The gap from the previous marked phase, or from the start.
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

    pub fn total(&self) -> Option<Duration> {
        self.at(Phase::FirstFrame)
    }

    #[cfg(test)]
    pub fn within_budget(&self) -> Option<bool> {
        Some(self.total()? <= BUDGET)
    }

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

/// `None` where either input cannot be parsed.
pub fn process_age(stat: &str, uptime: &str) -> Option<Duration> {
    // Fields count after the last `)`: the command name may hold spaces and brackets.
    // Start time is field 22, in 100 Hz ticks on every Linux.
    let after_name = &stat[stat.rfind(')')? + 1..];
    let started_ticks: u64 = after_name.split_whitespace().nth(19)?.parse().ok()?;
    let up: f64 = uptime.split_whitespace().next()?.parse().ok()?;
    let up_ms = (up * 1000.0).round() as u64;
    let started_ms = started_ticks.checked_mul(10)?;
    up_ms.checked_sub(started_ms).map(Duration::from_millis)
}

fn this_process_age() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let uptime = std::fs::read_to_string("/proc/uptime").ok()?;
    process_age(&stat, &uptime)
}

/// The tick callback removes itself after it runs.
pub fn on_first_frame<W: IsA<gtk::Widget>>(widget: &W, f: impl Fn() + 'static) {
    let pending = Rc::new(RefCell::new(Some(f)));
    // An already-mapped widget never emits `map` again, and the `activate` handler that presents
    // the window runs first; without this the work silently never runs.
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

/// Called by the composition root once the window is fed. A start with no store never calls it,
/// and `Timeline::report` then says `startup incomplete`.
pub fn report_usable<W: IsA<gtk::Window>>(window: &W, timeline: &Timeline) {
    let timeline = timeline.clone();
    let window = window.as_ref().clone();
    let quitting = window.clone();
    on_first_frame(&window, move || {
        timeline.mark(Phase::FirstFrame);
        if enabled(TRACE_ENV) {
            // Through tracing so `POSTIO_LOG` filters it; the benchmark path is
            // `POSTIO_STARTUP_EXIT`.
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

/// Strict: only `1` counts, so `POSTIO_STARTUP_EXIT=0` in a profile cannot quit the app.
pub fn enabled(var: &str) -> bool {
    switched_on(std::env::var(var).ok().as_deref())
}

fn switched_on(value: Option<&str>) -> bool {
    value == Some("1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_is_its_own_phase_and_no_longer_precedes_the_window() {
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
        // `total` must mean mail in the window, not first pixels, or the budget passes every slow
        // launch.
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

    fn logged(body: impl FnOnce()) -> String {
        use postio_test_support::logs::Captured;
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
        // Fields count from the last `)`.
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
        // Styles, not Fonts: the gap runs from the last mark.
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
        // Read from the value, not by setting the variable: setting one is
        // `unsafe`, which this crate forbids.
        assert!(!switched_on(None));
        assert!(!switched_on(Some("0")));
        assert!(!switched_on(Some("true")));
        assert!(switched_on(Some("1")));
    }
}
