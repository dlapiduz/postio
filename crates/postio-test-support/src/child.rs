//! Running a test again as a child of itself.
//!
//! A case that needs a process of its own -- an environment the parent must
//! not have, a lock a process never conflicts with itself on -- runs its own
//! binary again with `<name> --exact`. That name is the case's full libtest
//! path, which is the module path inside a suite binary and the bare name in
//! a file of its own. Written by hand, it is wrong the day the file moves,
//! and a wrong name does not fail: libtest runs nothing and exits 0, so the
//! child "succeeds" having done nothing at all.
//!
//! [`this_test!`](crate::this_test) derives it from where it is written.

/// `case`'s full libtest name, given the `module_path!()` it is written in.
///
/// The first segment of a module path is the test binary's crate, which
/// libtest leaves out of a test's name.
pub fn full_name(module_path: &str, case: &str) -> String {
    match module_path.split_once("::") {
        Some((_, module)) => format!("{module}::{case}"),
        None => case.to_owned(),
    }
}

/// The full libtest name of `case` in the module this is written in, for
/// `Command::new(current_exe()).args([&this_test!(CASE), "--exact"])`.
#[macro_export]
macro_rules! this_test {
    ($case:expr) => {
        $crate::child::full_name(module_path!(), $case)
    };
}

/// Set on a case [`alone`] runs again, to the case's own name.
const ALONE: &str = "POSTIO_TEST_ALONE";

/// Whether this process runs `case` and nothing else: nextest gives every
/// case a process, and [`alone`]'s child is one.
fn own_process(case: &str) -> bool {
    std::env::var_os("NEXTEST_EXECUTION_MODE").is_some_and(|mode| mode == "process-per-test")
        || std::env::var_os(ALONE).is_some_and(|name| name == case)
}

/// Give `case` a process of its own, for a case that reads something the
/// whole process shares: a counter every thread adds to, a slot the last
/// session to open filled, the tracing dispatch a callsite cached.
///
/// A suite is one binary, and `cargo test` runs its cases as threads of one
/// process, so such a case reads what its neighbours did too (#1976).
/// Where the process is already the case's own -- under nextest, or in the
/// child this starts -- it returns `false` and the case goes on. Anywhere
/// else it runs this binary again with `case` alone, fails if that did, and
/// returns `true`: the caller returns at once. Through [`alone!`](crate::alone),
/// which names the case from where it is written.
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

/// [`alone`] for the case named `case` in the module this is written in:
/// `if postio_test_support::alone!("the_case") { return; }`.
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
        // Whoever runs this -- nextest, or `cargo test` with every case on a
        // thread of one process -- the body runs where nothing else does.
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
