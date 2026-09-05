# A window with a pending resize has no picture, forever (2026-09-03, #809)

`/gtk-design` ends with rendering a screen and looking at it, and on this
workstation that step had stopped working: `shot` printed `no frame after
5000ms — is the screen blanked or the window occluded?` and wrote no PNG.

The message named the trigger and hid the mechanism, and the difference is
the whole fix. Measured with a probe, on a screen that had blanked, with an
`AdwApplicationWindow` mapped, presented, and reporting a 600x400 allocation
at every step:

```text
after present                 picture: yes
after queue_resize + pump     picture: NO
after request_phase(LAYOUT)   picture: NO
after request_phase(PAINT)    picture: NO
after allocating the child    picture: yes
```

GTK refuses to snapshot a widget with a pending resize — it says so on
stderr, `Trying to snapshot AdwDialogHost without a current allocation`, once
per attempt — and a pending resize is serviced in the frame clock's layout
phase. A compositor that has stopped presenting never runs one. So **any**
invalidation after the last presented frame leaves the window permanently
unrenderable, and it is not a race that a longer wait resolves: waiting is
the one thing that cannot work.

Three things about it are worth carrying forward.

**The window's own width and height keep reporting the last good
allocation.** Nothing in the widget's public state says "this cannot be
drawn". That is why the failure reads as a mysterious missing frame rather
than as a queued layout, and why the first fix attempt went looking at the
compositor.

**Asking the frame clock for the phase does not work.**
`request_phase(LAYOUT)` and `request_phase(PAINT)` are both throttled by
exactly the thing that has stopped. Doing the layout directly does —
`child.allocate(child.width(), child.height(), -1, None)`, which is the call
a parent makes on its child, at the size the compositor last agreed to, so it
invents no geometry — and it is repeatable across further invalidations.

**A `GtkWidgetPaintable` over a native widget is the wrong tool for a
screenshot and the right tool for this question.** It answers out of the
surface, so on a stalled toplevel it is empty while the same call one level
down works fine. `postio_gtk::capture` therefore takes the picture from the
window's child and uses the toplevel paintable only to decide whether to warn.

### What a picture off a stalled surface cannot show

The widgets are drawn correctly. What is missing is anything a *different*
process composites: the reader's WebKit view comes out as a black rectangle,
which reads exactly like a broken reader. `shot` and `surface` now say so
when it happens, because handing someone that picture in silence is worse
than not rendering at all.

`GDK_TOPLEVEL_STATE_SUSPENDED` would be the compositor's own word for this
and would be better if it were set; mutter does not set it for a window that
has merely stopped receiving frame callbacks, measured here. So the test is
the empty toplevel paintable — after a wait, because on its own it cannot
tell "no frame yet" from "no frame ever".

### There is no offscreen path

Worth stating because #809 hoped for one, and it is the first thing the next
person will reach for: GTK will not snapshot an unmapped widget at all, so a
window that was realized but never presented has no picture and no way to get
one. `WidgetPaintable` and `snapshot_child` both return nothing.
`gtk_capture.rs` pins that, so the floor is written down rather than
rediscovered.

A compositor is therefore still required, which is why `headless-runner.sh`
now routes `shot` and `surface` onto the private one. #315 sends examples to
the real display on the grounds that an example is someone launching a
program to look at — these two render a file and exit, and routing them at
the session's display is what made the visual check depend on whether the
maintainer's screen happened to be awake.
