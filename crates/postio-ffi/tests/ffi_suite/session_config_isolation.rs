//! An in-memory session must not read the machine's own `config.toml` (#1219), or tests pass or
//! fail by the developer's preferences.
//! The installed file is set with `POSTIO_CONFIG` on a child process (this binary re-run with one
//! test), because `set_var` is unsafe and `postio-ffi` forbids unsafe.

use std::io::Write as _;
use std::process::Command;

const CHILD: &str = "POSTIO_1219_CHILD";

/// Every value differs from the built-in default, so a session that read this file cannot pass as
/// one that ignored it.
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
            postio_test_support::this_test!("an_in_memory_session_ignores_the_installed_config")
                .as_str(),
            "--nocapture",
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

fn the_assertions() {
    // Without this the case passes whenever the override silently fails to apply.
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
        Some("e"),
        "an in-memory session took its key bindings from the machine's \
         config.toml -- the failure that silently changes what every keyboard \
         test resolves"
    );

    session.shutdown();
}
