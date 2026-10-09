# One feature set for the landing gate: measured, and left alone

*2026-10-01.*

The landing gate mixes per-crate commands (`clippy -p`, `doc -p`, `test --doc -p`, suites `-p`) with workspace ones (`test --workspace --lib`, `check --workspace --all-targets`). Cargo resolves a dependency's features from the packages a command selects, so the two kinds build the same dependency more than once. A landing tree held 1,640 rlibs for 596 crate names. The proposal was to make every gate command select the whole workspace, so that each dependency is built once.

## What `-p` actually costs

Measured on a warm tree (`postio-storage` as the changed crate). The figure is how many of a command's dependency artifacts are not shared with the workspace form:

| Command | Not shared |
|---|---|
| `clippy -p postio-storage --all-targets` vs `clippy --workspace --all-targets` | 145 of 359 |
| `doc --no-deps -p postio-storage` vs the same | 144 of 358 |
| `test -p postio-storage --lib`/`--tests` vs `test --workspace --lib` | 233 of 570 |
| `test --workspace --test storage_suite` vs `test --workspace --lib` | 0 of 584 |

The variants are real. But they are built once per tree, and seeded trees inherit them from the sibling they copy.

## Why the gate stays as it is

**One `clippy --workspace` is slower, not faster.** After a one-line change to `postio-storage`:

| Run | Current: `clippy -p` + `check --workspace --all-targets` | Proposed: `clippy --workspace --all-targets` |
|---|---|---|
| 1 | 12 s + 6 s = 18 s | 103 s |
| 2 | 11 s + 64 s = 75 s | 136 s |

Clippy lints every crate downstream of the change, and linting costs far more than type-checking. The current pair lints only the changed crate and type-checks the rest. CI's Clippy job lints the whole workspace anyway.

**Workspace doctests cost every landing.** `cargo test --workspace --doc` is 36 s for 39 doctests, against about 2 s per changed crate. Doctests stay per-crate, and they hold the same build-mode variants the per-crate suites would. So moving only the suites to `--workspace --test` saves nothing.

If cargo's `-Zfeature-unification=workspace` reaches stable, the variants go away without changing any of these commands. That is the fix to watch for.
