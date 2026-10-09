# Focus on the Mac, phase 8: the key map against screen 20, and the menu bar

2026-10-08, specs/009-focus-macos T107 (FR-061). The phase numbering
follows tasks.md: phase 10 of the spec is US8, and this is the eighth Mac
comparison.

**Captures.**
- `20-key-map`: `scripts/macos-shot.sh` over the `small` demo at 1440×900,
  light and dark, with `POSTIO_DEMO_KEYS='?'`: the key goes through the
  resolver and `invoke` to `postio-focus`'s controller, which opens the key
  map (`FocusOpenKeyMap`).
- `p7-refusal`: `--seed no-such-seed`, the store refusal page (T100), which
  a demo that does not exist reaches as `StoreUnavailable`.
- Captures live in the main checkout's untracked
  `Design/review/focus-macos/`. Reference:
  `Design/focus-macos-design/screens/20-key-map.png`.

The groups, their order, the rows, their titles and every key are
`postio_ui::keymap_sheet::key_map_on(.., Focus, Apple)`'s, the key map GTK
draws less what the Mac does not offer, in the four columns
`pack_columns` packs. The Mac lays them out and spells each key as the
menus do.

## What matches

- A panel centred over the dimmed list: "Keys" large and bold, the
  subtitle beside it, and on the right `?` or Esc's cap and "close".
- Four columns of groups, each a bold heading and rows of the command's
  words on the left, its keys in one cap on the right, a hairline under
  each: Move and select, Open, Act (row or selection), Invites, Go and
  find, In search, Digests and filtering, and the rest.
- Under a hairline, the rebind footer in monospace on the left and the
  mouse line on the right.
- `?` or Escape closes it through the controller, and a click on the dim
  closes it too (reported once as `focus_surface_closed(KeyMap)`).

## Differences, each fixed or explained

1. **Fixed: keypad keys leaked their names.** `zoom_in`'s alternates drew
   `⌘KP_ADD`, `⌘KP_SUBTRACT` and `⌘KP_0`. `MenuPlan.accelerator` now draws
   a keypad key as the character it types.
2. **Fixed: the panel overflowed the window.** The real key map holds every
   command Focus offers here -- many more rows than the pack's
   picture -- so it grew past the window and lost its heading. The columns
   now scroll inside the panel; the heading and the footers stay put.
3. **The footer names `config.toml` under `[keys]`; the pack names
   `keys.toml`.** C3, and the FFI words it.
4. **More rows, and some in other groups.** The shared sheet lists the
   classic reader's verbs the Mac still offers (Reader view, Scroll reading
   pane, Show remote images), the pickers' keys, Settings and Quit; the
   pack is a curated subset. The grouping is `postio_ui`'s, so both apps
   teach the same sheet; trimming it is a question for that table.
5. **A chord beside the letter** ("e ⌘R" for Reply, "c ⌘N" for Compose).
   Every binding in force is shown; the pack shows the letter alone.
6. **Escape is `⎋`, Shift a glyph** (`⇧J`), as C22 spells keys; the pack
   writes "Esc".
7. **The subtitle is the controller's** ("Single keys act on the focused
   row, or on the selection if there is one."), without the pack's
   sentence about ⌘ shortcuts; the Mac's subtitle is `postio_ui`'s
   `subtitle(Apple)`, which drops GTK's "Ctrl becomes ⌘".
8. **The toolbar is not dimmed.** The panel is a SwiftUI overlay in the
   window's content, and the toolbar is the window's own; an AppKit sheet
   would dim nothing and drop from the toolbar rather than centre. The
   list, the strip and the action bar dim, as the pack's do.
9. **The toolbar's sync label is blank** in these captures: the demo has
   said nothing about sync (phase 7, difference 7).

## The menu bar (T104, T105)

Not on screen 20, and not photographed (`screencapture -l` takes a
window, not the menu bar); `FocusMenuBarTests` and `MenuBarMountedTests`
assert it. `MenuPlan.bar` plans the whole bar: the registry's menus for
what Focus offers on Apple, every item showing the key `bindingsFor`
answers, and the standard items -- About, Settings ⌘,, Hide, Quit ⌘Q
(once; AppKit's and the registry's were both drawn), File › New Message
⌘N and Close ⌘W, Edit › Undo (the registry's `undo`, through
`UndoRouter`) then Redo, Cut, Copy, Paste and Select All, and Window ›
Minimize and Zoom. `MenuBarPlan` plans it again on `KeymapChanged`.

**Found on the way: ⌘W quit the app.** `quit`'s alternate `mod+w` is
GTK's (its one window closing is quitting). On the Mac it expanded to
⌘W, and the key monitor resolves a key before any menu item sees it, so
⌘W in the message window -- which the contract says closes it -- ended
Postio. `registry::alternate_offered_on` now keeps that alternate off the
Mac, and ⌘W is Window › Close's again.

## The refusal page (T100)

`p7-refusal` shows "Postio can't open your mail", the store layer's
sentence and "Try again", filled with the label colour. Photographing a
build without demos showed it first drawn under the header strip, which
floated mid-window; it now replaces the whole inbox. With no session there
is no toolbar either: the traffic lights are the window's only chrome.
