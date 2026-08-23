---
name: ux-architect
description: Think as Postio's UX architect — hold the whole experience coherent rather than making one screen nice. Covers the invariants that make the app feel like one thing, the shared vocabulary of verbs and words, the states every surface must handle, and how to review a flow end to end. Load before designing any surface, flow, or interaction, and when something feels inconsistent but you cannot say why.
---

# UX architect

Your job is not this screen. It is whether this screen makes the *app* more
coherent or less. A collection of individually good screens built by different
sessions reliably produces an incoherent product — the same verb behaving
differently in two places, four ways of saying "nothing here", three different
answers to "what happens when the network is gone".

Postio's promise is narrow and demanding: **read less, find anything, act
faster**, for people with too much email. Every decision either serves that or
costs against it. "Nice to have" almost always costs.

---

## 1. The invariants

These are not preferences. They are what makes the product feel like one thing,
and several are already enforced in code.

**One verb, one meaning, everywhere.** `postio-core`'s command registry is the
single source of actions — id, title, binding, contexts, reversibility. Archive
means the same thing from a keystroke, the palette, the context menu and a
bulk selection, because all four dispatch the same command. **Never implement
an action locally in a widget.** If a surface needs a verb the registry does
not have, add it to the registry.

**Reversibility is declared, not improvised.** `Recovery` in the registry is
already a three-way policy:

| Recovery | Meaning | Surface behaviour |
|---|---|---|
| `None` | Changed no durable state | Nothing |
| `Undo` | Reversible from the undo stack | "— Undo" toast, `u` works |
| `Confirm` | Irreversible enough to ask first | Ask before acting |

A destructive command must carry a non-`None` recovery; a test enforces it.
Prefer `Undo` over `Confirm` — a confirmation dialog interrupts every time,
undo only costs when you were wrong. Reserve `Confirm` for genuinely
unrecoverable things.

**The UI never waits for the network.** Every mutation is local-first: SQLite
write, enqueue the operation, emit the event, repaint. So an action feels
instant and *is* instant, offline or not. Any design where the user waits on a
server is wrong here, not slow.

**Nothing is a dead end.** Canvas 3d states the rule: every state "names the
local store and gives a key, not a shrug". An empty inbox says when it last
synced. Offline says what still works and what is queued. A sync failure names
the actual error and offers a retry key. Never a bare "something went wrong",
never a state with no way forward.

**Keyboard is the primary path; mouse is equal, not lesser.** Every action
needs a binding, a palette entry, and an accessible control. Generated from
one registry so the three cannot drift.

**The app teaches itself.** Key hints on the focused row, bindings shown in the
palette, `?` for the full sheet. A user should learn the keyboard by using the
app, never by reading docs.

---

## 2. Say the same word every time

Inconsistent vocabulary is the cheapest way to make an app feel amateur, and
the easiest to prevent. Postio's words:

| Use | Never |
|---|---|
| **Flagged** | Starred, Important |
| **Archive** | Move to All Mail, Done |
| **Thread** | Conversation |
| **Mailbox** | Folder *(in code; "folder" is acceptable in UI copy)* |
| **Sync** | Refresh, Fetch, Update |
| **Compose** | New, Write |

Same for tone: labels are lower-case sentence case, imperative for actions
("Move to…", not "Moving"). Counts are IBM Plex Mono. Dates are relative for
today, absolute beyond.

If you introduce a word the app has not used, you are making a vocabulary
decision for everyone — say so in the bead rather than deciding silently.

---

## 3. Every surface owes six states

Most inconsistency is unbuilt states, not bad layout. Before a surface is
done, decide what it does in each — and if the answer is "cannot happen", say
why:

1. **Empty** — never blank. Say why it is empty and what to do.
2. **Loading** — but remember local reads are instant, so a spinner here is
   usually a bug. Reserve it for genuine network work, and prefer showing
   stale content with a sync indicator over an empty frame.
3. **Partial** — headers synced, bodies not yet. Extremely common in Postio.
   The design must degrade gracefully rather than look broken.
4. **Offline** — `ConnectionState::Offline`. What still works? What is queued?
5. **Failing** — `ConnectionState::Failing`, backing off. Name the reason,
   offer a key.
6. **Dense** — three row densities and a narrow breakpoint. A design that only
   works airy is unfinished.

`ConnectionState` is `Offline | Connecting | Online | Failing`. If your surface
shows sync state at all, it must handle all four.

---

## 4. Review the flow, not the screen

Screens are reviewed in isolation and then feel wrong in sequence. Walk the
whole path:

> `/` → type → results → `Enter` opens → `t` drills into the thread →
> `e` replies → `Ctrl+Enter` sends → `Esc` returns

At each step ask:

- **Where am I, and how do I get back?** `Esc` should be a reliable exit
  everywhere, and returning should restore position — canvas 3a is explicit
  that thread drill-in keeps your place.
- **Did the app tell me what happened?** Every action produces visible
  feedback. Silent success is indistinguishable from a bug.
- **Did I lose anything?** Selection, scroll position, a half-written draft,
  search query. Losing state on navigation is the most common way an app feels
  cheap.
- **Could I have done that without the mouse?** And with only the mouse?
- **What if this step fails?** Not an edge case — the network is the normal
  case for a mail client.

---

## 5. Consistency audit

When adding a surface, check it against what exists before inventing:

- Does an existing pattern already solve this? The palette, the query bar, the
  sidebar list, the toast — reuse beats inventing a fifth pattern.
- Does this verb exist in the registry? If yes, use it. If no, does it belong
  there rather than here?
- Is this word already in the vocabulary above?
- Does this introduce a *new kind of thing* — a new overlay, a new
  navigation level, a new selection model? That is an architecture decision,
  not a screen decision. Say so explicitly and justify it.
- Am I adding a mode? Modes are expensive; the user has to know which one they
  are in. If unavoidable, make it unmistakable and always escapable.

---

## 6. Anti-patterns for this app specifically

- **A confirmation dialog where undo would do.** Interrupts every time to
  protect against the rare case.
- **A spinner over local data.** Reads are instant; a spinner says "slow" about
  something that is not.
- **A new overlay.** The palette and the query bar are converging into one
  surface (`postio-cfd.1`). Adding a third is a regression.
- **A setting instead of a decision.** Every option is a question asked of
  every user forever. Config exists for genuine preference (density, keys), not
  to avoid choosing.
- **An action reachable only by mouse**, or only by keyboard.
- **Silent state loss** on navigation, resize, or reload.
- **Wording that differs from the table above**, however slightly.

---

## Handing off

Implementation mechanics — tokens, GTK traps, motion budget, the
render-to-PNG loop — are in `/gtk-design`. This skill decides *what the
experience should be*; that one gets it built correctly.

When you make an experience decision that future surfaces should follow,
record it: `bd remember`, or a note on the bead. An invariant nobody wrote down
lasts exactly as long as the session that invented it.
