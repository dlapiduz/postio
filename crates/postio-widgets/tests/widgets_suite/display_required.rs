use gtk::gdk;

fn the_runner_ran() -> bool {
    std::env::var_os("POSTIO_TEST_RUNNER").is_some()
}

pub fn ci_has_a_display_to_run_the_gtk_suites_on() {
    if std::env::var("POSTIO_HEADLESS").is_ok_and(|value| value == "0") {
        eprintln!("POSTIO_HEADLESS=0: the runner was bypassed on purpose");
        return;
    }

    let has_display = adw::init().is_ok() && gdk::Display::default().is_some();

    // Not gated on `CI`: on Linux the runner applies to every test binary, so its absence is a
    // configuration bug anywhere (#551).
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
        return;
    }

    // Without a compositor the runner failed open (no mutter or XDG_RUNTIME_DIR): skips are right
    // on a contributor's shell, never in CI.
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
         under test; it is the display. There is no Xvfb to fall back to any \
         more (#830: X11 is not a supported configuration), so the nested \
         `mutter --headless` is the only display and it did not come up. \
         `scripts/headless-runner.sh` says which fallback it took and prints \
         mutter\'s own log — read that first. A mutter that logged \"Running \
         Mutter\" and nothing else was still starting when the runner gave up \
         waiting; that wait scales with POSTIO_TEST_PATIENCE."
    );
}
