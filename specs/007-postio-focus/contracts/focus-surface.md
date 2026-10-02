# Contract: Focus's surfaces

Every screen from 01 to 20 is built against its PNG
(`Design/postio-focus-design/screens/`). Exact sizes and copy come from the
mockups (`source/*.dc.html`). This contract is what a build is checked
against, before the PNG comparison records what still differs (spec FR-095;
[quickstart.md](../quickstart.md)). The measurements are for a 1440×900
window, the size the screens were drawn at.

**Colour.** Colour comes from libadwaita's named colours (research R11). The
accent is reserved for four things:

- action markers;
- the focus ring;
- the has-action toggle when it is on;
- the unread dot's place on a row that has a marker.

"Raised" buttons (Send, Create, Archive all) are plain raised buttons with bold
labels, never the suggested-action style.

**Keycaps** come from the registry through the shared `keyhint` widgets,
never from literals (`check-key-hints-are-derived.py`).

## The window (01, 02)

| Part | Height | Contents, left to right |
|---|---|---|
| Top bar | 46 px | Compose (pencil icon); the command-bar field (480 px, centred): "Search mail, go to a folder, or run a command", keycaps `/` and `Ctrl K`; the sync label ("Synced 16:09", with its icon); the main menu (☰); close (×) |
| Header strip | 36 px | "Inbox ▾", keycap `g o`; "312 · 41 unread"; a divider; the toggle "Has action · 7" with keycap `!`; on the right, "186 filtered today" `g f` and "4 digest rules" `g d` |
| Banner slot | 42 px when shown | One `AdwBanner` (17–19) |
| Day heading | 32 px | "Today · Saturday 26 September"; with the has-action filter on, "Has action · 7" |
| List | the rest | Rows (below) |
| Bulk bar | 44 px when anything is selected | "3 selected"; Archive `a`, Snooze `s`, Mark read `r`, Digest these… `d`, Task `t` (milestone 3), Label `l`, Move `m`; on the right, "`x` toggle · `⇧J ⇧K` extend · `Esc` clear" |

- **The header counts only appear once their features exist:** the filtered
  count once filtering does, the rule count once digests do (spec FR-018).
- **With the has-action filter on (03),** the toggle is drawn in the accent,
  and the strip adds "Showing 7 of 312 · ! again to show all".
- **The main menu** holds Settings (`mod+comma`), Keyboard shortcuts (`?`),
  About, and Quit (`mod+q`).

## Rows

**Heights.** Two, fixed by the row's kind, never by its content:

- **40 px:** a conversation with no marker, or a digest delivery;
- **72 px:** a conversation with a marker, or a fired reminder.

**Columns:**

| Column | Contents |
|---|---|
| Gutter (24 px) | The accent dot when the row has a marker; a checked box when the row is selected; the stack icon on a digest |
| Sender (x = 56, about 234 px) | The display name, bold when unread |
| Subject | Bold when unread; then up to two label pills (a colour dot and the name); then the first line, dimmed, truncated at the end |
| Trailing | The attachment icon; the count badge, when the conversation holds more than one message; the time, bold when unread |

**The second line** on a 72 px row:

- a kind chip, outlined in the accent: "Invite", "Question", "To-do" or
  "No reply";
- the date in the accent, bold: "Tue 29 Sep · 10:00–10:45", "Wed 30 Sep", or
  "since Sat 26 Sep";
- for a question or to-do, the quoted sentence in the accent, italic, inside
  quotation marks;
- the answering actions on the right, each with its keycap: Accept `y` and
  Decline `Y`; or Reply `e`; or Task `t` (milestone 3) and Snooze `s`.

**States:**

- **Focus:** the focus ring is a 2 px accent border around the row.
- **Selection:** a selected row has a neutral background and a checked box.
- **Neither changes the row's height.**

**A digest row:**

- the sender column reads "Weekly · digest";
- the subject reads "Newsletters · 14 messages";
- the first line is the digest's senders in milestone 1, and the summary's
  opening in milestone 2 (FR-124);
- the count badge shows 14, and the time 16:00.

**A reminder row** has no screen of its own. It uses a marked row's anatomy,
and the conversation's latest message is its first line. Its second line
holds "No reply", "since <date>" and Reply `e`.

**What a screen reader hears:** the sender, subject, first line, "unread",
the marker's kind and date, and then the shortcut. A keycap is exposed as its
control's shortcut, never read as text (FR-096).

## The open-email dialog (04)

The dialog is an `AdwDialog`, 980×820 px.

- **Header (50 px).**
  - Left: Close `Esc`.
  - Centre: the title (the subject) and a subtitle, "Message 5 of 312 ·
    thread of 6".
  - Right: ∧ `k` and ∨ `j`.
- **Toolbar (42 px).** Reply `e`, Reply all `E`, Forward `f`, Archive `a`,
  Snooze `s`, Remind `h`, Label `l`, Move `m`. Task `t` and Note `n` join them
  in milestone 3.
