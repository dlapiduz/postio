# Postio in a terminal

`postio-tui` is Postio in a terminal: Postio Focus drawn in character
cells, with the same inbox, commands and keys as the desktop app, local or
over SSH, with the mouse as well as the keyboard. Mail is read and written
as Markdown. While it has your mail open, Focus's filtering, digests and
reminders act exactly as they do while the desktop app has it.

It is not a second mail client. The desktop app and the terminal use the
same mailbox: what you archive in one is archived when you open the other.
They take turns, though. Only one of them can have your mail open at a time,
so close one before opening the other
([ADR 0041](https://github.com/dlapiduz/postio/blob/main/docs/decisions/0041-one-app-opens-the-store-at-a-time.md)).

Where the two apps still differ is set out in
[The desktop app and the terminal, side by side](desktop-and-terminal.md).

## Running it

From a checkout:

```bash
cargo run -p postio-tui
```

It opens your mail store itself and syncs while it runs, as the desktop app
does. If the desktop app already has the store open, the terminal says
"Postio is already open in another window. Close it to open Postio here."
and exits without touching anything. Close the desktop app and start the
terminal again. The desktop app does the same the other way round.

There is no published package for the terminal frontend yet. When there is
one, it will be its own Flatpak, much smaller than the desktop one, sharing
the desktop app's store.

## First run

With no account yet, the terminal asks for one. It asks the same questions
the desktop app does, in the same order:
- the address;
- the servers Postio found for it, or the ones you type;
- a password or app-specific password;
- how far back to sync.

For a provider that signs in with a browser, the terminal shows the whole
sign-in address. It opens nothing on its own:
- **Enter** opens the address;
- **y** copies it;
- **Esc** gives up.

Over SSH, copy the address and open it on your own machine.

## Keys and the mouse

The keys are the desktop app's and come from the same `[keys]` table in
`config.toml`. There is no separate terminal keymap. **`?`** shows the key
map, as the terminal can send each key. The command bar is **`/`**, for
search, and **`Ctrl+K`** opens it on the commands:
- plain words become operator chips;
- `>` limits it to commands, each shown with its key;
- `in:` goes to a folder; **`g o`** lists folders and labels;
- **`Alt+1`**–**`Alt+4`** run the pinned saved searches, and **`Ctrl+S`**
  saves the one you typed.

A message opens over the list with **`Enter`**, and **`Esc`** brings you
back to the same row. **`F8`** reads beside the list instead, on a terminal
at least 128 columns wide. **`s`**, **`h`**, **`l`** and **`m`** open the
snooze, remind, label and move pickers at the row.

Some terminals cannot send every chord the desktop uses, such as
`Ctrl+Shift+E`. Where the terminal speaks the kitty keyboard protocol,
Postio asks for it. Where it does not, those commands have an `Alt`
alternative, and the command bar and `?` show whichever one your terminal
can actually send.

Everything the keys do, the mouse does too:
- a click moves the cursor;
- `Ctrl`-click and `Shift`-click select;
- a row's answers, the strip, the bulk bar and every button take a click;
- the wheel scrolls what is under the pointer.

To keep your terminal's own text selection instead, turn the mouse off:

```toml
[tui]
mouse = false
```

## Reading and writing Markdown

A message is shown as Markdown. Its structure survives: headings, emphasis,
lists, quotes and code. Quoted history is folded. A click on a link shows
where it goes, and a second click opens it. Remote images stay blocked per
sender, exactly as in the desktop app.

Images are shown as labelled placeholders, `[image: …]`. **`o`** lists a
message's links and attachments and opens any of them in your system's
viewer. Drawing images
inside the terminal is planned for a later release.

The composer opens over the list and takes Markdown, and sends it as a
formatted message, with a plain-text part beside it. **`Alt+O`** gives it the
whole screen.
- **`Alt+P`** shows how the message will look.
- **`Alt+E`** edits it in your `$EDITOR` and brings it back.
- To show the preview beside the editor instead of in its place:

  ```toml
  [tui]
  preview = "split"
  ```

## Attachments, drag and drop, and the clipboard

This follows the pattern other terminal applications use:
- **Dropping a file** onto the terminal pastes its path, and Postio attaches
  the file.
- **Pasting a path** does the same.
- **Pasting while an image is on the clipboard** inserts that image inline.
  Postio reads it from the system clipboard, trying Wayland first, then
  `wl-paste`, then `xclip`.

Copying, such as a sign-in address or a link, uses the terminal's own
clipboard escape (OSC 52). That reaches your machine's clipboard even over
SSH, if your terminal allows it; some terminals ask first or turn it off by
default.

Over SSH, Postio reads the *server's* clipboard, not yours, so it usually has
no image to paste. It says so rather than inserting nothing. Attach the file
by path instead.

With no opener on the machine, such as on a server, a link goes to the
clipboard instead of opening.

## Colours

Postio uses your terminal's colours. It follows `NO_COLOR`, in which case
marks and weight carry every meaning, and `COLORTERM` for full colour, where
the accent comes from Postio's own design. The accent marks only what the
desktop app marks with it: action markers, links, the cursor's bar and the
has-action filter while it is on. The roles are `text`, `dim`, `accent`, `selection`,
`focus`, `unread`, `flagged`, `link`, `quote`, `code`, `error`, `warning`,
`success` and `surface`; `selection` and `surface` are backgrounds. Any role's
colour can be changed:

```toml
[tui.colors]
flagged = "#ff8800"
```

## The log

The terminal writes its log to the systemd journal, never to the screen:

```bash
journalctl --user -t postio-tui -f
```

`POSTIO_LOG` and `[logging]` in `config.toml` set the level, as for the
desktop app, and a change to the file takes effect in a running terminal.
On a machine without a journal the log goes nowhere.
