# The desktop app and the terminal, side by side

Postio has two apps on Linux: the desktop app (GTK) and the terminal app,
`postio-tui`. Both are Postio Focus: the same dense inbox, the same keys,
the same verbs, and the same four things done to mail (actions called out,
digests on a cadence, spam and updates filtered, Obsidian capture). They use
the same mailbox, one at a time, and Focus's filtering, digests and reminders
act while either of them is open. This table says where they differ.

- ✓ has it
- ◐ has it, differently or in part (the note says how)
- — not applicable

Every command the terminal is offered has a key and does something there.
`registry_parity` in `crates/postio-tui/tests` fails for one that does not.
What is left below is how the two apps do the same thing, and what only
pixels can do.

## The inbox

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| The inbox as it arrived, across every account | ✓ | ✓ | |
| Invitation, question and to-do markers, answered from the row | ✓ | ✓ | `y`, `Y`, `e`, `s`, `t`, `-` |
| Digest rows and fired reminders | ✓ | ✓ | |
| The has-action filter | ✓ | ✓ | `!` |
| Cursor and selection kept apart; the bulk bar | ✓ | ✓ | `x`, `J`/`K`, `X` |
| Undo after the toast has gone | ✓ | ✓ | `Ctrl+Z` |
| Folders, labels and the mailboxes | ✓ | ✓ | `g o`, the `g` keys, and `in:` in the command bar |
| A row menu | ✓ | ◐ | In the terminal, the command bar acts on the focused row or the selection |
| The mouse: click, select, scroll | ✓ | ✓ | With `[tui] mouse = false`, every key still works |

## Reading

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| A message opens over the list, or beside it | ✓ | ✓ | `F8`; the terminal places it beside the list from 128 columns |
| One message at a time; `j`/`k` through the list, `[`/`]` through the thread | ✓ | ✓ | |
| The action card, and the sentence it quotes highlighted in the body | ✓ | ✓ | |
| HTML mail, sanitised | ✓ | ◐ | The terminal draws it as styled text in its own colours |
| App colours, or the original on paper (`O`) | ✓ | — | A terminal has one treatment, so it does not offer the switch |
| Zoom; darken a designed message | ✓ | — | The terminal draws text in its own font and colours |
| Images in a message | ✓ | ◐ | Labelled placeholders in the terminal |
| Remote images allowed per sender | ✓ | ✓ | The same allow list |
| Links and attachments | ✓ | ✓ | `o`; a click on a link shows where it goes, a second opens it |
| Find in the message | ✓ | ✓ | `Ctrl+F` |
| Raw source | ✓ | ✓ | `v` |
| Unsubscribe | ✓ | ✓ | `U` |
| Drag a message or a part out to another app | ✓ | — | A terminal has nothing to drag to |

## Writing

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| New, reply, reply all, forward; drafts | ✓ | ✓ | In the frame over the list |
| The editor | rich text | Markdown | Markdown is sent as HTML with the Markdown as the plain-text part |
| Preview what will be sent; edit in `$EDITOR` | — | ✓ | `Alt+P`, `Alt+E` |
| Recipients from contacts; Cc and Bcc; identities | ✓ | ✓ | |
| Attach a file; paste an image; drop a file | ✓ | ✓ | In the terminal a drop arrives as its path |
| Send later; remind if no reply | ✓ | ✓ | `Ctrl+H` opens the same remind picker as `h` on a row |
| Detach the composer | a window | the whole screen | `Alt+O` |

## Finding and going

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| The command bar: search, commands, go to | ✓ | ✓ | `/`, `Ctrl+K` |
| Plain words turned into operator chips | ✓ | ✓ | |
| Saved searches | ✓ | ✓ | `Alt+1`–`Alt+4`; `Ctrl+S` saves the query |
| The pickers: snooze, remind, label, move | ✓ | ✓ | `s`, `h`, `l`, `m` |
| The key map | ✓ | ✓ | `?` |

## Filtered and digests

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| Filtered, with its reasons and tabs; restore; sweep | ✓ | ✓ | `g f`, `R`, `F` |
| The digest window: summary, messages, the email from a reference | ✓ | ✓ | `]`/`[`, `Tab`, `A`, `D` |
| Digest rules and the rule dialog | ✓ | ✓ | `g d`, `d` |
| Capture a task or a note into Obsidian | ✓ | ✓ | `t`, `n`; with `[focus.vault]` set |

## Settings

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| Accounts: add, sign in, enable, remove, make default, map a folder's role | ✓ | ✓ | |
| Every section of `config.toml` | panes | `$EDITOR` | The terminal opens the file at the section |
| Signatures | ✓ | ✓ | In the terminal the text is written in `$EDITOR` |
| The privacy section | ✓ | ✓ | The same words from the same place |

## Everything else

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| A main menu | ✓ | — | Each item is a command with its key and a command-bar row |
| New-mail notifications | ✓ | ✓ | The terminal uses its bottom line and `notify-send` |
| Colours | the system's, light and dark | the terminal's own | `NO_COLOR` is honoured; roles are set in `[tui.colors]` |
| One mailbox, whichever app is open | ✓ | ✓ | One app at a time |
| Runs over SSH, with no display | — | ✓ | |
