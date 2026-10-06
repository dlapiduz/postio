# The desktop app and the terminal, side by side

Postio has two apps on Linux: the desktop app (GTK) and the terminal app,
`postio-tui`. They use the same mailbox, one at a time, and share one
implementation of everything they do to it. They are meant to do the same
things, each in its own medium. This table says where they differ today.

The desktop app is one dense inbox with no folder sidebar: a message opens
over the list or beside it, and `g o` lists every place. The terminal is
being rebuilt to the same design; its column describes it as it stands,
and changes with that work.

- ✓ has it
- ◐ has it, differently or in part (the note says how)
- ✗ missing
- — not applicable

The terminal's gaps are tracked by a test,
`every_command_is_answered_here_or_by_the_dispatcher` in
`crates/postio-tui/src/app.rs`. It fails for any command the terminal neither
handles itself nor passes on to something that does, unless its `GAPS` list
names it. The list is empty now: every command does something in both apps.
What is left below is how they do it, and the features that are not a
single command.

## Accounts

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| First run: find the servers, sign in with a password | ✓ | ✓ | The same steps and sentences |
| Sign in with a browser (OAuth) | ✓ | ✓ | The terminal shows the whole address; Enter opens it, `y` copies it |
| Add a second account | ✓ | ✓ | `Alt+N` from anywhere; Escape goes back to the mail |
| Enable, disable, remove (with undo), make default, rebuild index, update credential | ✓ | ✓ | From Settings |
| Map a folder's role (Sent, Archive, …) | ✓ | ✓ | `M` asks for the role, then the folder |
| Cycle the account scope, including all accounts at once | ◐ | ✓ | The desktop inbox is every account's at once; `account:` in a search narrows it to one |

## The list

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| Folders and the Flagged, Snoozed, Drafts and Outbox views | ✓ | ✓ | The desktop lists them in the folders popover, `g o` |
| Saved searches: run, rename, reorder, delete | ◐ | ✓ | The desktop runs the pinned ones from the command bar (`Alt+1`…`Alt+4`); the rest is `config.toml`. The terminal: `r`, `Shift+↑`/`Shift+↓`, `d` (twice: it asks first) |
| Go-to keys (inbox, sent, drafts, flagged) | ✓ | ✓ | |
| Back to the previous view | ✓ | ✓ | |
| Folders nested as the server keeps them; fold one | — | ✓ | The desktop's popover filters them as you type instead; the terminal folds with Space, or a click on its mark |
| The conversation rail | — | — | The desktop opens one message at a time |
| Cursor and selection kept apart; multiple selection | ✓ | ✓ | |
| Archive, delete, move, flag, mark unread, label, snooze, undo | ✓ | ✓ | |
| Conversations, and walking one | ✓ | ✓ | `]`/`[` on the desktop |
| Toggle the sidebar | — | ✓ | The desktop has none. On a narrow terminal it is brought forward instead |
| The mouse: click, select, scroll, drag the divider | ◐ | ✓ | The desktop has no divider to drag |

## Reading

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| HTML mail, sanitised | ✓ | ◐ | The terminal draws it as styled Markdown |
| Fold and unfold every quote | ✓ | ✓ | |
| Fold and unfold one quote | ✓ | ✓ | By a click on it, in both |
| Fold a message in a conversation to its header | — | ✓ | The desktop shows one message at a time |
| Images in a message | ✓ | ◐ | Labelled placeholders; drawing them is the next iteration |
| Remote images allowed per sender | ✓ | ✓ | The same allow list |
| Open a link | ✓ | ◐ | A click shows where it goes and a second opens it; no key yet |
| Attachments: open, save, save all | ✓ | ✓ | |
| Reader view, or the sender's own markup (`View original`) | ✓ | ✓ | |
| Find in the message on screen (`Ctrl+F`, next and previous match) | ✓ | — | Not yet in the terminal: its `GAPS` list names the three commands |
| Zoom a message in and out; darken a designed message in dark mode | ◐ | — | The desktop zooms, and draws a designed message on paper or in its own colours (`O`) rather than darkening it. The terminal draws text in its own font and colours, so there is nothing for these to act on |
| Open a part with another app | ✓ | ◐ | The terminal opens every part with the system's default app |
| Show a held-back part once, with what it references | — | — | The parts panel is the macOS app's; a terminal cannot draw the images either |
| Unsubscribe | ✓ | ✓ | |
| Drag a message or a part out to another app | ✓ | — | A terminal has nothing to drag to |

## Writing

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| New, reply, reply all, forward; drafts; resume a draft | ✓ | ✓ | |
| The editor | rich text | Markdown | Markdown is sent as HTML with the Markdown as the plain-text part |
| Formatting: bold, italic, lists, link, quote | ✓ | ◐ | As Markdown: around the selection, or a pair to type into; lists and quotes toggle on the line |
| Preview what will be sent; edit in `$EDITOR` | — | ✓ | `Alt+P`, `Alt+E` |
| Recipients from contacts; Cc and Bcc; identities | ✓ | ✓ | |
| Attach a file; paste an image; drop a file | ✓ | ✓ | In the terminal a drop arrives as its path |
| Schedule send; undo send; why a send failed | ✓ | ✓ | |
| Save the draft now | ✓ | ✓ | It also saves as you type |
| Where the composer opens | the open message's place, or a window | tab | |

## Search

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| The query language, operators shown as chips | ✓ | ✓ | |
| Facets: search in a scope (all mail, inbox only, lists), refine by what the matches share | ✓ | ✓ | A row over the results; Tab walks it, Enter or a click picks |
| The finder's modes (`>` `#` `+` `@`), the palette, the key list | ✓ | ✓ | |

## Settings

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| Every section of `config.toml` | panes | `$EDITOR` | The terminal opens the file at the section |
| Signatures: write, add, rename, delete | ✓ | ✓ | `s` on an account in Settings; the text is written in `$EDITOR` |
| The privacy pane: senders allowed remote images, lists left, read receipts, recent connections | ✓ | ✓ | The same words from the same place; `d` on a sender asks for its images again |

## Everything else

| Feature | Desktop | Terminal | Notes |
|---|---|---|---|
| New-mail notifications | ✓ | ✓ | The terminal uses its status line and `notify-send` |
| Colours | light and dark | the terminal's own | `NO_COLOR` is honoured; roles are set in `[tui.colors]` |
| One mailbox, whichever app is open | ✓ | ✓ | One app at a time |
| Runs over SSH, with no display | ✗ | ✓ | |
