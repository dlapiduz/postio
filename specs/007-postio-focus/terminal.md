# Focus in the terminal

The terminal interface (`postio-tui`) is Postio Focus drawn in character
cells (C29, User Story 16, FR-186 to FR-199). It has the GTK interface's
surfaces, verbs, words and keys. Only the drawing differs. This file is the terminal's
`contracts/focus-surface.md`: what each surface looks like in cells, and the
few places a terminal has to differ from a window, each with its reason.

Everything in [contracts/focus-surface.md](contracts/focus-surface.md) holds
unless a row below says otherwise. The behaviour is in
[spec.md](spec.md), the keys in [contracts/keymap.md](contracts/keymap.md),
and the rules a terminal adds (hostile text, `NO_COLOR`, the mouse, the store
lock) in `specs/005-tui-frontend`.

## What carries over and what changes

| | Desktop | Terminal |
|---|---|---|
| Home | The dense inbox, unified across accounts | The same |
| Folders | No sidebar: `g o`, `in:` and the command bar | The same. The three-pane layout, the sidebar, the panes and the parts panel are gone |
| A message | Opens in a dialog over the list, or beside it (`F8`) | The same, as a framed overlay or a pane |
| Body | App colours or the original on paper (`O`) | Always text in the terminal's colours: HTML is converted, images stay blocked. `O` is not offered (`Requirement::Graphical`) |
| Composer | Postio's composer in the dialog frame | The terminal's Markdown composer in the dialog frame. Detach takes the whole screen |
| Pickers | Popovers at the focused row | Framed boxes at the focused row |
| Toast | `AdwToast`, 8 s | The bottom line, 8 s |
| Menu | ☰: Settings, Read beside the list, Keyboard shortcuts, About, Quit | No menu. Each item is a command with its key and a command-bar row |
| Accent | The system accent | `postio_ui::tokens::accent_rgb()` on true-colour terminals; the terminal's own palette otherwise (spec 005 FR-054) |
| Engine | `Host::enable_focus` | The same: filtering, digests, reminders and the needs-action detector act while the terminal runs |

## Units

A cell is the unit. Every width below comes from the terminal's size `W × H`
and from nothing else, so stepping through mail never moves a frame. The
minimum stays 50×12, below which the screen says "Terminal too small: needs
50×12" and draws nothing else.

## The screen (01, 02)

```
 Compose c   ⌕ Search mail, go to a folder, or run a command   / ctrl+k       ✓ Synced 16:09   ? keys
 Inbox ▾ g o   312 · 41 unread   ⚑ Has action · 7 !                   186 filtered today g f   4 digest rules g d
 Today · Saturday 26 September
 ● Grace Oyelaran      Invitation: Harbor design review ●Harbor  Tue 29 Sep 10:00–10:45, Room 3B…   16:02
                       Invite  Tue 29 Sep · 10:00–10:45                            Accept y  Decline Y
 ≡ Weekly · digest     Newsletters · 14 messages  Summary of 14 messages from 6 senders: rail…   14  16:00
▌● Ada Moreno          Re: Atlas Q3 budget, final numbers ●Atlas  Hi, the final Q3 numbers are…  ⎘ 3  15:51
▌                      Question  “Can you approve these by Friday so finance can close the quarter?”  Reply e
 ✓ Tomás Reyes         Atlas staffing plan for Q4 ●Atlas  Sharing the draft before Monday’s sync…     15:40
   Marco Ruiz          Cabinet order: please sign ●Kitchen reno ●Home  Attached the final order…  ⎘ 4  14:31
 …
 3 selected   Archive a  Snooze s  Mark read r  Digest these… d  Label l  Move m  Delete Del    x toggle  J K extend  Esc clear
```

