# An event with no consumer is a feature that does not exist (2026-08-28, #396)

`postio_runtime::engine` had emitted `Event::BodyLoaded` since it was written.
It was documented, it was covered by `crates/postio-core/tests/core_suite/events.rs`, and the
only match arm on it anywhere was `SyncTracker::apply` returning `false` on
purpose. So a person who opened a message whose body was not local watched the
"Downloading this message" plate stay up after the bytes had landed, until
some unrelated redraw happened to correct it. Every layer passed.

This is `postio-bl2`'s shape (a bead id from the retired pre-GitHub tracker;
the reader-never-mounted bug CLAUDE.md still cites by that name) one layer up,
and worth naming separately because
the usual check does not catch it. "Can a person reach it?" asks whether a
*gesture* has a handler. This is the opposite direction: an *announcement* with
no listener. The same question works, asked backwards — for every event the
runtime emits, who repaints?

**Where a consumer went in the classic app** (removed in spec 007 T256, with
every seam named here). `Feeds::apply` was the one call its composition root
made with every event; the reading pane's contents were `postio-app`'s, so
the seam was `Feeds::connect_event`, and everything on screen was still fed
by that one call rather than by a second event stream nobody remembers to
drain. The rule outlives the code: one feed, consumers registered on it.

**Two things every such consumer needs**, both of which the classic reading
pane got wrong-by-omission first:

- **Who it is for.** A backfill commits thousands of bodies. Only an arrival
  for what the surface is *showing* changes anything, and the guard belongs
  before the store read, not after it.
- **How often.** These arrive in bursts, so the repaint is coalesced onto the
  next turn of the main loop with a `queued: Cell<bool>` and
  `glib::idle_add_local_once` — the classic `Folders::reload` was the
  pattern, and it is the difference between one store read and twenty for
  the same message.

The classic conversation pane (ADR 0015 Q4) was never repainted this way
(#739).
