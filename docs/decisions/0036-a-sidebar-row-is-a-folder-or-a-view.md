# ADR 0036 — A sidebar row is a folder or a view, and a view is never a destination

- **Status:** Accepted (2026-09-12). Holds for the macOS sidebar and for Focus, which has no sidebar (ADR 0043): its folders popover lists folders and the Outbox, and `g *` and `g z` reach Flagged and Snoozed.
- **Decision:** Every `MailboxRole` is a folder role or a view role, a property of the role and not something each reader infers. A view names no server folder, is never stored in `mailboxes`, and is never a destination.

## Rules
| Kind | Roles | Names a server folder | Storable | A destination |
|---|---|---|---|---|
| `Folder` | `Inbox` `Archive` `Sent` `Drafts` `Trash` `Junk` `Regular` `Flagged` | yes | yes | yes |
| `View` | `Snoozed` `Outbox` | no | no | no |

- A view has a name, a position and a count: no path, UIDVALIDITY, sync state or id.
- The split is whether RFC 6154 defines a `SPECIAL-USE` attribute, not whether the sidebar draws a synthetic row. `\Flagged` is defined (Gmail's "Starred"), so `Flagged` is a folder. The sidebar synthesises a Flagged row only for servers that advertise none.
- `MailboxRole::kind()` in `postio-model` is exhaustive with no fallback arm, so a new role is a compile error until classified.
- `MailboxRepository::create` and `update` refuse a `View` with a typed error that names the role.
- `check-view-roles-are-not-storable.py` fails when the SQL `CHECK` list and the `Folder` set disagree.
- No negative or sentinel `MailboxId` anywhere (`check-no-sentinel-mailbox-ids.py`). A view row is unassigned.
- View rows are built once in `postio-ui::sidebar`.
- Views share the unassigned id, so code must not identify a sidebar or popover row by its id. Carry the row, or a type that says folder or view. The id decides folder-ness: a real `\Flagged` folder has both an id and the Flagged role.
- `ListScope::mailbox()` answers `None` for a view.

## Why
- Sentinels travel like real ids and fail silently: `InMailbox { mailbox: -1 }` matches nothing and reports success. A frontend that never knew the convention (macOS) had no Flagged or Snoozed rows.
- The `CHECK` once excluded `Snoozed` only by accident of what existed that day.
- Rejected: a distinct `SidebarRow` type (a much larger diff for three call sites), documenting the sentinels harder.

## Revisit when
- Saved searches become views of this kind.
