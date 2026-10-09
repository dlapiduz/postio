# Contract: `MailBackend::create_mailbox`

**Implementors**: `postio-account::imap::ImapBackend`, `MockBackend`,
`postio-gmail`, `postio-jmap`.
**Caller**: `postio-sync::discover` — and nothing else.

## Why it is new

The trait (`crates/postio-account/src/backend/mod.rs:107`) has no way to create,
rename, delete or subscribe to a mailbox. FR-027 needs one.

**Pimalaya already has it**, so no wire code is written: `io-imap` `=0.6.0`
implements RFC 3501 `CREATE` (`src/rfc3501/create.rs`, exposed at
`client.rs:559`). The constitution asks for that survey before protocol code,
and this is its answer.

## The method

```rust
/// Creates `path` on the server, and subscribes to it where the protocol
/// separates the two.
///
/// Idempotent from the caller's point of view: a server that reports the
/// mailbox already exists is success, not failure — two clients may race,
/// and the caller wants the folder to exist, not to have been the one to
/// make it.
async fn create_mailbox(&self, path: &str) -> BackendResult<()>;
```

**No default implementation.** A backend that cannot create a folder must say so
explicitly — a default returning `Ok(())` would report success for something
that never happened, and a default returning `Unsupported` would let a new
backend forget to answer. `postio-gmail` and `postio-jmap` return `Unsupported`
today, which is the existing shape for capabilities those adapters lack.

## What the caller guarantees

`postio-sync::discover` is the only caller, and it calls this **only** when:

1. a **reserved** role (`MailboxRole::RESERVED`) resolves to no folder, after
   every tier has been tried — the account's own map, `[mailboxes]`,
   `SPECIAL-USE`, the name guess;
2. the role is **not `Inbox`** (FR-029) — RFC 3501 names that folder and every
   server has it; one that does not is reported, not repaired;
3. no refusal is recorded for that account and role;
4. discovery is already connected. Nothing here opens a connection of its own,
   and nothing blocks the UI on it (FR-032).

**The name comes from the role or the provider preset table, never a constant**
(FR-030). Providers are data, not code — a branch naming one provider's folder
is exactly what Principle VII forbids.

## What a refusal means

A server may refuse: no permission, the name taken by a non-selectable node, the
hierarchy forbidding it. On refusal the caller:

- leaves the role **unmapped and visible as unmapped** rather than omitting the
  row (FR-031);
- records the refusal and the server's own words in `mailbox_roles`, so the user
  is told what the server said rather than a guess;
- **does not try again on the next pass.** A server that will never allow a
  folder must not be asked forever;
- clears the record when the map changes or the folder appears.

A refusal is never fatal to discovery: the other roles resolve, the pass
completes, and the account stays usable.

## Privacy

This is the one place in the feature where something leaves the machine that the
user did not individually ask for. The bounds above — reserved roles only, never
the Inbox, only when nothing resolves, once — are the containment the maintainer's
2026-09-11 decision was taken under, and they are tested, not assumed.

Logs record the account id, the role and the outcome. **Never the folder name a
server rejected and never its message verbatim in a log** — the user sees the
server's words in the settings pane, which is not a log.

## Tests

Against `MockBackend`, with no network anywhere:

1. A server listing only `INBOX` ends discovery with one selectable mailbox per
   reserved role, and exactly one `create_mailbox` call per missing role.
2. A server with every role already resolving issues **zero** calls — and a
   second pass over case 1 also issues zero (SC-007, FR-028).
3. `Inbox` is never created, even when absent.
4. A refusal leaves the role unmapped-and-shown, records the reason, and the
   next pass makes no second attempt.
5. A refusal for one role does not stop the others resolving.
6. "Already exists" is treated as success.
7. The created name comes from the role or the preset table — asserted by
   running two different presets and seeing two different names, which is what
   a named constant could not do.
