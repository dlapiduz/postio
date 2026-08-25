//! CI must have a display, or the GTK suites are theatre.
//!
//! Sixty test files in this workspace open with some spelling of
//!
//! ```ignore
//! if adw::init().is_err() || gdk::Display::default().is_none() {
//!     eprintln!("skipping: no display");
//!     return;
//! }
//! ```
//!
//! which is right for a contributor on a headless shell and wrong for CI: a
//! runner with no display returns early from every one of them and reports
//! success. `docs/PRODUCT.md` §20 makes accessibility a shipping requirement
//! and the audit that enforces it is one of the sixty, so the guarantee has
//! been unenforced on every push (#114). A green tick meaning "nothing ran"
//! is worse than a red one.
//!
//! This is the guard against that, and it is deliberately *one* test rather
//! than sixty edits. If a display is present in CI, none of the sixty skip;
//! if it is absent, this fails and names the reason. So the property worth
//! asserting is exactly "CI has a display", and it is asserted once.
//!
//! Locally it does nothing. A contributor without a display gets the skips,
//! which is what they are for.

use gtk::gdk;

#[test]
fn ci_has_a_display_to_run_the_gtk_suites_on() {
    // `CI` is set by GitHub Actions, and by essentially every other runner.
    // Absent means a person's machine, where skipping is the correct
    // behaviour and this test has nothing to say.
    if std::env::var_os("CI").is_none() {
        eprintln!("not CI: the display-less skips are correct here");
        return;
    }

    let started = adw::init().is_ok();
    assert!(
        started && gdk::Display::default().is_some(),
        "CI has no display, so every GTK test in this workspace returned \
         early and reported success — including the accessibility audit that \
         docs/PRODUCT.md §20 depends on. This is not a failure of the code \
         under test; it is the workflow. `.cargo/config.toml` names \
         `scripts/headless-runner.sh` as cargo's test runner and that script \
         fails open when it cannot find `mutter` or `XDG_RUNTIME_DIR`, so \
         check that the CI job still installs mutter and sets \
         XDG_RUNTIME_DIR before the test step."
    );
}
