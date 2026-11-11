# Contract: what both Focus frontends receive and send

The controller (`postio-focus`) decides and `postio-ui` composes the words.
AppKit (through `postio-ffi`) and GTK (directly) draw the result and route
keys. Neither decides scope, which chip shows, what a footer says, or where
the cursor goes (spec FR-025, 009 FR-004).

## Inputs (frontend → controller)

| Input | When |
|---|---|
| `Key(CommandId)` | Every new command in research R10, routed by the registry's contexts as every key is |
| `ChevronClicked(position)` | A click on a thread row's chevron |
| `ThreadBarChipClicked(index)` / `MoreChipClicked` | A click in the bar; More opens the outline |
| `ThreadMenu(entry)` | A menu entry chosen by click |
| `OutlineTyped(text)` / `OutlinePerson(name)` | The outline's filter field and person chips |
| `ForwardFormClicked(form)` | A chooser card clicked |
| `BarRoom(chips: u32)` | The window's width allows this many chips (sent on resize) |

## Intents (controller → frontend)

| Intent | Frontend does |
|---|---|
| `Splice { anchor, children: Vec<ChildRowView> }` | Insert child rows after the anchor (Mac `insertRows`, GTK model splice); no animation under reduced motion |
| `Unsplice { anchor }` | Remove them |
| `Footer(FooterView)` | Draw the footer: scope title, then keycap + label pairs, then the right-hand sentence |
| `ThreadRow(position, ThreadRowView)` | Redraw one row in place (participants, count, new, muted, time) |
| `ThreadBar(ThreadBarView)` | Draw the label column, chips and Thread ▾ |
| `Kicker(String)`, `OlderBanner(Option<BannerView>)`, `NewReply(Option<BannerView>)`, `MatchBanner(Option<BannerView>)`, `ReplyTo(Option<String>)` | Draw them above, or in, the sender block |
| `ThreadMenu(MenuView)` / `Outline(OutlineView)` | Open the 340-point popover or the 440-point popover from Thread ▾ |
| `ForwardChooser(ChooserView)` / `OlderReply(BannerView)` | In the composer |

## Views (all composed in Rust)

- `ChildRowView`: `initials`, `sender`, `bold` (unread), `tag` ("↳ to Ana" /
  "message 5 · file"), `line` (own line or passage with highlight ranges),
  `attachment`, `time`, `draft`, `focused_hint` (keycaps + "this one").
- `ThreadRowView`: `participants`, `count`, `subject`, `labels`,
  `latest_line` or `match_summary` ("3 of 6 messages match"), `new`,
  `attachment`, `muted`, `time`, `chevron` (`None`, `Collapsed`, `Expanded`).
- `GapRowView` (search): "Message 4 doesn't match · Ben, Thu".
- `ThreadBarView`: `title` ("Thread"), `count_line` ("6 · 2 new"),
  `chips: Vec<Chip>` (data-model), `menu_label`.
- `FooterView`: `title`, `keys: Vec<(keycap, label)>`, `note`.

Keycaps everywhere come from the live keymap (`postio_ui::hints`), so a
rebinding shows up in the footer, kicker, banners and menu.

## Platform differences (chrome only)

| | Mac | Linux |
|---|---|---|
| Message window | Its own `NSWindow` (009) | The Focus dialog (007) |
| Popovers | `NSPopover` | `GtkPopover` |
| Chevron rotation | Core Animation, off under Reduce Motion | CSS transition, off under `gtk-enable-animations=false` |
| `mod` | ⌘ | Ctrl |
| Sizes | Points from `Design/focus-macos-threads/SPEC.md` | The same numbers through spec 007's metric roles (`postio_ui::tokens`) |

Anything else that differs is a defect, to be fixed or listed against the
screen it shows on (FR-027).
