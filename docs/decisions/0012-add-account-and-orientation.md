# ADR 0012 — Adding a second account, and orienting a first-time user

- **Status:** Accepted — **GO** (2026-08-24)
- **Date:** 2026-08-24
- **Issue:** [#64 Setup wizard: add-account flow and first-run keyboard orientation](https://github.com/dlapiduz/postio/issues/64)
- **Related:** [ADR 0005](0005-multiple-accounts.md) (what a second account
  needs to exist), [#57](https://github.com/dlapiduz/postio/issues/57) (probe
  cancellation), `docs/ARCHITECTURE.md` §2 and §10
- **Decision:** the onboarding **form** stays exactly as it is and gains a
  second *host* — a dialogue over the running shell — rather than a second
  implementation. The real work is in the composition root: `open_account` is
  split so an account can **join a running application** without a restart. The
  keyboard orientation is a one-time plate whose text is **rendered from the
  keymap**, dismissed by the first *command* the user runs, and remembered in
  the `settings` table.

---

## What is already built

`postio-gtk/src/onboarding.rs` is unusually well set up for this, and its
doc comment says why: it draws the form and nothing else, because the view
layer may not link `io-imap` or `rusqlite`. The seam is already there —
`connect_probe` and `connect_submit` — with `postio-app/src/onboarding.rs`
doing the probe, the connection test and the two writes on the other side.

| Piece | State |
|---|---|
| One-screen form: address → probe → confirm | Built, and deliberately one step |
| `connect_probe` / `connect_submit` seam | Built |
| `Settings` as a view-owned shape, not `AccountSettings` | Built |
| Probe on *commit*, never on keystroke, for the privacy reason | Built |
| `onboarding::install` replacing the window's content | Built — and single-use by construction |
| A path for an account to join a running app | **Absent** |
| Anything that tells a new user the app is keyboard-first | **Absent** |

---

## Q1 — One form, two hosts

The issue is right that a second implementation would be wrong, and the reason
it cannot simply be reused as-is is presentation: `install` *replaces the
window's content*, which is correct at first launch, when there is nothing to
replace, and wrong when there is a mailbox on screen behind it.

**Decision: `Onboarding` keeps being a plain widget with no opinion about where
it lives. Two hosts present it.**

| | Host | Why |
|---|---|---|
| First run | the window's content, as today | There is no shell yet, and a dialogue over an empty window is a dialogue over nothing |
| Add account | an `AdwDialog` over the shell | The mailbox stays visible and the flow is escapable, which is what makes it feel like a setting rather than a restart |

`install` splits into `Onboarding::new()` plus two thin presenters. The form,
its states, its probe timing and its seam are untouched — which is the test of
whether this decision is right: if the widget needed changing, the reuse was
not real.

**The entry point is a registered command**, `account.add`, in
`Context::Sidebar` and everywhere the Settings surface is. Per
`ARCHITECTURE.md` §2, a command that is not in the registry does not exist —
so this is what puts *Add account* in the palette and the cheat sheet, which is
where a keyboard-first user will look for it before they look in Settings.

---

## Q2 — The work is not the form; it is joining a running application

`postio-app/src/lib.rs` decides once, at startup, whether to run
`open_account` or `onboarding::install`. `open_account` installs the feeds,
starts the engine, and wires the window — for *the* account.

**Decision: split `open_account` into `attach_account(account, …)` and call it
from both places.** Attaching means: create the account row and its keyring
entry, start an engine for it, register its feeds, add it to the sidebar, and
emit the events that make the frontend repaint. Nothing about it is
startup-specific once it is written that way, and the difference between "the
app started with one account" and "the app gained one" stops existing.

This is where [ADR 0005](0005-multiple-accounts.md) is a hard prerequisite and
not a related issue. Without it there is `first_account()`, one engine and
`AppState.account`, and "add a second account" has nowhere to put the result.
With it, `attach_account` is the natural shape of what ADR 0005 already
requires the composition root to do N times at startup — so this issue is
mostly the *entry point* to work ADR 0005 pays for anyway.

**Order of work:** ADR 0005's engine-per-account and `Scope`, then
`attach_account`, then the dialogue host. Building the dialogue first produces
a form that collects an account and has nowhere to put it.

---

## Q3 — [#57](https://github.com/dlapiduz/postio/issues/57) stops being latent

The probe's cancellation bug is currently hard to see, because at first launch
there is exactly one probe and the window has nothing else in it. Add-account
runs the probe a **second time in one process**, over a live shell, and the
user can cancel by closing the dialogue.

A probe whose callback fires after its screen is gone now lands somewhere with
state: a stale `Settings` written into a dialogue the user re-opened for a
different address, or a status set on a widget that is no longer in the tree.

**Decision: fix #57 first, as part of this work rather than after it.** The
mechanism already exists — `postio_imap::cancel::CancelToken`, which
`discovery` takes and does not honour on every path — and closing the dialogue
must cancel the token. This is also the same token
[ADR 0006](0006-oauth-and-provider-presets.md) hands to the OAuth flow, so
getting it right once is worth doing before there are two callers.

---

## Q4 — The orientation moment: what it is, and what it must not be

The gap the issue names is real and it is the single biggest thing a new user
will not discover: a small *Keys `?`* button is not a signal that a command
palette and a whole keyboard system exist.

**Decision: one dismissible plate, after the first successful sync, and never
again.**

- **Not a tour.** No steps, no *Next*, nothing that has to be completed. A tour
  is a thing to get through before using the app, and Postio's whole claim is
  that the app is fast.
- **Not modal.** It does not take focus, does not block the list, and does not
  intercept keys. A modal that appears when mail arrives is a modal that
  arrives while the user is reading.
- **After the first successful sync**, not at launch. Before there is mail on
  screen, *"press `j` and `k` to move between messages"* refers to nothing.
- **Three lines, one dismissal**: the palette (`Ctrl+K`), the cheat sheet
  (`?`), and `j`/`k`. Everything else is in the cheat sheet, which is the
  point of naming it.
- **Motion budget applies**: appearance is ≤ 100 ms or absent, and
  `prefers-reduced-motion` is honoured.

---

## Q5 — Its text is rendered, not written

The obvious implementation puts `"Ctrl+K"`, `"?"` and `"j / k"` in a label.
That label is wrong for any user who has rebound them in `[keys]` — and the
first thing a keyboard-first user does is rebind things.

**Decision: the orientation renders its bindings from the resolved keymap**,
the same source the `?` cheat sheet and the focused row's key hints already
use. `ARCHITECTURE.md` §2 already says three hand-maintained lists drift within
a release; this would be a fourth, and it would drift on the user's machine
rather than in the repository, where nobody would ever see it.

Practically: the plate asks the keymap for the binding of `CommandId::Palette`,
`CommandId::CheatSheet` and the next/previous-message commands, and renders
whatever comes back.

---

## Q6 — "Seen" is app state, not configuration

The schema already draws this line, in the `settings` table's own comment: *the
user's configuration is TOML and belongs to `postio-config`; this is state the
app owns.* Whether an orientation has been shown is exactly the latter — nobody
edits it in `$EDITOR`, and a `config.toml` that accumulated
`orientation_seen = true` would be Postio writing to the user's file for its
own bookkeeping.

**Decision: a row in `settings`.** Written when the plate is dismissed *and*
when it self-dismisses, so the two paths cannot disagree.

**And the second half of the criterion — "never after the user's first
keystroke" — is about *commands*, not key events.** A modifier press, a click,
or typing in the search field are not evidence that the user knows about the
keyboard system. **The trigger is the first successfully dispatched
`ActionId`.** That is a precise, testable definition, it lives at the layer
that already exists, and it means a user who presses `j` before the plate
appears never sees it — which is the correct outcome, because they have already
demonstrated the thing it was going to teach them.

**A second account never triggers it.** The flag is global, not per account.

---

## Q7 — The boundary holds

The issue's third criterion — no new `io-imap` or `rusqlite` in `postio-gtk` —
falls out of the arrangement rather than needing care. The dialogue host is a
GTK presenter; the probe, the account write, the keyring entry and the engine
start all happen in `postio-app`, which is where `compose.rs`, `feed.rs` and
the existing `onboarding.rs` already join the two halves.
`scripts/check-crate-boundaries.py` proves it on every push, over the resolved
dependency graph rather than over source text.

---

## Alternatives

**A multi-step wizard.** `postio-hiy` already considered and dropped a
store-path step to keep this one screen, and that decision is right: every step
is a place to abandon setting up a mail client.

**A second, simpler add-account form in Settings.** Two forms that must agree
about autoconfig, app-specific passwords, manual entry and error states,
forever.

**A coach-mark tour over the real UI.** Blocks the inbox, is the thing users
dismiss without reading, and needs a per-widget anchoring system for a moment
that should happen once.

**Static text in the orientation.** Cheapest, and lies to the users most likely
to care (Q5).

**Dismiss on any GDK key press.** Simpler than watching dispatch, and a
`Shift` press or typing an address into the search box would count as
"the user knows about the keyboard system" (Q6).

---

## Consequences

- Depends on ADR 0005 for `attach_account` to have anywhere to attach to.
- #57 becomes part of this work rather than a separate fix, and its token is
  the one ADR 0006 reuses.
- `postio-core` gains `account.add` and the orientation's dismissal state has
  a home in `settings`; neither is new machinery.
- The orientation's text has no string constants for bindings, which is the
  one thing to check in review.
