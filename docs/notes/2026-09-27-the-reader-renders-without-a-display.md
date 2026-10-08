# The reader renders without a display (2026-09-27, spec 006)

**The constraint:** reader layout and pixels are asserted in
`postio-render`'s own tests, headlessly, against the snapshot the renderer
returns. The wall [the 2026-09-09 note](2026-09-09-the-suite-cannot-see-a-laid-out-page.md)
describes -- the test display lays nothing out, so every
`getBoundingClientRect` is zero -- no longer applies to the reader, and a
reader test that still skips "because this display reports no layout" is
skipping for a reason that has gone.

## Why it changed

The reading pane was WebKit, a separate process drawing into a surface the
headless compositor never presented. Spec 006 replaced it with
`postio-render`: Blitz lays the document out in Postio's process, and the
render returns a `RenderedDocument` -- the display list, the laid-out size,
every message's and link's box, and a text index whose clusters carry their
rectangles, colours and painted grounds. `rasterize` turns it into pixels
with `vello_cpu`, on the CPU, with no display at all.

So the questions the 2026-09-09 note said could not be asked can be:

| Question | Asked of |
|---|---|
| Where is this text drawn, and on what ground? | `TextIndex::clusters` (`rect`, `color`, `painted_ground`) |
| Does one message paint over another? | the raster, at the other message's text (`thread_document`) |
| Is this box where the layout put it? | `MessageBox::rect`, `LinkBox`, `anchors` |
| Is the text legible in the dark theme? | cluster colour against painted ground (`contrast`) |

## What still needs a display

The widget: `BodyView` tiles the snapshot into a GTK scroller, and its
scrolling, selection, zoom gestures and accessibility are GTK behaviour,
asserted in `widgets_suite`'s `body_view*` cases on the headless compositor.
They read the adjustment and the snapshot, not the screen -- GTK's
`WidgetPaintable` answers `None` until a real repaint, which is why those
cases pump the main loop and read through the scroller.

## Where to put a new reader assertion

Layout, colour, containment, fidelity: `postio-render/tests/`, at the
cheapest layer that can fail. What a person does to the view:
`widgets_suite`. That the application joins them: `focus_suite`, asserting the
words the open message drew (`body_text`), never the document it was handed.
