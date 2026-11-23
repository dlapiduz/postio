# Contract: the formatting picker

The words and their order live in `postio_ui::format_picker` (pure, unit
tested). GTK draws it on `postio_widgets::widgets::pickers::Picker`, and
the Mac draws it in SwiftUI. Neither decides anything.

## Model

```text
FormatPicker::new(at: Caret, in_force: &[CommandId]) -> FormatPicker
  .title() -> "Formatting"
  .acts_on() -> "Selection · 4 words" | "Paragraph" | "Heading 2" | "List item"
               | "Task" | "Quote" | "Code block · rust" | "Table · row 2, column 3"
  .groups(filter: &str) -> Vec<Group>
Group { title: "Text" | "Paragraph" | "Lists" | "Blocks", rows: Vec<Row> }
Row {
  command: CommandId,
  title,                      // the registry's title
  sequence: Option<&str>,     // "~~text~~", "## ", "- [ ] ", "```"
  in_force: bool,
  unavailable: Option<&str>,  // the reason, e.g. "Tables don't go inside tables"
}
.empty(filter) -> Option<String>   // "No formatting called “xyz”"
```

- **The keycap** is not in the model. Each frontend reads it from the keymap
  in force (`hints::short`), so a `[keys]` override shows.
- **Filtering** matches the title, case-folded, by word prefix: "str" and
  "through" both find Strikethrough. A row's sequence matches too, so `##`
  finds Heading 2.
- **Order** is fixed within a group, and the groups are in the order above.
  Heading 1–6 and Normal text are one row of seven cells in "Paragraph".

## Keys (in `Context::Picker`)

| Key | Does |
|---|---|
| typing | filters |
| `Up` / `Down` | moves between rows |
| `Return` (`picker_confirm`) | runs the row's command, closes |
| `0`–`6` (`picker_choose_N`), filter empty | Normal text, Heading 1–6; closes |
| `Escape` (`back`) | closes; nothing changes; the caret is where it was |
| any formatting command's own chord | runs it; closes |

## Anchoring

- From `formatting` (the key or the command bar): at the caret's line,
  below it, or above it where there is no room below. The popover never
  covers the caret's line.
- From the toolbar button: below the button.
- Width 380 px, the frame's. It fits the composer's 615 px column at
  1024 × 768.

## The table picker and the code language picker

The same frame. The table picker's rows are the `Context::TablePicker`
commands, in the order of `data-model.md`. Its title is "Table" and it
acts on "Row 2, column 3". The code language picker lists the last four
languages as `1`–`4`, with a field to type any other (`Language::parse`
rejects a bad one, with the reason under the field). Its title is "Code
language" and it acts on "Code block".

## The toolbar

The order:
- Bold, Italic, Strikethrough;
- Bulleted, Numbered, Task list;
- Quote, Inline code;
- Link, Image;
- Formatting.

Each is `dress_icon()` with the registry's title and key as its tooltip
and accessible name. Its pressed state follows the reflection channel.

Overflow at narrow widths: buttons leave from Image leftwards. Formatting
stays, and a hidden button's command is still in the picker.

Icons are the theme's symbolic set:
- `format-text-strikethrough-symbolic`;
- `checkbox-checked-symbolic` for Task list;
- `code-symbolic`, or the bundled equivalent where the theme lacks it;
- `format-text-rich-symbolic` for Formatting.

`/gtk-design`'s render-to-PNG check confirms each draws at 16 px in light
and dark.
