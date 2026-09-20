# Building a reading pane on a WebView (2026-09-08, #1316)

Written while building ADR 0032's one-document conversation, but none of it is
about that proposal: these are the rules any Postio surface backed by a
`WebView` has to obey, and most of them were learned by breaking them first.

## A render is a whole document, so it must be worth one

JavaScript is off in the reader by construction (ADR 0003). There is no
incremental path: changing anything means composing a new document and handing
it to WebKit, which tears the old one down and parses the new one. The scroll
position goes with it.

That makes "how many documents did this cost" the number to watch, and it is a
count rather than a duration, so it is the same on every machine. `#749`'s
fourth cause was a repaint that changed nothing; a conversation pane repeated
it a year later in a new place. Two rules came out of it:

- **Coalesce arrivals.** Bodies arrive one per turn of the main loop, so a
  pane that draws on arrival costs one document per message. Draw when the
  thing is whole, with a deadline so something that never arrives cannot stall
  the pane.
- **Refuse a document identical to the one loaded.** Several things queue a
  redraw and they overlap; the guard belongs in one place rather than at each
  of them.

`ConversationView::thread_renders` is the observable, and an `app_suite` case
holds it to a bound.

## What the user does inside the document is invisible

With JavaScript off, Postio cannot see a `<details>` toggle, a text selection,
or a scroll. This is the constraint that most shapes the design, and it has two
consequences worth stating separately:

- **Reloading destroys state the application never knew about.** Which is the
  real reason the coalescing above matters: it is not only speed.
- **Anything the application must know, it has to own.** Expansion is the
  reader's state, not a function of the model. Deriving it on each redraw from
  `seen` and focus meant a message folded shut under the person reading it the
  moment resting on it marked it read — the model moved underneath a decision
  that was never the model's to make. Decide once per message, then keep it.

A verb the application must observe has to be a navigation intercepted in
`decide_policy`, which is a reload — so it is affordable for a verb and not for
a disclosure triangle.

## One document may hold several senders, but only because of the sanitiser

`postio_body::sanitize` removes `<style>` tag-and-contents, strips every inline
`style`, and parses to a tree rather than passing text through. That — and only
that — is why several senders' markup can share a page without contaminating
each other. It was true for reasons that had nothing to do with this, and
anything that weakens it takes the one-document design with it.

Two things follow for addressing:

- **A private scheme must never be sender-writable.** `postio-cid:` was: only
  `cid:` is rewritten, and the scheme sits in `add_url_schemes`, so a sender
  writing `src="postio-cid:..."` reached the reader's own resolver. Harmless
  while a document is one message; under one document it reaches *another
  message's* parts. Dropped now, in both modes.
- **A reference has to name its message** once a document holds more than one.
  `sanitize_body_in` stamps the scope on the way out, `/` separates, and that
  is unambiguous because `percent_encode` escapes `/` — an encoded
  `Content-ID` can never contain a literal one.

## Measuring it: two counters that lie

Both of these produced a confident wrong number in one afternoon.

- **`pgrep -P` does not find the web process.** WebKit puts it under a `bwrap`
  sandbox, so it is a grandchild. `-P` reported *zero web processes*, which
  reads exactly like the result a one-document experiment is hoping for. Walk
  the parent chain. And match `WebKitWebProces` — Linux truncates `comm` to
  fifteen characters, so `-x WebKitWebProcess` matches nothing and says so only
  on stderr.
- **`Reader::loads` counts every load that reader ever did**, not the ones a
  pane caused. It read four when the pane had drawn twice, which made a
  working fix look ineffective. Count the thing the claim is about.

And an assertion two apart from the behaviour it is refusing turns on timing:
`<= 2` on a four-message thread went flaky immediately, where twelve messages
and `<= 4` is the same claim with the difference unmistakable.

## What no test here can see

`scripts/headless-runner.sh` sets `WEBKIT_DISABLE_DMABUF_RENDERER=1`, which is
the right mitigation for #272 and should stay. It pins WebKit to its software
path, so **no test in this repository exercises the rendering path a user
gets** — #1307. Every WebKit number in this note and in
`2026-09-07-what-a-list-repaint-actually-costs.md` is a software-path number,
and the black flicker of #749/#947 lives on the path the suite cannot reach.
