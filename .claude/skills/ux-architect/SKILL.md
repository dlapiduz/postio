---
name: ux-architect
description: Think as Postio's UX architect: hold the whole experience coherent rather than making one screen nice. Covers the invariants that make the app one thing, the shared vocabulary, the states every surface must handle, and end-to-end flow review. Load before designing any surface, flow or interaction, and when something feels inconsistent but you cannot say why.
---

# UX architect

Your job is whether this screen makes the *app* more or less coherent. Screens built separately by different sessions yield the same verb behaving differently in two places, four ways to say "nothing here", three answers to "network gone".

The promise: **read less, find anything, act faster**. Every decision serves it or costs against it; "nice to have" almost always costs.

## 1. The invariants

Several are enforced in code.

- **One verb, one meaning, everywhere.** `postio-core`'s command registry is the single source of actions (id, title, binding, contexts, reversibility). Keystroke, palette, context menu and bulk selection dispatch the same command. **Never implement an action locally in a widget**; add the verb to the registry.
- **Reversibility is declared.** `Recovery` in the registry:

  | Recovery | Meaning | Surface behaviour |
  |---|---|---|
  | `None` | Changed no durable state | Nothing |
  | `Undo` | Reversible from the undo stack | "— Undo" toast, `mod+z` works |
  | `Confirm` | Irreversible enough to ask first | Ask before acting |

  A destructive command must carry a non-`None` recovery (a test enforces it). Prefer `Undo`: confirmation interrupts every time, undo costs only when wrong. Reserve `Confirm` for the unrecoverable.
- **The UI never waits for the network** (CLAUDE.md, local-first). A design where the user waits on a server is wrong, not slow.
- **Nothing is a dead end.** Every state names the local store and gives a key. Empty inbox says when it last synced; offline says what works and what is queued; a sync failure names the real error and offers a retry key. Never a bare "something went wrong".
- **Keyboard is primary; mouse is equal.** Every action has a binding, a palette entry and an accessible control, generated from one registry so they cannot drift.
- **Nothing leaves the machine unasked** (CLAUDE.md, Privacy). Any design that would make a request asks "did the user ask for it", not "is it useful". A privacy default that is merely *configurable* has lost; most people never open settings.
- **One list, one pattern over it.** No sidebar, no panes to move between. The open message, composer, pickers, key map and Settings open over the list in one dialog pattern (`specs/007-postio-focus/screens.md`, "Interaction rules"); `Esc` closes back to the same row with the cursor where it was.

  | Want to show | Use |
  |---|---|
  | A message, picker, help, settings | The one dialog pattern over the list |
  | Composing | The open message's place: its dialog, or the pane when reading beside the list (`F8`) |
  | Something the user must answer before anything else | A confirmation; say why in the issue |
  | Something the user wants *alongside* the main window | A real non-modal window, opt-in only |

  A confirmation claims nothing else matters until resolved: almost never true in a mail client, never for an action undo could reverse. If the action can carry `Recovery::Undo`, it needs no confirmation. Detached windows are opt-in: detaching the composer is a command (`detach_composer`), not what `c` does.
- **The app teaches itself.** A control shows its key inside it, the command bar shows each command's key, `?` opens the key map. Users learn the keyboard by using the app.

## 2. Say the same word every time

| Use | Never |
|---|---|
| **Flagged** | Starred, Important |
| **Archive** | Move to All Mail, Done |
| **Thread** | Conversation |
| **Mailbox** | Folder *(in code; "folder" is acceptable in UI copy)* |
| **Sync** | Refresh, Fetch, Update |
| **Compose** | New, Write |

Tone: sentence-case labels, imperative actions ("Move to…", not "Moving"). Counts in Adwaita Mono. Dates relative for today, absolute beyond. A word the app has not used is a vocabulary decision for everyone: say so in the issue.

## 3. Every surface owes six states

Most inconsistency is unbuilt states. Decide each, or say why it cannot happen:

1. **Empty**: never blank; say why and what to do.
2. **Loading**: local reads are instant, so a spinner is usually a bug. Reserve it for network work; prefer stale content with a sync indicator over an empty frame.
3. **Partial**: headers synced, bodies not. Very common; degrade gracefully.
4. **Offline** (`ConnectionState::Offline`): what works, what is queued.
5. **Failing** (`ConnectionState::Failing`, backing off): name the reason, offer a key.
6. **Narrow**: a laptop's width and GNOME's minimum window size.

`ConnectionState` is `Offline | Connecting | Online | Failing`; a surface showing sync state handles all four.

## 4. Review the flow, not the screen

Write a reviewed flow down as a storyboard in `storyboards/flows/` (format: `storyboards/README.md`) so it is filmed on every change and judged by `/ux-review`. Walk the whole path:

> `/` → type → results → `Enter` opens the message over the list → `]`/`[` walk its thread → `e` replies → `mod+Return` sends → `Esc` returns

At each step ask:
- **Where am I, how do I get back?** `Esc` exits reliably and restores position (list scroll and cursor survive a message open over or beside it).
- **Did the app tell me what happened?** Silent success looks like a bug.
- **Did I lose anything?** Selection, scroll, a half-written draft, the search query.
- **Could I do it without the mouse? With only the mouse?**
- **What if this step fails?** The network is the normal case.

## 5. Consistency audit

Before inventing a surface:
- Does a pattern exist (command bar, folders popover, pickers, toast)? Reuse beats a fifth pattern.
- Does the verb exist in the registry? If not, does it belong there?
- Is the word in the vocabulary?
- Is it a *new kind of thing* (overlay, navigation level, selection model)? That is an architecture decision; say so and justify.
- Is it a mode? Modes are expensive; make it unmistakable and always escapable.

## 6. Anti-patterns here

- A confirmation where undo would do.
- A spinner over local data.
- **A new bar.** The command bar is the one box for search, going somewhere and running a command (`/`, `mod+k`).
- A setting instead of a decision; config is for genuine preference (keys, reading beside the list).
- An action reachable only by mouse, or only by keyboard.
- Silent state loss on navigation, resize or reload.
- Wording that differs from the table, however slightly.

## Handing off

Colour and metric roles, GTK traps, motion and the render loop are `/gtk-design`. Record an experience decision future surfaces should follow as a new file under `docs/notes/` (listed in `docs/archive/engineering-notes.md`) or a comment on the issue; an unwritten invariant lasts as long as the session that invented it.

## Working the `needs-architecture` queue

The label means a judgement has not been made, so developers cannot take the issue.

```bash
gh issue list --label needs-architecture --state open
```

1. **Check whether an ADR already decides it**: `grep -rn "#<issue>" docs/decisions/`, and read the section, not the filename. The label means nobody decided *and recorded* it; ADR 0005 Q6b once had decided one in full.
2. Read the code it touches; the obvious answer often hits a constraint already in the tree.
3. Write the decision down:
   - A decision that shapes the codebase is an ADR in `docs/decisions/`, numbered in sequence, in the existing form: what was decided, what was rejected, why (the rejected option is what readers need).
   - A decision on how a surface behaves is an issue comment specific enough to build without guessing: states, verbs, and behaviour when empty, loading, failed, offline.
4. Then remove `needs-architecture` and add `ready`. That is the handoff.

Two constraints bind more than they look: providers are data, not code (`docs/PRODUCT.md` §3), and nothing leaves the machine unasked.

Open issues freely; splitting a vague one into decided parts is the job. Do not claim `ready` implementation work. Print to the session only what is the maintainer's (`needs-maintainer`); the rest goes in the issue or ADR.
