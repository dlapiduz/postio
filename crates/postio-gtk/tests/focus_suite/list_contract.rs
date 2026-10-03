//! The suite's `--list` output is a contract with whatever runs it, and
//! breaking it fails **green**.
//!
//! `cargo test` only ever asks this harness to run tests. A process-per-test
//! runner asks it two questions first — every test, then the ignored subset —
//! and believes the answers. Get the second one wrong and the runner concludes
//! that everything is ignored: it runs nothing, reports success, and finishes
//! in about a second. That is the failure this repository has now paid for in
//! four costumes (#114, #355, #551, #596), and it is the reason this is a test
//! rather than a comment asking people to be careful.
//!
//! It was also a real bug, not a hypothetical: `--list --format terse
//! --ignored` printed *every* case, because the harness matched on `--list`
//! and ignored the rest of the line.
//!
//! # What is checked
//!
//! * every line of `--list --format terse` ends in `: test`, which is what a
//!   runner parses and rejects anything else for;
//! * `--format terse` carries no trailing count — real libtest emits the names
//!   and nothing else, and the count is only for the non-terse form the
//!   tooling's test counting reads;
//! * the ignored list is a subset of the full list;
//! * every name in `IGNORED` is a real row in `CASES`.
//!
//! That last one is the quiet one. A mute names a case by string, so renaming
//! or deleting the case leaves an entry that matches nothing — the mute is
//! dead, nothing says so, and the case either runs when it was meant not to or
//! was never held out at all. Same shape as the stale `rerun-if-changed` in
//! `docs/engineering-notes.md`: no error, just a wrong answer forever.

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
