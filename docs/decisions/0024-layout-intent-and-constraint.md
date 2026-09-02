# ADR 0024 — Layout intent is stored; the viewport's constraint is applied, never written back

- **Status:** Accepted (2026-09-02)
- **Date:** 2026-09-02
- **Decision by:** a `/ux-architect` session, on the question
  [#825](https://github.com/dlapiduz/postio/issues/825) raised: what should
  the layout do at laptop width?
- **Issue:** [#825](https://github.com/dlapiduz/postio/issues/825)
- **Related:** `PRODUCT.md` §9 (three panes, adapting), §19 (PLATE 1b),
  `crates/postio-gtk/src/shell.rs` (the modes and their thresholds),
  [#502](https://github.com/dlapiduz/postio/issues/502) (one owner for the
  reading pane), CLAUDE.md's motion budget
- **Decision:** The adaptive layout is **already designed and implemented**;
  what is missing is not a breakpoint but a distinction. Two different facts
  are stored in one boolean, and one of them is persisted. **A surface records
  what the user asked for, and the viewport's constraint is applied on top of
  it at render time. A constraint is never written back into the intent.**

---

## The question, and why the obvious answer is wrong

#825 was filed off a screenshot at 1100×700 showing three panes, against
`PRODUCT.md` §9's promise of "three panes on a desktop monitor, two on a
laptop". It read as an unimplemented feature.

It is not. `shell.rs` documents three modes and their thresholds, and
`install_breakpoints` wires them with `AdwBreakpoint`:

| Mode | Window | Shows |
|---|---|---|
| `ThreePane` | ≥ 1040px | sidebar, list, reader |
| `TwoPane` | ≥ 720px | list, reader |
| `MessageFocused` | < 720px | list *or* reader |

1100px is above 1040px. **Three panes at that width is the design working**,
and the screenshot was evidence of nothing. The prior art also settles the
questions #825 asked as open: which pane yields (the sidebar first, then the
list or the reader), and whether it is automatic (yes, by breakpoint) — and
`shell.rs` records why `GtkPaned` beats `AdwNavigationSplitView` here, which
is the draggable divider and the motion budget's "pane switches use no
transition". None of that needs revisiting.

So this ADR is not the decision #825 asked for. It is the decision the code
turns out to need, which the research found underneath.

## What is actually wrong

### One boolean, two facts

`Shell::sidebar_visible` is written by two authorities that mean different
things by it:

- `set_mode` writes it as **what the viewport affords** —
  `self.set_sidebar_visible(mode == Mode::ThreePane)`
- the header toggle writes it as **what the user asked for**

and `Window::save_state` persists whatever it currently holds.

The consequence is a bug nobody would find by reading either half. Narrow the
window until the sidebar drops, quit, and reopen at desktop width: the sidebar
is gone. The breakpoint's answer was saved as the user's preference, and
nothing at startup corrects it — `restore` runs before `install_breakpoints`,
and at a wide size no breakpoint matches, so no `apply` or `unapply` handler
ever runs. The user is left with a missing sidebar, no explanation, and a
toggle they must rediscover.

`set_mode`'s own comment — *"A toggle afterwards still wins — the property is
the last word"* — is true and is exactly the problem: the property is the last
word for two different sentences.

### A mode with no navigation into it

`Mode::MessageFocused` shows the list *or* the reader, chosen by
`Shell::focused_pane`, which defaults to `Pane::List`. `set_focused_pane` is
called from two places: the finder, saving and restoring around the palette,
and the composer, claiming the pane while composing.

**Opening a message does not call it.** `Command::OpenMessage` fills the
reader and leaves the shell showing the list, so in the narrowest mode the
primary action of a mail client displays nothing new. There is no `Back` path
either — nothing returns `focused_pane` to the list once something has claimed
the reader.

The mode is reachable: the panes' minimums are 280px for the list and 320px
for the reader, so a window can be dragged to 600px and below.

## The principle

> **Intent is stored. Constraint is applied. A constraint is never written
> back into the intent.**

Two properties, two owners, one derivation:

- **Intent** — `sidebar_wanted`, `focused_pane`. Written only by the user's
  own actions: the toggle, opening a message, going back. Persisted where it
  makes sense to persist.
- **Constraint** — `mode`. Written only by breakpoints, from the window's
  width. Never persisted, because a window that opens at a different size has
  a different answer and last week's answer is worse than none.
- **Effective state** — computed from both, every time, in `apply()`.

This is the same shape #502 arrived at for the reading pane: one function of
the active facts, computed fresh, rather than several owners each restoring
its own snapshot. The lesson generalises, and this ADR is where it is written
down so the next adaptive surface does not have to rediscover it.

## What that means concretely

**The sidebar.**

- `sidebar_wanted: bool` — the user's standing preference. Persisted.
- `set_mode(m)` sets effective visibility to `wanted && m == ThreePane`. It
  does **not** touch `wanted`.
- The toggle sets effective visibility directly, and updates `wanted` only in
  `ThreePane` — so asking for the sidebar on a narrow window is an override
  for as long as you stay narrow, not a new standing preference. This keeps
  `shell.rs`'s existing promise that *"the sidebar is still reachable in the
  narrower modes"*.
- `save_state` persists `wanted`, never the effective value.
- At startup the default mode is `ThreePane`, so `effective = wanted` before
  any breakpoint fires, and a narrow window corrects it a frame later. This is
  why `restore` running first is safe.

**The focused pane.**

- Every navigation that changes what the user is looking at declares it:
  `OpenMessage` and the cursor-fills-the-pane path claim `Pane::Reader`;
  `Back` claims `Pane::List`.
- Those calls are unconditional. `set_focused_pane` already documents itself
  as *"harmless in the wider modes: it is recorded, and takes effect if the
  window is ever narrowed"*, which is exactly the property that makes this
  safe to call everywhere rather than behind a mode check.
- No caller asks what mode it is in. That is the point: a surface that
  branches on the viewport is a surface that will be wrong in the mode its
  author was not thinking about.

## Consequences

- Narrowing and widening is lossless. The sidebar comes back because the
  preference was never overwritten.
- Quitting narrow and reopening wide behaves.
- One-pane mode becomes usable: opening a message shows the message, and
  `Esc` comes back to the list.
- Two new pieces of state, both cheap, and one of them replaces a persisted
  field rather than adding one.
- A mode check in a navigation handler becomes a smell with a name.

## What is explicitly not changing

- The thresholds (1040px, 720px) and the mode names. They came from canvas
  1b's proportions and nothing here disputes them.
- `GtkPaned` over `AdwNavigationSplitView`, and the draggable divider.
- No transition on a mode switch. `apply()` is `set_visible` calls and stays
  that way.
- `PRODUCT.md` §9's sentence. It is right; it is just the only place a reader
  would look and it does not name the widths. §9 gains a pointer to the table
  in `shell.rs` rather than a copy of it, so the two cannot drift.

## How this is verified

The derivation is two pure functions over a boolean and an enum, tested in
`shell.rs` without a display: `sidebar_shown` and
`sidebar_wanted_after_toggle`.

What broke, though, was not the rule but *which authority wrote the
property* — so the cases that matter drive a real window, in
`tests/gtk_suite/gtk_layout_intent.rs`:

- narrowing hides the sidebar and widening brings it back
- a sidebar the user turned off stays off across a resize
- reaching for it on a narrow window is not a standing preference
- opening a message gives the reader the screen when there is room for one

Two of those fail against the previous code for the reasons above: `set_mode`
used to re-show the sidebar on widening whatever the user had asked, and
nothing moved `focused_pane` when a message opened.

**One gap, stated rather than hidden.** That `save_state` persists the
preference and not the constraint is a one-token substitution
(`sidebar_wanted()` for `sidebar_visible()`) and is *not* covered by a test.
`Window::save_state` writes to the real user state file, and `gtk_suite` is a
shared process where setting `XDG_STATE_HOME` would leak into every other
case. The property it depends on — that narrowing leaves `sidebar_wanted`
alone — is tested. Closing the gap properly wants `save_state` to take a path,
which is a wider change than this one.
