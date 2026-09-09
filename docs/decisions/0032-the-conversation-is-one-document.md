# ADR 0032 — Proposed: the conversation is one document, not one WebView per message

- **Status:** **Proposed** (2026-09-06) — written to be argued with, not to be implemented from
- **Date:** 2026-09-06
- **Raised by:** the maintainer, reporting a black flicker when moving between messages, and asking directly: *"Why do we need a view per message in the conversation view? Isn't there a way to render all messages in the single view? Maybe with html?"*
- **Issue:** [#1216](https://github.com/dlapiduz/postio/issues/1216)
- **Revisits:** ADR 0015 Q4 (the conversation pane stacks every message of a thread)
- **Touches:** ADR 0003 (script off in the reader), ADR 0023 (fonts served over a custom scheme), `PRODUCT.md` §20 (accessibility)
- **Proposal:** render a whole conversation as **one document in one `WebView`**, with per-message chrome expressed in HTML, replacing the current one-`WebView`-per-expanded-message.

---

## The observation this starts from

A black flicker moving from one message to the next, and — watched live — a
WebKit web process spawning and dying about once a second while doing it.

Those are one thing. A process that has just started is still relocating its
libraries; the profile of such a burst is `do_lookup_x`,
`_dl_relocate_object_no_relro` and little else. A process with nothing drawn
yet composites black.

Two tests in `gtk_reader.rs` pin the mechanism:

* `rendering_the_next_message_keeps_the_web_process` — rendering a *second*
  message into the *same* reader reuses the process. `render` is not the cause.
* `each_reader_costs_a_web_process_of_its_own` — one reader is one process,
  two readers are two.

The conversation pane builds a `Reader` per expanded message. It is already
careful about it — `EAGER_EXPANSION_CAP` opens at most three, and `collapse`
keeps the widget so reopening is cheap — but the *first* expansion of each
message still makes one, **and nothing ever releases them**. Scrolling a
thirty-message thread ends with thirty processes.

The module doc predicted exactly this cost and bounded the wrong end of it:

> Where focus opens and how much expands are the two decisions with real
> consequences — one for whether the pane lands where you stopped reading, the
> other for whether a thirty-message conversation instantiates thirty
> `WebKitWebView`s.

Three at open. Unbounded on scroll.

## What was ruled out first, by measurement

**Sharing a `WebContext` does not share a process.** Three `WebView`s built on
one context produced three web processes. WebKitGTK runs a web process per
*view*, not per context, so the obvious fix — one context, scheme handlers
routed by URI — buys nothing at all. Worth recording because it is the first
thing anyone will reach for.

## Why one document is possible here, and would not be in most mail clients

The objection to putting several senders' HTML in one document is that they
contaminate each other: one message's CSS restyles the next, one unclosed
element swallows the rest. **In Postio they cannot.**

> **This paragraph's original reason is no longer true.** It read: the
> sanitizer "already removes `<style>` tag-and-contents and strips every
> inline `style` attribute — *so postio CSS always wins*", so every message is
> rendered under Postio's stylesheet and nothing else. #1325 admitted the
> inline attribute and #1326 admitted the `<style>` block, so a sender's CSS
> is no longer absent. **The decision stands; its argument had to be
> rebuilt.** An ADR whose reasoning is false is worse than one that is merely
> out of date, because the next person reasons from the reasoning.
>
> What holds now, in three parts, each with something that fails when it
> stops holding:
>
> * **A `<style>` block's selectors are rewritten** under the message's own
>   container before the document is composed (`postio_body::styles`,
>   #1326), so a rule naming `p` — or naming Postio's own chrome — can match
>   only inside the message it arrived in.
> * **Inline declarations are contained** by `sanitize::contain_declarations`
>   and the refusal tables, which drop what escapes a message's own block.
> * **`contain_body`'s non-visible overflow** is what actually stops a
>   `transform` painting over a neighbour (#1346). This one is load-bearing
>   for containment and not only for the visible edge it was added for
>   (#323): removing or flattening it looks cosmetic and is not.
>
> Parsing to a tree rather than passing text through is unchanged, and still
> answers the unclosed element.

That is the precondition, and it is met — now by construction rather than by
accident. It was originally met for reasons that had nothing to do with this,
which is exactly why it needed re-establishing when those reasons went.

Expansion needs no script either, which matters because the reader runs with
JavaScript off by construction (ADR 0003). `<details>` and `<summary>` are a
disclosure widget in HTML itself.

## What it would cost

**The chrome is GTK, and this is the whole of the difficulty.** Each message
carries a `ThreadRowView` header, an `ActionBar`, a remote-image banner, an
unsubscribe banner and a decode notice. They are GTK widgets *interleaved
between* rendered bodies, and a `WebView` cannot contain GTK widgets. One
document means moving all of it into HTML:

* **Accessibility.** Each body is `AccessibleRole::Article` today and Orca
  reads the GTK widget tree. `PRODUCT.md` §20 asks for a screen-reader smoke
  test before a screen is called done; this would move that surface into a
  document and make the HTML's own semantics the accessibility story.
* **The design system.** `ActionBar`, `NoticeBar` and the row widgets are real
  widgets, themed from the token layer. In HTML they would be re-implemented
  against the same tokens, in a second place.
* **Buttons without script.** Reply, archive, unsubscribe and "show images"
  are `connect_clicked` today. In a document with JavaScript off they become
  links navigated through a custom scheme and intercepted in
  `decide_policy` — which is a mechanism the reader already has, and which is
  also how a mistake becomes a navigation rather than a no-op.
* **`cid:` routing.** `postio-cid:` resolves against *whichever message is
  currently open*. With every message in one document that handle is
  ambiguous, so URIs must carry a per-message token and the handler must route
  on it. Mandatory here, where it was merely optional for the shared-context
  idea.
* **Remote images are a per-sender decision.** The banner allows images for
  *this sender*; a document-level network policy cannot express that, so the
  distinction has to move into how each message's images are addressed.

## What it would buy

* **One view, one process, forever** — independent of thread length. A
  200-message thread costs what a 2-message thread costs.
* **The flicker goes**, because nothing starts a process when focus moves.
* **Scrolling is the document's**, not a stack of widgets each with its own
  scroller.
* **Expansion becomes state in the document** rather than widget lifecycle,
  which is where the `expanded`/`shown` bookkeeping and its `collapse`-keeps-
  the-widget subtlety currently lives.

## The alternatives, and why they are worse or smaller

1. **Window the views.** Keep three or four `Reader`s and recycle them as the
   conversation scrolls, exactly as the message list is windowed over paged
   SQLite (`CLAUDE.md`: *never load a whole mailbox into memory*). Bounds the
   cost permanently, keeps every widget, keeps accessibility, and needs no
   scheme changes. **Strictly smaller than this proposal and strictly less
   good**: recycling still tears down and rebuilds a view when you scroll far
   enough, so the flicker returns at the edges rather than going away.
2. **One message at a time.** A single `Reader`, re-rendered as focus moves —
   which the tests show reuses its process. Simplest by far, and it reverses
   ADR 0015 Q4: the conversation stops being a stack and becomes a reading
   pane with navigation.
3. **Pre-warm a spare view.** Hides the latency without removing it. Process
   count unchanged; adds a warming state machine to hide a cost rather than
   fix it.
4. **Do nothing.** The cost is bounded by thread length and released when the
   thread changes. It is a flicker and some memory, not lost mail.

## What would have to be true to accept this

- A screen-reader pass over an HTML conversation is at least as good as the
  widget tree it replaces. **This is the one that should decide it**, and it is
  not a matter of opinion — it is testable with Orca before anything is built.
- The action verbs work through `decide_policy` navigation as reliably as
  `connect_clicked`, including the ones that are destructive.
- Per-sender image policy survives the move to one document.
- The token layer can dress HTML chrome without a second implementation
  drifting from the first.

## Open questions

- Does the stacked conversation earn its cost in daily use at all? If one
  message at a time is what actually gets used, alternative 2 is the answer and
  this proposal is a lot of work for a surface nobody wanted.
- Is a 200-message thread a real case, or is thread length bounded in practice
  by how mail is actually used?
- ~~Does WebKit's own memory for one large document beat N small processes? Not
  measured.~~ **Measured — see below.**

## What the experiment measured (2026-09-08, #1316)

Built behind `POSTIO_ONE_DOCUMENT`, on a feature branch, next to the stacked
pane rather than instead of it. The numbers, on this workstation:

| messages | web processes | document handed | resident |
|---|---|---|---|
| 1 | 1 | 607 µs | +4 MiB |
| 50 | 1 | 7.2 ms | +5 MiB |
| 200 | 1 | 25.6 ms | +5 MiB |

**One process and flat memory, whatever the thread's length.** That settles the
open question above, and settles it in this proposal's favour: the obvious
rebuttal does not hold. `gtk_reader::a_whole_thread_costs_one_web_process` pins
it — two messages and thirty cost the same, and the view adds exactly one
process.

The cost it *does* have, which the proposal did not predict: handing the
document over is linear in thread length and crosses the 16 ms interaction
budget somewhere past a hundred messages. Not what the stacked pane was failing
at, and worth knowing before anyone calls this free.

The maintainer's own reading of it, trying it on real mail: *"outside of
stylistic issues it seems to perform much better."*

### Three things the building of it found

**Every render is a full teardown and reload.** JavaScript is off (ADR 0003),
so there is no incremental path: a changed document is a new document. Bodies
arrive one per turn of the main loop, so rendering on arrival cost one document
per message — the first open of a thread was visibly slower than every return
to it. The pane draws when the thread is whole, holds out 400 ms for bodies
that have not come, and refuses a document identical to the one loaded.

**Expansion is the reader's state, not a function of the model.** Recomputing
it on each redraw from `seen` and focus meant a message folded shut under the
person reading it the moment resting on it marked it read. Decided once per
message and then kept.

**And with JavaScript off, the application cannot see a `<details>` toggle.**
This is the constraint the proposal did not state and the one that most shapes
what is still open: expansion the *user* performs is invisible to Postio, so
any reload loses it. It does not arise while nothing reloads — which is why
the coalescing above matters for more than speed.

### A hole it found in code that already ships

A sender writing `src="postio-cid:..."` directly passed through the sanitiser
untouched: only `cid:` is rewritten, and the scheme is in `add_url_schemes`.
Harmless while one document is one message, because it reaches that message's
own parts. Under one document it reaches *another message's*. Dropped now, in
both modes. Worth landing whether or not this proposal is ever accepted.

## Status

Still **Proposed** (reviewed 2026-09-09), and now built, measured and depended
on — but **not** accepted, because the thing this ADR names as deciding it has
still not happened: a screen-reader pass over an HTML conversation, against the
widget tree it would replace.

That is deliberate. Orca is a person's to run (`/gtk-design`), and this ADR
trades an accessibility guarantee for performance; letting the measurements
alone carry it to Accepted would be answering the easy half of its own
question. Everything below is what an agent could settle. **The outstanding
gate is a human's.**

### What has been settled since (2026-09-09)

The cost question, which the proposal left open, is answered. #1348 measured
both panes with one instrument, in Pss rather than RSS — the first attempt
reported 7.3 GB, mostly the same pages counted fifty times:

| | one document | a view per message |
|---|---|---|
| memory | flat, ~101 MiB | ~31 MiB per message, 1559 MiB at fifty |
| time to show a thread | 47–102 ms | up to 1.34 s at fifty |

The pane is what `specs/001-conversation-reading-pane/` builds on, and several
of its requirements now depend on this shape: FR-013 (every body visible) is
only affordable because of the flat line above, and FR-015, the conversation
rail and the per-message verbs are all built against one document.

### Amendment: containment is not free

The proposal assumed the sender's markup arrived stripped of styling. #1325
admitted the inline `style` attribute, so a document holding several senders
has to contain what their CSS can reach — which the widget tree got for free by
giving each message its own view. That cost is now part of "one document":

- `sanitize::contain_declarations` and the refusal table, which drop the
  declarations that escape a message's own block (`position`, `z-index`, and
  the viewport units)
- `contain_body`'s non-visible overflow, which is what actually stops a
  `transform` painting over a neighbour (#1346 corrected my claim that an
  inline style "has no selector therefore no reach")
- `style-src` naming no source to fetch from, so a sender's stylesheet cannot
  phone home (#1383). That day has arrived: #1326 admits `<style>` blocks, so
  this is no longer an unexercised second layer but a live one, behind
  `postio_body::styles` refusing `@import` and `@font-face` outright
- selector rewriting itself (#1326), which is the part the widget tree never
  needed because a message that owns its own view cannot name anything in
  anyone else's

Originally: proposed, and deliberately not started. It revisits an accepted ADR, moves a
surface out of the widget layer, and trades accessibility guarantees for
performance — none of which should be decided by whoever happened to be
profiling that week.
