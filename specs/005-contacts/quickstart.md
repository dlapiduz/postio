# Quickstart: validating Contacts

How to prove the feature works end to end, story by story. Commands run from
the feature worktree. Test names are the ones tasks.md will create; a test is
the proof, the manual walk is the sanity check.

## Prerequisites

- The feature worktree (`~/src/postio-worktrees/contacts`), sccache warm.
- No network needed at any step — which is itself SC-007.

## Fast loop

```bash
scripts/test-fast.sh                                  # changed crates, --lib
cargo nextest run -p postio-storage --test storage_suite contacts
cargo nextest run -p postio-vcard
cargo nextest run -p postio-search --test parse
cargo nextest run -p postio-index --test index_suite with_filter group_filter
cargo nextest run -p postio-gtk --test gtk_suite contacts
cargo test -p postio-app --test app_suite contacts
```

## Story by story

| Story | Proof | What a person would see |
|---|---|---|
| 1 — the screen | `app_suite::contacts_screen` | `g c` from the inbox shows people from the seeded mail; typing `ada` narrows; `Esc` returns to the same message, same selection and scroll (the assertions `gtk_composer_detach.rs` makes, repeated for Contacts) |
| 1 — default view | `storage_suite::contacts_list_views` | people written to are listed; a newsletter sender only under "Everyone from mail", yet still completes in the composer |
| 2 — join | `app_suite::contacts_join` | two people become one; completion on "ada" shows one name, two addresses; `u` restores both, each with its own counts |
| 2 — names everywhere | `app_suite::contact_name_in_list_and_reader` | after naming the joined person, the message list and reader show the name for both addresses; the reader's details still show the raw header; mail from an unowned address claiming the name shows its header |
| 3 — create/edit/delete | `storage_suite::contacts_lifecycle` + `app_suite::contacts_delete_restore` | a created person completes at once; a deleted mail person stays gone after another message; restore from the Deleted view returns them whole |
| 4 — suggestions | `storage_suite::contact_suggestions` | same-name pair offered; dismissed stays dismissed after more mail and after a join on either side |
| 5 — groups | `app_suite::contact_groups` + `index_suite::group_filter` | picking a group fills preferred addresses; `group:family` finds mail through any member address |
| 6 — vCard | `postio-vcard` corpus round trip + `app_suite::contacts_import_export` | a 3.0 file imports as people with several addresses and a group; export is 4.0 and every unmodelled property is intact |

## Budgets (Principle V — counts, not timings)

```bash
cargo nextest run -p postio-storage --test storage_suite contacts_budget list_statement_count
```

- A contacts page on a 20,000-person store: one statement, rows ≤ page size,
  no `SCAN` (`counting::scans`) — for each of the three views and for a
  filtered page.
- Recording a message: statements per address within the stated ceiling
  (research R2).
- A message-list page: statement and row counts unchanged by name
  substitution (research R7).

## Looking at it

```bash
scripts/run-isolated.sh --shot      # a pinned build, throwaway store; not while others build
```

The `/gtk-design` render loop renders the Contacts pane to PNG in light and
dark, wide and narrow; compare against the canvas tokens.

## Invariants

```bash
scripts/check.sh                    # includes the postio-vcard boundary rule
POSTIO_UPDATE_DOCS=1 cargo test -p postio-ui   # regenerates docs/keybindings.md after command changes
```
