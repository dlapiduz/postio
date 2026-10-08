# Contract: Focus's surfaces on the Mac

Behaviour is spec 007's contract
([focus-surface.md](../../007-postio-focus/contracts/focus-surface.md)) for
the same screen. This file adds only the Mac frame for each surface: what
holds it, where it anchors, how it closes, and where the keyboard returns.
The visual reference is `Design/focus-macos-design/` (screens, `SPEC.md`,
`message-window/`).

## Colour and type

- **Colours** are the semantic `NSColor`s only:
  `windowBackgroundColor`, `textBackgroundColor`,
  `controlBackgroundColor`, `labelColor`, `secondaryLabelColor`,
  `tertiaryLabelColor`, `quaternaryLabelColor` (keycap and chip fill),
  `separatorColor`, `gridColor` (strong hairline), `controlAccentColor`,
  and `systemRed` at low opacity for the error strip.
- **No hex in Swift.** A test greps `Sources/` for `#[0-9a-f]{6}` and
  `Color(red:`.
- **The accent** is used only for markers, the focus ring, the Has action
  toggle when on, the action card (8% light, 12% dark, C26), links, and the
  citation banner.
- **Default buttons** (Send, Create, Add task, Archive all) are filled with
  `labelColor`, with inverted text.
- **Type:** the system font for the chrome; `.monospaced` for keys,
  addresses, dates and counts; Barlow for a body in app colours (C25).
- **Keycap:** one `KeyCap` view. Monospaced 10 pt, 15 pt tall, 1 px
  `separatorColor` outline, radius 3, `quaternaryLabelColor` fill. Text
  from the FFI's spelled key (C22).

## Main window

- **Toolbar** (`NSToolbar`, unified):
  - compose button (`c`);
  - flexible space;
  - sync label (from `SyncLabel`);
  - `NSSearchToolbarItem`, with "⌘K" shown as a keycap in the field.
- **Header strip** (SwiftUI): Inbox ▾ (`g o`) with its count; the Has action
  toggle (`!`) with its count; on the right, "N filtered today" (`g f`) and
  "N digest rules" (`g d`), each shown only while in use (C10).
- **List:** `FocusListTable` (`NSTableView`, view-based).
  - Row types: day header, one-line, two-line (marker).
  - Heights are fixed per type, so there is no measuring.
  - The focus ring is drawn by the row view in the accent, around the cursor
    row only.
  - The selected gutter shows a checked box.
  - It never uses `NSTableView` selection for Focus's selection. Table
    selection is disabled, and the cursor and selection come from intents.
- **Action bar** (SwiftUI, bottom), while there is a selection: the count,
  the actions with keycaps, and hints (`x` toggle, `⇧J ⇧K` extend, `Esc`
  clear).
- **Undo pill** (SwiftUI overlay, bottom centre): the text, an Undo button,
  and a ⌘Z keycap. It fades after the toast's seconds.
- **Banner strip:** full width under the header strip, for first sync,
  offline and sign-in error.

## Secondary windows: `SecondaryWindowController`

Email, digest, compose and capture.

- **One at a time** (M4). Opening another replaces the window's content, or
  closes it and opens the new one, keeping the frame when the kinds match.
- **Placement.**
  - Each window is a titled `NSWindow` with traffic lights and no
    minimise-to-dock behaviour. It is a child of the main window, so it
    moves with it.
  - It is centred over the main window and sized from M1. The list behind is
    not dimmed.
  - The person may resize it. The column is then recomputed from the
    window's own width, and `j`/`k` keep the size.
- **Esc and ⌘W** close it. The controller emits `KeyboardHome(CursorRow)`,
  and the main window makes the table first responder with the cursor row
  shown and the selection kept.
- **Closing the main window** closes the secondary one. A composer saves its
  draft first.
- **Email window** (`message-window/SPEC.md`):
  - A 52 pt title area: the subject and position line in the centre; `k j`
    keycap and the joined ↑/↓ pair on the right. From a digest,
    "‹ Summary" with Esc on the left.
  - A 44 pt action row: Reply `e`, Reply all `E`, Forward `f`, Archive `a`,
    Snooze `s`, Remind `h`, Label `l`, Move `m`, Delete `⌫`. Label, Move
    and Delete fold into More `.` below 700. From a digest: Reply, Forward,
    Archive, Note `n`, Label, Unsubscribe `U`, Stop digesting `D` (M3).
  - The content is one centred column. The header block is SwiftUI; the
    body is a `WKWebView`.
- **Body web view:**
  - `allowsContentJavaScript = false`; `websiteDataStore = .nonPersistent()`;
    loaded with `loadHTMLString(_, baseURL: nil)`.
  - A `WKContentRuleList` blocks every load not on the `postio-cid:` or
    `postio-reader:` scheme.
  - Paper view: light appearance forced, white `underPageBackgroundColor`,
    radius 6 and a hairline edge, `brightness(0.92)` in dark appearance, and
    zoom to fit no lower than 0.85.
  - `evaluateJavaScript` is called only on the main actor.

## Command bar: `CommandBarPanel`

- A borderless, non-activating child `NSPanel` whose top edge meets the
  search item's bottom, as wide as the field or 640, whichever is wider.
- Keyboard focus stays in the toolbar's field.
- `/` opens search. ⌘K opens commands with `>` typed (C24). ⌘F opens search
  from the main window.
- Results are one line each, with their keycaps.
- Esc or a click outside closes it, and the keyboard goes home.

## Popovers

- **Pickers** (`s`, `h`, `l`, `m`) are `NSPopover`s with `.transient`
  behaviour, shown relative to the cursor row's rect in `FocusListTable`, or
  to the action button in the email window.
- **Folders** (`g o` or Inbox ▾) anchor to the Inbox ▾ button.
- The content is SwiftUI from the picker models.
- Esc closes, and the keyboard goes home.

## Sheets

- Key map (`?`), digest this sender (`d`), and update password are sheets
  on the main window, which dims as a sheet does.
- `?` toggles the key map; Esc closes any of them.

## Menu bar

- It is generated from the registry's menus and commands, offered on Apple
  for `Frontend::Focus`. Each item shows the spelled key; key equivalents
  are display-only, because dispatch goes through `KeyMonitor`.
- Edit › Undo is backed by `PostioUndoManager` (R7). Cut, copy, paste and
  select all stay AppKit's for text fields.
- The standard items are App › Settings… ⌘, and Quit ⌘Q; File › New Message
  ⌘N and Close ⌘W; and Window.

## `postio://` and `mailto:`

- **`postio://message/<id>`** brings the app forward and opens that message
  in the email window. An unknown or gone message shows the FFI's sentence
  in the pill.
- **`mailto:`** opens the composer, as today.
