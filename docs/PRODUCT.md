# Postio — what it is, and what it must do

The product definition: the promises Postio makes, the budgets it holds itself
to, and the scope it has deliberately cut. Written for whoever is about to
build something and needs to know whether it belongs.

This replaces `spec.md`, which was the original brief. It kept its section
numbers — the codebase cites them from about eighty doc comments and tests, and
those citations are how a constraint stays attached to the code that honours
it — but not its content, most of which had been overtaken by the tree, the
design canvas, or an ADR.

**One fact, one home.** Where something is recorded elsewhere, this document
says where and stops. It is a map as much as a specification:

| For | Read |
|---|---|
| How Postio is put together, and why | [`ARCHITECTURE.md`](ARCHITECTURE.md) |
| A decision and the alternatives it rejected | [`decisions/`](decisions/) |
| Every key, generated from the registry | [`keybindings.md`](keybindings.md) |
| Visual detail — spacing, colour, the chosen direction | `Design/Mail Client.dc.html` |
| Hard-won lessons | [`engineering-notes.md`](engineering-notes.md) |
| What is planned and not yet built | the [Postio Roadmap](https://github.com/users/dlapiduz/projects/2) |

---

## 1. What Postio is

> Postio is a local-first, keyboard-first email client built for people who have
> too much email.
>
> **Read less. Find anything. Act faster.**

Three things Postio must do better than the alternatives: **speed, search, and
keyboard interaction.** Everything else — AI, the interface, protocols,
integrations — earns its place by reinforcing one of those three, or it does
not ship.

### The principles

**Instant.** Opening the app is immediate. Navigation never waits for the
network. Search is local and effectively instantaneous. §18 turns this into
numbers, and CI enforces them.

**Keyboard first.** Every operation has a keyboard shortcut. The mouse stays
excellent and is never required.

**Search first.** Search is a primary way to move around, not a feature in a
box. §7.

**Local first.** Mail syncs locally; the UI works against the local database;
the network happens elsewhere. `ARCHITECTURE.md` §1 is the mechanism.

**Native.** A real desktop application, not a website in a window.

**Predictable.** *Destructive operations are confirmed or undoable* — an
invariant the command registry machine-checks rather than a habit
(`ARCHITECTURE.md` §2). Sync state is visible. The user always knows what
happened.

**Private.** Nothing leaves this machine that the user did not ask for. §21.

**AI-native, eventually.** AI helps understand, find, write and act on mail,
integrated into the workflow rather than parked in a sidebar — and it is
deliberately absent from v1. §12, §23.

---

## 2. Platforms

**v1 is Linux only**: GTK4 and libadwaita, Wayland first, X11 where it happens
to work. Verified against gtk4 4.22, libadwaita 1.9, WebKitGTK 2.52.

macOS and Windows frontends over the same Rust engine were always possible,
and that possibility is the reason for two CI-enforced boundaries rather than
an aspiration in a document: `postio-core` must not depend on GTK, and
`postio-gtk` must not depend on SQLite or the protocol crates
(`ARCHITECTURE.md` §9).

**A native macOS frontend is now scheduled** — Swift over the same engine,
[ADR 0019](decisions/0019-macos-frontend.md), tracked in its own milestone and
not part of v1. The invariant it was kept for turned out to be load-bearing and
not merely tidy: thirteen of the fifteen crates build and test on macOS with no
changes at all. Windows remains unscheduled.

---

## 3. Accounts and providers

v1 connects over **IMAP and SMTP**, authenticated with a password, an
app-specific password, or OAuth 2 — the bearer mechanisms reach the IMAP and
SMTP sessions as of #193, and
[ADR 0006](decisions/0006-oauth-and-provider-presets.md) is the design.
Multiple accounts are in scope and are
[ADR 0005](decisions/0005-multiple-accounts.md), tracked under #1. JMAP, the Gmail API
and Microsoft Graph are unscheduled, and the `MailBackend` seam
(`ARCHITECTURE.md` §8) is what keeps them possible.

**Providers are data, not code.** Server settings live in a preset table where
every provider is one row — never a named constant, never a special-cased
branch, never an identifier naming a vendor. Postio is not one provider's
client, and the maintainer's own provider must not be visible in the shape of
the code. Naming a provider in a *comment* is fine where it explains a real
compatibility quirk.

Credentials live in the OS keyring. Never in `config.toml`, never in a log.

---

## 4. The domain model is Postio's own

Postio's types are not IMAP's. `Account`, `Mailbox`, `Message`, `Thread`,
`Attachment`, `Contact`, `Label`, `Flag`, `Draft`, `Identity` and `Rule` are
defined in `postio-model` and would survive a second protocol without changing
shape. That is what makes §3's future protocols a backend rather than a
rewrite.

---

## 5. Threading is local

Threads are reconstructed locally with JWZ over `Message-ID`, `In-Reply-To`,
`References` and subject normalisation, using server threading as a hint where
it exists and never as the answer. A server that threads badly, or not at all,
must not make Postio thread badly.

In a thread the reader can expand and collapse messages, jump between them,
open one on its own, and expand or collapse quoted content — quoting folds
into `<details>` with no script involved (`postio-body`,
[ADR 0004](decisions/0004-composer-document-model.md)).

**A thread belongs to one account.** The unified inbox groups across accounts
at read time instead; the reasoning is
[ADR 0005](decisions/0005-multiple-accounts.md) Q2.

---

## 6. What is stored locally

SQLite for everything listable and searchable, plus a **content-addressed blob
directory** for raw messages and attachments. No maildir, no mbox, no notmuch.

The database must hold `accounts`, `identities`, `mailboxes`, `messages`,
`threads`, `recipients`, `attachments`, `labels`, `message_labels`, `drafts`,
`sync_state`, `settings` and `operation_queue` — a migrations test asserts
exactly that list, so this is a checked requirement rather than a description.
`contacts` is there too, beyond what this section requires, because recipient
autocomplete has to rank from somewhere ([ADR 0007](decisions/0007-address-book.md)).

**Secrets are not among them.** No password and no token is ever written to the
database or to `config.toml`.

Search is FTS5 over that database, in `postio-index`. Tantivy and hybrid
lexical/vector retrieval were considered; the vector half is now
[ADR 0009](decisions/0009-ai-subsystem.md), which re-ranks FTS5 results rather
than replacing them. **The index stores no second copy of a body**: SQLite holds
the inverted index, the message row holds the text (compressed — ADR 0020),
and result highlighting is generated from that. The blob store holds
attachment payloads and raw `.eml`.

**The store is a complete replica, and it has a budget.** Under §14's backfill
every message's text ends up local, so the database and blob store together hold
the whole mailbox rather than a recent slice of it. Blobs are compressed at rest,
and because everything except drafts and the operation queue can be re-synced,
the store has a configurable size limit past which it evicts what it can refetch
— raw source first, then attachment payloads, never the text that search is made
of. [ADR 0017](decisions/0017-backfill-cost-attachments-memory-disk-encryption.md)
is the reasoning.

---

## 7. Search

Search is a defining feature and a primary way to navigate.

**One query language, everywhere.** The same string means the same thing typed
in the search bar, saved to the sidebar, or written into `config.toml`. A
saved search is a query with a name; a virtual folder is a saved search that is
pinned; a rule is a saved search plus actions. `ARCHITECTURE.md` §6 holds the
boundary that keeps this true, and
[ADR 0008](decisions/0008-filters-and-rules.md) extends it to rules.

Operators compose, and a leading `-` negates:

```
from:ada after:2026-01-01 has:attach
subject:invoice -in:archive
```

`from:` `to:` `subject:` `body:` `in:` `list:` `filename:` `has:attach`
`is:unread` `is:read` `is:flagged` `before:` `after:` `larger:` `smaller:`
`account:` `group:`

Adjacent operators mean **and**. `OR` — uppercase, always — joins alternatives,
and parentheses group them:

```
from:ada OR from:grace
(from:ada OR from:grace) has:attach
```

`OR` binds *looser* than the space between two operators, so
`from:ada OR from:grace has:attach` is *ada, or grace-with-an-attachment*.
There is no `AND` keyword: adjacency already means it. `OR` is uppercase only
because "cats or dogs" is three words somebody is searching for.

`body:` narrows a term to the message text. Plain free text already searches
the body *and* the metadata, so `invoice` finds a message whose subject says
so; `body:invoice` finds the ones that say it in the message.

`account:` names an account by the name it shows in the sidebar or by its
address, and composes with everything else — `account:work is:unread` is one
query rather than a mode you switch into. It is what keeps a saved search
pinned to one account however it is opened, and `-account:work` means every
other one.

**It is `is:flagged`, not `is:starred`** — the sidebar says Flagged, and the
older spelling is accepted as an alias so that nobody's muscle memory or saved
query breaks. Likewise `has:attach` with `has:attachment` as an alias.

The parser never errors on a half-typed query: `is:` and `after:2026-` are
ordinary intermediate states, and anything unrecognised stays free text.
Results appear while typing, and one box — with a prefix selecting its mode —
searches mail, runs a command, jumps to a folder, or finds a correspondent.

Natural-language search (*invoices from Acme I haven't answered*) is
[ADR 0009](decisions/0009-ai-subsystem.md), and it lowers to this same
language rather than becoming a second one.

---

## 8. The keyboard is a system, not a list of shortcuts

**Every command has a keyboard shortcut, a command-palette entry and an
accessible action.** That is a structural requirement, and it is met by having
exactly one enumerable table — `postio-core::registry` — from which the keymap,
the `Ctrl+K` palette, the `?` cheat sheet, the context menu, the key hints on
the focused row and [`keybindings.md`](keybindings.md) are all derived. Three
hand-maintained lists drift within a release; one table cannot.

**A command that is not in the registry does not exist** — not merely unbound,
but absent from every way a user could discover it.

**The bindings themselves are in [`keybindings.md`](keybindings.md)**, generated
from the registry by a test that fails when the file drifts. They are not
repeated here, for the same reason they are not repeated anywhere else.

Worth knowing before reading that table: `e` replies and `a` archives, `A`
archives a thread, `u` undoes, `t` opens a thread. The original brief proposed
`r` for reply and `u` for mark-unread; the design canvas is newer and won, and
that is now simply what the bindings are. Every one is overridable from
`[keys]` in `config.toml`, keyed by command id — which makes command ids a file
format that cannot be renamed casually (`ARCHITECTURE.md` §3).

---

## 9. Layout

Three panes — sidebar, message list, reading pane — with the sidebar
deliberately not consuming the screen. The list is windowed over paged SQLite
and is never fully materialised (§18).

The layout adapts rather than being fixed: three panes on a desktop monitor,
two on a laptop, message-focused for reading and writing, and search-focused
when results take over the workspace. The widths that divide those are in
`postio-gtk::shell`'s own table rather than repeated here, for the same reason
the bindings are not repeated in §8 — and what the sidebar does across a
resize is
[ADR 0024](decisions/0024-layout-intent-and-constraint.md): the window's width
decides what is *shown*, never what the user asked for.

**The list has a cursor *and* a selection**, and they are not the same thing —
the cursor is where the keyboard is, the selection is what `a` would archive.
Conflating them is the classic bug, because it only surfaces once a selection
is more than one row (`ARCHITECTURE.md` §4).

The chosen visual direction is **PLATE (canvas option 1b)**: airy desktop,
40px rows, key hints revealed on the focused row only. §19.

---

## 10. Compose

The composer takes over the reading pane. It is **not a separate window** — the
original brief implied one and the design canvas is explicit that it is not,
and this is the resolved decision rather than a disagreement to arbitrate.
The list keeps its scroll and its selection underneath, so context never
disappears and `Esc` returns you exactly where you were.

A composition **can** be popped out into a window of its own (`ctrl+shift+o`,
or the button beside the composer's heading), which is the inverse of what
most clients default to and deliberately so: losing your place in the list to
write a reply is the failure the in-place design exists to avoid, so nothing
ever opens detached and the pop-out is only ever asked for. It exists because
writing while reading something *else* is the one thing an in-place composer
genuinely cannot do. It is the same composition either way — the same widget,
moved — so there is never a second composer to keep in step.

Recipients autocomplete from explicit contacts and from correspondents seen in
the mailbox ([ADR 0007](decisions/0007-address-book.md)); Cc and Bcc appear on
demand; identities are pickable; drafts autosave; attachments drag and drop.

**The document is Postio's own, not the toolkit's.** The composer edits a
neutral document type and each platform's editor is a view over it, so a second
frontend's composer is a port rather than a rewrite and identical gestures
produce identical bytes on the wire. Rich text is
[ADR 0003](decisions/0003-rich-text-compose.md); where the document lives is
[ADR 0004](decisions/0004-composer-document-model.md). **v1 composes plain
text over that model** — which is a perfectly good v1, and is the point of
deciding the model first.

Outgoing HTML is *generated* from that document, never passed through, so
nothing a sender wrote is ever re-emitted to a third party (§21).

---

## 11. Attachments

Download, open, save as, and preview where practical — inline images and PDF
first. Indexed by filename: `has:attach`, `filename:contract`.

**Attachments are the one thing fetched lazily.** Bodies are not: every message's
text is pulled unprompted and to completion (§14), because search and offline
reading depend on it. Attachment payloads are different — they are ~90% of a
mailbox by weight and contribute nothing to search but their filename — so they
arrive when the user opens or saves one, and a message whose text is local but
whose payloads are not is a normal, fully searchable, fully readable state.
Inline images small enough to matter for rendering come with the text, so HTML
mail reads correctly offline. Someone who wants a complete archive can ask for
attachments eagerly; someone on a metered link can refuse them entirely.
[ADR 0017](decisions/0017-backfill-cost-attachments-memory-disk-encryption.md)
has the measurements and the policy.

---

## 12. AI

**Deferred to post-v1 by decision, not by accident** (§23). The two constraints
that bind whenever it does arrive are already fixed:

- **AI must never silently modify or send mail.** Read and search may be
  exposed relatively freely; every externally visible action — send, forward,
  delete, move, mark read — requires explicit human confirmation in the Postio
  UI.
- **Mail is attacker-controlled text.** An agent reading mail is exposed to
  prompt injection: an attacker emails the user and the body contains
  instructions. This is an actively exploited class of attack against
  mail-reading agents, and it is the dominant design constraint rather than an
  afterthought.

The intended capabilities — thread summary, action items and decisions, draft
reply, semantic search, triage — and how both constraints become structural
rather than procedural are
[ADR 0009](decisions/0009-ai-subsystem.md). Exposing the mailbox to external
agents is [ADR 0010](decisions/0010-mcp-surface.md).

---

## 13. AI subsystem shape

See [ADR 0009](decisions/0009-ai-subsystem.md). In short: a `postio-ai` crate
behind a provider trait covering local and cloud models, with per-account and
per-feature data-sharing permissions — and with no send path in its dependency
closure, so "AI cannot send mail" is a fact CI checks rather than a rule
somebody remembers.

---

## 14. Synchronization

Invisible most of the time. Incremental sync, automatic reconnection with
exponential backoff, IMAP IDLE, QRESYNC/CONDSTORE where the server has them,
eager body backfill and lazy attachment fetch, cancellation, and progress
reporting.

**Backfill runs to completion, not to a fixed initial pull.** Every
selectable folder pulls every message's body, eventually, in the
background — not just the newest few hundred. A folder can be excluded
explicitly; nothing is excluded by default. [ADR
0016](decisions/0016-full-mailbox-backfill-by-default.md) is the reasoning;
`postio-sync::BackfillPolicy` is what throttles it responsibly (size cap,
metered/active pauses) without capping *how much* eventually arrives.

**Bodies are eager; attachments are lazy.** Backfill has two axes and they are
governed separately. The *text axis* — headers, every `text/*` part, and inline
images small enough to matter for rendering — runs to completion for every
message, and is what §7's search and §15's offline promise are made of. The
*payload axis* — attachment bytes — defaults to fetching on open, because it is
around nine tenths of a mailbox by weight and contributes nothing to search but
a filename. A message with its text local and its payloads not is the ordinary
steady state, not a half-synced one.
[ADR 0017](decisions/0017-backfill-cost-attachments-memory-disk-encryption.md)
prices both axes against a real 81,744-message account and settles the memory,
disk, compression and encryption consequences.

**The UI never awaits the network.** Every mutating action is: SQLite write →
enqueue the remote operation → emit the event → repaint. The sync engine drains
the queue later and somewhere else. `ARCHITECTURE.md` §1.

---

## 15. Offline

Fully usable offline after the first sync: read, search, compose, reply,
forward, archive, delete, move, label, mark read or unread. Operations enter
the local queue and reconcile when the link returns.

This is not a mode. It is the same code path as being online, which is why it
works.

---

## 16. Undo

> *Archived 12 messages — Undo*

Destructive operations are undoable, and undo is **local and immediate** — it
does not wait for the server, which catches up afterwards.

Two properties make the promise honest: a burst of actions is **one unit**, so
twelve keystrokes are one toast and one `u`; and the stack **forgets**, because
putting back something archived an hour ago is a surprise rather than a mercy.
`ARCHITECTURE.md` §5.

---

## 17. Notifications

Desktop notifications for new mail and, later, for mail that needs a response.
Configurable per account and per mailbox.

**Never a read receipt.** A notification is Postio telling the user something;
it must never become Postio telling the *sender* something. §21.

---

## 18. Performance is a functional requirement

A budget, enforced by benches in CI, not checked by hand at the end.

| | Target |
|---|---|
| Startup to usable UI, populated database | **< 500 ms** |
| Ordinary UI interaction | **< 16 ms** |
| Local search | **< 100 ms** |
| Transitions | **≤ 100 ms, or absent** |

Pane switches and thread drill-in use *no* transition, and
`prefers-reduced-motion` is always honoured.

**A mailbox is never loaded into memory.** The message list is windowed over
paged SQLite, and "select all" is a predicate — `Everything { except }` — not a
hundred thousand ids. This constraint shapes the store, the list widget and
the selection model, and it is the single most-cited line in this document.

Opening a synchronised message requires no network. Postio is one process, not
a browser engine per window.

---

## 19. Visual design

A beautiful native application, not a terminal-inspired one, and not something
that looks like *"a GTK developer made an email client"* — a premium
application that happens to be built with GTK.

Generous typography, real spacing, subtle hierarchy, restrained colour,
excellent dark *and* light mode, minimal chrome, very good message rendering.

**The design canvas is the authority on visual detail** — `Design/Mail
Client.dc.html`, direction **PLATE (1b)**. From the Industry design system Postio
keeps the *identity*: Barlow / Barlow Condensed / IBM Plex Mono, steel accent
`#5980a6`, hairline dividers, airy 40px rows, an accent-tinted selected row with
a 3px left border. It drops the *wireframe chrome*: no blueprint registration
marks, no transparent line-drawing cards. Real Adwaita window chrome, so it
reads as a GNOME application.

Tokens are **generated from the design system, never retyped**
(`ARCHITECTURE.md` §10).

---

## 20. Accessibility is first-class

Not a later pass. Keyboard navigation, screen readers, high contrast, reduced
motion, scalable text, focus indicators, semantic controls.

Two consequences that are easy to miss and are the ones that actually bite:
colour is never the only carrier of meaning, and every command is reachable
without the mouse *by construction*, because §8's registry is what generates
the accessible actions.

---

## 21. Privacy and security

Email is the most sensitive thing on most people's machines, and mail is
attacker-controlled content that actively tries to phone home. The commitment
is one sentence: **nothing leaves this machine that the user did not ask for.**

- Remote images and tracking pixels are blocked until the user allows them,
  **per sender**. Never a global default-on.
- **Read receipts are never sent automatically.** `Disposition-Notification-To`
  is tracking with a friendly name.
- `List-Unsubscribe` One-Click fires only on deliberate activation — sending it
  confirms to a spammer that the address is live.
- No link prefetch, no favicon fetch, no speculative connections. The reader's
  WebView has JavaScript off and network off; `cid:` images resolve from the
  local blob store.
- Replies and forwards carry nothing outward: quoted content is sanitised on
  the way in and the outgoing body is generated from Postio's own types, so a
  forwarded phishing mail cannot make a recipient run what its own user was
  protected from.
- No telemetry, no crash reporting, no update ping.
- **The local store holds the whole mailbox, and it is encrypted.** §14's
  backfill means this machine ends up with a complete copy of the user's mail
  rather than a recent slice, which is exactly why
  [ADR 0014](decisions/0014-encryption-at-rest.md)'s at-rest encryption is a
  requirement and not a nicety. A consequence worth stating plainly rather than
  burying: a backup of `$XDG_DATA_HOME` is a backup of the entire mailbox, and
  the key that opens it lives in the keyring, not beside it. Losing the keyring
  entry costs a re-sync; it never costs mail.
- Credentials in the OS keyring; TLS wherever the server offers it.
- **Logs never carry message content** — no bodies, subjects or recipient
  addresses, at any level. Ids, counts and outcomes.

When adding anything that could make a network request, the question is not
"is this useful" but **"did the user ask for it"**. If the answer is no, it does
not ship. `scripts/checks/check-no-silent-tracking.py` refuses the two mechanisms that
are easiest to add by accident; `ARCHITECTURE.md` §11 is the enforcement story.

Phishing and link warnings, PGP and S/MIME are unscheduled.

---

## 22. Architecture

See [`ARCHITECTURE.md`](ARCHITECTURE.md), which describes what is actually
built and why, and [`decisions/`](decisions/) for the long-form arguments. The
original brief's sketch — one `postio-core` holding the domain model, search,
commands and state — is not what the workspace became, and keeping a second
drawing of it here would be a picture that is wrong.

---

## 23. What v1 is, and what it is not

**In:** one IMAP + SMTP account with an app-specific password; inbox, folders,
threads; read/unread, archive, delete, flag, move; HTML and plaintext reading
with attachments and quoted-message folding; compose, reply, reply-all,
forward, attachments, drafts; local FTS5 search with operators and an instant
search box; vim-style navigation, a command palette and configurable shortcuts;
SQLite, background sync, offline reading, undo.

**Out, deliberately:** Rules. Contacts management.
Snooze and scheduled send. **And AI** — a founding principle, deferred so that
core mail, search and the keyboard land excellently first. Shipping AI over a
mediocre mail client would produce a mediocre mail client with AI in it.

Each of these has an issue and an ADR; none of them is forgotten.

---

## 24. After v1

The [Postio Roadmap](https://github.com/users/dlapiduz/projects/2) is the list,
grouped into epics with real sub-issue links. It is a plan rather than a queue,
which is why roadmap issues are not labelled `ready`.

A second copy of it here would be out of date within a week.

---

## 25. The workflow this is all for

```
/     invoices from Acme that I haven't responded to
```

Postio finds them. `Enter` opens the conversation. `a` archives it.

Or `Ctrl+K → Summarise`:

> Acme is waiting for approval of the $42,300 invoice. They sent the revised
> invoice on 19 August. You have not responded.

Then `e` opens the reply, a suggested draft appears, the user edits it, and
`Ctrl+Enter` sends.

**The whole of that without reaching for the mouse.** That is what makes Postio
more than another Thunderbird alternative: an application designed around
navigating and acting on a huge stream of information quickly, rather than
around displaying a folder.

Everything in this document either serves that workflow or is a constraint that
keeps it honest.
