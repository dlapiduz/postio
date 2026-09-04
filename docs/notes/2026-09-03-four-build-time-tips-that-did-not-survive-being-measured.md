# Four build-time tips that did not survive being measured (2026-09-03)

All four are standard advice, three of them from corrode.dev's "Tips for
Faster Rust Compile Times". Each was applied here, measured, and reverted.
Written down so the next person spends the five minutes reading rather than
the hour re-deriving.

Method throughout: `cargo clean` then
`cargo test --no-run -p postio-app --test app_suite`, `-j6`, sccache
*disabled* (`RUSTC_WRAPPER=`) so both sides compile for real, on an idle box.
Baseline reproduced twice at **301s** and **307s**, so ~2% is noise and
anything under ~10s means nothing.

**`[profile.dev.package."*"] opt-level = 1` (from 2) -- reverted, and it is
the important one.** The argument for it was that no gate can see it: the
perf budgets are asserted as counts off SQLite's trace hook, not timings, and
`cargo bench` measures release. That argument is true and beside the point.
What a dependency's optimization level moves is the wall clock of the suites
people wait on:

```
cargo test -p postio-app --test app_suite
  deps at opt-level = 2    200s
  deps at opt-level = 1    382s      <- nearly double
```

for about 20s of cold build saved. The `opt-level = 2` line in the root
`Cargo.toml` is not a leftover; it is why the integration suites are bearable,
and nothing in any gate's pass-or-fail would have told you.

**`[profile.dev.build-override] opt-level = 3` -- reverted, +63s.** The tip is
that proc macros are *executed* during compilation, so optimizing them pays
off across hundreds of dependents. Here the cold build went 325s -> 388s.
Optimizing `syn` (three copies, v2 and v3), `serde_derive`, `glib-macros` and
`zbus_macros` costs more to compile once than it saves in running them, and a
cold build is where this workspace actually spends its time.

**cargo-hakari -- cannot be adopted, see the entry above.** The feature-
unification churn it fixes is real and measured, but a `workspace-hack` every
member depends on puts `tokio`, `rusqlite` and `gtk4` in `postio-model`'s
transitive graph, and `check-crate-boundaries.py` walks that graph.

**Reordering the gate chain to group `check` steps together -- pointless, and
this one was my own idea rather than an article's.** The theory was that
`issue-land.sh` alternates check-mode and build-mode passes and at two
different feature widths, so each switch re-does work. It does not. Cargo
gives every unit a distinct metadata hash from its profile, features and mode,
so check units, build units and the same crate at two feature widths all
**coexist** in `target/`:

```
cargo clippy -p postio-core --all-targets   67 units
cargo check --workspace --all-targets      324 units
cargo clippy -p postio-core --all-targets    0 units   <- nothing to redo
```

What is true, and is the useful form of the observation: a cold worktree
compiles the dependency graph roughly **twice**, once as check units and once
as build units. That is inherent to running both `clippy`/`check` and
`test`, it is not an ordering mistake, and the only lever on it is not paying
for a cold worktree in the first place (a reused or seeded claim, #1102).
