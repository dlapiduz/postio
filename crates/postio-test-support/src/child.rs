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

#[cfg(test)]
mod tests {
    use super::full_name;

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
