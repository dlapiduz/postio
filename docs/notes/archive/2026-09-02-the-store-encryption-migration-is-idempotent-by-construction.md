# The store encryption migration is idempotent by construction, and that is the whole design

*Archived 2026-09-14: describes `postio_storage::encrypt`, the plaintext-to-SQLCipher swap; the engine is Turso since ADR 0038, the module is gone and there is no plaintext store left to migrate.*

`postio_storage::encrypt` builds the encrypted store
beside the old one, verifies it by reading every referenced blob back through
its own AEAD, and only then moves the originals aside and the replacements in
— deleting the plaintext copy last. A swap is several renames and no
filesystem call does several at once, so what makes it safe is that re-running
the same guarded sequence from any interruption point converges. Two rules
hold it up, and both are easy to break by accident:

- **Every original moves aside before any replacement moves in.** The window
  where the store path holds *neither* half is the only one a resume can read
  unambiguously. An entry that is present is then either untouched plaintext
  (its aside copy is missing) or the finished encrypted one (its aside copy is
  there). Interleaving the moves reintroduces a state where a plaintext blob
  directory sits beside an encrypted database and nothing can tell.
- **The aside directory is created only after the staged store verifies**, so
  its existence is what says "a swap began" and finishing forward is always
  right. There is no case that puts the plaintext store back.

The first version of this passed every test but one: `database_parts` was
handed the database *file* as its root, so the plaintext database never moved
aside and the staged one was renamed straight over it. Every test that only
checked the mail afterwards passed, because the mail was fine — the one that
caught it stops mid-swap and asserts the plaintext copy is still on disk. A
migration test that only looks at the end cannot see the window it is about.
