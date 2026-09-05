# The vendored OpenSSL is perl, not C, and no compiler cache can help (2026-09-03)

`cargo build --timings` puts `openssl-sys build-script (run)` at the top of a
cold build -- 198.8s, more than all twenty of our own crates put together
(115.6s). That number is misleading twice over, and both corrections matter
before anyone tries to "fix" it again.

**First: a `--timings` duration is wall-clock under contention, not cost.**
Rebuilt on its own, `cargo build -p openssl-sys` takes **28s**. The 198.8s is
how long the unit was in flight at `-j6` while five other jobs competed; what
it really says is that this is a long *serial* step holding a job slot on the
critical path into `libsqlite3-sys` → `rusqlite` → `postio-storage`. Read
`--timings` for the shape of the graph, not for the price of a unit.

**Second: the C in it is already cached, and caching it harder does nothing.**
ccache hits 96.7% across the build (#736 did that). The remaining hypothesis --
that those hits were all *preprocessed* hits (86%) and would be much cheaper in
direct mode -- was tested and is false:

```
current config (preprocessed mode)   28s
CCACHE_DEPEND=1, cold                28s
CCACHE_DEPEND=1, warm                29s     direct hits +2282, i.e. it engaged
```

Depend mode turned every one of those compiles into a direct hit and bought
nothing, because the compiles were never the cost. `perl` is the top process
while it runs: the 28s is OpenSSL's `Configure`, its make orchestration and
`ar`, none of which any compiler cache exists to serve. **Do not add
`CCACHE_DEPEND` to `scripts/cc-wrapper.sh`** -- it is measured, and it is a
no-op here.

So the only ways not to pay it are not to *rebuild* it and not to *build* it:

* Not rebuild -- `scripts/issue-claim.sh --reuse` locally (already the default,
  #1012), and CI's `actions/cache` on `target/`, which already skips it
  whenever `Cargo.lock` is unchanged. Both are in place.
* Not build -- ADR 0014's own recorded alternative, `bundled-sqlcipher` with
  system libcrypto, which removes `openssl-src` from the graph entirely. ADR
  0014 parked it explicitly ("the recorded alternative if the vendored
  OpenSSL's build cost ever earns it") and this is the measurement it asked
  for: 28s serial per cold tree, on the critical path. It is a decision about
  hermetic builds for CI, the Flatpak and the release artifact, not a tuning
  knob, which is why it is written down here rather than done.
