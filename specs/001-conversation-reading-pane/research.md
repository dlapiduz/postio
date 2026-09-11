# Phase 0 research: the conversation reading pane

**Feature**: `specs/001-conversation-reading-pane` | **Date**: 2026-09-08

Seven questions the spec deliberately left open, resolved against the tree
rather than from first principles. R3 is the one that decides the shape of this
work, and it needs the maintainer.

---

## R1 — How a conversation is rendered

**Decision**: **one document per conversation, in one rendering surface** —
ADR 0032's proposal.

**Rationale**: the spec chooses this without naming it. FR-013 requires every
message's body visible; FR-057 requires one surface reused across messages and
conversations; FR-063 requires resources released on leaving. A surface per
message satisfies none of them at a thread length worth caring about —
`gtk_reader.rs`'s `each_reader_costs_a_web_process_of_its_own` measured exactly
that, and ADR 0032 recorded thirty processes for a thirty-message thread.

Two of ADR 0032's four open questions are answered by the spec rather than by
measurement: whether the stacked conversation earns its cost (FR-013 says it
ships) and whether per-message chrome moves into the document (it must, since
a rendering surface cannot contain toolkit widgets).

**Alternatives considered**:

- *Window the views* (ADR 0032 alt. 1). Bounds cost, keeps accessibility, needs
  no scheme changes — but recycling tears a view down when scrolled far enough,
  which is FR-060's unpainted frame returning at the edges.
- *One message at a time* (alt. 2). Simplest, and reverses FR-013.
- *Per-message `<iframe srcdoc>` inside one document.* Attractive for R2 —
  an iframe is a hard containment boundary — but an iframe does not size to its
  content without script, so it fails the same way R3 does, and would need R3's
  answer anyway.

**Consequence**: ADR 0032 moves from Proposed to Accepted, amended by R2.

---

## R2 — Containing sender CSS once it is admitted

**Decision**: contain at **sanitize time by scoping**, not by trusting the
engine: every sender rule is rewritten to apply only within its own message's
container, an enumerable set of escape-capable properties is refused, and each
message keeps its own `.postio-body` box.

**Rationale**: this is the requirement C1 created. ADR 0032's argument that one
document is safe rests on *"the sanitizer already removes `<style>`
tag-and-contents and strips every inline `style` attribute"* — messages cannot
contaminate each other because their CSS is gone. FR-019 deletes that premise,
so containment stops being free exactly where it is hardest, with several
senders' rules live in one document.

Three parts of the answer already exist:

