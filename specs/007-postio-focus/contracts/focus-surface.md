# Contract: Focus's surfaces

Each screen is built against its PNG (`Design/postio-focus-design/screens/`,
and `Design/focus-message-dialog/` for the open message). Exact sizes and
copy come from the mockups (`source/*.dc.html`). This contract is what a
build is checked against; [screens.md](../screens.md) records what still
differs from each PNG, and holds the designs that have none (the open
message's treatments, the reading pane, sending states, the row menu and
Settings). The measurements are for a 1440×900 window, the size the screens
were drawn at.

**Colour.** Colour comes from libadwaita's named colours (research R11), and
the accent is the system's (C26). It is reserved for:

- action markers, and the open message's action card, its tag and the body's
  links;
- the focus ring;
- the has-action toggle when it is on;
- the unread dot's place on a row that has a marker.

"Raised" buttons (Send, Create, Add task, Archive all) are plain raised
buttons with bold labels, never the suggested-action style.

**Type.** Adwaita Sans for the chrome, Adwaita Mono for keys, addresses,
counts and operators (C25).

**Keycaps** come from the registry through the shared `keyhint` widgets,
never from literals (`check-key-hints-are-derived.py`). A key is taught
inside the control it runs.

**One of each.** One close button (an X at the right end of a header), one
icon button, one keycap, one dialog pattern for every surface over the list,
and one picker pattern (screens.md, "Interaction rules").

## The window (01, 02)

| Part | Height | Contents, left to right |
|---|---|---|
| Top bar | 46 px | Compose (pencil icon); the command-bar field (480 px, centred): "Search mail, go to a folder, or run a command", keycaps `/` and `ctrl+k`; the sync label ("Synced 16:09", with its icon); the main menu (☰); close (×) |
| Header strip | 36 px | "Inbox ▾", keycap `g o`; "312 · 41 unread"; a divider; the toggle "Has action · 7" with keycap `!`; on the right, "186 filtered today" `g f` and "4 digest rules" `g d` |
| Banner slot | 42 px when shown | One `AdwBanner` (17–19) |
| Day heading | 32 px | "Today · Saturday 26 September"; with the has-action filter on, "Has action · 7" |
| List | the rest | Rows (below); with the reading pane, the list on the left and the pane on the right |
| Bulk bar | 44 px when anything is selected | "3 selected"; Archive `a`, Snooze `s`, Mark read `r`, Digest these… `d`, Label `l`, Move `m`, Delete; on the right, the selection keys |

- **The header counts show only while their feature is in use:** the
  filtered count while filtering is on, the rule count while there are rules
  (FR-018).
- **With the has-action filter on (03),** the toggle is drawn in the accent,
  and the strip adds "Showing 7 of 312 · ! again to show all".
- **The main menu** holds Settings (`mod+comma`), Read beside the list (`F8`,
  a check item), Keyboard shortcuts (`?`), About, and Quit (`mod+q`).

## Rows

**Heights.** Two, fixed by the row's kind, never by its content:

- **40 px:** a conversation with no marker, or a digest delivery;
- **72 px:** a conversation with a marker, or a fired reminder.

**Columns:**

| Column | Contents |
|---|---|
| Gutter (24 px) | The accent dot when the row has a marker; a checked box when the row is selected; the stack icon on a digest |
| Sender (x = 56, 222 px) | The display name, bold when unread. In a list narrower than the inbox's (beside the reading pane) it gives way first, to no less than 120 |
| Subject | Bold when unread; then up to two label pills (a colour dot and the name); then the first line, dimmed, truncated at the end |
| Trailing | In Drafts and the Outbox, the draft's sending state; the attachment icon; the count badge, when the conversation holds more than one message; the time, bold when unread |

**The second line** on a 72 px row:

- a kind chip, outlined in the accent: "Invite", "Question", "To-do" or
  "No reply";
- the date in the accent, bold: "Tue 29 Sep · 10:00–10:45", "Wed 30 Sep", or
  "since Sat 26 Sep";
- for a question or to-do, the quoted sentence in the accent, italic, inside
  quotation marks;
- the answering actions on the right, each with its keycap: Accept `y` and
  Decline `Y`; or Reply `e`; or Snooze `s`, with Task `t` before it when a
  vault is configured. An answered, cancelled or past invitation says so in
  dim ink where its buttons were.