- **Column (860 px).** In order:
  1. a chip, "Latest of 6 in this thread · `[` earlier message";
  2. the subject, as a heading;
  3. its label pills, and "+ Label `l`";
  4. the header card: From (name and `<address>`, the address in mono), To,
     Cc, and the date on the right;
  5. the marker card, outlined in the accent, with the kind chip, date,
     quote and actions;
  6. the body;
  7. attachment cards: a file icon, the name in mono, the size;
  8. the fold line: "› 31 quoted lines from v2 folded · v shows the raw
     source".
- **Scrolling (research R2).** The header and marker cards stay in place, and
  the body scrolls beneath them. The comparison records this.

## Compose (05, 06)

The composer is an `AdwDialog` at the message dialog's size
(`postio_ui::focus_dialog`), and can be detached. Its layout is the message
dialog's with the composer's verbs (T221; screens.md, "The composer").

- **Header (52 px).**
  - Left: Detach, an icon button.
  - Centre: "New message", "Reply", "Reply to all" or "Forward", with
    "Plain text · 58 words · Draft saved locally 16:12" under it in mono.
  - Right: the shared close X (T192).
- **Action row (44 px, hairlines above and below).** Send `ctrl+↵`, the one
  primary (raised, bold); Send later `ctrl+⇧+↵` ▾; Attach `ctrl+⇧+a`;
  "Remind · Tue 29 Sep" `ctrl+h` (its tooltip "Remind if no reply"); Task
  after sending `ctrl+t` (milestone 3).
- **One column,** `min(480, dialog − 96)`, centred, for everything below.
- **Fields,** 40 px each, one label column:
  - **To:** recipient chips (name, address in mono, ×). On the right,
    "+ Cc".
  - **Cc:** when replying to all.
  - **From:** the identity's name, with the address and ▾.
  - **Subject.**
  - **Labels:** chips, with "from the thread" on the right when replying.
- **Formatting toolbar:** icon buttons, 26 px.
- **Body:** on the dialog's surface, in its ink, at the column's edges.
- **Attachments:** cards, as in 04.
- **Suggestions.** A 440 px popover. Each row shows the name (bold), the
  address (mono), and "wrote 42 times", "wrote twice" or "list". Its footer
  reads "↑↓ choose · ↵ or Tab add · from your address book and mail on this
  computer".

Differences recorded against the PNGs:

- the Markdown toggle is gone (spec C7);
- completion opens at four characters (spec C23);
- Cc and Bcc share the composer's one `copy_fields` command until
  `/ux-architect` splits them.

## The command bar (07, 08, 09)

The bar opens in place (spec C24): its input is drawn over the top bar's own
field -- same width, same place -- and the results hang below it in a panel
860 px wide. `/` opens it for mail search; `Ctrl K` opens it in command mode,
`>` already typed.

- **Saved row.** "Saved" pills, each with its count and `Alt n`. On the
  right, "Ctrl S saves the current query".
- **Input row.** The search icon, the chips, the caret, and `Esc`.
- **Echo line (07).** "You typed "the invoice Ada sent last month" · editing
  after:". On the right, "`Tab` next chip · `Ctrl ⌫` back to plain words".
- **Results (07, 08).** A section heading ("Conversations · 3 matches", or
  "Receipts · folder · 214 conversations · newest first"). Each row holds the
  sender, the subject in bold, the first line dimmed, where it lives in mono
  (`in:Inbox`), and the date.
- **Commands (09).** Three sections:
  - **Commands:** the title with the match in bold, underlined; a dimmed
    detail ("the focused message · Re: Atlas Q3 budget"); the key on the
    right.
  - **Go to:** for example "in:Archive · 18,204 conversations `g r`".
  - **Search:** "Search mail for "arch"", with "subject, body, attachments"
    and `⇧↵`.
- **Footer.** "↑↓ move · ↵ open / run · Alt 1–4 saved searches · Ctrl S save
  query". On the right, "Local index". In the command view, the footer adds
  "> commands only".

## The folders popover (10)

A 400 px popover, anchored to "Inbox ▾".

- **Field:** "Go to folder or label".
- **Mailboxes:** Inbox `g i`, Drafts `g t`, Sent `g s`, Snoozed `g z`,
  Archive `g r`, Filtered `g f`. Each row has an icon, its name, its count
  and its key.
- **Folders.**
- **Labels,** each with its colour dot.
- **Footer:** "↵ open · Esc close · same as in:Receipts in the command bar".

## Pickers (11–14)

Each picker is a 380 px popover anchored to the focused row.

- **Title row.** On the left, "Snooze until", "Remind me if no one replies
  by", "Labels" or "Move to folder". On the right, the target: "Ada Moreno ·
  Atlas Q3 budget".
- **Snooze and Remind:**
  - four preset rows, each with a bold name, the time on the right, and its
    number key;
  - the field "Or type a date: "tue 9am"", with "Tab to type" under it;
  - a footnote. Snooze: "The message leaves the inbox and comes back at the
    top at that time. Snoozed mail is under g z." Remind: "If anyone replies
    first, the reminder is cancelled. If not, the thread comes back to the
    top of the inbox marked "No reply since Sat 26 Sep"".