| Row of the screen | Height | Contents |
|---|---|---|
| Top bar | 1 | `Compose c`; the command field, `min(60, W − 40)` wide and centred, with `/` and `ctrl+k`; on the right the sync label (`focus_state::sync_label_here`) and `? keys`. Below 80 columns the field shrinks to its placeholder's first word and the keys |
| Strip | 1 | `Inbox ▾ g o` (the place's name); `focus_row::strip_counts`; the toggle `⚑ Has action · N !`; on the right `filtered::today` with `g f` and "N digest rules" with `g d`. Each count shows only while its feature is in use (C10). Narrowing drops the right side first, then the counts |
| Banner | 1 when shown | `focus_state::banner` on the surface background: heading in bold, sentence, then its action and key (`Retry now F5`, `Update password…`). First sync adds `Syncing a of b` and a bar of `━` and `─` |
| List | the rest | Day headings and rows |
| Bottom line | 1 | The bulk bar while anything is selected; otherwise the toast, a pending chord (`g …`), or nothing |

There is no hairline under the strip. The day heading and the strip's
surface background separate the two.

With the has-action filter on (03), the toggle takes the accent (reversed
under `NO_COLOR`), the strip adds `focus_row::showing`, and the one heading
is `focus_row::has_action_label`.

## Rows

A row is **one line**, or **two** when it has a marker or is a fired
reminder (`focus_row` decides which). Rows have no rule between them, so a
36-line terminal shows about 30 rows. Heights depend only on the row's kind.

| Column | Cells | Contents |
|---|---|---|
| Cursor | 1 | `▌` in the accent on every line of the cursor's row. Nothing else about the row changes |
| Mark | 2 | `●` unread (in the accent when the row has a marker), `✓` selected, `≡` a digest, `↺` a reminder; then a space |
| Sender | 20 from 100 columns, 16 below, 12 beside the pane | Bold when unread, truncated by display width |
| Subject | the rest | Bold when unread; then up to two pills (`●` in the label's colour from `label_colour`, then the name, dimmed); then the first line, dimmed, cut with `…` |
| Trailing | as needed | The draft's sending state in Drafts and the Outbox; `⎘` for an attachment; the count when above one (`focus_row::count_badge`); the time (`row::timestamp`, five cells, right-aligned), bold when unread |

The first line gives way first as the terminal narrows, then the pills. The
sender, subject and time always stay.

**Second line**, starting under the subject column: the chip (`Invite`,
`Question`, `To-do`, `No reply`) bold in the accent; the date in the accent;
for a question or to-do the quote, italic in the accent, between quotation
marks; and on the right the answering actions from `focus_row::marker_line`,
each a label then its key, dimmed: `Accept y  Decline Y`, `Reply e`, or
`Task t  Snooze s` (Task only with a vault). An answered, cancelled or past
invitation shows its status in dim ink where the actions were.

**Selection** is `✓` in the mark column and the surface background across
the row. **The cursor** is the `▌` alone. The two stay distinguishable under
`NO_COLOR`, where the surface background is reversed video and the `▌` is
kept.

**Digest row:** `≡ Weekly · digest`, then `focus_row::digest_subject` in
bold, then `focus_row::digest_line` dimmed, the message count and the time.
`Enter` opens it, and selection skips it.

## The open message (04 and the message dialog's screens)

```
 Compose c   ⌕ Search mail, go to a folder, or run a command   / ctrl+k       ✓ Synced 16:09   ? keys
 In░░╭──────────────────────────────────────────────────────────────────────────────────────╮░░░░░░░░░
 ░░░░│ ↑ k  ↓ j                     Harbor API draft v3                              Esc ✕   │░░░░░░░░░
 ░░░░│                       Message 5 of 312 · thread of 6                                  │░░░░░░░░░
 ░░░░│ Reply e  Reply all E  Forward f  Archive a  Snooze s  Remind h  More .                 │░░░░░░░░░
 ░░░░├──────────────────────────────────────────────────────────────────────────────────────┤░░░░░░░░░
 ░░░░│        Latest of 6 in this thread  [ earlier message                                 │░░░░░░░░░
 ░░░░│        Harbor API draft v3                                                           │░░░░░░░░░
 ░░░░│        ●Harbor  + Label l                                                            │░░░░░░░░░
 ░░░░│        ───────────────────────────────────────────────────────────────────           │░░░░░░░░░
 ░░░░│        From  Lena Park <lena@example.org>                         Today, 15:22       │░░░░░░░░░
 ░░░░│        To    you, Ben Adeyemi, Grace Oyelaran                                        │░░░░░░░░░
 ░░░░│        ───────────────────────────────────────────────────────────────────           │░░░░░░░░░
 ░░░░│        To-do  Wed 30 Sep  “Please leave comments by Wednesday”  Snooze s  Dismiss -  │░░░░░░░░░
 ░░░░│                                                                                      │░░░░░░░░░
 ░░░░│        Hi all,                                                                       │░░░░░░░░░
 ░░░░│        Uploaded v3 of the Harbor API draft with the pagination changes.              │░░░░░░░░░
 ░░░░│        …                                                                             │░░░░░░░░░
 ░░░░│        ⎘ Harbor-API-v3.pdf 212 KB   ⎘ harbor-openapi.yaml 38 KB                       │░░░░░░░░░
 ░░░░│        ▸ 31 quoted lines                                                             │░░░░░░░░░
 ░░░░╰──────────────────────────────────────────────────────────────────────────────────────╯░░░░░░░░░
 3 selected   Archive a  Snooze s  …
```

- **Painting.** The frame is painted once its thread and body have
  landed, or after 16 ms (one frame of the interaction budget) if they are
  slow, so it does not jump as each read arrives. Every frame is written
  inside a synchronized update, so a terminal that keeps them never shows
  one half drawn.
- **Frame.** Rounded border. It covers every row but the top bar and the
  bottom line, so the bulk bar stays in sight, as on 04. Width
  `clamp(76, W − 2·max(4, ⌊0.12·W⌋), 100)`, or the whole width below 80
  columns: 92 at 120, 100 from 136. The list behind is drawn dimmed.
- **Header**, three fixed rows. Row 1: the step keys `↑ k  ↓ j` on the left,
  the subject in bold centred, `Esc ✕` on the right. Row 2: the position
  line, dimmed and centred. Row 3: the action row
  (`focus_dialog::send_verbs` for a draft on its way). Below 96 columns of
  frame, Label, Move and Delete fold into `More .`. A hairline closes the
  header.
- **Column.** Everything below the header scrolls together, in one column
  `min(72, frame − 8)` wide and centred, which keeps a line to about 70
  characters. The rhythm is the dialog's, in rows: the thread line; the
  subject in bold; the pills and `+ Label l`; a hairline; From, To and Cc in
  a grid (label column 6, the date on the right); a hairline; the action
  card; one blank row; the body; one blank row; the attachments; the fold
  line. A block that is absent takes its gap with it.
- **Action card.** It is the only filled element: the surface background on
  its one or two rows, with the chip, date and quote in the accent and
  `Snooze s  Dismiss -` on the right. The quoted sentence is found in the
  body as the desktop finds it, and drawn there on the surface background
  with an accent underline.
- **Body.** spec 005's reader: HTML converted to text, links underlined in
  the accent and numbered, `▸ N quoted lines` folds. A line wider than the
  column wraps. No `O` line: a terminal has one treatment.
- **Keys.** `j`/`k` step through the list without closing; `[`/`]` step
  through the thread; `o` opens the chooser of links and attachments; `v` the
  raw source; `Esc` closes on the same row with the selection kept. Read on
  dwell is the desktop's ("Reading marks it read" in screens.md).

## Find in the message

`ctrl+f` opens a one-line field at the foot of the open message's frame, or
the pane: `⌕ <typed>`, then "N of M" or "No matches" and the step and close
keys. Every match is reversed in the accent and the current one is also bold
and underlined, so they hold under `NO_COLOR`. `Return` and `ctrl+g` step
forward, `shift+F3` back, wrapping at the ends and scrolling the match into
view; `Esc` closes the field before the message. While `v` shows the raw
source, find searches that.

## Reading beside the list (`F8`)

`[focus] reading = "pane"` places the open message beside the list when the
terminal is at least 128 columns wide. The pane is `min(100, W − 56)` wide,
with a `│` between it and the list, and the list keeps the rest (at least
56). The pane is the message view above with no border, and its header rows
sit under the strip. Narrower than 128, a message opens in the overlay and
the setting is kept. Crossing the line with a message open moves it, still
open, at its place in the thread. The rest is screens.md, "Reading beside
the list".

## The composer (05, 06)

The open message's frame, holding spec 005's composer:

- Header row 1: `Detach alt+o` on the left; the title (`New message`,
  `Reply`, `Reply all`, `Forward`) in bold centred; `Esc ✕` on the right.
- Header row 2: the subtitle `Markdown · 42 words · Draft saved locally 16:10`.
- Header row 3: `Send ctrl+↵` (bold, the one primary), `Send later`,
  `Attach`, `Remind · Tue ctrl+h`.

Below the header sit the fields (To, Cc and Bcc, From, Subject) and the
modeless body with its preview, exactly as spec 005 has them.

`Detach` gives the composer the whole screen between the top bar and the
bottom line, where the three-pane app had its tab. Detaching again, or
`Esc`, brings it back.

## The command bar (07, 08, 09)

`/` or `ctrl+k` turns the top bar's field into an input. A box opens under
it, `min(100, W − 4)` wide and centred, as tall as its results:

```
 ╭──────────────────────────────────────────────────────────────────────────────────╮
 │ Saved  Waiting on reply alt+1  Atlas alt+2  Receipts this month alt+3   ctrl+s saves │
 │ ⌕ arch▏                                                                     Esc  │
 ├──────────────────────────────────────────────────────────────────────────────────┤
 │ Commands                                                                         │
 │▌Archive                                                                       a  │
 │ Archive selection                                                             a  │
 │ Go to                                                                            │
 │  in:Archive                                                                 g r  │
 │  Search mail for “arch”                                                          │
 ├──────────────────────────────────────────────────────────────────────────────────┤
 │ ↑↓ move  ↵ run  > commands only                                     Local index  │
 ╰──────────────────────────────────────────────────────────────────────────────────╯
```

The rows, sections, chips, `in:` folders, saved searches and result headings
are `postio_ui::finder::blend` and `postio_search::natural`, as on the
desktop. Results open over the list as rows do.

## Folders and labels (10)

`g o`, or a click on `Inbox ▾`, opens a box 44 wide under the strip's left
end: a filter field, then Mailboxes (each with its `g` key; Outbox only while
it holds mail; Filtered while filtering is on), Folders, and Labels with
their coloured `●`. Counts are dimmed on the right.

## Pickers (11–14)

Snooze `s`, Remind `h`, Label `l` and Move `m` open a box 48 wide, anchored
under the focused row's subject column, or above the row when it does not fit
below:

```
 ╭─ Snooze until ───────────── Ada Moreno · Atlas Q3 budget ─╮
 │▌1  Later today                 18:00                       │
 │ 2  Tomorrow morning            Sun 27 Sep, 08:00           │
 │ 3  Monday morning              Mon 28 Sep, 08:00           │
 │ 4  Next week                   Sat 3 Oct, 08:00            │
 │ Or type a date: “tue 9am”                          Tab     │
 │ The message leaves the inbox and comes back at the top at │
 │ that time. Snoozed mail is under g z.                      │
 ╰────────────────────────────────────────────────────────────╯
```

The words, presets and typed dates are `postio_ui::pickers` and
`postio_ui::schedule`. Label shows `✓` for an applied label and toggles with
`Space`; Move lists Recent then All folders.

## The undo toast (15)

The bottom line holds `✓ Archived 3 messages · Undo ctrl+z` for 8 seconds,
or until the next action. Its words are the host's. While anything is
selected the bulk bar keeps the line, and the toast takes its right end in
place of the selection keys. `ctrl+z` undoes after the toast has gone.

## States (16–19)

- **Empty inbox.** `focus_state::empty_inbox` centred in the list: the
  heading, the detail and next-digest line, then its shortcuts on one line
  (`Filtered g f   Archive g r   Compose c`).
- **First sync, offline, sign-in error.** The one banner row, as above.
- **Store in use.** spec 005's sentence, before the alternate screen.

## Key map (20)

`?` opens an overlay over everything but the top bar:
`postio_ui::keymap_sheet::key_map` for the terminal, as columns 38 wide, as
many as fit, scrolling when they do not. Its footer names `[keys]` in
`config.toml`.

## Filtered (21)

`g f` replaces the strip and the list; `Return` opens the focused message
in the frame, aimed at that message, and `Esc` comes back to the same row.
The strip becomes
`‹ Inbox g i   Filtered · 186 today   Nothing here is deleted automatically      Sweep the inbox… F`.
Next comes a tab line (`1 All 186  2 Promotions 90  …`, the current tab in
bold and underlined), then the day-headed rows, each with its reason pill
(`filtered::pill`) before the time. `R` restores the focused row.

## Digests (22–24)

- **The digest window** is the message frame. Its header is the digest's
  title, subtitle and `Archive all 14 A`. Its tab row is
  `Summary  14 messages  Tab`, with the rule line and `Edit rule and cadence d`
  on the right. The summary's statements are paragraphs, each ending in its
  reference number `[6]`. The focused reference is reversed in the accent,
  and its message's card (sender, subject, date, opening, `↵ open the full
  email`) sits under its paragraph. The window's bottom row is the key line:
  `] [ next / previous reference   ↵ open   Tab summary / messages   D stop digesting the sender`.
  A referenced email opens inside the window, as on 23.
- **Rules** (`g d`) use Filtered's full view.
- **The rule dialog** (`d`) is a frame 64 wide holding the form and its
  preview.

## Obsidian capture (25)

`t` and `n` open a frame 72 wide: the Task/Note pair, the text, due quick
picks, the project with its reason, and the exact markdown line as the
preview. `ctrl+↵` writes.

## Compared with the drawings

Each surface is rendered by `cargo run -p postio-tui --example shot
--features test-support -- <out.svg> <width> <height> <state>` from the
states in `test_support::sample`, and read beside its drawing above. The
sample mail and its dates are the example's, not the drawings'. What still
differs, and why:

| Surface | Difference | Reason |
|---|---|---|
| Every surface | Keys read as the registry spells them, shortened in a tight place (`Esc`, `Del`, `↵`), and a chord the terminal cannot deliver shows its alternate: Send is `alt+s` and capture's write is `alt+↵` without the kitty keyboard protocol | C22; `postio_ui::terminal::deliverable_binding` |
| Inbox | Unread rows' times are bold, as the senders are | The row's unread rule, as on the desktop |
| Command bar | A dim echo line under the input says what was typed or which chip is being edited, and chips take a row of their own | The editing hint does not fit beside the input |
| Pickers | The title and the target sit in the box's top border | One row more for the presets in a short terminal |
| Filtered, digest rules | A line of keys at the foot of the view: restore, tabs, open and back; edit, remove and back | A full view has no frame to carry its keys, and the desktop's buttons have none to show |
| Digest window | `Archive all N A` is on the header's left, with the title centred and `Esc ✕` right | The frame's header has one shape: steps or primary on the left |
| Rule dialog, capture | The frame's buttons are bracketed, `[ Save ↵ ]`, and the primary is bold | A button in cells needs an edge to read as one |
| Capture | The due day's quick picks take a row of their own under the date | The date and five picks do not fit side by side in the frame |

## Colour and marks

Every role is spec 005's (`theme.rs`), and the accent is used only as the
desktop uses it: markers, the action card's text, links, the cursor's `▌`
and the has-action toggle while it is on. Under `NO_COLOR`, every state keeps
a mark that is not a colour: `▌` cursor, `✓` selected, `●` unread, `≡`
digest, `↺` reminder, `⎘` attachment, `⚑` the toggle, bold chips, italic
quotes, and reversed video for the surface background.

## Mouse

Everything the keyboard reaches has a click (spec 005 FR-030):

- rows: click, with `ctrl` to toggle and `shift` to extend;
- a row's answering actions;
- the strip's place, toggle and counts;
- the top bar's field, Compose and `? keys`;
- the bulk bar's verbs;
- a frame's step keys, close, action row and card;
- picker and bar rows;
- tabs;
- links, which open on a second click.

The wheel scrolls whatever is under the pointer. A click outside a frame
does nothing; `Esc` and `✕` close it.

## Not in the terminal

- The original-on-paper treatment and zoom (`Requirement::Graphical`).
- Images, which stay blocked.
- A main menu, About and a window close. `mod+q` quits, as everywhere.
