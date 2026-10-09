# Focus on the Mac, phase 6: the pickers and the undo pill against screens 11 to 15

2026-10-08, specs/009-focus-macos T094 (FR-061). The phase numbering
follows tasks.md: phase 8 of the spec is US6, and this is the sixth Mac
comparison.

**Captures.**
- Made with `scripts/macos-shot.sh` over the in-memory `small` demo
  store, at 1440×900, in light and dark.
- A picker is an `NSPopover`, whose window is a child of the window it
  hangs from. `screencapture -l` takes a window with its children, so the
  popover is in the picture, as the places popover was in phase 5; the
  script needed nothing new.
- Each state is reached by keys replayed once the list has landed
  (`POSTIO_DEMO_KEYS`). A replay now reaches a picker as a press would:
  ↓/↑ move its highlight, words go into whichever field holds the
  keyboard, and Return, Tab, Space, the digits and Escape resolve through
  the keymap in the picker's context.
  - `p6-11-snooze`: `'j j s'`; `p6-11-typed`: `'j j s ⇥ tue ␣ 9am'`;
    `p6-11-message`: `'j j ⏎ s'`, the picker from the open message;
  - `p6-12-remind`: `'j j h ↓'`;
  - `p6-13-label`: `'j j l'`; `p6-13-toggled`: `'x j j x l ␣ ↓ ␣'`, two
    labels on two selected rows; `p6-13-create`: `'j j l Plans'`;
  - `p6-14-move`: `'j j m'`; `p6-14-moved`: `'j j m 1'`;
  - `p6-15-pill`: `'x j j x j x a'`; `p6-15-plain`: `'j j a'`.
- Captures live in the main checkout's untracked
  `Design/review/focus-macos/`.
- References: `Design/focus-macos-design/screens/11-picker-snooze.png`,
  `12-picker-remind-if-no-reply.png`, `13-picker-label.png`,
  `14-picker-move.png` and `15-undo-toast.png`.

Every key above went through the resolver and `invoke` to
`postio-focus`'s controller (slice 9). Which picker opens, what it acts
on, its rows, their words and times, what a typed date means, and what
choosing a row does are the controller's and `postio_ui::pickers`', as on
GTK. The Mac draws them and decides only where the highlight stands.

## What matches

**The pickers (11 to 14).**
- Each hangs from the cursor's row, just under its lines, its left edge
  on the subject column. From the open message it hangs from the action
  row's button for the verb (Snooze, Remind, Label, Move), or More's
  when that verb has folded away.
- The title is bold on the left and the target on the right: the row's
  sender and subject, or "2 conversations" over a selection.
- **Snooze and remind** list the four presets, each with its time and
  its number key 1 to 4; the first is highlighted as it opens. The date
  box under them says "Or type a date: “tue 9am”" and "Tab to type".
  - Tab puts the keyboard in the box, ringed in the accent, and takes the
    highlight off the presets.
  - Typing "tue 9am" reads the date back under the words ("Tue 13 Oct,
    09:00"); Return snoozes until it.
  - Remind's title wraps onto two lines, as the pack draws it, and its
    footnote names the "No reply since" marker.
- **Label** has a filter, "Filter, or type a new label", holding the
  keyboard. Each label has its dot and its count, or "✓ applied".
  - Space toggles the highlighted label and the picker stays up; the
    highlight stays on the label just toggled.
  - The rows' pills change as each label goes on.
  - A name nobody has is offered as "Create label “Plans”", first and
    highlighted.
- **Move** has "Filter folders", then Recent (numbered) and All folders,
  as sections. `1` moves to the first recent folder and the row leaves.
- The footnotes are the controller's words.
- Light and dark both follow the semantic colours. Nothing in Swift
  names a colour.

**The pill (15).**
- Bottom centre, above the action bar when there is one. It says the
  controller's words ("Archived 3 messages", "Moved 1 message",
  "Labelled 4 messages") with Undo and the ⌘Z cap from the binding in
  force.
- It is filled with the primary label colour and lettered in the
  background's, as the pack draws it; in dark mode it turns over with
  everything else.
- A new toast replaces the one showing. It goes after the toast's
  seconds (the controller's eight) with a fade of at most 100 ms, none
  under Reduce Motion. ⌘Z still undoes once it has gone.

## Differences, and why

| Difference | Why | Where it is settled |
|---|---|---|
| The popover has an arrow and the system's popover material, where the pack draws a plain sheet | `NSPopover`, as the contract asks; its arrow cannot be hidden with public API | Contract, as phase 5 |
| Footnotes spell keys in the keymap's words: "space toggles a label · Return closes", "cmd+z undoes". The pack says "Space", "Enter", "⌘Z" | The controller composes them with `postio_ui::hints`; the Mac re-spells only the keycaps it draws itself | C22: the shared short spelling is not exported |
| Label dots are grey | The demo's labels carry no colour, and `postio_ui::label_colour` (a colour from the name) is not exported; a `None` colour is drawn in the secondary colour, as the places popover's dots | Fixture, and an FFI gap |
| Move lists only Receipts and Archive | The demo store has two folders | Fixture |
| The target wraps under the title instead of being cut ("… Atlas Q3 budget, / final numbers") | Nothing is cut short on the Mac; the pack wraps remind's the same way | Decision |
| "Create label" is drawn regular with a + before it; the pack does not draw the row | It is an action, not a label, and has no dot | Decision |
| Return in the label picker closes it; it does not toggle the highlighted label | The controller's footnote says "Return closes", and GTK's picker closed. The T089 note had Return on a highlighted label toggle it | Decision, recorded in tasks.md T091 |
| The pill is as wide as its words (at least 320); the pack's is about 400 | Fits the controller's words, which vary in length | Decision |
| "Labelled 4 messages" over two conversations, where "Archived 3 messages" counts the three chosen | The controller's toast words count differently for labels | Controller; noted for whoever next touches the toast words |

Found and fixed on the way:
- **The list did not have the keyboard at launch.** The window's first
  key view did: the strip's Inbox ▾, drawn ringed beside the pill. The
  key monitor still resolved every press, but a key it left alone would
  have reached the button. The table now takes the keyboard once, when
  the first page lands (C30).
- **The popover was sized for the rows it had before a redraw.** The
  hosting controller measures the view it last updated, so a move picker
  whose rows arrived after it opened was sized for none. It now measures
  again on the next turn.

## What this phase does not cover

- The storyboards (T090) are filmed on Linux; this Mac cannot build GTK.
- GTK has not adopted the controller's pickers or its toast policy
  (slice 9's As built), so the two apps' pickers have the same rules but
  not yet the same code on GTK's side.
- The row menu stays GTK's (slice 9's "Not here").
