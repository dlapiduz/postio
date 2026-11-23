use std::process::Command;

/// Set on the case's child process so a harness that runs more than asked fails instead of spawning
/// copies of itself.
const CHILD: &str = "POSTIO_WIDGETS_SUITE_CHILD";

pub fn an_empty_case() {}

pub fn an_empty_case_is_listed_and_runs() {
    assert!(
        std::env::var_os(CHILD).is_none(),
        "asked to run one other case by its exact name, the harness ran this \
         one too: `--exact` is not being honoured"
    );
    let name = "harness::an_empty_case";
    let executable = std::env::current_exe().expect("the running test binary has a path");

    let listed = Command::new(&executable)
        .args(["--list", "--format", "terse"])
        .env(CHILD, "1")
        .output()
        .expect("asking this binary for its test list");
    let listed = String::from_utf8(listed.stdout).expect("a test list is UTF-8");
    assert!(
        listed.lines().any(|line| line == format!("{name}: test")),
        "{name} is a row in CASES, and --list does not name it:\n{listed}"
    );

    let ran = Command::new(&executable)
        .args(["--exact", name])
        .env(CHILD, "1")
        .output()
        .expect("asking this binary to run one case");
    let ran = String::from_utf8(ran.stdout).expect("a test run's output is UTF-8");
    assert!(
        ran.contains(&format!("test {name} ... ok")) && ran.contains("1 passed; 0 failed"),
        "asked to run {name} alone, the harness did not run exactly it:\n{ran}"
    );
}
