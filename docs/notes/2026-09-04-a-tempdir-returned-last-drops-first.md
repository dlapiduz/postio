# A `TempDir` returned last drops first (2026-09-04, #724)

`target/tmp` collected blob directories that nothing removed -- five on one
run, none on the next identical one. `TempDir::drop` calls `remove_dir_all`
and *swallows the error*, so a failure to remove is silent by design; the
directories were not failing to be removed, they were being recreated
afterwards.

The mechanism is drop order. Locals drop in **reverse** declaration order, so
a test that writes

```rust
let (engine, database, report, events, backend, directory) = engine_with_backend();
```

releases `directory` **first** -- while the engine thread is still fetching.
`remove_dir_all` takes the tree away, the sync pass then commits a blob, and
`BlobStore::commit` does `create_dir_all(parent)` for the parent it needs.
The directory is back, with nothing left to own it. Whether anything was in
flight at that instant is what decides it, which is the whole reason it looked
intermittent.

`Engine` already gets this right internally, and says so: its `jobs` sender is
declared before its `thread` handle so the channel closes before anything
waits on it. The *tuples the test builders returned* had no such ordering, and
a tuple cannot express one -- the order that matters is the call site's
binding order, not the builder's.

So the guarantee lives in a `Drop` body instead.
`crates/postio-runtime/tests/harness/mod.rs` has `BlobDir`, which holds the
engine and the directory and calls the synchronous, idempotent `Engine::stop()`
before the `TempDir` field goes. No call site can get that wrong.

Two things worth keeping:

- **Five of the seven runtime test files were already correct**, because they
  build the directory inline and declare it *before* the engine. Only the
  builders that returned a `TempDir` in a tuple were affected. The issue
  guessed this ("not all of them do") and it was right.
- **The test for this does not race.** `Engine::stop()` closes the job channel
  for every handle, not just the last one, so a clone retained past the
  bundle's death answers the question directly: drop the directory, then assert
  that a request on the retained handle is refused. Green means the engine
  stopped first; red means the tree went while it was still running. No
  latency, no sleep, no in-flight window to hit.
