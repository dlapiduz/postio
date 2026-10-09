# Three cycles, and why fixing them one at a time looked like no fix (2026-09-02, #794)

A test binary that stood up a `WebView` passed and then died on the way out,
WebKit saying once per live view that the WebProcess had not exited after the
UI process closed the connection. It was intermittent, and it failed pull
requests that had nothing to do with it — three separate branches in one day.

The cause was three independent `Window -> … -> Window` reference cycles:

```rust
reader.connect_rendered({ let window = self.clone(); … });   // Reader holds it
reader.connect_command({  let window = self.clone(); … });   // Reader holds it
let source = { let window = self.clone(); move |cid| … };    // WebContext holds it
```

The window's imp holds the `Reader`; the `Reader` holds the first two
handlers; and the third becomes the `Rc<dyn BlobSource>` the reader hands to
its `WebContext` — so that one closes the loop **inside WebKit**, which is why
destroying the window never broke it.

**Each was found and fixed alone first, and each looked like a failure.** Any
one of the three keeps the window alive, so the measurement does not move
until the last one goes. Two other candidate fixes were built and disproved on
the way — destroying the toplevels, and breaking the cycle without the blob
source — and both were abandoned as "not it" when they were partly it.

**When several owners can each independently pin an object, a fix for one is
indistinguishable from no fix.** The way out is not a better guess: it is a
measurement that isolates one holder at a time. What finally worked was asking
progressively smaller questions — does the *window* leak, or the reader? does
a window leak with no reader at all? does a bare window leak once destroyed? —
until a single line changed the answer.

The other half is GTK's, not ours: a `GtkWindow` joins the toplevel list when
it is **constructed**, not when it is presented, and leaves on destroy. So
dropping the Rust handle is never enough on its own, and a test that builds
windows has to destroy them. Both halves are asserted in
`gtk_window_teardown.rs`, including the GTK half, so that a future reader does
not delete the destroy as redundant.

Tested as a leak, not as a crash. The segfault is a race that has never
reproduced on this workstation — the binary that failed on CI passes 25 runs
out of 25 here, with and without the fix — so a green run proves nothing about
it. The leak underneath is deterministic and takes milliseconds: hold a
`WeakRef`, drop, turn the loop, ask.
