# io-imap discards all but the last untagged SEARCH line

*2026-09-17*

`io-imap` 0.6.0 returns the wrong answer for a `UID SEARCH` whose result the
server splits across several untagged responses. It keeps only the last line,
and a final line carrying no numbers throws the whole result away. Postio hit
this against iCloud, where it made large mailboxes look empty and recorded
them as fully synchronised.

## The defect

`src/rfc3501/search.rs`, in the coroutine's completion arm:

```rust
let mut ids = Vec::new();
for data in out.data {
    if let Data::Search(search_ids, _) = data {
        ids = search_ids;          // assignment, not extend
    }
}
```

A server may answer one SEARCH with several untagged `* SEARCH` lines, and
servers do that for results too large for one line. Each assignment discards
the previous line.

`src/rfc5256/sort.rs` (`ids = Some(sort_ids)`) and `src/rfc5256/thread.rs`
(`threads = Some(t)`) are written the same way. Postio calls neither yet; if
that changes, check them first.

0.6.0 is the latest release as of this date — there is no version to upgrade
to, and no ESEARCH support in the crate to sidestep it with (`RETURN (ALL)`
would answer in one line, but io-imap has no RFC 4731 module).

## What it looked like

Syncing a real iCloud account:

```text
the server listed fewer UIDs than it says the mailbox holds
  mailbox=2   listed=0  exists=60934    Archive
  mailbox=13  listed=0  exists=20324    Sent Messages
  mailbox=7   listed=0  exists=830      Deleted Messages
```

INBOX's 173 messages fit on one untagged line and synced correctly, which is
why this looked mailbox-specific rather than size-specific at first.

An empty listing is indistinguishable from an empty mailbox, so
`initial::enumerate` fetched nothing, *completed*, and `complete_full_sync`
stamped `last_full_sync_at`. Every later pass then planned `Incremental`
against a mailbox that had never been enumerated. The mail was not slow to
arrive; it was unreachable until the store was deleted.

## Reproduced

`crates/postio-account/tests/io_imap_search_defect.rs` drives the real
coroutine against a canned response:

```text
* SEARCH 1 2 3 / * SEARCH 4 5 6   ->  [4, 5, 6]
* SEARCH 1 2 3 / * SEARCH         ->  []
```

Those tests assert what io-imap **currently does**, not what Postio wants.
When a future version fixes it they will fail, and that failure is the signal
to delete them along with the workarounds below.

## The second path to the same symptom

`io-imap` also drops any untagged response it cannot *decode*, completing the
command `Ok` — see `imap::skip_counter` and ADR 0001. A `* SEARCH` line that
fails to parse removes its UIDs just as silently. Both paths end in a short
listing, so both are guarded.

## What Postio does about it

Three layers, because the failure is silent and each catches a different part:

1. **`imap::fetch::existing_uids` brackets the SEARCH with the skip counter**,
   exactly as `fetch_batch` already did for `CHANGEDSINCE` fetches. A skipped
   untagged response means the listing is missing whatever that line carried.
2. **`existing_uids` refuses a listing shorter than `EXISTS`** — the count the
   same `SELECT` just reported. Both refusals are transient, so
   `ConnectionPool::execute` discards the connection.
3. **`initial::enumerate` re-checks the listing against `EXISTS`** and walks
   the UID space instead, which is the path a refused SEARCH has taken since
   #727. This is the backend-independent net: it holds for any `MailBackend`,
   not just IMAP.

Only the short direction is refused. A listing *longer* than `EXISTS` is mail
delivered between the `SELECT` and the `SEARCH`, and rejecting it would churn
connections on every busy mailbox.

`sync::resync` separately re-enumerates a mailbox that is still short of
`EXISTS` after a pass, which is what repairs a store already damaged by this.

## Still open

Whether iCloud splits its SEARCH results, sends a trailing empty line, or
sends something io-imap cannot decode. The three guards above make the
distinction moot for correctness, but an upstream report wants it, and
`POSTIO_LOG=io_imap=trace` on a large mailbox would settle it. Note that the
trace carries raw wire bytes, so it holds real mail and must be redacted
before it leaves this machine.
