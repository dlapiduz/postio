# Adding a crate is the edit the per-crate gate cannot describe (2026-09-02, #585)

`issue-land.sh` runs clippy and tests over the crates a branch changed. That
is the right trade and it is not what this note is about. It has exactly one
blind spot, and the blind spot is structural rather than a matter of degree.

`postio-session/src/logging.rs` keeps a list of every workspace crate, so a
bare `POSTIO_LOG=debug` does not hold Postio's own crates at `warn`, and it
has a test that enumerates the workspace and fails when one is missing.
`postio-ui` arrived with #566 and `postio-ffi` with #571, from two different
sessions. Neither branch changed `postio-session`, so neither gate chain had
any reason to compile that test, and `cargo test -p postio-session` was red
on `main` across two landings that could not have seen it.

**Adding a crate is the one edit whose blast radius is definitionally outside
the crates it touches.** Every other change is bounded by its own call sites,
which is what makes a changed-crate list a fair description of the risk. A
new member is not: anything that *enumerates* the workspace can start failing
somewhere nobody looked, and that set is open-ended.

The workspace `cargo check --all-targets` added in #419 cannot cover it
either, and the reason is worth keeping straight. `check` answers "does
everyone still compile", and the test that broke here compiled perfectly --
it just failed. Type-checking cannot catch a test whose assertion is about
the shape of the workspace.

So a branch whose diff changes the root manifest's `members` runs the
workspace tests, and every other branch is untouched. Keyed on the members
list actually changing, not on `Cargo.toml` being touched, so a dependency
bump does not buy a full test run.

Surveyed the other things that enumerate the workspace while here, since a
silent one would have been the worse bug:

- `check-lint-floor.py` iterates the members and fails loudly for a new crate
  that does not inherit the floor. Good as it is.
- `check-crate-boundaries.py` iterates its own `RULES` table -- ten entries
  against nineteen crates -- so a new crate is simply not guarded until
  someone writes a rule for it. That is opt-in by design rather than a
  failure to detect anything, and most crates legitimately have no rule. It
  is recorded here because "my crate passed the boundary check" and "the
  boundary check looked at my crate" are not the same sentence.
