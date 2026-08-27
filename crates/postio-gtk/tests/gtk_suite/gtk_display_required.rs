//! A GTK suite that skipped is theatre, so something has to notice.
//!
//! Around 117 test files in this workspace open with some spelling of
//!
//! ```ignore
//! if adw::init().is_err() || gdk::Display::default().is_none() {
//!     eprintln!("skipping: no display");
//!     return;
//! }
//! ```
//!
//! which is right for a contributor on a headless shell and wrong everywhere
//! else: with no display every one of them returns early and reports success.
//! `docs/PRODUCT.md` §20 makes accessibility a shipping requirement and the
//! audit that enforces it is one of the 117, so a green tick can mean nothing
//! ran (#114). This is the guard against that, and it is deliberately *one*
//! test rather than 117 edits.
//!
//! # Why "is there a display?" was not a good enough question (#551)
//!
//! The first version asked only that, and only when `CI` was set — because
//! locally, "no display" was assumed to mean a contributor on an ssh shell,
//! where skipping is correct.
//!
//! That conflated two situations, and the difference is where the bug lived.
//! `.cargo/config.toml` puts every test binary on a private compositor via
//! `scripts/headless-runner.sh`, so on a Linux box with mutter there is
//! *always* a display. "No display" therefore means one of:
//!
//! 1. the runner ran and failed open — no mutter, no `XDG_RUNTIME_DIR` — which
//!    is a contributor's machine, and skipping is right;
//! 2. **the runner never ran at all**, because nothing routed this binary to
//!    it. That is a build-configuration bug, not a fact about the machine.
//!
//! Case 2 is what #551 was: `runner` sat under
//! `[target.x86_64-unknown-linux-gnu]`, so on aarch64 Linux it never applied,
//! no compositor started, and the whole GTK suite skipped silently. Gating on
//! `CI` hid it, because the aarch64 runs anybody actually did were local.
//!
//! So the runner exports `POSTIO_TEST_RUNNER` on every path it can take,
//! including the fail-open ones, and this reads it. "Did a compositor start"
//! and "did cargo route us through the runner" become separate questions with
//! separate answers, which is what makes case 2 assertable on a developer's
//! machine rather than only in CI.

use gtk::gdk;

/// Whether cargo sent this binary through `scripts/headless-runner.sh`.
fn the_runner_ran() -> bool {
    std::env::var_os("POSTIO_TEST_RUNNER").is_some()
}

pub fn ci_has_a_display_to_run_the_gtk_suites_on() {
    // Deliberately bypassed, to watch a run on the real display. Nothing to
    // say: the person asked for this.
    if std::env::var("POSTIO_HEADLESS").is_ok_and(|value| value == "0") {
        eprintln!("POSTIO_HEADLESS=0: the runner was bypassed on purpose");
        return;
    }

    let has_display = adw::init().is_ok() && gdk::Display::default().is_some();

    // The check that would have caught #551, and the reason it is not gated on
    // `CI`: on Linux the runner is meant to apply to every test binary, so its
    // absence is a configuration bug wherever it happens.
    #[cfg(target_os = "linux")]
    assert!(
        the_runner_ran(),
        "cargo did not route this test binary through \
         scripts/headless-runner.sh, so no compositor was started and every \
         GTK test in this workspace has been taking its 'no display' branch \
         and reporting success without asserting anything.\n\n\
         This is `.cargo/config.toml` rather than the code under test. The \
         `runner` key has to apply to this target: it lives under \
         `[target.'cfg(target_os = \"linux\")']` so that it covers every Linux \
         triple, and #551 is what happened when it sat under one of them \
         (x86_64) and the machine was aarch64.\n\n\
         Building for: {}",
        std::env::consts::ARCH
    );

    if !the_runner_ran() {
        // Not Linux: no runner is expected, and there is nothing to say about
        // a display the platform manages itself.
        return;
    }

    // The runner ran. Either it started a compositor, or it failed open
    // because this machine has no mutter or no XDG_RUNTIME_DIR — a
    // contributor's shell, where the skips are correct and this has nothing to
    // add. In CI it is never acceptable, whatever the reason.
    if std::env::var_os("CI").is_none() {
        if !has_display {
            eprintln!(
                "the runner ran and could not start a compositor (no mutter, \
                 or no XDG_RUNTIME_DIR): the display-less skips are correct \
                 here"
            );
        }
        return;
    }

    assert!(
        has_display,
        "CI has no display, so every GTK test in this workspace returned \
         early and reported success — including the accessibility audit that \
         docs/PRODUCT.md §20 depends on. This is not a failure of the code \
         under test; it is the workflow. `scripts/headless-runner.sh` ran and \
         fails open when it cannot find `mutter` or `XDG_RUNTIME_DIR`, so \
         check that the CI job still installs mutter and sets \
         XDG_RUNTIME_DIR before the test step."
    );
}
