# What a list repaint actually costs (2026-09-07, #1216)

`GtkListView` builds **one row widget per item in a changed range**, whether or
not that item is anywhere near the viewport. Everything else here follows from
that one measured fact, so it is worth stating first and plainly.

Measured per emission, over 20,000 messages, on a list whose viewport holds ten
rows (`app_suite::navigation_cost` prints these):

| what the model said | widgets built |
|---|---|
| `items_changed(0, 799, 0)` — reset to empty | 0 |
| `items_changed(0, 0, 799)` — the total arrived | 205 |
| `items_changed(0, 50, 50)` — a page delivered | 50 |
| `items_changed(50, 50, 50)` | 50 |
| `items_changed(200, 50, 50)` — partly past the tracked window | 5 |
| `items_changed(250, 50, 50)` — past it | 0 |

Two readings. The cost is linear in the *changed range*, not in what is
visible; and `GtkListView` tracks about 205 items, independent of viewport
height — 317px and 748px viewports both gave 205.

## Which made page deliveries the dominant cost

A folder switch built 410 row widgets, and 205 of them were page deliveries
re-announcing positions that already existed. A page landing on positions that
are already there is not "these positions hold different rows now"; it is the
same positions, holding the same messages, finally able to say what they are.
Saying it as a replacement is what cost the widgets.

The fix is in `postio-gtk::list`: one `MessageRow` per position, kept for as
long as the position means the same thing, filled in place by a delivery, and
announcing itself with `changed` rather than through the model. Effects, all
counted rather than timed:

- folder switch: 410 widgets to 205, eight `items_changed` to two
- reading one message: one emission and a page of rebuilt widgets to zero of each
- scrolling twenty pages over 799 rows: 536ms to 295ms with deliveries landing
  (the same loop, the same binary, the announcement re-added behind an env var).
  Both figures include a fixed ~8ms drain per step; net of it, about 2.8x.

Scrolling now costs **1.47ms a step and builds one widget over twenty pages**,
against a 16ms interaction budget.

## What was tried and did not help, so nobody tries it again

- **Deferring the total's emission to an idle turn.** Still 205 widgets, and
  slightly slower overall. The count is not about when the model speaks.
- **Not resetting to the previous scope's length first.** `source.total()` is
  already 0 at that point, so the reset emission is `(0, n, 0)` and costs
  nothing. There was nothing there to win.
- **Making a row cheaper to build** — dropping the two hidden `GtkLabel`s
  (`probe`, `sentinel`) and the `EventControllerMotion` that every row
  constructs. No measurable difference at either end. The 205 is GTK's own
  allocate-and-validate work, not Postio's per-row construction.
- **Renderer choice.** Settled already by #790 (see `docs/PERFORMANCE.md`) and
  worth not re-opening: five runs each of `gtk_reader` under `GSK_RENDERER=gl`
  and `=vulkan` completed 0/5 at a 45s cap and 1/3 and 0/3 at 90s — the
  variance is the test's, not the renderer's. `ngl` still resolves to
  `GskGLRenderer` on GTK 4.22, with a deprecation warning.

## The measurement caveat that applies to all of the above

`scripts/headless-runner.sh` exports `WEBKIT_DISABLE_DMABUF_RENDERER=1`, which
is the documented mitigation for #272's wedged handshake under nested mutter.
It pins WebKit to its software path, so **no test in this repository exercises
the rendering path a user gets.** Every reader and composer number measured
here — including the 28.7ms first `load_html` that `Composer::warm` now pays
early — is a software-path number. The list figures above are unaffected;
`postio-gtk::row` draws in one `snapshot()` and never touches WebKit.

## What is left

`take_pane` costs about 7ms on *every* composer open, which is most of the 9ms
one costs in total. It sets a CSS class on the shell and moves the focused
pane, so a full restyle is the obvious suspect. Inside budget, so not urgent —
but it is the next thing to look at if composing ever needs to be faster.

The reader's own remaining lever is ADR 0032 (the conversation as one
document), which would collapse a thread's N `WebView`s into one. Proposed,
not decided.
