# Feature Specification: Contacts for Postio Focus

**Feature Branch**: `feature/focus-contacts` (cut from `main` once spec 005's
backend port, `feature/contacts-mac`, has landed its person model)

**Created**: 2026-10-19; **Revised**: 2026-10-19, to the designer's handoff

**Status**: Draft, revised to the handoff and the maintainer's decisions of
2026-10-19; awaiting the maintainer's OK of `plan.md` and answers to the
questions at the end

**Input**: the designer's handoff `Design/focus-macos-contacts/` (the
maintainer's local, untracked folder: `SPEC.md`, `PROMPT.md`, `README.md`,
`screens/01` to `08`, `source/*.dc.html`), which builds on the earlier macOS
handoff `Design/focus-macos-design/`. **`Design/focus-macos-contacts/SPEC.md`
is the source of truth for the design.** Nothing from `Design/` is copied into
the repository (CLAUDE.md); the handoff's `PROMPT.md` suggests copying the
folder in, and that is declined. Screens are cited by filename, as
`specs/007-postio-focus/screens.md` does, and the handoff's example names are
not reproduced: this spec's examples are invented and use reserved domains.

## What changed from the first draft of this spec

The first draft (merged as #1924) was written with no design reference. The
handoff supersedes it wherever they differ. The table is the record, so a
decision is not re-argued from the old text.

| First draft | Now | Why |
|---|---|---|
| K1: `g p` opens Contacts | **`g c`**, on the Mac. `next_scope` is not offered on Apple, so nothing collides there; it moves to `g n` when Linux adopts (M2, Q9) | Maintainer, 2026-10-19 |
| K2: Contacts is a mode of the main window | Kept | `01-contacts-list-and-card.png` |
| K3: a person opens in a secondary window | **Superseded.** The card is the right part of the Contacts mode, beside a 440 pt list | `01-contacts-list-and-card.png` |
| K4: five tabs: Written to, Everyone, Groups, Duplicates, Deleted | **Superseded.** Kind tabs: All, People, Companies, Lists and newsletters, Automated. Duplicates are a "Worth a look" line and a Merge sheet; no Deleted tab, no Groups tab (Q5, Q6) | `01`, `07` |
| K5: `/` opens the one box in `@` mode | **Superseded** by a "Filter contacts" field in the strip, focused by `/` (Q4) | `01` |
| K6: per-field editing, no Save | **Superseded.** Edit is a form: Save ⌘S, Cancel Esc | Maintainer; `06-edit-contact.png` |
| K7: join through a picker at the row | **Superseded.** A Merge sheet with the evidence and a preview of the result | `07-merge-duplicates.png` |
| K8: Groups tab and group picker | Groups are chips on the card and in Edit; `to:Quarry crew` and `from:@Finance` use them (Q5) | `06` |
| K9: the word is **Join**; "Preferred" | **Merge**; **primary** for the address replies go to | Maintainer; handoff |
| K10: neutral initials, no hue | **Superseded.** A palette hashed from the contact id, a shape per kind, a photo when there is one; the accent stays for focus and action markers only | `01`, `03` |
| K11: cursor and selection never conflated | Kept | Constitution II |
| K12: no confirmations | Kept, with one question: deleting a synced contact asks whether to delete it from the other store too | Handoff SPEC §2 |
| Contacts is Postio's address book only; the Mac's Contacts is read only for completion | **Superseded.** Apple Contacts and CardDAV are optional sync targets, built last, writes off until reviewed | Maintainer; handoff §2 |
| Q3: import is one undo step | **Resolved the other way:** import is not undoable for now (005's rule) | Maintainer |

## Context

Spec 005 (`specs/005-contacts`, on `origin/feature/contacts`) specifies the
address book: a person who owns many addresses, groups, possible duplicates,
vCard import and export, `with:` search, and the person's name shown in the
mail lists. Another session is porting its backend to `main` on
`feature/contacts-mac` (not pushed; on 2026-10-19 it holds the 005 specs, a
`postio-vcard` leaf crate and an `AddressId` type, and no schema). This spec is
the **surface** and the engine additions the handoff needs on top of 005:
`kind`, `saved`, per-contact settings, stats, review queue, sync. It cites 005
by FR number and does not restate its data rules. `plan.md` lists what it
needs from the port.

| What | Where it is decided |
|---|---|
| A person owns addresses; merge (005's "join"), detach, primary (005's "preferred"); sightings; groups; deletion and restore; vCard fidelity; name substitution in mail lists | Spec 005, unchanged. Cited as `005 FR-nnn` |
| `kind`, `saved`, per-contact settings, stats, signature details, review queue, sync links | **This spec and `plan.md`** |
| Where Contacts is, what it looks like, which key does what | **This spec**, against the handoff |
| Behaviour (which row a key reaches, the Esc ladder, cursor after a merge) | `postio-focus`, per ADR 0045 |

### What this spec inherits

- **Spec 005** for the data model, the store, the commands' payloads and undo
  kinds. 005's command **ids** stay (ids are a file format, constitution II);
  the Command table lists each id and what changed.
- **Specs 007, 009 and 010**: one list; secondary surfaces over it; `Esc`
  closes back to the same row; behaviour in `postio-focus` and the Mac draws
  intents; pickers at the row, the toast pill, the `?` key map, the one box;
  one secondary window at a time on the Mac (009 M4); results as a mode of the
  main window with History (010 D17); Mac-only commands carried by
  `registry::offered_on` until Linux adopts (010 D25).
- **The general macOS rules** of the earlier handoff: semantic colours only,
  the accent only for focus and action markers, SF Pro and SF Mono, every
  action has one key shown as a keycap, all logic in the Rust engine. Where its
  `KEYS.md` disagrees with the registry (it predates #1834's adoption of
  Gmail's keys), **the registry wins**; "Keys" lists each case.
- **The constitution**: I local-first, II one command table, III one query
  language, IV test-first, V performance gated as counts, VI privacy, VII
  boundaries.

## Decisions

### Recorded 2026-10-19 (maintainer)

| # | Decision |
|---|---|
| M1 | **Follow the handoff.** Its `SPEC.md` is the source of truth; where it disagrees with the first draft of this spec, it wins. |
| M2 | **`g c` opens Contacts.** Next scope moves to another key: `g n` is proposed (free in the registry), applied when Linux adopts Contacts (Q9). |
| M3 | The word is **Merge**, not Join; `m` merges. |
| M4 | **Import is not undoable for now.** |
| M5 | **Finder integration** (a `.vcf` association, dragging a contact out) **comes later.** |
| M6 | **The duplicates count shows only inside Contacts.** |
| M7 | **Apple Contacts and CardDAV sync is in scope as the last step.** Writes stay off until the maintainer has reviewed the dry-run output; tests use a scratch container, never the real address book; Contacts permission is asked for only when sync is turned on (this changes 009 FR-051's prompt, Q3). |
| M8 | **Edit is the handoff's form**: Save ⌘S, Cancel Esc. It replaces per-field editing. |

**M6 against the handoff's "Worth a look" line: no conflict.** The line
(`01`, `03`, `06`, `07`) is drawn only under the Contacts strip. It summarises
the review queue (possible duplicates, bounced addresses, frequent
correspondents who are not saved), and the duplicates count is a part of that
line, so it appears only inside Contacts. One clarification, recorded here:
Review (`g w`) is reachable from every surface, but it *goes to* Contacts; it
puts no count anywhere else. There is no dot on Inbox ▾, in the places popover
or in the Dock. The new-sender popover (`05`) appears in the inbox but states
one fact about one sender, not a count.

### From the handoff (restated so the build can cite them)

| # | Decision |
|---|---|
| H1 | **Postio owns its contacts**, in the store beside mail. A store with no sync account is a normal, fully working state. |
| H2 | **Every sender and recipient gets a contact automatically** the first time they appear, so stats, the popover and per-contact settings work from day one. |
| H3 | **Saved and not saved.** Saved when the user adds the contact (⌘↩ in the new-sender popover, New contact, or automatically when "Save people I write to, automatically" is on). **Only saved contacts sync.** An unsaved contact has a full card. |
| H4 | **One contact, many addresses**, one **primary** (replies and new mail go to it). Mail from any address counts as that contact everywhere: list name, search, digest and filter rules, stats. |
| H5 | **Kind**: Person, Company, List, Newsletter, Automated; detected from headers, editable, the user's choice always wins. |
| H6 | **Groups**: named sets that work as recipients (`to:Quarry crew`) and as search filters (`from:@Finance`). |
| H7 | **Per-contact settings**, stored once on the contact: where mail arrives (Inbox, Digest, Filtered out), notify, always show HTML as sent, remind-if-no-reply by default, archive issues after N days (newsletters), private notes with a linked note path. |
| H8 | **Stats are local**: messages, from you, typical reply time, last message, 12 months; for newsletters issues, opened, frequency, last issue; and what is **open between you**. |
| H9 | **Merge** keeps every address and never changes mail; undoable with ⌘Z; a merged contact can be split from its card. |
| H10 | **Contact work happens where you meet senders**: `i` on any message opens a popover; the Contacts view is for reviewing and tidying. |

## The surface

Sizes are in points for a 1440×900 window and come from the handoff's
`SPEC.md`; `source/*.dc.html` is consulted for exact values only.

### D1. Entry and the ladder

```text
anywhere ── g c ──▶ Contacts (mode of the main window) ◀── g w (Review)
 any row ── i ──▶ popover ── ⌘I ──▶ Contacts on that card
Esc climbs one rung: popover, edit form, filter text, selection, Contacts back
to where it was opened from (History: the inbox with its cursor and scroll).
```

Also offered in the places popover (Inbox ▾), the command bar, and Go ▸
Contacts in the menu bar.

### D2. The Contacts view (`01-contacts-list-and-card.png`, `02-contacts-dark.png`)

- **Toolbar** as the inbox's, with the sync status ("Synced with Contacts
  16:09") where the inbox shows mail sync.
- **Strip, 46**: "Contacts ▾ `g c`" and the total; kind tabs **All · People
  · Companies · Lists and newsletters · Automated**, each with a count; on the
  right the filter field (`/`) and **New contact ⌘⇧N**.
- **Worth a look, 36**: "3 contacts may be the same person · 2 addresses
  bounced recently · 5 people you write to often aren't saved" and **Review
  `g w`**. Hidden when there is nothing to review.
- **List, 440 wide**, windowed over the paged store, alphabetical under
  26-tall letter headers. Rows 52 tall: avatar 32; name (semibold 13.5) with
  an optional tag pill (`notify`, `digest weekly`, `filtered`, `always
  original`); the primary address in SF Mono 11.5 with "+2" for the rest; on
  the right the last message date over a count line ("431 messages", "list ·
  112 messages", "newsletter · 48 issues", "company · 3 people · 57
  messages"). The cursor row has the accent ring, nothing else.
- **Avatars**: a circle with initials for people; a rounded square (radius 8
  in the list, 14 on the card) with initials for companies, ≡ for lists, ✉ for
  newsletters, ⚙ for automated. The colour comes from a fixed palette hashed
  from the contact id; a photo replaces initials.
- **Card**, padding 26/36. **Header**: avatar 64, name at 24 bold, and a role
  line "Kind · title, company · saved, synced with Contacts (iCloud) · groups:
  …". Buttons by kind: person or company **Write `c`** (primary), **All mail
  `↩`**, **Edit `⌘E`**; list, newsletter or automated **All issues/messages
  `↩`** (primary), **Unsubscribe `U`**, **Edit `⌘E`**.
  - *Left column*: **Addresses** (★ marks the primary; each row has a note,
    "old work · last used March 2025"; a bounced address is flagged in orange
    with the date; below, "Also seen as …" display names). **How mail from X
    arrives**: a segmented control Inbox / Digest / Filtered out, then
    switches: notify; always show HTML as sent; remind if no reply by default
    (for newsletters: the digest line "in the weekly digest, Saturday 16:00,
    edit the rule with `d`", and archive after 30 days). **Notes**, with the
    linked note path ("Kept on this Mac").
  - *Right column*: **Your mail together** ("Its mail" for non-people): four
    stat tiles, a 12-month bar strip and its axis labels; **Open between you**
    ("Recent issues" for newsletters): rows with a kind on the left, the text,
    and a when or state on the right; **Recent files from X** (hidden for
    newsletters).
- **Footer**: key hints, and on the right "1,533 contacts · 640 saved ·
  synced with Contacts (iCloud) 16:09".
- **Dark** (`02`): the same; switches on use a light track with a dark knob.
- **A newsletter** (`03-newsletter-card.png`): Digest selected; the digest
  cadence line; always show HTML as sent; archive issues after N days; stats
  for issues, opened, frequency and last issue; Recent issues with state (in
  digest, opened, not opened).

### D3. The popover, known sender (`04-popover-known-sender.png`)

`i` on any inbox row, search result or open message, or a click on a sender
name, opens an `NSPopover` anchored to the name, 400 wide: avatar, name,
address, role; three stat tiles; up to two open items; the arrival segmented
control (a change applies at once and shows an undo toast); **All mail `↩`**
(primary), **Write `c`**, **Open card `⌘I`**.

### D4. The popover, new sender (`05-popover-new-sender.png`)

Opens above the row when there is no room below. It says "Not saved yet" and
how the sender reached the person (who is on copy, who mentioned them); **From
the signature**: title, company, phone, website, parsed on this Mac, "Added
only if you save"; the arrival control; **Add contact `⌘↩`**, **Not now
`Esc`**, **Ignore sender `⌘⌫`** (never suggest this sender again).

### D5. Edit (`06-edit-contact.png`)

`⌘E` on the card: an accent-tinted bar "Editing X · saved in Postio, then
synced to …" with **Cancel `Esc`** and **Save `⌘S`**; a form with right-aligned
labels in a 110 column and fields up to 560 wide: **Photo**; **Kind**
(segmented, with the note "Detected from her mail. Only people and companies
sync to Contacts."); **Name** (first, last); **Show as** (how the inbox
displays the contact whatever name their mail sends: "First Last", "First",
"Last, First"); **Work** (title, company); **Addresses** (a radio for the
primary, the address, a type menu, remove, "+ Add address"); **Seen in mail**
(details found in headers and signatures, each with Ignore or Add); **Groups**
(chips and "+ Add to group"). Footer: Tab next field, ⌘S save, Esc cancel. One
write on Save; one undo entry.

### D6. Merge (`07-merge-duplicates.png`)

A sheet from the toolbar over the dimmed Contacts view, 780 wide: "These 3
look like the same person", one card per candidate (the record whose name and
photo are kept has the accent ring and says "Keep this name and photo"; the
others say "Address kept"), **Why we think so** (the evidence: the same
signature, one address replacing another in the same threads, forwarding
headers), a preview of the result, and **Not the same `n`** (never suggest this
pair again), **Later `Esc`**, **Merge `↩`**. Candidates come from the review
queue, or from selecting contacts and pressing `m`. "Undo with ⌘Z. Merged
records can be split again from the card."

### D7. Settings › Contacts (`08-settings-sync.png`)

An explainer; **Sync with**: "Contacts on this Mac (iCloud)" and CardDAV
accounts, each two-way or read only, each row with matched count, items
waiting and last synced; "+ Sync with another account (CardDAV)"; **What
syncs**: People / People and companies (default) / Everything saved. Four
**Suggestions** switches: save people I write to automatically (off);
recognise lists, newsletters and automated senders (on); read signatures for
details (on); suggest merging duplicates (on).

## Model rules the handoff adds to 005

- **Kind** is detected (`plan.md`, "Kind detection") and editable. A user's
  choice survives every later detection.
- **Saved** is a state of a contact, not of its mail. Unsaved contacts are
  complete: stats, popover, card, settings. Saving makes a person or company
  eligible to sync; Lists, Newsletters and Automated stay in Postio unless the
  person picks "Everything saved".
- **Stats** are defined once. *Messages*: mail to or from any of the
  contact's addresses. *From you*: those the user sent. *Replies in*: the
  median of up to the last 25 gaps between a message the user sent and the
  contact's next message in the same conversation. *Last*: the newest message
  either way. *The bar strip*: messages per calendar month for the last 12
  months, this one included. Newsletters: *issues this year*; *you opened*
  (read); *frequency* (the median gap of the last eight issues, spelled
  "weekly on Saturdays", "monthly" or "irregular"); *last issue*.
- **Open between you**: undismissed action markers on the contact's mail
  (waiting on the user); sent messages with an unfired remind-if-no-reply
  (waiting on the contact); tasks captured to the vault from their mail where
  the store records the link (`plan.md`).
- **Recent files**: the three newest attachments among the contact's last 50
  messages.
- **Show as** is a display style (full, first, last-first) or custom text; the
  list, conversation and reader read it for every address of the contact (005
  FR-032); the reader's header details still show the name and address the
  mail carried.
- **Ignore sender** keeps the contact (stats and card still work) and stops the
  new-sender popover and the review queue suggesting it.

## Sync (the last slices; `contracts/contact-sync.md` has the shapes)

- **Targets**: Contacts on this Mac (`CNContactStore`, Swift) and CardDAV
  accounts (Rust). Postio is always the source of truth for kind, settings,
  notes, stats and Postio-only groups.
- **Scope** (What syncs): People, People and companies (default), Everything
  saved. Only *saved* contacts of those kinds are written outward.
- **Matching**: by any shared normalised email first, then by name plus phone.
  A match is kept as a stable link (the external identifier); an email that
  matches more than one contact is not auto-linked and goes to the review
  queue.
- **Field ownership**: name, photo, addresses, phone, organisation and job
  title sync. Everything else stays in Postio. If the external store has
  groups, Postio groups map to them for synced contacts only.
- **Conflicts**: both sides changed the same field since the last sync → a
  small list in Settings, **Keep mine / Keep theirs**. Different fields merge
  field by field, from a stored base.
- **Deletes**: deleted in Contacts → unsaved in Postio, card and stats kept.
  Deleted in Postio → asks whether to delete it from Contacts too.
- **Permissions**: Contacts access is requested **only when the person turns
  that sync on**, never at first launch.
- **Safety**: until the maintainer has reviewed the dry-run output, the engine
  must never delete or overwrite anything in an external store. Writes are off
  by default and the gate is a reviewed commit (`plan.md`, S12). Tests run
  against fake adapters and, by hand, a scratch container the maintainer names;
  the real address book is never used. The dry-run report is shown in Settings
  or written to a file the person asked for; **it is never a log line** (logs
  carry ids and counts only).

## Keys

All of Focus's bindings are Gmail's (PRODUCT.md §8) and the registry is the
truth. A new context **`Contacts`** carries the list and card; **`ContactEdit`**
the form; **`ContactPopover`** the two popovers; **`ContactSheet`** the merge
sheet. All are `offered_on` Apple only until Linux adopts (010 D25). Movement
and selection reuse the existing commands, which gain the contexts: `j`/`k`,
`g g`, `G`, `x`, `J`/`K`, `X`, `Esc`, `z`.

Checked on 2026-10-19 against `crates/postio-core/src/registry.rs`,
`docs/keybindings.md` (generated), the earlier handoff's `KEYS.md`, and the
four keys the maintainer named: `U`, `i`, `x`, `g a`. The registry's collision
test is platform-aware: two commands may share a key if no platform offers
both (`tests/core_suite/command_registry.rs:232`).

| Handoff key | Where | Command id | Binding here | Collision and resolution |
|---|---|---|---|---|
| `g c` | anywhere | `open_contacts` | `g c` | `next_scope` is `g c` (Sidebar, List, registry.rs:2097) but **not offered on Apple** (`offered_on`, registry.rs:2818), so the collision test passes. Linux keeps `g c` = next scope until it adopts, then `next_scope` moves to `g n` (M2, Q9) |
| `g a` | anywhere | `review_contacts` | **`g w`** ("worth a look") | `g a` is **Go to archive** (Gmail's all mail) on both platforms, registry.rs:1869; the handoff predates #1834 and `KEYS.md` has the archive on `g r`. `g w` is free and mnemonic. The Review button's keycap reads `g w` |
| `i` | List, search, message window | `contact_popover` | `i` | `i i` (Show remote images) and `i a` (Always show images) make `i` a chord prefix in List, Conversation and Reader, offered on both platforms (registry.rs:1725, 1736). Resolution (Q2): the images commands move to `alt+i` and `alt+I`, a change to two Linux bindings. Fallback: the popover on `@` |
| `⌘I` | popover | `contact_open_card` | `mod+i` | `ctrl+i` is Italic in Composer only: another context |
| `U` | newsletter or list card | `contact_unsubscribe` | `U` | `U` is an alternate of `toggle_read` (List, Conversation, Reader, Filtered), and Unsubscribe is **`M`** in the registry (the handoff's `KEYS.md` has `U`). A Contacts-only command bound `U` runs the same unsubscribe; `toggle_read` is not offered in Contacts, so no collision. `M` stays everywhere else |
| `x` | Contacts | `toggle_selection` | `x` | The command gains the `Contacts` context. No collision |
| `m` | Contacts | `contact_merge` | `m` | `m` is only `set_default_account` (Accounts context). Move is `v` in the registry (the earlier `KEYS.md` has it on `m`, stale). No collision |
| `c` | Contacts, popover | `contact_compose` | `c`, `mod+n` | `compose` is `c` in List, Conversation, Reader; a new Contacts command composes **to the contact** |
| `↩` | Contacts, popover | `contact_show_mail` | `Return` | `open_message` is `Return` in List and others; not in Contacts, where the card is already beside the list |
| `⌘E` | Contacts | `contact_edit` | `mod+e` | `edit_config` is `ctrl+e` (List, Conversation, Reader); not in Contacts |
| `⌘⇧N` | Contacts | `contact_new` | `mod+shift+n` | **`add_account` is `mod+shift+n` in every context** (`ContextSet::ANY`, registry.rs:1690-1695), so the test would fail. Resolution: narrow `add_account` to every context but `Contacts`; it stays in Settings › Accounts and the palette. Linux is unaffected: `Contacts` is not offered there |
| `⌘S`, `Esc` | Edit | `contact_save`, `back` | `mod+s`, `Escape` | `ctrl+s` is Save search (Search) and Save draft (Composer): other contexts |
| `⌘↩` | new-sender popover | `contact_add` | `mod+Return` | `ctrl+Return` is Send (Composer) and Add to the vault (Capture sheet): other contexts |
| `⌘⌫` | new-sender popover | `contact_ignore` | `mod+BackSpace` | `ctrl+BackSpace` is Back to words (Search): another context |
| `Esc`, `n`, `↩` | Merge sheet | `contact_merge_later`, `contact_not_same`, `contact_merge_confirm` | as drawn | `n` is Next in conversation (Conversation, Reader): another context |
| `/` | Contacts | `contacts_filter` | `/`, `mod+f` | `search` is `/` in List and others; not in Contacts. The filter is a field in the strip (Q4) |
| `d` | newsletter card | `digest_rule` | `d` | gains the `Contacts` context |
| `⌘Z` | anywhere | `undo` | `z`, `mod+z` | gains `Contacts` and the popovers; the toast names the change |
| `⌘H` | composer | `remind_if_no_reply` | as is | "when you write to X, turn on ⌘H" is copy; the default is the contact's setting |

Not drawn by the handoff and needing a decision (Q6): **Delete** (`#`,
`Delete`, as the registry's `delete`; 005's `d` is Digest rule), **Unsave**,
**Split**.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See who a sender is, where I meet them (Priority: P1)

On any message row, search result or open message the person presses `i`. A
popover anchored to the sender's name shows the avatar, name, address and
role, three tiles (messages, how fast they reply, last), up to two open items
and the arrival control. `↩` shows all their mail, `c` writes to them, `⌘I`
opens the card, `Esc` closes back to the same row.

**Why this priority**: This is the entry the design chose (H10), and it needs
no Contacts view.

**Independent Test**: Over the demo store, `j j i`: the popover opens on the
focused message's sender and `Esc` returns the keyboard to that row. `i` on a
sender with no contact yet creates one (H2) and still opens within budget.

**Acceptance Scenarios**:

1. **Given** a sender seen on three addresses merged into one contact, **When**
   `i` is pressed on mail from any of them, **Then** one card opens with the
   three counts summed.
2. **Given** the arrival control on Digest, **When** the person picks Inbox,
   **Then** the rule changes at once, a toast says "Mail from X arrives in
   Inbox · Undo ⌘Z", and `z` restores Digest with its cadence.
3. **Given** a sender that is a mailing list, **When** the popover opens,
   **Then** the avatar, role line and actions are the list's (All messages, no
   Write).
4. **Given** a click on a sender name, **Then** the same popover opens.

---

### User Story 2 - Review and tidy contacts (Priority: P1)

`g c` opens Contacts: every sender and recipient is there, alphabetical, with
kind tabs. Selecting a row shows its card beside the list; `j`/`k` move and the
card follows. `↩` shows all mail with them as an ordinary results view; `c`
writes to them. "Worth a look" says what to review; `g w` goes to it.

**Independent Test**: `g c`, `j j`: the card names the third contact; `↩` lands
on a results view whose query names every address of that contact; `Esc`
returns to Contacts on the same row; `Esc` again returns to the inbox row.

**Acceptance Scenarios**:

1. **Given** the demo store, **When** `g c` is pressed, **Then** All is
   selected, the tabs carry counts, the cursor is on the first row and its card
   is shown, within the interaction budget (SC-001).
2. **Given** the Lists and newsletters tab, **Then** it lists both kinds and
   the card's primary button reads All issues or All messages.
3. **Given** nothing to review, **Then** the Worth a look line is absent.
4. **Given** the filter field focused with `/` and "mir" typed, **Then** only
   contacts whose name, organisation or any address matches remain, within
   100 ms, and `Esc` clears the text, then leaves the field.
5. **Given** a narrow window, **Then** the list keeps its width and the card's
   two columns stack, the name first.
6. **Given** a contact whose stats are not built yet (catch-up running),
   **Then** the card shows what is known and the tiles show "—", never a
   spinner or a blank.

---

### User Story 3 - Settings on the contact (Priority: P2)

On a card the person sets how mail from this contact arrives (Inbox, Digest,
Filtered out) and switches notify, always-original HTML and remind-if-no-reply
by default. Each change is one store write and one undo entry and takes effect
for every address of the contact.

**Independent Test**: Set Digest on a two-address contact; mail from either
address is held for the digest; `z` restores the earlier arrival.

**Acceptance Scenarios**:

1. **Given** a store migrated from one whose digest rule held `from:` one
   address, **When** the card opens, **Then** it says Digest with that cadence
   and mail arrives exactly as before (SC-005).
2. **Given** remind-if-no-reply on, **When** the person writes to the contact,
   **Then** the composer opens with the reminder preset to the stated days.
3. **Given** Filtered out, **Then** new mail is filed in Filtered with the
   reason the classifier gives, and `R` restores a message and never re-files
   that contact (the existing rule).

---

### User Story 4 - Edit a contact (Priority: P2)

`⌘E` turns the card into the form of `06-edit-contact.png`. Tab moves between
fields, `⌘S` saves, `Esc` cancels. Saving is one write and one undo entry.
Editing a mail-derived contact keeps its history.

**Acceptance Scenarios**:

1. **Given** a changed name and primary address, **When** `⌘S` is pressed,
   **Then** the list row, the card and the inbox's sender name show the new
   name and replies go to the new primary.
2. **Given** edits made, **When** `Esc` is pressed, **Then** the footer says
   "Esc again to discard" and a second `Esc` discards (Q10).
3. **Given** a kind chosen by the person, **When** later mail looks like a
   newsletter, **Then** the kind does not change.
4. **Given** a display name under Seen in mail, **When** Ignore is pressed,
   **Then** it is not offered again for this contact.
5. **Given** a write fails, **Then** the form stays open with its values and a
   one-line "Couldn't save. Nothing changed. <reason>".

---

### User Story 5 - Save a new sender from their first mail (Priority: P2)

On the first mail from someone, `i` shows "Not saved yet", the signature
details found on this Mac, the arrival control and **Add contact `⌘↩`**. The
details are added only if the person saves.

**Acceptance Scenarios**:

1. **Given** a signature with a title, company and phone, **When** the popover
   opens, **Then** they are listed under "From the signature" and none is
   stored until `⌘↩`.
2. **Given** Ignore sender `⌘⌫`, **Then** the popover and the review queue
   never suggest this sender again and the card still works.
3. **Given** "Read signatures for details" off, **Then** the section is absent.
4. **Given** a message near the window's foot, **Then** the popover opens above
   the row.

---

### User Story 6 - Merge the same person's addresses (Priority: P1)

The person sees one human three times. They press `g w`, or select two and
press `m`; the sheet shows the candidates, why Postio thinks so, and the
result. `↩` merges; `n` says they are not the same; `Esc` is later. One contact
remains with every address, all history, every group and note; `z` undoes it;
the card can split an address off again.

**Independent Test**: With three address contacts that share a signature, the
sheet lists the three and the evidence; `↩` leaves one row; `z` restores three.

**Acceptance Scenarios**:

1. **Given** candidates with different names, **Then** the one with the most
   mail is the default kept record and the person can pick another (the ring
   moves).
2. **Given** contacts with different arrival settings, **When** they merge,
   **Then** the kept record's settings win and the preview names what changes
   (Q8).
3. **Given** `n`, **Then** the pair never returns; there is no undo and no
   question, since it hides a hint, not data.
4. **Given** "Suggest merging duplicates" off, **Then** no candidate is
   proposed, and `m` on a selection still works.
5. **Given** new mail from any merged address, **Then** it counts toward the
   contact.
6. **Given** a merge, **Then** no message, label or digest hold changes.

---

### User Story 7 - Contacts everywhere else (Priority: P3)

Recipient completion shows a contact once, with its addresses, the primary
first. A name the person set reads in the list, conversation and reader for
every address. `to:Quarry crew` expands to a group's primary addresses; `from:`
and All mail find mail from every address (005 SC-003).

**Acceptance Scenarios**:

1. **Given** a merged contact, **When** "mir" is typed in To, **Then** one
   entry shows with its addresses.
2. **Given** All mail on a contact, **Then** the results view's visible query
   names every address and finds the messages of all.
3. **Given** `from:@Finance`, **Then** mail from every member of the group.

---

### User Story 8 - Sync with Contacts or CardDAV (Priority: P3, last)

In Settings › Contacts the person turns on "Contacts on this Mac". macOS asks
for access then, not before. The first run is **read only** and shows the
dry-run report: matches, what would be created, what would change. Writes stay
off until the maintainer has reviewed the output (the gate is code, S12).

**Acceptance Scenarios**:

1. **Given** sync is off, **Then** no Contacts permission prompt has ever been
   shown by Postio.
2. **Given** a saved person whose address matches a record, **Then** a link is
   made, their title and phone fill in, and nothing is written back in read
   only.
3. **Given** both sides changed a phone, **Then** a conflict row offers Keep
   mine / Keep theirs and nothing is overwritten meanwhile.
4. **Given** a record deleted in Contacts, **Then** the contact is unsaved and
   keeps its card and stats.
5. **Given** a synced contact deleted in Postio, **Then** a question asks
   whether to delete it from Contacts too.
6. **Given** Lists and newsletters, **Then** they never sync under "People and
   companies".

---

### User Story 9 - Import and export vCard (Priority: P4)

Kept from the first draft because 005's backend has it and the handoff is
silent: File ▸ Import vCard… and Export vCard… (system panels, `.vcf`), a
summary sheet, **not undoable** (M4). The Finder association and drag-out are
later (M5).

## Edge Cases

- **Empty, never blank.** No contacts yet: "Contacts appear as mail arrives."
  A filter with no match: "No one matches 'xyz'. `Esc` clears the filter." A
  kind tab with none: "No lists or newsletters yet." Each with its key.
- **Catch-up running.** After the upgrade the store builds contacts in the
  background. The list shows what has landed and `i` on a sender without a
  contact creates it on the spot. The footer says "Building contacts, n of m"
  until done. It never blocks startup (< 500 ms).
- **Offline, failing, first sync.** Everything but sync is local and works
  offline; sync shows its own state in Settings and the toolbar label.
- **A write fails** (store locked or full): the visible change reverts, a
  one-line refusal, the intent kept so the same key retries.
- **Forged display names.** A name header never turns an unowned address into
  a contact (005); the user's own addresses never appear as contacts.
- **Recipients.** Contacts are made from recipients too (H2); someone who never
  writes back is a contact with "from you" above zero and no reply time.
- **A very large store.** 20,000 contacts: the list opens, scrolls and filters
  within budget and is never loaded whole; the card opens in under 50 ms.
- **Privacy.** No network: no avatar, no favicon, no directory lookup. A photo
  is a local file or comes from sync. Signature reading runs on this Mac and
  keeps nothing until the person saves.
- **Reduced motion.** No transitions to reduce; the toast fades as elsewhere
  and honours the setting.
- **VoiceOver and full keyboard access.** A row reads "Mira Okafor, person,
  mira@example.org plus 2, 431 messages, last today, notify"; every verb is a
  menu item and a palette entry.
- **Rebinding.** Every key is overridable by command id.
- **Two windows.** Opening Contacts or a popover closes a message window
  (009 M4) and reports it closed.

## Requirements *(mandatory)*

### Functional Requirements

**Engine**

- **FR-001**: Every address that appears as sender or recipient on a message
  MUST belong to a contact (H2), created on first sight and, for any address the
  catch-up has not reached, on demand.
- **FR-002**: A contact MUST have a kind, a saved state and a primary address;
  kind MUST be detected from headers by one pure function and MUST NOT override
  a kind the user chose.
- **FR-003**: Per-contact settings MUST apply to every address of the contact
  and MUST behave exactly as the per-sender settings they replace (SC-005).
- **FR-004**: Stats MUST be answered from maintained columns and bound index
  ranges, never a scan of a contact's mail, and the card MUST open in under
  50 ms (SC-002).
- **FR-005**: Merge MUST keep every address, change no mail, and be undone
  exactly; Split MUST return an address with its history (005 FR-012, FR-014).
- **FR-006**: No engine change may stop the GTK app building or change what
  Linux shows without the maintainer being told before merge (`plan.md`,
  "Linux").

**Surface**

- **FR-010**: `g c` MUST open Contacts from every surface that offers the other
  `g` destinations; Contacts MUST be a mode of the main window held by the
  shared controller, behind `Policy.caps.contacts_view`, with History entries
  and return to the place entered from.
- **FR-011**: The list MUST be windowed over the paged store, alphabetical
  under letter headers, with the kind tabs and counts of D2; the card MUST
  follow the cursor.
- **FR-012**: `i` MUST open the contact popover for the focused message's
  sender from the list, search results and the message window; a click on a
  sender name MUST do the same.
- **FR-013**: The arrival control MUST apply at once and be undoable.
- **FR-014**: `⌘E` MUST open the form; Save MUST be one write and one undo
  entry; Cancel MUST discard.
- **FR-015**: The Merge sheet MUST show the evidence and the result before
  anything is merged, and the app MUST NOT merge without the person's action
  (005 FR-018).
- **FR-016**: Worth a look MUST show only inside Contacts and only when there
  is something to review; the duplicates count MUST appear nowhere else (M6).
- **FR-017**: Cursor and selection MUST be separate; every verb acts on the
  marks when there are any, otherwise on the cursor row.
- **FR-018**: Every command MUST be in the registry with a binding, a palette
  entry and an accessible action, offered on Apple only until Linux adopts;
  `docs/keybindings.md` MUST regenerate with no Linux row changed except where
  the Keys table says so.
- **FR-019**: Words MUST come from `postio-ui`, not Swift literals.

**Sync**

- **FR-030**: No Contacts permission request, no `CNContactStore` read and no
  sync network request MAY happen before the person turns a sync on (this
  supersedes 009 FR-051's per-keystroke prompt, Q3).
- **FR-031**: Writes to an external store MUST be impossible until the
  reviewed gate is lifted; the dry-run plan MUST be producible and MUST list
  every create, update and delete it would make.
- **FR-032**: A conflict MUST never be resolved silently; a delete in either
  store MUST follow the rules above.
- **FR-033**: The dry-run report MUST NOT be written to a log.

**Verification**

- **FR-040**: Every rule MUST be tested in the shared layer first, the FFI
  exports in `ffi_suite`, and the Swift view models and keyboard paths without
  a window.
- **FR-041**: Each interaction MUST be written as a storyboard in
  `storyboards/contacts/`, red on the base, before it is built; Mac landings
  are labelled `interactions-unreviewed` until Linux films them.
- **FR-042**: Each screen MUST be captured at 1440×900 (light and dark for 01
  and 02), compared with its PNG, and every difference listed with its reason
  in `specs/011-focus-contacts/screens.md`.
- **FR-043**: Fixtures use reserved domains and invented names.

## Success Criteria *(mandatory)*

- **SC-001**: `g c` shows the first rows and a card within 16 ms on 20,000
  contacts, and every key, tab change and filter keystroke answers within
  16 ms, gated as counts; the filter within 100 ms.
- **SC-002**: The card (stats, open items, files) opens in under 50 ms, gated
  as a statement and row count with a plan assertion naming the bound columns
  (ADR 0040).
- **SC-003**: Kind detection over the sample-header corpus agrees with the
  labelled expectation on at least 95% of senders and never marks a person's
  correspondence Automated (precision before recall).
- **SC-004**: A merge is one key from the sheet (`↩`) and `z` undoes it in one.
- **SC-005**: A store migrated from before this feature delivers mail exactly
  as before: the same messages in the same digests, the same filtered mail, the
  same pinned senders and the same HTML treatments, proven by a before-and-after
  fixture.
- **SC-006**: No Contacts permission prompt and no sync network request before
  sync is turned on.
- **SC-007**: In a sync dry run no external record is created, changed or
  deleted; with writes off the adapter's write methods are unreachable.
- **SC-008**: The generated `keybindings.md` passes
  `bindings_do_not_collide_within_a_context` with the new contexts, and only
  the rows the Keys table names change on Linux.

## Command table

005's ids stay. All are `offered_on` Apple until Linux adopts.

| id | Title | Binding | Contexts | Recovery | vs the first draft |
|---|---|---|---|---|---|
| `open_contacts` | Contacts | `g c` | the `g` surfaces | None | was `g p` |
| `review_contacts` | Review contacts | `g w` | the `g` surfaces | None | new |
| `contact_popover` | Contact popover | `i` | List, Search, Reader | None | new |
| `contact_open_card` | Open card | `mod+i` | ContactPopover | None | new |
| `contacts_filter` | Filter contacts | `/`, `mod+f` | Contacts | None | a field in the strip |
| `contacts_tab_all` … `_automated` | Kind tab | none (click, palette) | Contacts | None | replaces tabs `1` to `5` |
| `contact_show_mail` | All mail | `Return` | Contacts, ContactPopover | None | was `alt+Return` |
| `contact_compose` | Write | `c`, `mod+n` | Contacts, ContactPopover | None | same |
| `contact_edit` | Edit | `mod+e` | Contacts | Undo | was `e` |
| `contact_save` | Save | `mod+s` | ContactEdit | Undo | new |
| `contact_new` | New contact | `mod+shift+n` | Contacts | Undo | was `n` |
| `contact_merge` | Merge | `m` | Contacts | Undo | was Join |
| `contact_merge_confirm`, `contact_merge_later`, `contact_not_same` | Merge, Later, Not the same | `Return`, `Escape`, `n` | ContactSheet | Undo, None, None | new |
| `contact_split` | Split address | none (card menu, palette) | Contacts | Undo | new |
| `contact_unsubscribe` | Unsubscribe | `U` | Contacts | None | new |
| `contact_add` | Add contact | `mod+Return` | ContactPopover | Undo | new |
| `contact_ignore` | Ignore sender | `mod+BackSpace` | ContactPopover | Undo | new |
| `contact_arrival_inbox`, `_digest`, `_filtered` | Arrives in… | none (the control, palette) | Contacts, ContactPopover | Undo | new |
| `contact_delete` | Delete | `#`, `Delete` | Contacts | Undo | same as the draft; Q6 |
| `contact_group_add` | Add to group… | none (Edit, palette) | ContactEdit | Undo | was `l` |
| `contacts_import`, `contacts_export` | Import, Export vCard… | `mod+shift+i`, `mod+shift+e` | Contacts | None | import no longer undoable (M4) |

`toggle_selection`, `extend_selection_*`, `select_all`, `next_message`,
`prev_message`, `first_message`, `last_message`, `digest_rule`, `back` and
`undo` gain the contexts rather than being duplicated; `add_account` loses
`Contacts`. The first draft's `contact_open`, `card_next_row`, `card_prev_row`,
`contacts_toggle_*`, `contacts_suggestions`, `contact_restore`,
`contact_add_address`, `contact_detach_address`, `contact_set_preferred`,
`contact_group_*` (except add) and `suggestion_dismiss` are dropped; the form
and the sheet carry what they did.

## Assumptions

- Spec 005's person model lands on `main` first; `plan.md` lists what this
  needs from it and how to ask.
- The Mac is the only app that offers these commands at first.
- The handoff's `KEYS.md` is stale where the registry disagrees; the registry
  is edited to fit the handoff only where the Keys table says so.
- No ADR for the feature's decisions (CLAUDE.md: a spec records its decisions).
  Two rules outlive it and get ADRs in the slice that needs them: *per-contact
  settings are the one home of sender-keyed settings* (S3) and *an external
  store is written only through a reviewed plan* (S12).
- Fields edited are 005's plus title, company, phones and websites (the last
  two are shown and added from signatures or sync); other vCard properties are
  kept and shown read-only.

## Out of Scope

Finder integration (`.vcf` association, dragging a contact out: M5); undo of an
import (M4); a Deleted tab and a Groups tab (Q5, Q6); dropping an image onto
the Photo row before sync exists (Q12); creating or opening the linked Obsidian
note (the path is stored; the vault integration owns the rest); Linux and
terminal adoption (later slices); contact-based scoring.

## Questions for the maintainer

Each has a recommendation. Nothing is built until the plan is OK'd.

**Q1. Where do per-sender settings live after the migration?** Today they are
in four homes: `[[focus.digests]]` queries, and `[focus.filter]` `never` and
`stop_markers`, in `config.toml` (`postio-config/src/focus.rs`); and the
"always for this sender" HTML treatment in the allowlist file's `[Treatment]`
section (`postio-ui/src/allowlist.rs`), which the GTK reader reads directly.
- A. Hybrid. *(Recommended.)* Move the address-keyed digest, `never` and
  `stop_markers` entries into `contact_settings` in the store, one-shot, with
  the file keeping every other rule (domains, `list:`, `subject:`,
  multi-address) and the digest engine reading both. The HTML treatment stays in
  its file, written through per address, until Linux adopts (the GTK reader
  needs a host call first). **Linux sees contact-derived rules in the rules
  page, so the maintainer is told before that slice merges.**
- B. Write-through only: the card's controls edit the existing homes per
  address; nothing moves. Behaviour cannot change; settings stay in four places
  and a merged contact with different arrivals shows "mixed".
- C. Move everything now, the treatment included, changing the GTK reader in
  the same PR (it cannot be built on this Mac).

**Q2. `i` against `i i` and `i a`.** `i` is the popover's key in the handoff
and the prefix of Show images and Always show images.
- A. Popover `i`; images move to `alt+i` and `alt+I`. *(Recommended: follows the
  handoff; it is the one change to Linux bindings.)*
- B. Popover on `@` and a keycap `@`; nothing on Linux changes; the handoff's
  keycap changes.

**Q3. One Contacts permission.** M7 says ask only when sync is turned on; 009
FR-051 asks on the first keystroke in a recipient field.
- A. One prompt, in Settings › Contacts. Recipient completion uses Postio's
  store; "Contacts on this Mac, read only" is how a person who wants their
  address book in completion gets it. 009's prompt and `ContactsSource` retire
  in slice 12. *(Recommended.)*
- B. Keep 009's prompt for completion and add sync's: two prompts, and the old
  behaviour for people who never turn sync on.

**Q4. The filter field and the one box.** The handoff draws "Filter contacts
`/`" in the strip; the first draft routed `/` to the one box (the UX
architect's no-second-bar rule).
- A. The handoff's field, a list filter only, `/` focuses it, and a recorded
  exception. *(Recommended: M1.)*
- B. Draw the field in the strip but implement it as the one box's `@` mode.

**Q5. Where are groups managed?** The handoff shows chips (card, Edit) and the
`to:` and `from:@` syntax, and no Groups tab.
- A. Chips and "+ Add to group" in Edit; typing `@Finance` in the filter field
  lists its members; rename and delete from the palette. *(Recommended.)*
- B. Also keep the first draft's Groups tab.

**Q6. Delete, Unsave, Split and Ignore.** The handoff draws only Ignore (in the
popover); 005 has delete and restore.
- A. Four distinct verbs. **Unsave**: stays in All, stops syncing (`⌘↩`
  toggles it in Contacts). **Ignore**: stays, never suggested. **Delete**
  (`#`): hides everywhere, kept for restore as 005, asks when synced. **Split**:
  card menu. No Deleted tab; restore from the palette. *(Recommended.)*
- B. Only Ignore and Unsave; no Delete in v1.

**Q7. Unmatched external contacts.** Create Postio contacts for records no mail
matches?
- A. No: only matches fill in titles and phones; the person may pick extra
  records to import by hand later. *(Recommended; the handoff says "for people
  you have mail with".)*
- B. Import every record as a saved contact with no mail.

**Q8. Merge when settings differ.** The kept record's settings win and the
preview names the change. *(Recommended.)* Or the most restrictive wins
(Filtered over Digest over Inbox).

**Q9. Review key and Next scope.** `g w` for Review; `g n` for Next scope,
applied at Linux's adoption and not now, so no Linux key moves until Contacts
exist there. *(Recommended.)* Alternatives: `g v` for Review; move Next scope
now.

**Q10. Esc on an edited form.** The first `Esc` arms ("Esc again to discard"),
the second discards. *(Recommended.)* Or discard at once, as the handoff reads.

**Q11. Kind of a merged contact.** The kept record's kind. *(Recommended.)*

**Q12. Photos.** Photos arrive with sync (slice 12); contacts without sync show
initials, and the Photo row's "drop an image" waits until then.
*(Recommended.)* Or store a dropped image from slice 4.

**Q13. vCard import and export** (story 9) as the last slice before sync, no
undo, no screens drawn. *(Recommended.)* Or drop it, since sync is the import.
