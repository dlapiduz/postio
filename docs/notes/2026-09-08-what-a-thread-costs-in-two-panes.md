# What a thread costs, in both panes (2026-09-08, #1348)

ADR 0032 proposes rendering a conversation as one document in one `WebView`,
and ends with an open question it could not answer:

> Does WebKit's own memory for one large document beat N small processes? Not
> measured. It is the obvious rebuttal to "one view is cheaper" and nothing
> here has tested it.

Measured now, both arrangements, same fixture thread, same instrument, one
machine. `cargo run -p postio-app --example pane_comparison -- <stacked|document> <n>`.

## The numbers

| messages | arrangement | web processes | handover | renders | web Pss | total |
|---|---|---|---|---|---|---|
| 2 | stacked | 2 | 83 ms | 4 | 146 MiB | 300 MiB |
| 2 | one document | 1 | 59 ms | 2 | 101 MiB | 253 MiB |
| 10 | stacked | 10 | 298 ms | 20 | 378 MiB | 537 MiB |
| 10 | one document | 1 | 47 ms | 2 | 101 MiB | 253 MiB |
| 50 | stacked | 50 | 1.34 s | 100 | 1559 MiB | 1755 MiB |
| 50 | one document | 1 | 102 ms | 2 | 104 MiB | 256 MiB |

*stacked* is what the pane does today: one `Reader`, one `WebView`, one web
process per message on screen. *one document* is ADR 0032's.

## What it answers

**The open question, decisively, against the rebuttal.** One large document
does not cost what N small processes cost. It costs a flat ~101 MiB whatever
the thread length, while the stacked arrangement grows by roughly **31 MiB of
Pss per message**. At fifty messages that is 1.56 GB against 104 MiB.

**Handover is flat, not merely faster.** 59/47/102 ms against 83/298/1340 ms.
The one-document pane hands a fifty-message thread over faster than the stacked
pane hands over *two*.

**The stacked pane loads two documents per message.** 100 loads for a
fifty-message thread — every `Reader` renders twice on the way up. That is
#749's shape again, at construction time rather than on a keystroke.

## Two things the numbers are not

**Pss, not RSS, and the difference is the finding's credibility.** Fifty web
processes map the same WebKit libraries, and RSS counts those pages in full in
each of them: the first version of this measurement reported **7.3 GB** for the
fifty-message stacked case. That number is not wrong so much as meaningless —
it is mostly the same pages counted fifty times. Pss divides each shared page
by the number of processes mapping it, and is the only figure that can be
summed across processes and compared against one. The honest ratio is 15x, not
70x.

**Run as an example, so it reaches the real display** rather than the suite's
software path. #1307 records that no *test* here exercises the renderer a user
gets, because `headless-runner.sh` sets `WEBKIT_DISABLE_DMABUF_RENDERER=1`.
These numbers do not have that limitation, though which renderer was selected
was not verified.

## A gap in the instrument, found by using it

`postio_ui::test_support::documents_built` and `document_bytes` report **zero**
for the one-document arrangement. `render_thread` composes through
`compose_thread`, not `document_for`, which is where `note_document` is hooked.

That is not only a measurement gap. The bulk assertion in #1341 — the one
guarding against #749's 1.21 MB of inlined fonts returning — only watches
`document_for`. **A thread document is currently unguarded by it.** Whoever
lands the one-document pane has to route its composition through the same
counter, or that regression can come back on the new path with the test still
green.

## What this does not settle

Speed and memory are not the whole of ADR 0032. Its own text says the deciding
question is elsewhere: whether a screen-reader pass over an HTML conversation
is at least as good as the widget tree it replaces. That is still unmeasured,
and these numbers do not license skipping it.
