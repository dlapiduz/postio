# The CI cache was the wrong shape, not cold (2026-09-03)

Long-standing complaint that CI "feels colder than it should". The cache was
fine -- exact key hit, 3.1 GB restored successfully in both jobs. It was the
wrong *kind* of artifact. Counting units rebuilt on run 33828724926:

```
Tests    21 units    <- 20 of them our own crates, which CI deliberately drops
Clippy  380 units    <- rebuilt from unicode-ident up
```

`cargo clippy` compiles through `clippy-driver` and emits **check** units;
`cargo test` emits **build** units. Cargo fingerprints them separately, so a
`target/` full of one is worth nothing to the other, and the Clippy job was
restoring 3.1 GB and then rebuilding the graph anyway -- about seven minutes
of runner on every pull request.

The fix is one cache per kind of build (`cache-name` in
`.github/actions/rust-workspace`), not a third-party cache action. Note what
it does **not** buy: Tests is 11.9 minutes and Clippy 8.7, they run in
parallel, so a landing is gated by Tests either way. This frees a runner and
makes Clippy a fast early signal; it does not shorten a landing.