**States:**

- **Focus:** the focus ring is a 2 px accent border around the row, tinting
  nothing.
- **Selection:** a selected row has a neutral background and a checked box.
- **Neither changes the row's height.**

**A digest row:**

- the sender column reads "Weekly · digest";
- the subject reads "Newsletters · 14 messages";
- the first line is the summary's opening when there is one, and the
  digest's senders otherwise (FR-124);
- the count badge shows 14, and the time 16:00.

**A reminder row** has a marked row's anatomy, and the conversation's latest
message is its first line. Its second line holds "No reply", "since <date>"
and Reply `e`.

**What a screen reader hears:** the sender, subject, first line, "unread",
the marker's kind and date, and then the shortcut. A keycap is exposed as its
control's shortcut, never read as text (FR-096).

## The open message (04)

An `AdwDialog` over the list, or a pane beside it (FR-038). Its geometry is
`postio_ui::focus_dialog`, and its full design is screens.md, "The open
message".

- **Size.** `clamp(640, W − 2·max(96, 0.18·W), 820)` wide and the window's
  height less 80, centred, radius 12, the list dimmed behind. In the pane,
  `min(820, W − 404)` wide beside the list.
- **Header (52 px).** Left: the steps, one linked pair with `k` and `j`
  inside. Centre: the subject, and "Message 5 of 312 · thread of 6" in mono.
  Right: the X.
- **Action row (44 px, hairlines above and below).** Reply `e`, Reply all
  `E`, Forward `f`, Archive `a`, Snooze `s`, Remind `h`, Label `l`, Move `m`,
  Delete `Del`. Below 760 px, Label, Move and Delete fold into More `.`. A
  draft being sent shows its own verbs instead.
