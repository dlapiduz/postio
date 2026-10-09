# Focus on the Mac, phase 2: triage against screen 03

2026-10-08, specs/009-focus-macos T053 (FR-061).

**Captures.**
- Made with `scripts/macos-shot.sh` over the in-memory `small` demo store,
  at 1440×900, in light and dark.
- A state is reached by pressing keys once the list has landed, in a demo
  only (`POSTIO_DEMO_KEYS`):
  - `03`: `POSTIO_DEMO_KEYS='!'`, the Has action filter on.
  - `03-selected`: `POSTIO_DEMO_KEYS='x J J'`, rows marked, the action
    bar up (compared with screen 01's bar, since 03 has none).
  - `03-archived`: `POSTIO_DEMO_KEYS='x J J a'`, the selection archived and
    the undo line up.
- Captures live in the main checkout's untracked
  `Design/review/focus-macos/`.
- Reference: `Design/focus-macos-design/screens/03-inbox-has-action-filter.png`
  (and `01-inbox-light.png` for the action bar).

Every key above went through the resolver and `invoke` to `postio-focus`'s
controller, the one GTK's window drives. The ring, the boxes, the `!`
heading, the bar's count and the toast are its intents, applied by
`FocusIntents`. The Mac moves none of them itself, so a difference below is
a drawing difference or a fixture difference, not a second set of rules.

## What matches

**Has action (`!`).**
- The toggle is on: accent text, tinted ground and an accent outline, with
  its count ("Has action · 8 `!`").
- The list shows only the rows with an invite, question or to-do. Plain mail
  and the digest row are hidden.
- One heading, "Has action · 8", stands over the first row in place of the
  day headings.
- The cursor stays on the message it was on (here the first row, where the
  demo opened). The selection is cleared.

**Selection** (`03-selected`, against screen 01).
- A checked box in the gutter of each marked row, on a faint neutral ground.
  The cursor's accent ring is on its own row only.
- `J` walks over the digest row rather than taking it in, as the controller
  rules (no bulk verb reaches a digest).
- The action bar at the bottom:
  - the controller's count ("2 selected");
  - Archive `a` on the raised fill, then Snooze `s`, Mark read `r`,
    Digest these… `d`, Label `l`, Move `m`;
  - on the right, `x` toggle, extend, clear.

**Archive and undo** (`03-archived`).
- `a` archived the two marked rows, and the cursor went to the survivor
  below them.
- "Archived 2 messages · Undo `⌘Z`" sits at the bottom centre. It is said
  once: the old notice no longer repeats it.

**Dark.** Semantic colours throughout, including the marked rows' ground.

## Differences, and why

| Difference | Why | Where it is settled |
|---|---|---|
| Counts and names differ ("59 · 21 unread", "Has action · 8") | `postio-demo`'s seed, not the mockup's data | Fixture |
| The ring is on the invitation; the PNG has it on the second row | The PNG continues from screen 01's state; `!` keeps the cursor on the same message (contract invariant 9), and the demo opened on row 0 | Not a defect |
| No "Showing 7 of 312 · ! again to show all" beside the toggle | `postio_ui::focus_row` composes it, but `FocusStripFfi` does not carry it | A field on `FocusStripFfi`; not in tasks.md yet, and not filed |
| To-do rows offer "Snooze `s`" without "Task `t`", and the bar has no Task | Task appears only with a vault (C9). Nothing across the boundary says whether there is one yet, so the Mac leaves it out | Decision C9 |
| The bar's hints read `J` `K` and `⎋` where screen 01 has `⇧J ⇧K` and `Esc` | Caps are the keymap's spelling (C22) through `KeyCapSpelling`; the shared `hints::short` is not exported | FFI gap, recorded in `FocusList.swift` |
| The undo line is a plain capsule, not the pill of screen 15 | T052 asks for a minimal line | T093 |
| Edit › Undo is not in the pictures | `screencapture` cannot photograph an open menu. `UndoManagerTests` asserts the title ("Undo Archived 3 messages") and the routing | Tests |

Found and fixed on the way:
- **A marked row was a dark grey band** that the marker's accent words
  could not be read on. `withAlphaComponent(0.5)` replaces a label colour's
  alpha rather than scaling it, so it drew the ink at 50% (`f51db04b`).
- **The toggle was not outlined when on** (`7eb53108`).
- **`windowWillReturnUndoManager` cannot serve SwiftUI's window.** SwiftUI
  makes the window's `NSUndoManager` while the scene is built, and an
  `NSWindow` that has one never asks its delegate again. Edit › Undo is
  routed by `UndoRouter` instead (`36d3d25f`).

## What this phase does not cover

These come with later phases, and each has its own comparison:
- the undo pill (15), the pickers (11–14) and the email window (04);
- the 1024-wide check.
