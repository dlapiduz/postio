# cargo-hakari cannot be adopted here, and the reason is the boundary check (2026-09-03)

The problem it solves is real and measured. `cargo <cmd> -p <crate>` and
`cargo <cmd> --workspace` resolve **different feature sets** for shared
dependencies -- 15 to 29 of them depending on the crate:

```
-p postio-core     vs --workspace : 28 deps differ
-p postio-gtk      vs --workspace : 29
-p postio-storage  vs --workspace : 23
-p postio-sync     vs --workspace : 15
```

and not cheap ones: `tokio` (the workspace adds `test-util`), `futures-util`
(adds `async-await` and the `futures-macro` proc macro), `rustix`,
`linux-raw-sys`, `serde`, `hashbrown`, `getrandom`. Cargo rebuilds a unit when
its features change, so every switch between a `-p` command and a `--workspace`
one rebuilds them. `issue-land.sh`'s gate chain alternates exactly that --
`clippy -p`, `test -p`, then `test --workspace --lib` and `check --workspace
--all-targets` -- and so does moving between `test-fast.sh` and
`test-sanity.sh`.

`cargo-hakari` is the standard fix: a generated `workspace-hack` crate that
every member depends on, declaring the union of every feature, so both
resolutions agree. **It is incompatible with this repository's crate
boundaries.** `check-crate-boundaries.py` walks the *resolved, transitive*
graph, and `postio-model` bans `tokio`, `rusqlite`, `gtk4`, `ammonia` and
`html5ever` -- for the reason in its own rule, that "postio-model is what the
whole workspace waits on to compile". A workspace-hack every member depends on
puts all of them in postio-model's graph. The check would fail, and it would be
right to.

If this is picked up again, the only shape that can work is hakari's
`traversal-excludes` holding the guarded leaves (`postio-model`,
`postio-search`, `postio-body`, `postio-config`) out of the hack, so only the
heavy crates unify. That needs its own measurement first: hakari makes every
`-p` build compile the *union*, so it trades churn on the switch for a larger
`test-fast.sh`. Nobody has measured which is bigger here.
