//! Nothing in this crate's real code paths prints to stdout or stderr.
//!
//! `postio-b9t.3` put `tracing` across the workspace and gave that a written
//! acceptance criterion — *no `println!`/`eprintln!` left in crate source
//! outside examples and tests* — which every crate met except this one, and
//! this one only because two sessions were live in it at the time
//! (`postio-b9t.3.1`). Sixteen call sites survived in
//! `config`, `toast`, `app`, `window` and `settings`.
//!
//! A print is not a smaller version of a log line. It goes nowhere a user can
//! filter, nowhere the journal can collect, it carries no level and no
//! target, and `POSTIO_LOG` cannot turn it up or down. One left behind in a
//! path that runs often is a line nobody can act on and nobody can silence.
//!
//! So this is the criterion, enforced rather than remembered — the same
//! reason `postio-runtime`'s `logging_privacy` test exists rather than
//! trusting the privacy rule to be recalled.
//!
//! # What counts as a test
//!
//! A `#[cfg(test)]` module may print freely: `eprintln!("skipping: no
//! display")` is how every GTK test in this tree says why it did nothing, and
//! it belongs on stderr rather than in a subscriber that may not be
//! installed.
//!
//! This finds the *first* `#[cfg(test)]` in a file and treats everything from
//! there on as test code. That is exactly right for the layout this crate
//! uses — one test module, at the bottom — and would under-report a print
//! inside a `#[cfg(test)]` module in the middle of a file with real code
//! after it. Worth knowing; not worth a syn dependency to close, and a false
//! *negative* here costs a stray print rather than a broken build.

use std::path::{Path, PathBuf};

/// Where the crate's real code lives.
const SOURCE: &str = "src";

pub fn no_source_file_prints_outside_its_tests() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(SOURCE);
    let mut found: Vec<String> = Vec::new();

    for file in rust_files(&root) {
        let text = std::fs::read_to_string(&file).expect("a readable source file");
        // Everything from the first `#[cfg(test)]` is test code.
        let real = match text.find("#[cfg(test)]") {
            Some(offset) => &text[..offset],
            None => &text[..],
        };
        for (number, line) in real.lines().enumerate() {
            if line.contains("println!") || line.contains("eprintln!") {
                let relative = file.strip_prefix(&root).unwrap_or(&file);
                found.push(format!(
                    "{}:{}: {}",
                    relative.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        found.is_empty(),
        "these print instead of logging, so nothing can filter or collect \
         them and `POSTIO_LOG` cannot reach them:\n  {}",
        found.join("\n  ")
    );
}

/// Every `.rs` file under `root`, depth first.
fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(rust_files(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
    found.sort();
    found
}
