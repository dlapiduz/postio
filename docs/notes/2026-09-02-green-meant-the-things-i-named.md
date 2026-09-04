# Green meant "the things I named" (2026-09-02, #419)

`main` went red twice in one day, both times invisibly to the process that
admitted it, and both times the same shape: **a test target stopped
compiling and nothing that ran had any reason to compile it.**

`Event::BackfillProgress` gained a `footprint` field and `Attachment` gained
`part_headers`. Six call sites in `postio-gtk` — `src/feed.rs`,
`tests/gtk_suite/gtk_feeds.rs`, `tests/gtk_accessibility.rs` — still built
those types by literal. The *libraries* compiled, so `cargo build` was green
and `cargo check -p postio-core` was green. Only `--all-targets` on the
consuming crate said otherwise, and `issue-land.sh` runs its gates over the
crates **you changed** — which for that branch was `postio-core`.

The trade was deliberate and mostly right: proving your own crates is fast,
and CI proves the workspace. With CI paused, the reconcile pass is the only
backstop, and it runs days apart on a repository where several sessions land
concurrently. So the cost lands on whoever touches the broken crate next, who
has every reason to think they broke it.

**The fix is one line of gate, and it is a `check`.** `cargo check --workspace
--all-targets` after the per-crate gates: no codegen, no linking, nothing
executed — the cheapest question that covers the crates nobody named. It is
skipped on a host that cannot build the GTK crates, where it would fail on
system headers rather than on the branch, and that host's PR already carries
`needs-linux-verify`.

Measured here rather than asserted: **6m20s against a cold target directory,
0.6s warm.** A landing pays somewhere between the two, depending on how much
of the graph the per-crate gates already compiled — close to nothing for a
`postio-gtk` branch, most of the frontend for a leaf-crate one, which is
exactly the branch whose blast radius nobody can see. That is the cost of the
bug it prevents being invisible: the alternative was paying it in someone
else's session, with a red `main` and a false suspect.

**Reverse dependencies were considered and rejected.** "Check the crates that
depend on what you changed" is the precise answer, and computing it means
parsing the dependency graph in bash to save less than the whole-workspace
check costs once the per-crate gates have warmed the cache. Precision that
buys nothing is a second thing to get wrong.

### The other half: an observation that died silently

#327's regression test polled `SELECT body FROM search_documents`. #379
deleted that column. The query was wrapped in `unwrap_or_default()`, so *no
such column* became *empty string* became *not indexed yet* — and the test
timed out after five seconds accusing a feature that worked perfectly. It had
been dead since #379 landed, and its failure message was confident and wrong.

**`unwrap_or_default()` on a test's own observation converts schema drift
into a false accusation.** The default is indistinguishable from the "not
ready yet" the test is polling for, so the test cannot fail the way it was
written to fail — it can only time out blaming something else. In a test's
observation path, unwrap and let it panic: a test that cannot read what it is
watching should say *that*, not spend the timeout inventing a story about the
code under test.
