# An assertion about who did the work is not an assertion about the work (2026-09-02, #851)

`engine::a_seeded_body_is_actually_fetched` failed on CI and passed everywhere
else:

```
the seed left nothing worth fetching
```

The line was `assert!(queued > 0)` immediately after `seed_backfill`, and the
number it checks comes from `backfill::seed`, which counts only what
`enqueue` **accepted**:

```rust
let queued = candidates.into_iter()
    .filter(|candidate| backfill.enqueue(candidate.clone().into()))
    .count();
```

An already-queued message is refused, so `queued` answers *"did this call
create the work"* — not *"is there work"*. The engine is spawned and running
its own loop by then, and between the fixture giving the messages their uids
and the test's explicit call, that loop can queue them first. Then `queued`
is 0 with nothing wrong at all, and the test fails having proved nothing
about its own subject, which is the two assertions further down: a body is
claimed, fetched, and accounted for.

**A precondition that can be satisfied by somebody else is a race, however
obvious it looks.** The give-away is the phrasing: "the seed left nothing
worth fetching" describes the world, but the expression only describes this
caller's contribution to it. When those two readings come apart, the message
is what is true and the code is what runs.

The fixture now asserts the thing that actually has to hold — that there were
messages to give uids to — and returns the count, so a fixture that matches
nothing says so instead of letting the next line take the blame. That is the
same lesson as the `unwrap_or_default()` note above: a test's own setup
failing quietly turns into a confident accusation somewhere else.

Diagnosed by reading, not by reproducing: it has never failed on this
workstation, and the fix is not the sort that can be demonstrated by a green
run.
