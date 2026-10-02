# The store migrates what it can and starts over what it cannot (2026-10-01, spec 007 T215)

**The constraint:** a change to `postio_storage::schema::HEAD` comes with a
`Migration` from the stamp it replaces, and the replaced `HEAD` is kept
verbatim in `crates/postio-storage/tests/schemas/<stamp>.sql`. A store whose
stamp no migration leads from is refused with `Remedy::StartOver`, and no
surface offers "Try again" for it.

## What happened

`feature/postio-focus` was rebased onto `main`, which brought three indexes
(`idx_messages_body_state`, `idx_messages_body_problems`,
`idx_operation_queue_state`). `HEAD` changed, so its fingerprint changed, and
every store the branch had written was refused at open with
`SchemaFromAnotherBuild`. Focus drew that sentence under "Try again" -- which
re-read the same file and was refused again, forever. There was no way out
but deleting the store by hand, and no way to close the window either (T216).

The store was not unversioned: `user_version` already held an FNV-1a hash of
`HEAD`, checked at every open since 2026-09-17. It only ever answered
"equal or not". There was nothing that said what to do when not.

## The decision: migrate, and start over only where no migration can exist

**Migrations, chained by stamp.** `schema::MIGRATIONS` is a list of
`{ from, to, statements }`. `from` and `to` are the fingerprints the stamp
already records, so no second version number has to be remembered and the
stamps on every store already written stay meaningful. `Store::open` runs
the chain from the found stamp to `FINGERPRINT`, each statement idempotent
(`IF NOT EXISTS`), and stamps after: a store cut off midway runs them again.
The first step is the rebase's three indexes, from `3f95ddb1` (the branch at
355ac0cd) to `d8c1e5df`.

Two tests hold it. `schema::tests::a_schema_change_comes_with_the_migration_that_reaches_it`
fails the moment `HEAD` changes without a step reaching the new stamp, and
says what to write. The storage suite's `migrations` case builds a store from
every recorded schema, opens it, and requires the result to be shaped exactly
as a fresh store (tables by column, indexes and triggers by statement) with
its operation queue intact.

**Starting over, for what no step reaches.** A store older than the first
recorded step, or from a build that is not this one's ancestor (a `main`
build's store, before Focus's tables, changes `messages` in ways `ALTER
TABLE` cannot express), is refused with `Remedy::StartOver`.
`postio_session::start_over` then:

- moves the database, its sidecars and `blobs/` into
  `set-aside/<when>/` beside them -- renamed, not deleted, and still under
  the keyring's key, so it can be moved back;
- refuses while another Postio has the store open;
- opens a fresh store and copies `accounts`, `identities` and `signatures`
  across by their shared columns, so the next start syncs rather than asking
  for every account again (passwords never moved: they are in the keyring by
  address);
- never touches `config.toml`.

What stays behind is what exists only in the store: snoozes, reminders,
Focus's filing history and digests, and drafts and changes the server has
not heard about yet. Focus's page says so before the button is pressed.

Two ways to reach it: Focus's "Start a fresh store" on the page, and
`postio-store reset` (`cargo run -p postio-session --bin postio-store --
status|reset`; `scripts/run-isolated.sh HEAD --reset-store` for the scratch
store). `postio-tui` names the command in its refusal.

## Rejected

- **Reset only, no migrations.** `HEAD` is one `CREATE` file and the
  first answer was "there are no migrations, resync". That throws away the
  local-only state every time an index is added, which is the commonest
  schema change there is. CLAUDE.md's "no backwards compatibility" licenses
  not arguing about old shapes; it says in the same paragraph "still write
  the migration".
- **Diffing the live schema against `HEAD` and reconciling automatically.**
  It handles new tables and indexes for free, but a column needs its
  definition parsed out of a `CREATE TABLE` with comments and `CHECK`s, and a
  changed constraint cannot be reconciled at all -- so it would still need
  the start-over path, plus a parser whose failures are silent. An explicit
  step is a few reviewed lines and fails loudly in a test.
- **A hand-kept version number beside the fingerprint.** Two numbers for
  one fact; the hash is the one that cannot be forgotten.
- **Deleting the old store.** Setting it aside costs a rename and makes the
  action recoverable, which is why the page can offer it without a second
  confirmation.
- **Offering start over for `WrongStoreKey` too.** A replaced keyring entry
  can be restored, and starting over would then have been the wrong call;
  it keeps "Try again" until someone decides otherwise.
