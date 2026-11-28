# Contract: the Source surface

The spec's "The Source surface" is the behaviour, and FR-001 to FR-015 are
the requirements. This file pins what the tests assert.

## Commands (`postio-core::registry`)

| Id | Title | Key | Context | Recovery |
|---|---|---|---|---|
| `view_source` (existing id) | Source and verification | `V` | List, thread, reader | None |
| `source_tab_verification` | Verification | `1` | Source | None |
| `source_tab_headers` | Headers | `2` | Source | None |
| `source_tab_raw` | Raw | `3` | Source | None |
| `copy_source` | Copy all | `mod+c` with no selection | Source | None |
| `save_source` | Save as .eml… | `mod+s` | Source | None |
| `unfold_part` | Show | `Return` on a fold | Source (Raw) | None |
| `find_in_message` (existing) | Find | `mod+f` | Source (Headers, Raw) | None |

- A new `Context::Source`. `Esc` (`back`) closes the surface.
- `docs/keybindings.md` regenerates. The golden `linux-bindings.txt`
  changes in the same commit.

## Behaviour (`postio-focus`)

| Given | When | Then |
|---|---|---|
| a list row | `V` | Source opens on Verification over the list, and the cursor stays on the row |
| the open message | `V` | Source opens over the message. `Esc` returns to the message, and a second `Esc` returns to the list |
| Source on any tab | `1`, `2`, `3` | that tab. The other tabs keep their scroll |
| Source | `Esc` | closed. The row, the cursor and the scroll are as they were |
| Raw, on a fold | `Return` | that part unfolds. `Esc` closes the surface, and does not refold |
| Raw | `mod+s` | the save dialog; the file's bytes equal `Client::raw_source` |

## Reads (`postio-host`, counted)

- `Req::SourceView(message, tab)` returns a `SourceView`. With the raw bytes
  local, it costs a bounded number of statements: the message row, its header
  block, its verdict row (slice B), and the account for `authserv_ids`. It
  does not scan the mailbox.
- With raw bytes absent, `Raw` returns `state = Fetching` at once, and a
  `SourceReady` event follows the fetch. The UI never awaits it.

## Privacy

- No `SourceView` field is handed to `postio-render` or a web view. A test
  over every corpus fixture with the `hostile` or `prompt-injection`
  category opens all three tabs, and asserts zero renderer calls and zero
  connections through the counting transport.
- Text widgets are non-editable labels or text views with no link detection.

## States

As the spec's "Every state" table. Each row is one `postio-ui` unit case on
the strings, and one frontend case per path (ADR 0049).

## Storyboard

`storyboards/reader/source-surface.toml`:

1. `V` from the list;
2. `2`, then `3`, then `Esc` back to the same row;
3. `V` from the open message, then `Esc` twice.

It is red on `main` and judged by `/ux-review`.