- **Label:**
  - the field "Filter, or type a new label";
  - rows with the label's colour dot, its name, and its count or "✓
    applied";
  - the footnote "Space toggles a label · Enter closes · typing a name that
    doesn't exist offers "Create label".".
- **Move:**
  - the field "Filter folders";
  - "Recent", with rows `1` and `2`, then "All folders";
  - the footnote "Enter moves the message and it leaves the inbox · Ctrl Z
    undoes."

## The undo toast (15)

An `AdwToast` in a dark pill, centred at the bottom: "Archived 3 messages",
with Undo and `Ctrl Z`. It lasts 8 s (`crates/postio-gtk/src/widgets/toast.rs:35`).

For an RSVP, the toast reads "Accepted" or "Declined", and it lasts as long as
the send's window.

## States (16–19)

- **Empty inbox (16).**
  - A tray icon, centred, and "Inbox is empty" in bold.
  - "Next digest: Weekly · Newsletters, Saturday 16:00", only when there are
    digests.
  - Shortcuts, only for what exists: "`g f` 186 filtered today · `g r`
    archive · `c` compose".
- **Banners,** one at a time, choosing the one that asks something of the
  user first:

| State | Banner | Action | Sync label |
|---|---|---|---|
| Sign-in error (19) | "Can't sign in to \<server\>", then "The server rejected the password for \<address\>. Mail on this computer is still available." The error colour at low opacity | Update password… | "Sync failed" |
| Offline (18) | "You're offline", then "Everything you do is saved here and syncs when you're back." | Retry now | "Offline" |
| First sync (17) | "First sync", then "12,408 of 18,204 messages, newest first. You can read and search what's here." A progress bar | none | "Syncing 12,408 of 18,204" |

- **Store in use.** "Postio is already open in another window. Close it to
  open Postio here.", with "Try again" (spec FR-003).

## The key map (20)

An `AdwDialog`, 1100×760 px.

- **Title:** "Keys", with "Single keys act on the focused row, or on the
  selection if there is one. On macOS, Ctrl becomes ⌘." On the right, "`?`
  or `Esc` close".
- **Groups,** in four columns: Move and select, Open, Act (row or selection),
  Invites, Go and find, In search, Digests and filtering, and Obsidian
  (milestone 3).
- **Footer:** "Rebind anything in ~/.config/postio/config.toml under [keys]"
  (spec C3). On the right, "The mouse works everywhere: every key has a
  visible button."

## Filtered (21)

A full view, not a dialog.

- **Header bar:**
  - "‹ Inbox" on the left;
  - the title "Filtered", with "Archived automatically · newest first" under
    it;
  - search and close on the right.
- **Tabs:** All 186 · Spam 12 · Promotions 41 · Notifications 88 · Receipts
  19 · Shipping 14 · Social 12, reached with `1`–`7`. On the right, "Nothing
  here is deleted automatically · nothing here ever reached the inbox"
  (spec C4).
- **Day heading:** "Today · 186".
- **Rows,** one line each: the sender; the subject and first line; a reason
  pill ("notification · Forge"); the time. The focused row shows "Restore,
  never filter this sender `R`".
- **Footer:** "`R` restore + never filter sender · `1–7` reason tabs · `↵`
  open · `g i` inbox".

## The digest window (screen 22's frame)

An `AdwDialog`, 980×820 px.

- **Header.**
  - Left: Close `Esc`.
  - Centre: "⧉ Weekly · Newsletters", with "14 messages from 6 senders ·
    came due today 16:00" under it.
  - Right: "Archive all 14" `⇧A`, raised.
- **Sub-row.** In milestone 2 it gains "Summary | 14 messages `Tab`". On the
  right, "Weekly, Saturday 16:00 · Edit rule and cadence `d`".
- **Milestone 1** shows the message list: one-line rows, as in 08.
- **Milestone 2** opens on the summary (22). References are numbered chips.
  The focused reference shows its message under the paragraph. The footer
  reads "Written on this computer by the local model from these 14 messages
  only. Every statement links to the email it came from."

## "Digest this sender" (24)

An `AdwDialog`, 620 px wide.

- **Header.** Cancel `Esc`, "Digest this sender", and Create `↵`, raised.
- **From.** The address, read-only, in mono.
- **Deliver.** [Weekly ▾] on [Sunday ▾] at [09:00].
- **Preview.** "Would have caught 9 messages in the last 90 days", then four
  rows (subject and date), then "and 5 more".
- **Note.** "Mail from this sender with an invite, question or to-do still
  comes straight to the inbox."
- **"Match a list or a search instead…"** arrives in milestone 2, and is
  absent in milestone 1 (the comparison records it).

## Surfaces with no screen (spec C15)

`/ux-architect` designs these before their tasks, in the frames named:

- **The digest rules list (`g d`),** in 21's full-view frame. Each row holds
  the rule's name, its match, its cadence and time, its next delivery, and
  "holds N". `↵` edits the rule in 24's dialog, and `Delete` removes it.
- **The digest window's message list,** in 22's frame, with rows as in 08.
