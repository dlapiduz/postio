# ADR 0027 — Mailbox roles are mapped per account, in the store, and chosen in settings

- **Status:** Accepted (2026-09-04)
- **Numbered 0027, not 0025.** It was drafted as 0025 on this branch while
  ADR 0025 (arbitrary headers are indexed rows) was landing on `main`, so
  two accepted decisions would have shared a number and "ADR 0025" would
  have meant one thing in `postio-model/src/headers.rs` and another in
  `postio-core/src/registry.rs`. Renumbered when the branch was finished
  rather than left for a reader to disambiguate. #967.
- **Date:** 2026-09-03
- **Decision by:** the maintainer, on the question raised while reproducing
  [#943](https://github.com/dlapiduz/postio/issues/943): a live iCloud
  account had two folders wearing every role, and the one Postio filed
  into was the one another client had created.
- **Issue:** [#962](https://github.com/dlapiduz/postio/issues/962), delivered
  by its five children in order: [#963](https://github.com/dlapiduz/postio/issues/963)
  (the store's per-account map),
  [#964](https://github.com/dlapiduz/postio/issues/964) (discovery builds the
  overrides on every pass), [#965](https://github.com/dlapiduz/postio/issues/965)
  (`MapMailboxRole`, local-first and undoable),
  [#966](https://github.com/dlapiduz/postio/issues/966) (the Mailboxes rows in
  the account detail view) and
  [#967](https://github.com/dlapiduz/postio/issues/967) (this documentation).
- **Related:** ADR 0005 Q6b (an account is state, not preference),
  [#164](https://github.com/dlapiduz/postio/issues/164) (`[mailboxes]`),
  [#501](https://github.com/dlapiduz/postio/issues/501) (one sidebar row
  per role), [#880](https://github.com/dlapiduz/postio/issues/880) (the
  account detail view), [#959](https://github.com/dlapiduz/postio/issues/959)
  (the tie-break has no provider knowledge), `docs/engineering-notes.md`
  "Re-pointing a mailbox role relabels folders; it never moves mail"
- **Decision:** Postio keeps a fixed set of local role mailboxes — Inbox,
  Archive, Sent, Drafts, Trash, Junk — and **each account carries its own map
  from role to one of its server folders.** The map lives in the store beside
  the account, is edited from the account's settings, and takes effect at
  once without a network round trip. The global `[mailboxes]` section stays
  as the default under every account's map. Discovery reads the map on every
  pass and guarantees one folder per role.

---

## The question

"Which folder is the archive" has three answers on a real server, and they
disagree:

| Tier | Who says | Example |
|---|---|---|
| The user | `[mailboxes]` in `config.toml` (#164) | `archive = "Archive/2024"` |
| The server | RFC 6154 `SPECIAL-USE` | `\Sent` on `Sent Messages` |
| A guess | `MailboxRole::guess_from_name` | `Archive`, `Archives`, `Archived` |

The guess is good — it already knows iCloud's `Sent Messages`, Outlook's
`Sent Items`, Gmail's `Bin` — and the server's word beats it. What #943 found
is that a server can list *two* folders the guess accepts (`Sent` and
`Sent Messages`) and, when it declares nothing, the tie is broken by the
alphabet. Every layer then agrees, on the wrong folder. The user's tier is the
only one with the missing information, and today it has two defects:

1. **It is global.** One `[mailboxes]` table applies to every account, so
   `sent = "Sent Messages"` fixes iCloud and strips the Sent role from a
   Gmail account on the same installation, which then cannot send.
2. **It is unreachable.** Nothing in settings shows it, and it is read once
   at startup.

## Where the map lives: the store, not the file

ADR 0005 Q6b retired `[accounts.<id>]` from `config.toml` because an account's
identity is a database id, not a TOML key, and settled that **an account is
state, not preference**. A role map is a fact about one account's server —
the same kind of thing as its host and its credential — so it goes where the
account goes.

- A table, `mailbox_roles (account_id, role, path)`, primary key
  `(account_id, role)`. One folder per role by construction.
- Keyed by **path**, not by mailbox id: the map is what the user said about
  the server, and it must survive the row being retired and re-created when a
  folder vanishes and comes back. A path the server no longer lists is a
  *dangling* mapping, shown as such in settings rather than silently dropped.
- `[mailboxes]` in `config.toml` keeps working, as the layer beneath: an
  installation with one account loses nothing, and a hand-edited file still
  says what it always said. It is documented as "every account, unless the
  account says otherwise". No new per-account key is added to the file.

The alternative — `[mailboxes."ada@example.com"]` keyed by address — was
rejected: it re-opens the ADR 0005 question with a different key, it puts an
address in a file `check-no-personal-data` cannot reason about, and it makes
the settings pane a TOML patcher for a table whose identity lives elsewhere.

## Precedence, per account

```
account map  →  [mailboxes]  →  SPECIAL-USE  →  name guess, settled
```

The last tier is *settled*: `resolve_roles` (backend) already picks one
folder per role by the server's claim, then depth, then path, and after #943
discovery applies the user tiers **on top of that verdict** rather than
re-deriving the role from the name. Pinning a role to a folder takes it away
from whatever held it before — the rule from #164 — so the invariant after
every pass is: **at most one selectable row per role, per account.**

Discovery builds the account's `RoleOverrides` on every pass from the table
plus the config section. Nothing is frozen at startup any more; the engine's
`mailbox_roles` part becomes the config tier only.

## Changing the map is a command, and it is local-first

"Map Sent to *Sent Messages*" is a verb with a payload, so it is a registry
command (`MapMailboxRole { account, role, path: Option<String> }`) rather
than a widget-local write — one verb, one meaning, from the pane and from the
palette. Its action:

1. Writes the table row (or deletes it, for "Automatic").
2. Re-roles the account's rows in the same transaction: the chosen path gets
   the role, every other row wearing it goes back to `Regular`, and the
   previous holder's role is re-derived from what the server said about it so
   a demoted `Sent Messages` is not left `Regular` when it was the server's
   own choice.
3. Emits `Event::MailboxesChanged { account }` so the sidebar relabels.

No sync pass, no network, no restart. Discovery agrees on its next pass
because it reads the same table. Recovery is `Undo`: the previous map entry
is the inverse, and a wrong pick costs one keystroke rather than a dialog.
Mail is never moved (the #164 rule stands).

## The pane

Settings → Accounts → an account → **Mailboxes**, inside the account detail
view #880 is building, because that view is the one answer to "where do
per-account settings live". One row per mappable role (Inbox is RFC 3501's
and is not offered), each a `gtk::DropDown` in the existing settings row
pattern:

```
Sent      [ Automatic (Sent Messages)  ▾ ]
Archive   [ Archives                    ▾ ]   chosen by you
Trash     [ Automatic (Deleted Messages)▾ ]
Junk      [ Automatic (Junk)            ▾ ]   not on this server
Drafts    [ Automatic (Drafts)          ▾ ]
```

- Entry 0 is always **Automatic**, and its label says what automatic
  resolves to right now, so the user sees the guess before overriding it.
  The composer's signature picker reserves entry 0 the same way.
- The rest are the account's selectable folders, by path, in listing order.
- The subtitle is the state: nothing when automatic; "chosen by you" when
  mapped; "not on this server" when the mapped path is dangling, which is
  #943's acceptance criterion "surface unmapped roles proactively" met in the
  one place a person can fix it.
- The six states: **empty** (no folders discovered yet — "Folders appear
  after the first sync"; the account may be new or never connected);
  **loading** does not exist (local read); **partial** is the automatic
  label showing "none" for a role the server has no folder for; **offline**
  works fully (the write is local); **failing** is unaffected; **dense**
  follows the settings rows' density.
- Keyboard: the dropdown is a native GTK control; the pane is reached the
  way every settings pane is. No new context, no bare-letter bindings while
  the config text view is in the same panel (the `Context::Accounts`
  reasoning holds).

The mailbox list crosses the crate boundary the way the account list does —
`postio-app` reads `MailboxRepository::list_for_account` and pushes it into
the panel — because `postio-gtk` cannot see `postio-storage`.

## What this does not decide

- **The tie-break without provider knowledge** (#959). The map is the escape
  hatch; presets carrying folder names is the durable answer for silent
  servers, and rides the same `RoleOverrides` seam beneath the user tiers.
- **Creating a folder the server lacks.** A role with no folder is shown, not
  fixed; whether Postio should offer to CREATE one is a separate question.

## Children

1. Storage: `mailbox_roles` table, repository, migration 0006.
2. Discovery: per-account overrides built from table + config on every pass;
   the engine's part becomes the config tier.
3. Core/session: `MapMailboxRole` command, action, undo, event.
4. GTK/app: the Mailboxes rows in the account detail view, the setter, the
   app host, the app-suite wiring test.
5. Docs: `docs/config.md` says `[mailboxes]` is the default under every
   account; keybindings table regenerated.
