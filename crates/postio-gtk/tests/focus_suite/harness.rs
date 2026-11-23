use std::process::Command;

const CHILD: &str = "POSTIO_FOCUS_SUITE_CHILD";

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

/// The main loop is turned inside one poll of the case's future, so each tokio-awaiting future
/// spends the same 128-operation budget.
/// Once spent, each is woken again at once and `settle` never goes idle.
pub fn settle_outlasts_the_runtimes_budget() {
    use std::{cell::Cell, rc::Rc};
    crate::gtk_case(async {
        let landed = Rc::new(Cell::new(0_usize));
        for _ in 0..300 {
            let (answer, answered) = tokio::sync::oneshot::channel::<()>();
            let landed = Rc::clone(&landed);
            gtk::glib::spawn_future_local(async move {
                let _ = answered.await;
                landed.set(landed.get() + 1);
            });
            tokio::spawn(async move {
                let _ = answer.send(());
            });
        }
        let until = std::time::Instant::now()
            + postio_test_support::scaled(std::time::Duration::from_secs(10));
        while landed.get() < 300 && std::time::Instant::now() < until {
            crate::settle();
        }
        assert_eq!(landed.get(), 300, "reads that were answered never landed");
    });
}
