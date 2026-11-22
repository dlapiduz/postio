//! The suite's `--list` output is a contract with the runner, and breaking it fails green: a runner
//! that misreads `--list --ignored`
//! concludes everything is ignored, runs nothing and reports success (#114, #355, #551, #596).

use std::process::Command;

fn list(arguments: &[&str]) -> Vec<String> {
    let executable = std::env::current_exe().expect("the running test binary has a path");
    let output = Command::new(&executable)
        .args(arguments)
        .output()
        .expect("re-running this binary to ask it for its test list");
    assert!(
        output.status.success(),
        "{executable:?} {arguments:?} exited {:?}",
        output.status.code()
    );
    String::from_utf8(output.stdout)
        .expect("a test list is UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

pub fn the_list_output_stays_libtest_shaped() {
    let all = list(&["--list", "--format", "terse"]);
    assert!(
        !all.is_empty(),
        "--list --format terse named no tests at all"
    );
    for line in &all {
        assert!(
            line.ends_with(": test"),
            "every line of a terse list must end in \": test\", so that a \
             runner can parse it; this one does not: {line:?}. A trailing \
             count belongs only in the non-terse form."
        );
    }

    let ignored = list(&["--list", "--format", "terse", "--ignored"]);
    for line in &ignored {
        assert!(
            line.ends_with(": test"),
            "the ignored list is a list too: {line:?}"
        );
        assert!(
            all.contains(line),
            "{line:?} is reported ignored but is not in the full list, so a \
             runner cannot reconcile the two"
        );
    }
    assert!(
        ignored.len() < all.len(),
        "every one of the {} cases is reported ignored. A runner reads that \
         as 'nothing to do', runs none of them, and exits successfully in a \
         second. This is exactly the bug this test exists for.",
        all.len()
    );

    for name in crate::IGNORED {
        assert!(
            crate::CASES.iter().any(|(case, _)| case == name),
            "IGNORED names {name:?}, which is not a row in CASES. The mute is \
             dead: either the case was renamed and is running again, or it \
             never existed. Fix the name or drop the entry."
        );
    }
}
