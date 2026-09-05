# An event with no consumer is a feature that does not exist (2026-08-28, #396)

`postio_runtime::engine` had emitted `Event::BodyLoaded` since it was written.
It was documented, it was covered by `postio-core/tests/events.rs`, and the
only match arm on it anywhere was `SyncTracker::apply` returning `false` on
purpose. So a person who opened a message whose body was not local watched the
"Downloading this message" plate stay up after the bytes had landed, until
some unrelated redraw happened to correct it. Every layer passed.

This is `postio-bl2`'s shape one layer up, and worth naming separately because
the usual check does not catch it. "Can a person reach it?" asks whether a
*gesture* has a handler. This is the opposite direction: an *announcement* with
no listener. The same question works, asked backwards — for every event the
runtime emits, who repaints?

**Where a consumer goes when it is not the sidebar or the list.**
`Feeds::apply` is the one call the composition root makes with every event,
but the two feeds inside it are `postio-gtk`'s own, and `postio-gtk` may not
read a body. The reading pane's contents are `postio-app`'s. So the seam is
`Feeds::connect_event`: the composition root registers its surfaces, and
everything on screen is still fed by that one call rather than by a second
event stream nobody remembers to drain. Prefer it over threading a new handle
through `commands::apply` for the next surface that needs an event.

**Two things every such consumer needs**, both of which the reading pane got
wrong-by-omission first:

- **Who it is for.** A backfill commits thousands of bodies. Only an arrival
  for what the surface is *showing* changes anything, and the guard belongs
  before the store read, not after it.
- **How often.** These arrive in bursts, so the repaint is coalesced onto the
  next turn of the main loop with a `queued: Cell<bool>` and
  `glib::idle_add_local_once` — `Folders::reload` is the pattern, and it is
  the difference between one store read and twenty for the same message.

The conversation pane (ADR 0015 Q4) is still not repainted this way: its
entries are built by a factory and it has no seam for refilling one — #739.
