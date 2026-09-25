# The desktop app and the terminal, side by side

Postio has two apps on Linux: the desktop app (GTK) and the terminal app,
`postio-tui`. They use the same mailbox, one at a time, and share one
implementation of everything they do to it. They are meant to do the same
things, each in its own medium. This table says where they differ today.

- ✓ has it
- ◐ has it, differently or in part (the note says how)
- ✗ missing
- — not applicable

The terminal's gaps are tracked in
`crates/postio-tui/tests/registry_parity.rs`, which fails for any command the
terminal neither handles itself nor passes on to something that does. That
test and this table change together.

## Accounts

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| First run: find the servers, sign in with a password | ✓ | ✓ | The same steps and sentences |
| Sign in with a browser (OAuth) | ✓ | ✓ | The terminal shows the whole address; Enter opens it, `y` copies it |
| Add a second account | ✓ | ✗ | |
| Enable, disable, remove (with undo), make default, rebuild index, update credential | ✓ | ✓ | From Settings |
| Map a folder's role (Sent, Archive, …) | ✓ | ✓ | `M` asks for the role, then the folder |
| Cycle the account scope, including all accounts at once | ✓ | ✓ | |

## The list

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| Folders and the Flagged, Snoozed, Drafts and Outbox views | ✓ | ✓ | |
| Saved searches in the sidebar | ✓ | ◐ | Shown and run; rename, delete and reorder are missing |
| Go-to keys (inbox, sent, drafts, flagged) | ✓ | ✓ | |
| Back to the previous view | ✓ | ✓ | |
| Collapse a folder in the sidebar; toggle the account rail | ✓ | ✗ | |
| Cursor and selection kept apart; multiple selection | ✓ | ✓ | |
| Archive, delete, move, flag, mark unread, label, snooze, undo | ✓ | ✓ | |
| Conversations, and walking one with `J`/`K` | ✓ | ✓ | |
| Toggle the sidebar | ✓ | ✓ | On a narrow terminal it is brought forward instead |
| The mouse: click, select, scroll, drag the divider | ✓ | ✓ | |

## Reading

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| HTML mail, sanitised | ✓ | ◐ | The terminal draws it as styled Markdown |
| Fold and unfold every quote | ✓ | ✓ | |
| Fold and unfold one quote | ✓ | ◐ | By the mouse only; the key is missing |
| Images in a message | ✓ | ◐ | Labelled placeholders; drawing them is the next iteration |
| Remote images allowed per sender | ✓ | ✓ | The same allow list |
| Open a link | ✓ | ◐ | A click shows where it goes and a second opens it; no key yet |
| Attachments: open, save, save all | ✓ | ✓ | |
| Open a part with another app; show a part once; view the original source | ✓ | ✗ | |
| Unsubscribe | ✓ | ✓ | |
| Drag a message or a part out to another app | ✓ | — | A terminal has nothing to drag to |

## Writing

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| New, reply, reply all, forward; drafts; resume a draft | ✓ | ✓ | |
| The editor | rich text | Markdown | Markdown is sent as HTML with the Markdown as the plain-text part |
| Formatting: bold, italic, lists, link, quote | ✓ | ✗ | |
| Preview what will be sent; edit in `$EDITOR` | — | ✓ | `Alt+P`, `Alt+E` |
| Recipients from contacts; Cc and Bcc; identities | ✓ | ✓ | |
| Attach a file; paste an image; drop a file | ✓ | ✓ | In the terminal a drop arrives as its path |
| Schedule send; undo send; why a send failed | ✓ | ✓ | |
| Save the draft now | ✓ | ◐ | The terminal saves as you type; the command itself is missing |
| A composer of its own | window | tab | |

## Search

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| The query language, operators shown as chips | ✓ | ✓ | |
| Facets: refine by sender, folder, date | ✓ | ✗ | |
| The finder's modes (`>` `#` `+` `@`), the palette, the key list | ✓ | ✓ | |

## Settings

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| Every section of `config.toml` | panes | `$EDITOR` | The terminal opens the file at the section |
| The signature editor | ✓ | ✗ | |
| The connection log and the privacy log | ✓ | ✗ | |

## Everything else

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| New-mail notifications | ✓ | ✓ | The terminal uses its status line and `notify-send` |
| Colours | light and dark | the terminal's own | `NO_COLOR` is honoured; roles are set in `[tui.colors]` |
| One mailbox, whichever app is open | ✓ | ✓ | One app at a time |
| Runs over SSH, with no display | ✗ | ✓ | |
