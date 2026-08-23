//! The PLATE layout on a real display: the canvas' proportions, the three
//! adaptive modes, and the promise that switching between them costs nothing.
//!
//! One test function, in order, for the reason `gtk_style.rs` gives: GTK is
//! single-threaded and initialised once. Without a display it skips. Nothing
//! here touches the network.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::shell::{self, Mode, Pane};
use postio_gtk::state::WindowState;
use postio_gtk::{app, fonts, header, resources, style, window::Window};

#[test]
fn the_plate_layout_matches_the_canvas() {
    // Before any glib call: `g_get_user_state_dir` caches its answer, and a
    // test has no business writing into the developer's real state directory.
    let state_dir = std::env::temp_dir().join(format!("postio-shell-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    assert!(
        WindowState::path().starts_with(&state_dir),
        "the test should be writing to its own state directory, not {}",
        WindowState::path().display()
    );

    // ── the proportions ───────────────────────────────────────────────────
    let window = Window::default();
    let shell = window.shell();
    window.present();
    pump();

    assert_eq!(
        shell.divider_positions(),
        (shell::SIDEBAR_WIDTH, shell::LIST_WIDTH),
        "a first run should open at the canvas' proportions"
    );
    assert_eq!(shell.sidebar().width(), shell::SIDEBAR_WIDTH);
    assert_eq!(shell.list().width(), shell::LIST_WIDTH);
    assert!(
        shell.reader().width() > shell.list().width(),
        "the reader takes the slack: {} vs {}",
        shell.reader().width(),
        shell.list().width()
    );

    // Widening the window must not steal the list's width — the reader
    // absorbs it.
    let list_before = shell.list().width();
    window.set_default_size(1400, 700);
    pump();
    assert_eq!(
        shell.list().width(),
        list_before,
        "resizing the window must not move a divider the user set"
    );

    // ── the modes, and the promise that they are free ─────────────────────
    assert_eq!(shell.mode(), Mode::ThreePane);
    assert!(shell.sidebar().is_visible());

    // No pumping between the call and the assertion: a pane switch takes
    // effect in the same turn of the loop, which is what "no transition"
    // means when you write it down.
    shell.set_mode(Mode::TwoPane);
    assert!(!shell.sidebar().is_visible(), "the sidebar goes at once");
    assert!(shell.list().is_visible());
    assert!(shell.reader().is_visible());

    shell.set_mode(Mode::MessageFocused);
    assert!(!shell.sidebar().is_visible());
    assert!(
        shell.list().is_visible(),
        "the list holds the screen by default"
    );
    assert!(!shell.reader().is_visible());

    shell.set_focused_pane(Pane::Reader);
    assert!(!shell.list().is_visible());
    assert!(
        shell.reader().is_visible(),
        "drill-in swaps the pane, instantly"
    );

    shell.set_mode(Mode::ThreePane);
    assert!(
        shell.sidebar().is_visible(),
        "widening brings the sidebar back"
    );
    assert!(shell.list().is_visible());
    assert!(shell.reader().is_visible());

    // The sidebar stays reachable in the narrow modes: collapsed means "not
    // by default", never "not available".
    shell.set_mode(Mode::MessageFocused);
    shell.set_sidebar_visible(true);
    assert!(shell.sidebar().is_visible());

    // ── the state that survives a restart ─────────────────────────────────
    shell.set_mode(Mode::ThreePane);
    shell.set_divider_positions(240, 380);
    shell.set_sidebar_visible(false);
    window.save_state();

    let reopened = Window::default();
    assert_eq!(
        reopened.shell().divider_positions(),
        (240, 380),
        "a dragged divider should survive a restart"
    );
    assert!(!reopened.shell().sidebar_visible());
    reopened.destroy();

    // ── the header bar, at the canvas' measurements ───────────────────────
    // Measured in the window the app actually opens, because that is the only
    // place the canvas' numbers mean anything.
    let root = window.upcast_ref::<gtk::Widget>();
    let bar = find(root, &|w| w.is::<adw::HeaderBar>()).expect("the window has a header bar");
    assert_eq!(
        bar.measure(gtk::Orientation::Vertical, -1).0,
        52,
        "the canvas' header bar is 52px tall, hairline included"
    );

    // The field's cap is a font metric (see `header::SEARCH_WIDTH_CHARS`), so
    // what is worth pinning is the width it settles at with room to spare.
    //
    // Measured as natural-minus-margin, which is the box the canvas draws:
    // `width()` would report the *content* box, inside the padding and the
    // hairline, and the canvas' 600px includes both.
    let field = find(root, &|w| w.has_css_class("postio-search")).expect("the search field");
    let (minimum, natural, _, _) = field.measure(gtk::Orientation::Horizontal, -1);
    let width = natural - field.margin_start();
    assert!(
        (width - header::SEARCH_MAX_WIDTH).abs() <= 6,
        "the search field should settle at the canvas' {}px, not {width}px — \
         retune SEARCH_WIDTH_CHARS",
        header::SEARCH_MAX_WIDTH
    );
    assert!(
        minimum - field.margin_start() <= 200,
        "the field has to shrink for the message-focused mode"
    );

    window.destroy();
}

/// Depth-first search of a widget tree.
fn find(widget: &gtk::Widget, wanted: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if wanted(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, wanted) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

/// The motion budget, read back out of the stylesheet that has to keep it.
///
/// CLAUDE.md: transitions are <= 100ms or absent, and pane switches have none
/// at all. This is cheap to check and easy to break by pasting in a rule from
/// somewhere else.
#[test]
fn nothing_in_the_stylesheet_outruns_the_motion_budget() {
    let bytes = resources::read(resources::SHELL_CSS).expect("the bundle carries shell.css");
    let css = String::from_utf8(bytes.to_vec()).expect("shell.css is UTF-8");

    for (number, line) in css.lines().enumerate() {
        let line = line.trim();
        if line.starts_with("/*") || line.starts_with('*') || !line.contains("transition") {
            continue;
        }
        for duration in durations(line) {
            assert!(
                duration <= 100.0,
                "shell.css:{}: a {duration}ms transition is over the motion budget: {line}",
                number + 1
            );
        }
    }

    // The panes specifically get none, at any duration.
    for selector in [
        ".postio-shell",
        ".postio-sidebar",
        ".postio-list",
        ".postio-reader",
    ] {
        for block in blocks(&css, selector) {
            assert!(
                !block.contains("transition"),
                "`{selector}` must not animate: a pane switch is instant"
            );
        }
    }
}

/// Every duration in a declaration, in milliseconds.
fn durations(line: &str) -> Vec<f64> {
    let mut out = Vec::new();
    for token in line.split([' ', ',', ':', ';']) {
        if let Some(ms) = token.strip_suffix("ms")
            && let Ok(value) = ms.parse::<f64>()
        {
            out.push(value);
        } else if let Some(seconds) = token.strip_suffix('s')
            && let Ok(value) = seconds.parse::<f64>()
        {
            out.push(value * 1000.0);
        }
    }
    out
}

/// The body of every rule whose selector list mentions `selector`.
fn blocks<'a>(css: &'a str, selector: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        let (head, tail) = rest.split_at(open);
        let Some(close) = tail.find('}') else { break };
        if head
            .rsplit('}')
            .next()
            .is_some_and(|s| s.contains(selector))
        {
            out.push(&tail[1..close]);
        }
        rest = &tail[close + 1..];
    }
    out
}

fn pump() {
    for _ in 0..80 {
        glib::MainContext::default().iteration(false);
    }
}
