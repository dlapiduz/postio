# ADR 0035 — Mailbox roles are mapped per account, in the store, and chosen in settings

- **Status:** Accepted (2026-09-04)
- **Decision:** Postio keeps fixed local role mailboxes (Inbox, Archive, Sent, Drafts, Trash, Junk). Each account has its own map from role to one of its server folders, in the store beside the account, edited in the account's settings, effective at once without the network. `[mailboxes]` in `config.toml` stays as the default under every account's map.

## Rules
- Table `mailbox_roles (account_id, role, path)`, primary key `(account_id, role)`: one folder per role. It is keyed by path, not mailbox id, so it survives a folder vanishing and returning. A path the server no longer lists is dangling: shown, never dropped. `mailbox_role_refusals` records a role the server refused so the retry is not repeated on a timer.
- Precedence per account: account map, then `[mailboxes]`, then `SPECIAL-USE`, then the settled name guess (`resolve_roles`: server claim, depth, path). No provider knowledge in the tie-break (#959).
- Discovery builds `RoleOverrides` on every pass from the table and config, applies the user tiers on top of the settled verdict, and guarantees at most one selectable row per role per account. Pinning takes the role from its previous holder, whose role is re-derived from what the server said.
- `MapMailboxRole { account, role, path: Option<String> }` is a registry command, local-first: write or delete the row, re-role rows in one transaction, emit `Event::MailboxesChanged { account }`. Undo is the previous entry. Mail is never moved.
- No per-account key in `config.toml`. An address in the file would break `check-no-personal-data` and reopen ADR 0005's key question.
- Settings, Accounts, account, Mailboxes: one `gtk::DropDown` per mappable role (not Inbox). Entry 0 is "Automatic (<current resolution>)". Subtitle: empty when automatic, "chosen by you" when mapped, "not on this server" when dangling. Empty state: "Folders appear after the first sync". No loading state (local read). Works offline.
- The presenter (`postio_widgets::present::settings::accounts`) reads mailboxes through the client, because `postio-widgets` cannot see `postio-storage`.

## Why
- A server can list two folders the guess accepts, and the alphabet breaks the tie wrongly (#943). Only the user has the missing information. The global `[mailboxes]` table fixes iCloud and breaks Gmail.
- An account is state, not preference (ADR 0005 Q6b), and a role map is a fact about one server.

## Revisit when
- Presets carry folder names for silent servers.
- Postio offers to CREATE a missing folder.
