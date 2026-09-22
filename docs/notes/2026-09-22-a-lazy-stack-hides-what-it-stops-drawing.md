# A lazy stack hides what it stops drawing (2026-09-22, #1586)

Counting the macOS reader's web views — `postio_ui::reader::cost`, the same
counters `postio-gtk` notes into — found the conversation pane had never
released one while it lived. Two things are worth knowing before touching any
SwiftUI surface that holds a platform view, and a third before writing a Swift
test that waits for one.

## `LazyVStack` pools platform views; it does not dismantle them

When an element of a `LazyVStack` stops being drawn — it scrolled away, its
`if` flipped to the other branch, or its id left the `ForEach` — SwiftUI does
not call `dismantleNSView` and does not release the view. It **hides** it
(`isHidden` somewhere up its superview chain) and keeps it in a pool it reuses
for the next element that needs one. Measured on this frontend's own stack:

| what happened | web views held |
|---|---|
| open a conversation, three messages open | 3 |
| collapse one of them | 3 |
| five conversations in a row, same pane | 18 |
| scroll a thirty-message conversation to the end and back | 30 |
| take the pane away | 0 |

Changing an element's id does not help, and neither does `.id()` on the
element's content — both were tried. What releases the pool is the stack
itself going: a new identity on the `ScrollView` (the pane now takes the
conversation's thread), or its hosting view being removed. An eager `VStack`
does dismantle on removal, at the price of building every open message up
front.

So for a `WKWebView` in a lazy stack, "not drawn" and "released" are different
things, and a content process follows the second. Reuse is also why releasing
on `viewDidHide` would be the wrong fix: the pool is exactly what stops a
scroll back up rebuilding a reader, which is the flicker ADR 0032 was written
about.

## An in-flight render holds what it captured

`ReaderView.Coordinator.load` builds the document off the main actor and then
loads it into the view. The closure captured `view` strongly, so a reader
SwiftUI had already let go of stayed alive until the store finished — on a
busy machine, long enough to look like a leak in a test that waited five
seconds. Capture a platform view weakly in anything that can outlive the view,
the same way `self` already was.

## A main-actor test must suspend, not spin the run loop

`RunLoop.main.run(until:)` from inside a `@MainActor` test turns the run loop
but **does not drain the main queue re-entrantly**, and the main actor's jobs
are on that queue. So a reader's render never resumed, never finished, and —
holding its view — made every release assertion fail. It read exactly like a
leak and was not one.

Suspend instead (`try await Task.sleep`), which gives the main thread back to
the run loop and lets main-actor work land; SwiftUI's updates arrive the same
way. `ReaderSurfacesTests.turn()` is the pattern. The compiler already refuses
`run(until:)` in an `async` function; a synchronous helper called from one is
the way people get around that, and it is the way to get this wrong.

The counters are per thread and every `@MainActor` suite shares the main
thread, so any other suite that builds a reader's web view while one of these
is suspended lands in its delta. Both such suites sit under one `.serialized`
parent, `ReaderWebViews`. A new suite that builds a `PassingWebView` belongs
there too.

## What is still open

A collapsed message's reader is held until the conversation changes, and a
long conversation read to the end holds one reader per message. The lazy stack
defers the cost; it does not bound it. That is ADR 0032's question — one
document, one view — asked on a platform where it has not been answered, and
it is not a wiring fix.
