# Postio: what it is, and what it must do

The product definition: the promises Postio makes, its budgets, and the scope
it has cut. Section numbers are cited from code and tests; never renumber.
Where a fact lives elsewhere, this document links to it.

| For | Read |
|---|---|
| How Postio is put together | [`ARCHITECTURE.md`](ARCHITECTURE.md) |
| A decision and its rejected alternatives | [`decisions/`](decisions) |
| Every key, generated from the registry | [`keybindings.md`](keybindings.md) |
| Each screen against its reference | [`specs/007-postio-focus/screens.md`](../specs/007-postio-focus/screens.md) |
| What is planned and not built | the [Postio Roadmap](https://github.com/users/dlapiduz/projects/2) |

---

## 1. What Postio is

> Postio is a local-first, keyboard-first email client built for people who have
> too much email. **Read less. Find anything. Act faster.**

It must beat the alternatives at **speed, search and keyboard interaction**.
Everything else (AI, the interface, protocols, integrations) earns its place by
reinforcing one of those three, or it does not ship.

### The principles

- **Instant.** Navigation never waits for the network. Search is local. §18
  gives the numbers.
- **Keyboard first.** Every operation has a shortcut; the mouse stays
  excellent and is never required. §8.
- **Search first.** Search is a primary way to move around. §7.
- **Local first.** The UI works against the local database (constitution I).
- **Native.** A real desktop application, not a website in a window.
- **Predictable.** Destructive operations are confirmed or undoable, which the
  command registry machine-checks. Sync state is visible.
- **Private.** Nothing leaves this machine that the user did not ask for. §21.
- **AI-native, eventually.** Absent from v1. §12, §23.

---

## 2. Platforms

**v1 is Linux only:** GTK4 and libadwaita, Wayland first, X11 where it works.
Postio is one product with several interfaces on the same engine and store
(constitution, Scope).

**The desktop app is Focus** ([ADR 0043](decisions/0043-focus-is-the-one-desktop-app.md),
[`specs/007-postio-focus`](../specs/007-postio-focus/spec.md)): crate
`postio-gtk`, binary `postio`, app id `dev.postio.Postio`. Home is one dense
inbox driven from the keyboard that shows mail as it arrived. A message opens
over it, or beside it by choice, one at a time (§9). Beyond reading, writing,
searching and filing, Focus:

- calls out real actions: an invitation, a direct question, a to-do;
- holds some mail back into digests on a cadence the user sets;
- files spam and automated updates away, each with its reason and one key from
  restored;
- captures mail into an Obsidian vault as a task or a note.

Its filing, digest and reminder rules act only while a Focus app has the
store. Settings are `[focus]` in [`config.toml`](config.md). What Focus draws
outside its window lives in `postio-widgets`, which depends on no app.

**Focus may use a model the user runs on this computer**, named in
`[focus.model]`: optional, off until named, local only, never required.
Without it a rule-based detector marks questions and to-dos, and a digest
opens on its list of mail. Postio ships, downloads and starts no model.

**The terminal app** (`postio-tui`, [`docs/archive/specs/005-tui-frontend`](archive/specs/005-tui-frontend/spec.md))
has the same commands, keys and verbs, local or over SSH, with mouse and
keyboard, and reads and writes mail as Markdown. It drives the `postio-focus`
controller and draws the Focus design in character cells
([`terminal.md`](../specs/007-postio-focus/terminal.md)). Images are labelled
placeholders that open in the system viewer. It ships as its own, smaller
package.

**One app holds the store at a time.** Whichever starts first has the
mailbox; the other says so and asks for it to be closed
([ADR 0041](decisions/0041-one-app-opens-the-store-at-a-time.md)).

**The macOS app is built** (#1306; Swift over `postio-ffi` and `macos/`,
[ADR 0019](decisions/0019-macos-frontend.md)): first-run wizard, sync, list,
reader, conversations, search, palette, cheat sheet, a menu bar built from the
registry, compose, notifications and settings. Focus on macOS is
[`specs/009-focus-macos`](../specs/009-focus-macos/spec.md). It is built from
the checkout, has no signed bundle, and is not part of v1. Windows is
unscheduled.

**Every command reaches something, and that is a test.** `postio-ffi`'s
`command_coverage.rs` fails when a registry command reaches nothing and is not
listed as debt with an issue. `focus_suite`'s `every_command` does the same on
the desktop (gaps in `storyboards/gaps/focus.toml`). `detach_composer`,
`next_scope` and `darken_message` are scoped away on the Mac by design
(`postio_core::registry::offered_on`).

---

## 3. Accounts and providers

Postio connects over **IMAP and SMTP**, **JMAP** or the **Gmail REST API**,
authenticated by password, app-specific password or OAuth 2 (consent in the
system browser, loopback redirect, PKCE: `crates/postio-account/src/oauth/`,
[ADR 0006](decisions/0006-oauth-and-provider-presets.md)). JMAP and Gmail are
`MailBackend` implementations ([ADR 0018](decisions/0018-jmap-and-gmail-backends.md)),
chosen per account by the preset row. Microsoft Graph is unscheduled.

**Multiple accounts are built:** one engine per enabled account, one database,
and a unified inbox that is a scope, not a mailbox. It holds every enabled
account's inbox, not all their mail. `g a` walks scopes; `account:` names one
in a query ([ADR 0005](decisions/0005-multiple-accounts.md)).

**Providers are data, not code** (constitution VII): every provider is one row
of the preset table. No named constant, special-cased branch or identifier
names a vendor. A comment may name one to explain a real compatibility quirk.

Credentials live in the OS keyring, never in `config.toml` or a log.

---

## 4. The domain model is Postio's own

`Account`, `Mailbox`, `Message`, `Thread`, `Attachment`, `Contact`,
`ContactGroup`, `Label`, `Flag`, `Draft` and `Identity` are defined in
`postio-model` and are not IMAP's types, so a second protocol changes no
shape. A `Rule` is designed ([ADR 0008](decisions/0008-filters-and-rules.md))
and not built.

---

## 5. Threading is local

Threads are rebuilt locally with JWZ over `Message-ID`, `In-Reply-To`,
`References` and subject normalisation. Server threading is a hint, never the
answer.

- The desktop opens one message, the latest of its thread by default; `]`/`[`
  walk the thread (spec 007 FR-037). The Mac shows a thread as one document
  whose messages expand and collapse ([ADR 0032](decisions/0032-the-conversation-is-one-document.md)).
- Quoted content folds into `<details>`, no script involved
  ([ADR 0004](decisions/0004-composer-document-model.md)).
- A thread belongs to one account. The unified inbox groups across accounts at
  read time ([ADR 0005](decisions/0005-multiple-accounts.md) Q2).

---

## 6. What is stored locally

- **An encrypted database** (Turso, AES-256-GCM per page,
  [ADR 0038](decisions/0038-the-store-is-turso-not-sqlcipher.md)) for
  everything listable and searchable, plus a **content-addressed blob
  directory** for raw messages and attachments. No maildir, mbox or notmuch.
- **The schema is one declared `HEAD`** in `crates/postio-storage/src/schema.rs`.
  There are no numbered migrations; a store an older schema wrote is resynced.
  `schema::declared()` lists the tables, and a test holds that list against
  every object the old migrations built: an object leaves the schema only by
  being named as deliberately absent.
- **No secret is ever written** to the database or `config.toml`.
- **Search** is the engine's full-text index, owned by `postio-index`, over
  `search_documents` (one flattened metadata row per message) and
  `message_search_bodies` (the body, folded for search). The message row holds
  the text the reader shows. Vector retrieval, if built, re-ranks these
  results ([ADR 0009](decisions/0009-ai-subsystem.md)).
- **The body is zstd per row when smaller**, else plain
  (`postio_storage::body_codec`); the frame's magic bytes tell them apart.
- **The store is a complete replica with a budget.** Backfill (§14) puts every
  message's text local. Blobs are compressed at rest. A configurable size limit
  evicts what can be refetched: raw source first, then attachment payloads,
  never the text search is made of
  ([ADR 0017](decisions/0017-backfill-cost-attachments-memory-disk-encryption.md)).

---

## 7. Search

Search is a defining feature and a primary way to navigate.

**One query language, everywhere** (constitution III): the command bar, a
pinned search and `config.toml` take the same string. A saved search is a named
query; a virtual folder is a pinned one; a rule is a saved search plus actions
([ADR 0008](decisions/0008-filters-and-rules.md)).

Operators compose; a leading `-` negates:

```
from:ada after:2026-01-01 has:attach
subject:invoice -in:archive
```

`from:` `to:` `subject:` `in:` `list:` `filename:` `has:attach` `is:unread`
`is:read` `is:flagged` `is:bulk` `is:automated` `before:` `after:` `larger:`
`smaller:` `account:` `with:` `group:` `header:`

- **Braces take any of several values:** `from:{ada tomas}`,
  `label:{"Q3 close" atlas}`, `-from:{ada tomas}` (neither). Two clauses still
  mean both ([spec 010](../specs/010-focus-search/spec.md), D26).
- **`with:a@x.example,b@y.example`** is mail from, to or copied to any of those
  addresses, matched exactly. It names addresses, not a person, so a pinned
  search keeps meaning the addresses it named (`specs/005-contacts`).
- **`header:`** reaches any RFC 5322 field the envelope does not carry.
  `header:x-mailer` tests presence; `header:x-mailer=mutt` tests that the value
  contains `mutt` (case-insensitive substring); `header:x-mailer=` means
  presence, not an error. The name is matched exactly, ignoring case. The value
  splits at the first `=`. Headers arrive with the body, so a message whose
  body is not fetched is not a hit
  ([ADR 0025](decisions/0025-arbitrary-headers-are-indexed-rows.md)). Every
  header is indexed, with no list of names (constitution III). One that must
  match before the body arrives earns its own operator: `list:`, `is:bulk`,
  `is:automated`.
- **`account:`** takes a name or an address and composes with everything:
  `account:work is:unread`, `-account:work`.
- **Flagged, not starred.** `is:starred` is accepted as an alias of
  `is:flagged`, `has:attachment` of `has:attach`.
- The parser never errors on a half-typed query; results appear while typing.
  One box, with a prefix selecting the mode, searches mail, runs a command,
  jumps to a folder or finds a correspondent.

Natural-language search ([ADR 0009](decisions/0009-ai-subsystem.md)) lowers to
this language.

---

## 8. The keyboard is a system, not a list of shortcuts

**Every command has a shortcut, a command-palette entry and an accessible
action**, all derived from one table, `postio-core::registry`: the keymap, the
command bar (`mod+k`), the `?` key map, the row menu, every keycap and
[`keybindings.md`](keybindings.md), which a test regenerates. **A command not
in the registry does not exist.** The bindings are in `keybindings.md` and
nowhere else.

- Every Postio app has one keymap, the registry's
  (`specs/007-postio-focus/contracts/keymap.md`), and **it is Gmail's**
  wherever Postio has the command: `e` archive, `r` reply, `a` reply all, `f`
  forward, `#` delete, `s` flag, `z` undo, `c` compose, `/` search, `g i`
  inbox, and so on. A command Gmail has no key for keeps its own, or takes a
  new key in Gmail's style where Gmail's displaced it (`E` archives a thread,
  `N` captures a note, `M` unsubscribes, `g o` the outbox).
- `I` and `U` both reach `toggle_read`, which toggles. Gmail keys with no
  Postio command (`m`, `* n`, `:`, `g k`) stay unbound.
- A command only some apps offer keeps its key in the others and leaves it
  free there.
- Every binding is overridable from `[keys]` in `config.toml`, keyed by command
  id. Ids are a file format and are not renamed casually.

---

## 9. Layout

**One list.** The desktop shows every account's inbox as one dense list,
newest first under day headings, with no folder sidebar. A row shows mail as it
arrived: sender, subject and first line, verbatim, never a rewritten subject or
a score. The list is windowed over the paged store (§18). A top bar holds
Compose, the command bar, the sync state and the main menu. A header strip
names the place ("Inbox", `g o`), its counts and the "Has action" toggle
(`!`).

**Rows have two fixed heights:** 40px for a conversation, 72px for one with a
marker. Focus, hover and selection change what a row draws, never its height.

**Markers call out real actions** on a row's second line: an invitation
(Accept `y`, Decline `Y`), a direct question (Reply `e`), a to-do (Snooze `s`,
or Task `t` with a vault), a reply that has not come. A question or to-do
quotes its sentence verbatim. `!` shows only marked mail; `-` dismisses a wrong
marker, and that is remembered.

**Digests** hold mail back on a cadence the user sets (`d` makes a rule, `g d`
lists them). A delivery is one row. It opens on a summary when a local model is
connected (§12), each statement citing its mail; `Tab` switches to the plain
list. **Filtered** (`g f`) holds spam and automated updates, each with its
reason and `R` from restored. Filtered mail is archived, never deleted
automatically.

**The open message is a dialog over the list**, one at a time: `]`/`[` walk the
thread, `j`/`k` the list, `Esc` closes onto the same row. `F8` places it beside
the list (`[focus] reading = "pane"`). A window narrower than 980px always uses
the dialog: width decides what is shown, never what the user asked for.

**One box, one popover.** The command bar (`/` mail, `mod+k` commands) searches
as you type (§7). The folders popover (`g o`, or a click on the place name)
lists mailboxes, folders with counts, the Flagged (`g *`) and Snoozed (`g z`)
views and labels. A *folder* is one the server has. A *view* is a saved
question about mail filed elsewhere (Flagged, Snoozed, Filtered, Outbox) with a
name, a place and a count, and nothing can be moved into it. Every account has
a real folder for all six roles, created on the server if missing
([ADR 0036](decisions/0036-a-sidebar-row-is-a-folder-or-a-view.md)).

**The Outbox holds what is on its way** and is not listed when empty. Send puts
the message there at once, offline included; it leaves for Sent when the server
accepts it. Drafts holds what is being written and what has stopped, and a
draft's row says which.

**The list has a cursor and a selection** (constitution II). A selected row is
a neutral background and a checked box; the bulk bar acts on the selection.

Each surface's look is
[`contracts/focus-surface.md`](../specs/007-postio-focus/contracts/focus-surface.md)
held against [`screens.md`](../specs/007-postio-focus/screens.md). §19.

**Calendar is a mode of the main window** (`g e`; `specs/017-calendar`).
Agenda lists occurrences under day headings in the list's language; `c` adds
an event, `mod+e` edits one, `v` moves and `#` deletes it, each undoable, and
`mod+shift+o` gives Calendar a window of its own. Times are typed the way search
reads them ("thu 3pm", "1h"). An invitation in mail says what it clashes
with. Events live on this computer or in an account's CalDAV calendars, kept
in both directions; nothing in an event is fetched.

---

## 10. Compose

- The composer opens where the open message is: in its dialog, or in the pane
  when reading beside the list (spec 007 FR-050). It is not a separate window.
  The list keeps its scroll and cursor; `Esc` closes it, keeps the draft and
  returns you to the same place. There is one composer, `postio-widgets`'.
- A composition can be **detached** into its own window (`detach_composer`)
  only when the user asks; nothing opens detached. It is the same widget
  moved, never a second composer.
- Recipients autocomplete from the address book, one name per person with the
  preferred address first, whether the user or the mail made them
  ([`specs/005-contacts`](../specs/005-contacts/spec.md)). Cc and Bcc appear on
  demand; identities are pickable; drafts autosave; attachments drag and drop.
- **The document is Postio's own, not the toolkit's**
  ([ADR 0003](decisions/0003-rich-text-compose.md),
  [ADR 0004](decisions/0004-composer-document-model.md)). Each platform's
  editor is a view over `postio_body::Document`, so a second frontend is a port
  and identical gestures produce identical bytes. The editor is a WebView
  (`postio_widgets::composer::editor`). A message that used no formatting is
  sent as plain text.
- **The composer formats everything a Markdown file can** (CommonMark and GFM,
  [spec 014](../specs/014-compose-formatting/spec.md)): six heading levels;
  bold, italic, strikethrough; inline code and code blocks with a language;
  nested quotes; bulleted, numbered and task lists, nested; links, autolinks
  and inline images; tables with alignment; dividers and line breaks.
  Markdown is a way in and a view, never the stored format: `mod+alt+s`
  switches a draft to its Markdown source and back, losing nothing.
- Outgoing HTML is generated from the document, for a forward too. **A reply's
  quote** carries the original as the reader rendered it
  ([ADR 0033](decisions/0033-a-reply-quotes-what-the-reader-shows.md)),
  re-emitted only as the reader's sanitiser permits (§21).

---

## 11. Attachments

Download, open, save as, and preview inline images and PDF first. Indexed by
filename: `has:attach`, `filename:contract`.

**Attachments are the one thing fetched lazily.** Bodies are not (§14).
Payloads are about 90% of a mailbox by weight and add only a filename to
search, so they arrive when the user opens or saves one. A message with its
text local and payloads not is a normal, searchable, readable state.
Exceptions: new mail's attachments up to 25 MB a message arrive right after its
text, and inline images small enough to matter for rendering arrive with the
text. A user can ask for eager download or refuse it on a metered link
([ADR 0017](decisions/0017-backfill-cost-attachments-memory-disk-encryption.md)).

---

## 12. AI

**Deferred to post-v1 by decision** (§23), except the local model a user may
run for Focus (§2). Two constraints bind whenever AI arrives:

- **AI never silently modifies or sends mail.** Every externally visible
  action (send, forward, delete, move, mark read) needs explicit human
  confirmation in the Postio UI.
- **Mail is attacker-controlled text.** An agent reading it is exposed to
  prompt injection, and that is the dominant design constraint.

Capabilities and how both constraints become structural are
[ADR 0009](decisions/0009-ai-subsystem.md). Exposing the mailbox to external
agents is [ADR 0010](decisions/0010-mcp-surface.md).

---

## 13. AI subsystem shape

[ADR 0009](decisions/0009-ai-subsystem.md): a `postio-ai` crate behind a
provider trait, per-account and per-feature data-sharing permissions, and no
send path in its dependency closure, which CI checks. Built so far: Focus's
client for the user's own local model (§2).

---

## 14. Synchronization

Invisible most of the time: incremental sync, reconnection with exponential
backoff, IMAP IDLE, QRESYNC/CONDSTORE where available, eager body backfill,
lazy attachment fetch, cancellation and progress.

- **Backfill runs to completion.** Every selectable folder pulls every
  message's body in the background. A folder can be excluded explicitly;
  nothing is by default ([ADR 0016](decisions/0016-full-mailbox-backfill-by-default.md)).
  `postio-sync::BackfillPolicy` throttles it (size cap, metered and active
  pauses) without capping how much eventually arrives.
- **Bodies are eager, attachments lazy.** The text axis (headers, every
  `text/*` part, small inline images) completes for every message and is what
  §7 and §15 rest on. The payload axis defaults to fetch on open.
- **New mail jumps the backlog:** its text, then its attachments, behind only
  what the person has opened.
- **The UI never awaits the network** (constitution I).

---

## 15. Offline

Fully usable offline after the first sync: read, search, compose, reply,
forward, archive, delete, move, label, mark read or unread. Operations enter
the local queue and reconcile when the link returns. Offline is not a mode; it
is the online code path.

---

## 16. Undo

> *Archived 12 messages. Undo*

Destructive operations are undoable, and undo is local and immediate; the
server catches up afterwards. A burst of actions is one unit (twelve keystrokes
are one toast and one `mod+z`), and the stack forgets: restoring something
archived an hour ago is a surprise.

**Leaving a list offers to clear it.** After `M` unsubscribes, a notice says
`Unsubscribed from` the list's `List-Id`, or for a sender with none their exact
address (never the domain). When mail from it remains, it offers one action:
Archive, or Delete if `[focus] after_unsubscribe` says so. The mail excludes
Flagged and what is in Trash, Junk, Sent or Drafts. The notice never takes the
keyboard (toast on the desktop, status line in the terminal, banner row on the
Mac); ignoring it or `Esc` keeps everything. It is one command, so one undo.

**The one thing not undoable is emptying Junk and Trash on a clock.** It is off
until the user sets `[sync] empty_junk_after_days` / `empty_trash_after_days`,
deletes for good here and on the server, and never touches a Flagged message.
Everything else that removes mail goes to Trash first.

---

## 17. Notifications

Desktop notifications for new mail and, later, mail that needs a response,
configurable per account and per mailbox. Never a read receipt: a notification
is Postio telling the user something, never the sender (§21).

---

## 18. Performance is a functional requirement

Budgets are gated as counts, not stopwatches (constitution V;
[`PERFORMANCE.md`](PERFORMANCE.md)).

| | Target |
|---|---|
| Startup to usable UI, populated database | **< 500 ms** |
| Ordinary UI interaction | **< 16 ms** |
| Local search | **< 100 ms** |
| Transitions | **<= 100 ms, or absent** |

Pane switches use no transition; `prefers-reduced-motion` is always honoured.

**A mailbox is never loaded into memory.** The list is windowed over the paged
store, and "select all" is a predicate (`Everything { except }`), not a hundred
thousand ids. This shapes the store, the list widget and the selection model.

Opening a synchronised message needs no network. Postio is one process, not a
browser engine per window.

---

## 19. Visual design

A beautiful native application, not terminal-inspired and not "a GTK developer
made an email client". Generous typography, real spacing, subtle hierarchy,
restrained colour, excellent dark and light mode, minimal chrome, very good
message rendering.

Spec 007 is the authority on visual detail
([`contracts/focus-surface.md`](../specs/007-postio-focus/contracts/focus-surface.md),
[`screens.md`](../specs/007-postio-focus/screens.md)).

- The chrome is the system's: Adwaita Sans, Adwaita Mono for keys, addresses
  and counts (C25), libadwaita's colours so light, dark and the accent come
  from the desktop (C26). Real Adwaita window chrome.
- The accent is reserved for what needs the user: action markers, the focus
  ring, the has-action toggle. A selected row is neutral.
- A message body is drawn in the app's colours, set in Barlow, or as the
  original on a sheet of paper when it paints its own page.
- Spacing and sizes are generated from the design system, never retyped
  (`ARCHITECTURE.md` §10).

---

## 20. Accessibility is first-class

Not a later pass: keyboard navigation, screen readers, high contrast, reduced
motion, scalable text, focus indicators, semantic controls. Colour is never the
only carrier of meaning. Every command is reachable without the mouse by
construction, because §8's registry generates the accessible actions.

---

## 21. Privacy and security

Mail is attacker-controlled content that tries to phone home. **Nothing leaves
this machine that the user did not ask for** (constitution VI). The mechanisms:

- Remote images and tracking pixels are blocked until allowed **per sender**,
  never a global default-on.
- **Read receipts are never sent automatically**; `Disposition-Notification-To`
  is tracking.
- `List-Unsubscribe` One-Click fires only on deliberate activation, because
  sending it confirms the address is live.
- No link prefetch, favicon fetch or speculative connection. The reader's
  renderer cannot run script or reach the network (ADR 0042): it has no script
  engine and draws only the message's own parts from the local blob store plus
  a remote image the user allowed, fetched by the application. The
  conversation rail reads the visible message from the renderer's geometry.
- **A forward** is generated from Postio's own types, where script has no
  representation. **A reply** carries the sender's markup as the reader
  sanitised it ([ADR 0033](decisions/0033-a-reply-quotes-what-the-reader-shows.md)),
  remote images blocked regardless. The corpus-wide assertion in `postio-body`
  keeps this true: zero scripts, remote-loading references, tracking pixels or
  broken image elements across every HTML message in the corpus.
- **A sender who may not be who they say gets one quiet line in the reader's
  header; ordinary mail gets nothing** (#1928). Triggers: the provider stamped
  an `Authentication-Results` saying DMARC failed (or SPF and DKIM both failed
  for the From domain), or the display name is itself another address or
  domain. Only the topmost `Authentication-Results` whose authserv-id is in the
  provider's preset row (`authserv_ids`) is believed, since a sender writes
  every header it sends; with none, nothing is shown. There is no "verified"
  badge.
- No telemetry, crash reporting or update ping.
- **The local store holds the whole mailbox and is encrypted**
  ([ADR 0014](decisions/0014-encryption-at-rest.md)). A backup of
  `$XDG_DATA_HOME` is a backup of the mailbox; the key lives in the keyring,
  and losing it costs a re-sync, never mail.
- Credentials in the OS keyring; TLS wherever the server offers it.
- Logs carry ids, counts and outcomes, never message content.

For anything that could make a network request, ask "did the user ask for it";
if not, it does not ship. `scripts/checks/check-no-silent-tracking.py` refuses
the two easiest mechanisms to add by accident.

Link warnings, PGP and S/MIME are unscheduled.

---

## 22. Architecture

See [`ARCHITECTURE.md`](ARCHITECTURE.md) and [`decisions/`](decisions).

---

## 23. What v1 is, and what it is not

**In:**

- Multiple accounts and a unified inbox; IMAP + SMTP, JMAP or the Gmail API
  with password, app-specific password or OAuth 2.
- Inbox, folders, threads, labels; read/unread, archive, delete, flag, move,
  snooze.
- HTML and plaintext reading with attachments, quote folding, remote images
  blocked per sender, one-click unsubscribe on request.
- Rich-text compose, reply, reply-all, forward, attachments, drafts,
  signatures, identities, scheduled send, an outbox that sends at most once.
- People with several addresses, and groups, filled from mail and completing
  recipients, with vCard import and export that keeps what it does not model
  (`specs/005-contacts`; Focus's Contacts surface is `specs/011-focus-contacts`).
- Local full-text search with operators, one command bar, pinned saved
  searches; vim-style navigation and configurable shortcuts.
- An encrypted local store, background sync, offline reading, undo, desktop
  notifications.
- Flagging is `*` (key, row menu or command bar), `g *` lists flagged mail, and
  a row carries no flag mark (spec 007 C13).
- A calendar: Agenda, invitations answered from the inbox, events made and
  changed on this computer or in a CalDAV account (`specs/017-calendar`).
- Focus: one dense inbox with action markers, digests, filtering with reasons,
  Obsidian capture, and an optional local model (§2, §9). All of it in the
  terminal too, on the same store.

**Out, deliberately:**

- Rules: the language is shared and the design is
  [ADR 0008](decisions/0008-filters-and-rules.md), but no rule fires yet.
- A Contacts surface in apps that have not adopted it (the Mac's is next);
  CardDAV and provider contact sync.
- Microsoft Graph; PGP and S/MIME; phishing and link warnings; Windows;
  images drawn in the terminal.
- **AI**, beyond Focus's optional model: a founding principle, deferred so
  that core mail, search and the keyboard land first.

Each has an issue and an ADR.

---

## 24. After v1

The [Postio Roadmap](https://github.com/users/dlapiduz/projects/2) is the list.
It is a plan, not a queue, so roadmap issues are not labelled `ready`.

---

## 25. The workflow this is all for

```
/     invoices from Acme that I haven't responded to
```

Postio finds them. `Enter` opens the message. `a` archives it. Or `Ctrl+K`,
Summarise, then `e` opens the reply with a suggested draft, the user edits it,
and `Ctrl+Enter` sends.

**All of it without reaching for the mouse.** Everything in this document
either serves that workflow or is a constraint that keeps it honest.