- **`contain_body()`** (#323) gives each sender's content a bounded box with a
  visible edge, described in its own doc comment as *"a security affordance as
  much as a visual one"*. Per message rather than per document, it is FR-020's
  anchor and FR-025's mechanism.
- **`Sheet::Senders`** already renders a message on the sender's own light
  palette — the canvas's *"paper-white sheet inset from the dark chrome"*,
  which the design brief proposed and which is built.
- **The CSP** in `content_security_policy()` already narrows `img-src` to
  `postio-cid:`/`data:` and, since ADR 0023, `font-src` to `postio-font:`. It
  is the enforcement point for FR-022 and FR-023 and it extends to CSS-named
  resources for free — a `background-image: url(https://…)` is refused by the
  same `img-src` that refuses an `<img>`. `style-src` must be narrowed to
  refuse `@import`.

### Settled 2026-09-08 (#1325): the question does not arise for inline styling

**`@scope` was not measured, and the first slice does not need it.** Almost all
HTML email carries its styling in `style` attributes — precisely because
clients have always stripped `<style>` blocks — and **an inline declaration
needs no scoping at all**: it applies to the element it sits on, which is
already inside the container `contain_body` draws around that message. So
admitting the `style` attribute delivers most of FR-019a with containment
reducing to a question about *properties*, not selectors.

That is what `#1325` implemented: `style` admitted on every element, and
`contain_declarations` refusing, declaration by declaration, what would let a
message act outside its own box.

**A `<style>` block is the part that still needs scoping**, and it is deferred
rather than done, because it needs a CSS parser: `postio-body` has none, and
adding one to a pure leaf is a dependency decision of its own. When it is
taken, prefer **selector rewriting at sanitize time over `@scope`**, for a
reason stronger than availability: rewriting is one implementation in a
toolkit-free crate that both frontends inherit and that is provable in
milliseconds without a display, while `@scope` would have to be verified
separately against WebKitGTK and against WKWebView, and could not be tested
without a display on either.

**The refused-property set** (FR-019b requires it be enumerable): `position:
fixed` and `sticky`, anything establishing a stacking context above the
message's own, viewport-relative sizing that exceeds the message box, and
`transform`/`inset` combinations that place content outside it. Each refusal is
a named test in `postio-body`, not a judgement call at render time.

**Alternatives considered**: trusting CSS `contain` alone (a containment hint,
not a security boundary); per-message iframes (see R1); rendering sender CSS
only on `Sheet::Senders` and never inline (rejected — C1 asked for ordinary
correspondence to be styled too).

---

## R3 — Deriving the current message from scroll — **decided**

**The problem**: FR-034 and FR-035 require the rail to mark the message
occupying the greatest visible *area*, updated as the user scrolls. Once R1
puts the whole conversation in one document, the application no longer knows
where any message sits: message boundaries are in document coordinates that
only the rendering engine has.

**The reader has JavaScript off**, at the settings level and by construction:
`hardened_settings()` sets both `enable_javascript(false)` and
`enable_javascript_markup(false)`. `PRODUCT.md` §21 and `CLAUDE.md` both state
it as a privacy guarantee, and ADR 0003 is where it was decided.

The codebase has already met one half of this problem and solved it without
script. `scroll_markers()` plants invisible anchors at `top: Nvh` and moves the
document by same-document fragment navigation, precisely because *"both
[scrolling APIs] are closed"* with JS off. That is application → document. The
rail needs **document → application**, and no fragment trick answers it: the
engine will not tell an observer where the user stopped.

**Three ways out, and none is free:**

| Option | What it costs |
|---|---|
| **A. Application script, sender script still refused.** Set `enable_javascript(true)` while keeping `enable_javascript_markup(false)`, so script *in the document* is still ignored and only Postio's injected observer runs. | Amends ADR 0003 and contradicts a sentence in `PRODUCT.md` §21 and `CLAUDE.md` as they are written. Needs the maintainer. |
| **B. Keep JS off; mark by navigation only.** The rail marks where the user jumped, not where they scrolled. | Fails FR-034 and FR-035, which exist specifically to forbid *"the last one you clicked"*. The spec would need amending. |
| **C. The application owns the stack.** Per-message surfaces in a toolkit scroller, where positions are native. | Fails FR-057 and reinstates ADR 0032's process cost. R1 is what rejects it. |

**Decision: A**, taken by the maintainer on 2026-09-08, gated on the spike
below.

The distinction A relies on is already expressed in this codebase — the two
settings are separate lines in `hardened_settings()`, and
`enable_javascript_markup(false)` exists precisely to say *"script that
arrived in the message does not run"*. What changes is that Postio's own
script would. The privacy claim that matters to a user — a message cannot run
code, cannot phone home, cannot see the reader — is unchanged; the sentence
that becomes false is the literal *"the reader's WebKit view has JS off"*.

**Two things the spike must prove before A is built on:**

1. That `enable_javascript_markup(false)` with JavaScript enabled genuinely
   refuses script arriving in a message — inline `<script>`, event-handler
   attributes, `javascript:` URLs — measured against a fixture written to try
   all three.
2. That an injected observer runs in an isolated world exempt from the
   document's own CSP, since `content_security_policy()` will continue to send
   `script-src 'none'` for the sender's sake.

### Both proofs hold — measured 2026-09-08, #1323

**R3a: the switch separates whose script runs.** A document carrying an inline
`<script>`, a `body onload` and an `img onerror`, loaded with JavaScript
enabled:

| `enable_javascript_markup` | The message's script | Postio's injected script |
|---|---|---|
| `true` (control) | **runs** — it rewrote the title | runs |
| `false` | **refused** — title untouched | runs |

The control is what makes this worth anything. The first version of the spike
carried `script-src 'none'` in its own fixture, so a pass would not have said
whether the *switch* or the *CSP* refused the script. The fixture now carries
no CSP at all, and the same document is run through both settings: the only
variable is the switch, and the switch is what refuses.

**R3b: the CSP does not govern the application.** `script-src 'none'` is
about what the *page* may load and execute; it has never constrained the host
application's own injections. This was already true in the tree before the
spike — `gtk_reader.rs`'s `computed()` helper has been evaluating JavaScript
against `document_for(...)` documents, which all carry that directive, for as
long as it has existed.

`document.title` is the channel, because `WebView::title()` reads it from the
application **without** script — so a document that changed its own title is
caught even in a view where nothing could be evaluated to ask.

**The decision stands, and the mechanism is the one it assumed.** What remains
owed is the ADR amendment and the two document corrections, in the phase that
flips the setting for the app. #1323 deliberately changes no shipped setting.

**What the decision changes, and what it does not.** The invariant a user cares
about is untouched, and the spec already stated it in the form that survives —
FR-024: *"The pane MUST NOT execute any script **contained in a message**."*
Sender script stays refused by `enable_javascript_markup(false)`; the network
stays closed by the CSP and the scheme handlers; nothing in a message can
observe the reader. What becomes false is the broader sentence that the
reader's view has JavaScript off, because Postio's own observer will run in it.

**Three documents must change in the phase that lands this**, and none of them
before it:

| Document | What it says now | What it must say |
|---|---|---|
| `docs/decisions/0003-rich-text-compose.md` | JavaScript off in the reader | Amended: sender script refused; application script permitted, with the two proofs recorded |
| `docs/PRODUCT.md` §21 | *"the reader's WebView has JavaScript off and network off"* | Script that arrives in a message never runs; the network stays off |
| `CLAUDE.md` | *"the reader's WebKit view has JS and network off"* | The same distinction, in one line |

Amending the ADR is the deliverable, not a note: this is an architectural
decision, and the reasoning has to outlive the session that made it.

---

## R4 — Counting renders

**Decision**: a counting facility for the rendering surface, modelled on
`postio_storage::test_support::counting`, exposing renders issued, surfaces
created, and bytes handed to the renderer per document.

**Rationale**: FR-065 requires the cost of moving be asserted as counts, and
Principle V requires budgets be gated as counts rather than timings because a
shared runner cannot defend a millisecond. The storage half of this exists
(`counted()` reads statements, rows and trigger firings off SQLite's trace
hook); the rendering half does not. Without it, FR-057 to FR-064 are prose.

The counter belongs on the seam a frontend calls, not inside `postio-gtk`, so
that both frontends are held to it and the assertions run without a display.

**Alternatives considered**: timing assertions (refused by Principle V);
counting web processes only (catches FR-057, misses FR-058's duplicate loads,
which is the defect #749 actually found).

---

## R5 — Coordination with the in-flight experiment

**Decision**: this work does not touch #1316, and takes its measurements as
input.

**Rationale**: #1316 is open, assigned, and its branch
`issue-1316-experiment-render-a-conversation-as-one-` is checked out in another
worktree; `origin/feature/one-document-conversation` exists beside it. Another
session is doing exactly R1's experiment now. Its acceptance criteria —
process count by thread length, resident memory, time to first paint at 2, 10
and 50 messages — are the evidence R1 asserts without.

**Consequence**: R1 and R3 should be confirmed against #1316's numbers before
the rail is built on top of them, and this feature should run as an initiative
on a feature branch rather than as a single issue.

---

## R6 — Retiring the collapsed conversation

**Decision**: amend ADR 0015 and delete `collapsed_runs()` with its tests.

**Rationale**: FR-013 supersedes ADR 0015's *"read ones collapsed"* and FR-015
supersedes its *"first unread in focus"*. `postio-ui/src/conversation.rs`
implements the superseded rule, including the divider FR-013 forbids. An
accepted ADR that a shipped surface contradicts is worse than either.

Per the constitution's no-backwards-compatibility rule this is a deletion, not
a deprecation: no flag, no both-ways.

---

## R7 — What the registry is missing

**Decision**: add reply-to-focused-message, forward-focused-message, and
dismiss-rail as registry commands; take no keys from the design brief.

**Rationale**: FR-054 requires every verb the pane offers to be a registry
command, and Principle II requires that a command absent from the registry does
not exist. The registry already binds `e` reply, `E` reply all, `a` archive,
`A` archive thread, and thread navigation, which covers FR-006 and FR-008.

`docs/keybindings.md` is generated from the registry by a test that fails on
drift, so the new commands' keys are chosen there and documented by
regeneration — not transcribed from the brief, whose table assigns `E` and
`⇧e` to different verbs while they are the same keystroke.

---

## R8 — The header must be shared before it is extended

**Decision**: land #1285 first.

**Rationale**: FR-004 requires exactly one definition of the toolkit-free
header rules, verified by a check. `postio_ui::reader::header` exists with
`address_line`, `address_list`, `subject_text`, `absolute_date`,
`MessageHeader::of` and `ReaderAction::ALL`, and `postio-gtk` still keeps
private copies of all six. Extending the header (FR-002, FR-002a, FR-008a)
before deduplicating means writing the conversation header twice.

#1285 also carries a red test of its own —
`a_key_lost_to_another_command_hides_the_hint_rather_than_showing_a_wrong_one`
— which must go green before this feature's own gates mean anything.
