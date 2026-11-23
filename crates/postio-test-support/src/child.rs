//! Running a test again as a child of itself. A case needing its own process runs its binary again
//! with `<name> --exact`; a hand-written name is wrong the day the file moves, and a wrong name
//! does not fail: libtest runs nothing and exits 0. [`this_test!`](crate::this_test) derives it
//! from where it is written.

/// The first segment of a module path is the test binary's crate, which libtest leaves out of a
/// test's name.
pub fn full_name(module_path: &str, case: &str) -> String {
    match module_path.split_once("::") {
        Some((_, module)) => format!("{module}::{case}"),
        None => case.to_owned(),
    }
}

#[macro_export]
macro_rules! this_test {
    ($case:expr) => {
        $crate::child::full_name(module_path!(), $case)
    };
}

const ALONE: &str = "POSTIO_TEST_ALONE";

fn own_process(case: &str) -> bool {
    std::env::var_os("NEXTEST_EXECUTION_MODE").is_some_and(|mode| mode == "process-per-test")
        || std::env::var_os(ALONE).is_some_and(|name| name == case)
}

/// For a case that reads something the whole process shares (a counter, a slot, the tracing
/// dispatch a callsite cached): `cargo test` runs a suite's cases as threads of one process
/// (#1976). Where the process is already the case's own (nextest, or the child) it returns `false`;
/// elsewhere it reruns this binary with `case` alone, fails if that did, and returns `true`: the
/// caller returns at once.
pub fn alone(case: &str) -> bool {
    if own_process(case) {
        return false;
    }
    let ran = std::process::Command::new(std::env::current_exe().expect("this test binary"))
        .args([case, "--exact", "--nocapture", "--test-threads=1"])
        .env(ALONE, case)
        .output()
        .expect("run this case again");
    let said = String::from_utf8_lossy(&ran.stdout);
    assert!(
        ran.status.success(),
        "{case}, alone, failed:\n{said}\n{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    // libtest exits 0 having run nothing when no case has the name.
    assert!(
        said.contains("1 passed"),
        "{case} ran nothing alone:\n{said}"
    );
    true
}

#[macro_export]
macro_rules! alone {
    ($case:expr) => {
        $crate::child::alone(&$crate::this_test!($case))
    };
}

#[cfg(test)]
mod tests {
    use super::full_name;

    #[test]
    fn a_case_alone_runs_its_body_in_a_process_of_its_own() {
        if crate::alone!("a_case_alone_runs_its_body_in_a_process_of_its_own") {
            return;
        }
        assert!(
            super::own_process("child::tests::a_case_alone_runs_its_body_in_a_process_of_its_own"),
            "the body ran beside its neighbours"
        );
    }

    #[test]
    fn a_file_of_its_own_names_the_case_alone() {
        assert_eq!(full_name("store_in_use", "refused"), "refused");
    }

    #[test]
    fn a_suite_module_names_the_module_and_the_case() {
        assert_eq!(
            full_name("tui_suite::store_in_use", "refused"),
            "store_in_use::refused"
        );
    }

    #[test]
    fn a_nested_module_keeps_every_segment_but_the_binary() {
        assert_eq!(
            full_name("host_suite::oauth::inner", "case"),
            "oauth::inner::case"
        );
    }

    #[test]
    fn the_macro_reads_the_module_it_is_written_in() {
        assert_eq!(
            crate::this_test!("case"),
            "child::tests::case",
            "the unit-test binary's crate is postio_test_support, left out"
        );
    }
}
