# Contract: Contacts commands

Command ids are a file format (`[keys]` in `config.toml`, Principle II), so
this table is the contract; titles and bindings may be tuned in review,
**ids may not** once landed. Every row is a `command_ids!` entry
(`postio-core/src/command.rs`), a `SPECS` row in `registry.rs` in the same
order, a `menu.rs` section, and is either handled by the window or listed in
`WIRED` (`postio-session/src/actions.rs`) — `command_wiring.rs` fails
otherwise.

## The context

`Context::Contacts` — appended to `Context::ALL`. List-scoped: entered when
the contacts list or detail has focus, left when it loses it. Key layering
`[Contacts, Global]`.

**One context, dispatch on the focused row.** The registry scopes commands
by `Context` only; it has no notion of which row inside a surface has focus.
So the Contacts surface is one context, and the rows it shows have a kind —
person, address (in the detail), group, suggestion. A command acts on the
focused row's kind where that kind gives it a meaning (`contact_delete` on a
group row deletes the group; `contact_join` on a suggestion row joins that
pair) and on the *selection* where it acts on several people. A command with
no meaning for the focused row does nothing and shows a one-line hint naming
the row it wants. This keeps one binding per verb, which is what
`bindings_do_not_collide_within_a_context` requires, and needs no new
`Context` variants (research R10). It must be added to every exhaustive match:
`postio-ui/src/keymap.rs` (`KeyContext`, its layering, `From<Context>`),
`postio-ui/src/cheatsheet.rs::heading`, `postio-ffi/src/registry.rs`
(`UiContext`, both `From`s, the round-trip test),
`postio-ui/tests/ui_suite/keybindings_doc.rs::where_available`, and
`postio-ui/src/focus.rs`'s non-pane list.

## Commands

`R` = `Recovery`. `D` = destructive. Bindings are proposals checked by
`bindings_do_not_collide_within_a_context`.

| id | Title | Binding | Contexts | D | R | Spec |
|---|---|---|---|---|---|---|
| `open_contacts` | Contacts | `g c` | the `g` destinations' surfaces (list, conversation, reader, search, folder list) — not the composer, where `g` is a letter | | None | FR-001 |
| `contact_new` | New contact | `n` | Contacts | | Undo | FR-020 |
| `contact_edit` | Edit contact | `e` | Contacts (person) | | Undo | FR-021 |
| `contact_delete` | Delete | `d`, `Delete` (the message list's own `d`) | Contacts (person or group) | ✓ | Undo | FR-023, FR-040 |
| `contact_restore` | Restore contact | `r` | Contacts (person in the Deleted view) | | Undo | FR-023a |
| `contact_join` | Join | `m` | Contacts (selection, or a suggestion) | | Undo | FR-012/13, FR-019 |
| `contact_add_address` | Add address | `+` | Contacts | | Undo | FR-015 |
| `contact_detach_address` | Detach address | `-` | Contacts (address) | | Undo | FR-014 |
| `contact_set_preferred` | Use this address first | `*` | Contacts (address) | | Undo | FR-016 |
| `contact_show_mail` | Show mail | `Return`, `o` | Contacts (person or group) | | None | FR-007 |
| `contact_compose` | Write to | `c` | Contacts | | None | FR-007 |
| `contacts_filter` | Filter contacts | `/` | Contacts | | None | FR-004 |
| `contacts_toggle_everyone` | Everyone from mail | `v e` | Contacts | | None | FR-005 |
| `contacts_toggle_deleted` | Deleted contacts | `v d` | Contacts | | None | FR-023a |
| `contacts_suggestions` | Possible duplicates | `v s` | Contacts | | None | FR-019 |
| `suggestion_dismiss` | Not the same person | `X` | Contacts (suggestion) | | None¹ | FR-019 |
| `contact_group_new` | New group | `g n` | Contacts | | Undo | FR-040 |
| `contact_group_rename` | Rename group | `R` | Contacts (group) | | Undo | FR-040 |
| `contact_group_add` | Add to group | `l` | Contacts | | Undo | FR-040 |
| `contact_group_remove` | Remove from group | `L` | Contacts | | Undo | FR-040 |
| `contacts_import` | Import vCard | `v i` | Contacts | | None² | FR-050 |
| `contacts_export` | Export vCard | `v x` | Contacts | | None | FR-050 |

¹ A dismissal is reversible by nothing but is not destructive (it hides a
hint, not data), so the registry rule does not apply.
² Import only adds; its summary says what it joined (research R9).

Movement follows the precedent `Context::Accounts` and `Context::Keys` set:
the list's own arrow keys walk it, because `next_message`/`prev_message` would
print "Next message" under a Contacts heading. What Contacts borrows are the
commands that mean the same thing on a person as on a message:
`toggle_selection` `x`, `extend_selection_*` `J`/`K`, `select_all`, `back` and
**`undo` (`u`)** gain `Context::Contacts` in their context sets rather than
being duplicated — cursor and selection stay distinct (Principle II): `x`
marks, `m` joins the *selection*. `back` (`Esc`) closes the screen (FR-001).
`undo` is scoped to message surfaces and Accounts otherwise, and every
contacts edit is undone with it (FR-025).

Every command has a default binding (Principle II;
`every_command_has_an_id_a_title_and_a_default_binding`). 22 commands.

## Invocations (`Command` payloads)

Handled in `postio-session/src/actions.rs`, local-first: one store
transaction, then `Event::ContactsChanged`, then the undo entry.

```text
CreateContact   { name, addresses: [EmailAddress], organization, note }
EditContact     { id, name, organization, note }
JoinContacts    { into: ContactId, others: [ContactId], name, organization }
UnjoinContacts  { into, restore: [(ContactId, [AddressId])], prior, added_memberships }   // inverse only
AddAddress      { id, address: EmailAddress }          // fails with Owner(ContactId) if owned elsewhere
MoveAddress     { address: AddressId, to: ContactId }  // the "move it" answer, and the detach inverse
DetachAddress   { address: AddressId }                 // → new person
SetPreferred    { id, address: AddressId }
DeleteContact   { id }  /  RestoreContact { id }
DismissSuggestion { a: ContactId, b: ContactId }
Group: CreateGroup { name } / RenameGroup { id, name } / DeleteGroup { id } / RestoreGroup {…}
       AddMembers { group, contacts } / RemoveMembers { group, contacts }
ImportVcard     { path }   ExportVcard { path, selection: Option<[ContactId]>, view }
```

## Event

`Event::ContactsChanged { names_changed: bool }`. Subscribers: the contacts
list (refetch visible page), the finder (reload), completion (drop cache), and
— when `names_changed` — the message list and open conversation (refetch
visible page, FR-032).

## Undo kinds

`UndoKind::{CreateContact, EditContact, JoinContacts, AddAddress, MoveAddress,
DetachAddress, SetPreferred, DeleteContact, RestoreContact, GroupEdit}`, each
with toast text in `describe` ("Joined 2 contacts", "Deleted Ada Lovelace" —
the name comes from the payload, never a log line).
