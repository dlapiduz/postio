# mold looked like a memory win over lld and was not, once measured correctly (2026-09-01)

Tried, benchmarked, and deliberately not adopted — recorded so nobody spends
an afternoon re-deriving this. The hypothesis was reasonable: linking is the
documented memory peak here, not compiling (see the `jobs = 2` comment in
`.cargo/config.toml`), and mold's own selling point is exactly that step.

**The first measurement was wrong, and it was wrong in a specific,
repeatable way.** `/usr/bin/time -f %M` (or `-v`) around a whole
`cargo build -p postio-app` reports the *maximum single RSS seen across the
process and its reaped children* — not the linker's RSS. For a build that
also recompiles `postio-app`'s own crate, that number can just as easily be
the `rustc` compile step's peak as the link step's, and which one wins
varies run to run for reasons that have nothing to do with which linker is
configured. The first pass this way showed mold 36-56% below lld — a
plausible-looking number that happened to be measuring the wrong process.

**Isolating the linker invocation itself found nothing.** Wrapping just the
`cc`/`mold` step in `/usr/bin/time -v` (rather than the whole `cargo build`)
and comparing on an otherwise-identical, freshly-fingerprinted tree: mold
1,127,720 KB / 0.69s, lld 1,128,608 KB / 0.70s — within noise, not a
plausible-but-different number this time. Whatever headroom mold offers over
GNU ld's `lld` mode exists elsewhere; it does not show up linking this
particular GTK+WebKit binary on this machine.

**The methodology lesson matters more than the mold verdict**: to measure
one step of a multi-process pipeline, instrument that step directly, not the
outermost process. `/usr/bin/time` on `cargo build` measures cargo's build,
not the thing you changed inside it — the same trap as the sccache-vs-C
story above ("this was measured before being believed"), just one layer
further from the thing that was actually varied.

**A separate, real bug the investigation turned up** (fixed alongside this,
unrelated to the mold question either way): `postio-gtk` recompiled on
*every* rebuild of anything downstream of it, in a fresh worktree and an
already-built checkout alike. `crates/postio-gtk/build.rs` still declared
`cargo:rerun-if-changed=src/tokens.rs`, a file #569 removed when it moved
that logic into the `postio_ui::tokens` build-dependency — cargo treats a
*missing* watched file as unconditionally stale (it cannot confirm freshness
of something that is not there), so the build script reran, and with it the
crate, on every single build since #569 landed. Confirmed with
`CARGO_LOG=cargo::core::compiler::fingerprint=trace`, which printed
`stale: missing ".../postio-gtk/src/tokens.rs"` on a tree where nothing had
changed. The fix is the missing line's removal — the build dependency is
tracked by cargo automatically and needs no manual `rerun-if-changed`. Worth
knowing generically: a stale `rerun-if-changed` on a path a refactor deleted
does not fail loudly, it just makes the crate un-cacheable forever, quietly,
in a way that looks exactly like "this crate is just slow to build" until
someone traces a single untouched file through a rebuild.

**And separately: changing which linker is in effect is not scoped to the
crate being linked**, which is why the isolated-linker measurement above
needed a from-scratch fingerprint each time it toggled. Cargo fingerprints
every unit — including every rlib that is never itself linked — on the
flags that would apply to it, and `-C linker=…` is one of them. Toggling it,
whether via a config file, `RUSTFLAGS`, or
`CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER`, invalidates cargo's
fingerprint for the *entire* dependency graph, confirmed by watching a
single-file touch that should have recompiled only `postio-app` instead
recompile ~400 crates. `sccache` fares no better on the same switch: it
hashes the literal rustc command line, and the extra flag shows up in that
line even for a crate whose compiled output it does not affect — measured at
6.15% hits (15 of 244) on one such switch. Relevant to any future attempt in
this vein, mold or otherwise: budget one genuinely slow build for making the
change and one more for un-making it, not a quick A/B.
