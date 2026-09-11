# ADR 0034 — One composer in the pane, many in windows

- **Status:** Accepted (2026-09-11)
- **Date:** 2026-09-11
- **Decision by:** the maintainer, clarifying `specs/002-compose-editor/spec.md` on 2026-09-10. Asked how many drafts may be open at once and chose *one in the pane, many detached* over one composition total.
- **Feature:** `specs/002-compose-editor/` — FR-010, FR-011, FR-013, FR-014. Spec-driven work carries no issue (constitution 1.1.0).
- **Amends:** the singleton composer — `Window::composer` memoises one `Composer`, and `Composer::detach` reparents *that widget* into a window.
- **Related:** [ADR 0004](0004-composer-document-model.md) (the draft is the record, the DOM a working copy), #957 (the `WebView` budget this spends)
- **Decision:** **the composer stops being a singleton.** The reading pane holds at most one, every other open draft is a window of its own, and a draft is open in exactly one of them. The property that makes detaching lossless changes from *"the widget is never rebuilt"* to *"the draft is the record"*, which ADR 0004 already decided.

---

## What changes, precisely

Today there is exactly one `Composer`. `Window::composer` builds it once and
memoises it; `detach` moves that widget into an `adw::Window` and `attach`
moves it back. The doc comment on `detach` says why:

> The same widget, reparented — which is the entire reason nothing is lost.
> Every field, the identity override, the undo history and the cursor are in
> widgets that are never rebuilt, so "detaching keeps them" is a property of
> doing it this way rather than a list of things to remember to copy. A second
> composer built from the draft would have to copy each of them, and would be
> wrong about one of them eventually.

That argument is correct and it is why `c` while composing currently means
"show me the one I have" rather than "start another" — a deliberate reading,
and the one FR-011 reverses.

## Why reverse it

Because the alternative is worse in the case people actually hit. Someone
writing a long reply who needs to send a two-line note first has three
possible experiences: the app refuses (today), the app discards their reply
(nobody proposes this), or the app moves the reply aside and gives them a
fresh sheet. FR-011 names the third and forbids the first two by name — *"MUST
move the pane's draft into a detached window rather than refusing, discarding
it, or asking"*.

"Refusing" is what happens now, and it is quiet: `c` focuses the field you
were already in. Nothing explains that the app has declined; it simply looks
like the key did nothing.

## What replaces the safety property

Not a list of things to copy. ADR 0004 already decided the shape:

> the DOM is a working copy, never the record

The record is the `Draft`. A composer that opens a draft and a composer that
resumes one run the same path — `resume` — and `gtk_composer_resume.rs` now
asserts that the path carries text, formatting, recipients *and* attachments
(spec 002 T050). So moving a draft between surfaces is a save and a resume,
not a reparent, and what makes it lossless is a thing already tested rather
than a structural accident.

Two things genuinely do not survive that route, and both are named here so
nobody rediscovers them as bugs:

- **The undo history.** `EditHistory` lives in the `Editor`, and a new editor
  starts empty. A draft moved to a window keeps its text and loses its undo
  stack. Accepted: the alternative is serialising an edit history through the
  draft, which is a great deal of machinery for the rarer half of a rare
  gesture.
- **The caret.** Restored to the start of the body, as `apply_identity`
  already does after a signature swap, for the reason given there — preserving
  an exact offset across a reload needs a script round trip this has not
  earned.

`detach` keeps reparenting, because moving *the* composition to a window
loses nothing and there is no reason to make it worse. What is new is the
second composer, built when the pane's draft is pushed aside.

## Attaching, when the pane is taken

Found while writing the tests, and not obvious from the requirements: `attach`
moves a detached composition back into the reading pane, and once there can be
a second draft already there, it has somewhere to fail.

The rule that follows from FR-010 — the pane holds **at most one** — is that
attaching is the same gesture as opening: the pane's current draft is pushed
into a window of its own, and the arriving one takes the pane. Not refused
(the quiet failure this ADR exists to remove), and not a swap, which would
teach that `mod+shift+o` sometimes means "trade places".

An attach where the pane is empty is unchanged.

## What implementing this touches

Recorded because the shape is not obvious from the requirements and the first
session to pick it up should not have to rediscover it:

- `Window::composer` memoises into a `OnceCell` and every caller expects the
  pane's one. It becomes "the pane's composer", replaceable, with the detached
  ones held separately.
- The shell owns the reading pane through one occupant (#502). A composer
  leaving the pane has to hand it back before the next one takes it, in that
  order, or the pane briefly has two claimants.
- Command dispatch. Looked at properly after this ADR was first written, and
  the problem is sharper than "it reaches `window.composer()`": every composer
  subscribes to the window's commands itself, in `mount`, with

  ```rust
  window.connect_command(move |id| composer.dispatch(id));
  ```

  so **N composers means every command fires on all N of them**. `Send` would
  send every open draft. That is the thing to fix first, and it is a guard
  rather than a rewrite: a composer acts on a command only when it holds the
  keyboard, which for the pane's one means the main window is active and for a
  detached one means its own window is. `handle_key_in` already reads focus
  off the source window rather than the main one, so the notion exists.

  Two cases need deciding rather than guarding. `CommandId::Compose` pressed
  while a *detached* window has focus should still open the new draft in the
  pane — FR-010 says the pane is where composition happens — so that one is
  routed rather than filtered. And a command from the palette arrives with the
  main window active, which makes it the pane's, which is right.

- The detached window's own key controller already calls `handle_key` on *its*
  composer instance, so per-window key routing needs nothing. Only the
  `connect_command` broadcast above does.

## The cost, stated

Every composer is a `WebView`, and `gtk_suite` is already short of them
(#957): its WebView-heavy cases fail all retries under a loaded binary. N open
drafts is now N engine processes in the running application too.

This is a real cost and it is bounded by a person's willingness to open
windows, which is a much lower ceiling than a mailbox. It is recorded so that
if the composer ever grows a second `WebView` of its own, someone remembers
that the multiplier is open drafts.

## What was rejected

**One composition total** (today). Rejected by the maintainer on 2026-09-10
when the question was asked directly.

**Asking** — "you have a draft open, start another?" Rejected by FR-011 by
name, and it is the wrong instinct anyway: the answer is always yes, and a
dialog whose answer is always yes is training to dismiss dialogs.

**Many in the pane, as tabs.** Not offered, and worth saying why: the reading
pane is where a *message* is read, and the composer already takes it over
(`docs/PRODUCT.md`). A tab strip inside a surface that is itself borrowed is a
second navigation level inside a temporary one, and `/ux-architect` names a
new navigation level as an architecture decision rather than a screen
decision. Windows are the level the desktop already provides.

## How it is verified

- `gtk_suite`: starting a second draft while the pane holds one leaves two
  composers, the first in a window, with both drafts intact (FR-010, FR-011).
- `gtk_suite`: asking for a draft that is already open brings its surface
  forward rather than opening a second view of it (FR-013).
- `app_suite`: the keystroke path, from `c` in a window that already has a
  draft, since that is the join the widget tests cannot see.
- Every open composer says which draft it holds, and a detached window is
  identifiable from the window list without being focused (FR-014).
