//! An in-memory session must not read the machine's own `config.toml` (#1219).
//!
//! `SessionOptions::in_memory()` is what every test needing a `Session` opens.
//! With no config text it fell through to `Config::load()`, so a test
//! inherited whatever was in `~/.config/postio/config.toml` — green on CI,
//! where there is no such file, and red on a workstation according to the
//! preferences of whoever ran it. `[ui]` is where it was noticed
//! (`density = "compact"` in a real file failed a test asserting the default);
//! `[keys]` is where it would have been worse, since one rebinding in a
//! developer's own file quietly changes what every keyboard test resolves.
//!
//! A gate that passes for a reason unrelated to the code is not a gate.
//!
//! # Why this runs a child process
//!
//! Proving a session ignores the installed file means *having* an installed
//! file to ignore, and the only way to say where that is is `POSTIO_CONFIG`.
//! The environment is process-global — `std::env::set_var` is `unsafe` for
//! exactly that reason — and `postio-ffi` forbids unsafe on purpose, being
//! the crate that hands pointers to Swift. Weakening that for a test would be
//! a poor trade.
//!
//! So the environment is set on a *child*, which `Command::env` does safely,
//! and the child is this same binary re-run with a single test selected. The
//! parent asserts it succeeded. Nothing here needs an unsafe block, and no
//! thread races another for the environment.

use std::io::Write as _;
use std::process::Command;

/// Set on the child, so the one test below knows which half it is running.
const CHILD: &str = "POSTIO_1219_CHILD";

/// The installed file this test pretends the developer has.
///
/// Every value is deliberately *not* the built-in default, so a session that
/// read this file cannot be mistaken for one that ignored it.
const A_MACHINE_WITH_OPINIONS: &str = r#"
[ui]
density = "compact"

[keys]
archive = "ctrl+shift+z"
"#;

#[test]
fn an_in_memory_session_ignores_the_installed_config() {
    match std::env::var(CHILD) {
        Ok(_) => the_assertions(),
        Err(_) => run_the_child(),
    }
}

/// The parent: write a config, point the child at it, and require a pass.
fn run_the_child() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("config.toml");
    let mut file = std::fs::File::create(&path).expect("create the fixture");
    file.write_all(A_MACHINE_WITH_OPINIONS.as_bytes())
        .expect("write the fixture");
    drop(file);

    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([
            "--exact",
            "an_in_memory_session_ignores_the_installed_config",
            "--nocapture",
            // One thread, so the child is the only thing running while its
            // environment says what it says.
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .env(postio_config::paths::CONFIG_PATH_ENV, &path)
        .output()
        .expect("re-run this binary");

    assert!(
        output.status.success(),
        "the isolated case failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// The child: there is an installed config, and the session must not read it.
fn the_assertions() {
    // The fixture is reachable. Without this the case passes whenever the
    // override silently fails to apply, which is the same vacuous green it
    // exists to stop.
    let installed = postio_config::Config::load().expect("the fixture loads");
    assert_eq!(
        installed.ui.density,
        postio_config::Density::Compact,
        "POSTIO_CONFIG did not take, so this case proves nothing"
    );

    let session = postio_ffi::Session::open(postio_ffi::SessionOptions::in_memory())
        .expect("an in-memory session");

    assert_eq!(
        session.appearance().density,
        postio_ffi::DensityFfi::Airy,
        "an in-memory session read the machine's config.toml rather than the \
         built-in defaults"
    );
    assert_eq!(
        session.binding_for("archive".to_owned()).as_deref(),
        // The registry's default. The fixture says `ctrl+shift+z`, so a
        // session that read it fails here with the fixture's own value.
        Some("a"),
        "an in-memory session took its key bindings from the machine's \
         config.toml -- the failure that silently changes what every keyboard \
         test resolves"
    );

    session.shutdown();
}