- **Column** (`min(480, dialog − 96)` in app colours, `min(640, dialog −
  48)` on paper), centred, in order:
  1. a thread marker, "Latest of 6 in this thread · `[` earlier message";
  2. the subject, Adwaita Sans 600 30/34;
  3. its label pills, and "+ Label `l`";
  4. the sender block, hairlines above and below: From (name, and the
     address in mono), To, Cc, and the date on the right;
  5. the action card, the only filled element (the accent at 8% light, 12%
     dark), with the kind tag, date, quote and actions, Dismiss `-` among
     them;
  6. for an HTML body, the render-mode line ("App colours · sender colours
     and fonts removed · Show original O");
  7. the body;
  8. attachment chips: a file icon, the name, the size in mono;
  9. the fold line, "31 quoted lines".
- **Scrolling.** Everything below the action row scrolls together as one
  column.

## Compose (05, 06)

The composer is an `AdwDialog` at the open message's size, or the reading
pane's occupant, and can be detached. Its layout is the open message's with
the composer's verbs (screens.md, "The composer").

- **Header (52 px).**
  - Left: Detach, an icon button.
  - Centre: "New message", "Reply", "Reply to all" or "Forward", with
    "Plain text · 58 words · Draft saved locally 16:12" under it in mono.
  - Right: the X.
- **Action row (44 px, hairlines above and below).** Send `ctrl+↵`, the one
  primary (raised, bold); Send later `ctrl+⇧+↵` ▾; Attach `ctrl+⇧+a`;
  "Remind · Tue 29 Sep" `ctrl+h` (its tooltip "Remind if no reply").
- **One column,** `min(480, dialog − 96)`, centred, for everything below.
- **Fields,** 40 px each, one label column:
  - **To:** recipient chips (name, address in mono, ×). On the right,
    "+ Cc".
  - **Cc** and **Bcc:** when shown, and Cc when replying to all.
  - **From:** the identity.
  - **Subject.**
  - **Labels:** chips, with "from the thread" on the right when replying.
- **Formatting toolbar:** icon buttons, 26 px.
- **Body:** on the dialog's surface, in its ink, at the column's edges.
- **Attachments:** under the body, each with its name, size and remove
  button.
- **Suggestions.** A 440 px popover. Each row shows the name (bold), the
  address (mono), and "wrote 42 times", "wrote twice" or "list". Its footer
  reads "↑↓ choose · ↵ or Tab add · from your address book and mail on this
  computer".

## The command bar (07, 08, 09)

The bar opens in place (C24): its input is drawn over the top bar's own
field, same width, same place, and the results hang below it in a panel
860 px wide. `/` opens it for mail search; `ctrl+k` opens it in command mode,
`>` already typed.

- **Saved row.** "Saved", then a pill for each pinned saved search with its
  key (`alt+1` …). `mod+s` saves the current query.
- **Input row.** The search icon, the chips, the caret, and `Escape`.
- **Echo line (07).** "You typed “the invoice Ada sent last month” · editing
  after: · `Tab` next chip · `ctrl+BackSpace` back to plain words".
- **Results (07, 08).** A section heading ("Conversations · 3 matches", or
  "Receipts · folder · 214 conversations · newest first"). Each row holds the
  sender, the subject in bold, the first line dimmed, where it lives in mono
  (`in:Inbox`), and the date. A misspelling adds "Search instead for “…”",
  and the list says "Sorted by relevance" or "Sorted by date" (`O`).
- **Commands (09).** Three sections:
  - **Commands:** the title; the key on the right.
  - **Go to:** for example "in:Archive `g r`".
  - **Search:** "Search mail for “arch”".
- **Footer.** "↑↓ move · ↵ open / run · > commands only · Local index".

## The folders popover (10)

A 400 px popover, anchored to "Inbox ▾".

- **Field:** "Go to folder or label".
- **Mailboxes:** Inbox `g i`, Drafts `g t` (with the Outbox under it while
  it holds anything), Sent `g s`, Archive `g r`, Snoozed `g z`, Flagged
  `g *`, and Filtered `g f`. Each row has an icon, its name, its count and
  its key.
- **Folders.**
- **Labels,** each with its colour dot.
- **Footer:** "↵ open · Esc close · same as in:Receipts in the command bar".

## Pickers (11–14)

Each picker is a 380 px popover anchored to the focused row.

- **Title row.** On the left, "Snooze until", "Remind me if no one replies
  by", "Labels" or "Move to folder". On the right, the target: "Ada Moreno ·
  Atlas Q3 budget", or "3 conversations".
- **Snooze and Remind:**
  - four preset rows, each with a bold name, the time on the right, and its
    number key;
  - the field "Or type a date: “tue 9am”", with "Tab to type" under it;
  - a footnote. Snooze: "The message leaves the inbox and comes back at the
    top at that time. Snoozed mail is under g z." Remind: "If anyone replies
    first, the reminder is cancelled. If not, the thread comes back to the
    top of the inbox marked “No reply since Sat 26 Sep”".
- **Label:**
  - the field "Filter, or type a new label";
  - rows with the label's colour dot, its name, and its count or "✓
    applied";
  - the footnote "Space toggles a label · Enter closes · typing a name that
    doesn't exist offers “Create label”.".
- **Move:**
  - the field "Filter folders";
  - "Recent", with rows `1` and `2`, then "All folders";
  - the footnote "Enter moves the message and it leaves the inbox · mod+z
    undoes."

## The undo toast (15)

An `AdwToast` in a dark pill, centred at the bottom: "Archived 3 messages",
with Undo. It lasts 8 s (`postio_widgets::widgets::toast::TOAST_TIMEOUT`).
For an RSVP, the toast reads "Accepted" or "Declined", and it lasts as long
as the send's window.

## States (16–19)

- **Empty inbox (16).**
  - A tray icon, centred, and "Inbox is empty" in bold.
  - "Next digest: Weekly · Newsletters, Saturday 16:00", only when there are
    digests.
  - Shortcuts, only for what exists: "`g f` 186 filtered today · `g r`
    archive · `c` compose".
  - While the first sync has not finished its first pass, it says it is
    syncing and how far, not that the inbox is empty.
- **Banners,** one at a time, choosing the one that asks something of the
  user first:

| State | Banner | Action | Sync label |
|---|---|---|---|
| Sign-in error (19) | "Can't sign in to \<server\>", then "The server rejected the password for \<address\>. Mail on this computer is still available." The error colour at low opacity | Update password… | "Sync failed" |
| An account failing for another reason | The account and the reason, in the sync's own words when it gave them | Retry now | "Sync failed" |
| Offline (18) | "You're offline", then "Everything you do is saved here and syncs when you're back." | Retry now | "Offline" |
| First sync (17) | "First sync", then "12,408 of 18,204 messages, newest first. You can read and search what's here." A progress bar | none | "Syncing 12,408 of 18,204" |

- **Store in use.** "Postio is already open in another window. Close it to
  open Postio here.", with "Try again" (FR-003).
- **A store no migration reaches.** What is lost and kept, with "Start a
  fresh store". Every page before the inbox has the window's close button.

## The key map (20)

An `AdwDialog`, 1100 px wide.

- **Title:** "Keys", with "Single keys act on the focused row, or on the
  selection if there is one. On macOS, Ctrl becomes ⌘." On the right, `?`
  or `Escape` close.
- **Groups,** in four columns: Move and select, Open, Act (row or selection),
  Invites, Go and find, In search, Digests and filtering, and Obsidian. One
  row per command, with every binding.
- **Footer:** "Rebind anything in ~/.config/postio/config.toml under [keys]"
  (C3). On the right, "The mouse works everywhere: every key has a visible
  button."

## Filtered (21)

A full view, not a dialog.

- **Header bar:**
  - "‹ Inbox" on the left;
  - the title "Filtered", with "Archived automatically · newest first" under
    it;
  - "Sweep the inbox… `F`" on the right.
- **Tabs:** All 186 · Spam 12 · Promotions 41 · Notifications 88 · Receipts
  19 · Shipping 14 · Social 12, reached with `1`–`7`. On the right, "Nothing
  here is deleted automatically · nothing here ever reached the inbox" (C4).
- **Day heading:** "Today · 186".
- **Rows,** one line each: the sender; the subject and first line; a reason
  pill ("notification · Forge"); the time. The focused row shows "Restore,
  never filter this sender `R`".
- **Footer:** "`R` restore + never filter sender · `1–7` reason tabs · `↵`
  open · `g i` inbox".

## The digest rules list (`g d`)

A full view in 21's frame (C15). Each row holds the rule's name, its match,
its cadence and time, its next delivery, and what it holds now. `Return`
edits the rule in 24's dialog, and `Delete` removes it, releasing what it
held.

## The digest window (22, 23)

An `AdwDialog`, 980 px wide.

- **Header.**
  - Left: the X, or "Summary" with `Escape` when an email from a reference
    is open (23).
  - Centre: "⧉ Weekly · Newsletters", with "14 messages from 6 senders ·
    came due today 16:00" under it.
  - Right: "Archive all 14" `A`, raised.
- **Sub-row.** "Summary | 14 messages `Tab`" when there is a summary. On the
  right, "Weekly, Saturday 16:00 · Edit rule and cadence `d`".
- **The plain list:** one-line rows, as in 08.
- **The summary (22):** statements grouped by topic, each ending in a
  numbered reference; the focused reference shows its message under the
  statements. The footer reads "Written on this computer by the local model
  from these 14 messages only. Every statement links to the email it came
  from."
- **An email from a reference (23):** a banner saying where it was cited,
  the passage highlighted, and `j`/`k` through the digest's sources.

## "Digest this sender" (24)

An `AdwDialog`, 620 px wide.

- **Header.** Cancel `Escape`, "Digest this sender", and Create `Return`,
  raised.
- **From.** The address, read-only, in mono.
- **Deliver.** [Weekly ▾] on [Sunday ▾] at [09:00].
- **Preview.** "Would have caught 9 messages in the last 90 days", then four
  rows (subject and date), then "and 5 more".
- **Note.** "Mail from this sender with an invite, question or to-do still
  comes straight to the inbox."
- **"Match a list or a search instead…"** turns the rule into a list or
  query rule, previewed the same way. "Digest mail like this" appears only
  with a model configured.

## Obsidian capture (25)

An `AdwDialog`, 660 px wide, over the list: Task and Note as a linked pair,
the text (the action sentence, or the subject with `alt+s`), the due date
with its quick picks, the suggested project with its reason (`mod+p`
changes it), and the exact markdown line as a preview. Add task
(`mod+Return`) appends it.

## Settings

An `AdwDialog` at the open message's size, opened by `mod+comma` or the main
menu (screens.md, "Settings"): a find field where the open message has its
steps, "Settings" centred, the X; the section list down the left (214 px)
and one pane; the foot strip with the file's state and "Open in $EDITOR"
`mod+e`.
