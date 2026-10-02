# A POSTIO_LOG filter that hid the error it was set to find (2026-09-07, #1176)

#1176 asks for a log, and gives the command to collect it:

```
POSTIO_LOG=postio_sync=debug,postio_runtime=debug,postio_account::imap=debug postio
```

Run against the macOS application, that produced **an empty file**. Not a
short one — zero bytes, through two launches, while the application was on
screen with a window. The session had not opened, and the log that exists to
say why said nothing.

## Why

`POSTIO_LOG` is an `EnvFilter`, and a filter made only of per-target
directives sets the default for **every other target to off**. The three
targets named were the ones the issue cared about; the line that mattered was

```
ERROR postio_session: cannot open the store ... error=the local store was
written before the page MAC changed and this build cannot read it
```

in `postio_session`, which was not named. So the filter was working exactly as
specified and suppressing the only thing worth reading. Nothing was broken and
nothing was buffered — the answer was being discarded at the filter.

`postio_ffi::logging`'s own `tracing::info!("postio starting")` was suppressed
by the same rule, which is the tell: **if a `POSTIO_LOG` run does not start
with `postio starting`, the filter is eating everything and the run has not
begun to tell you anything.** That line is `info` in `postio_ffi`, so any
filter with a bare level in it shows it and any filter without one does not.

## What to do instead

Put a bare level first, then narrow:

```
POSTIO_LOG=info,postio_sync=debug,postio_runtime=debug,postio_account=debug
```

`info` is the floor for everything unnamed, and the targets after it are
raised above it. That is the shape every `POSTIO_LOG` in an issue or a
runbook should have, and the three that do not are worth fixing when touched.

## The unrelated thing it was hiding

Worth recording separately because the diagnosis took three launches to reach
and the error itself is already good: a store written before Postio moved its
SQLCipher page MAC from HMAC-SHA512 to HMAC-SHA256 cannot be read, and
`Error::StorePredatesPageMac` says so in those words, including what to do.
Pre-v1 there is no migration by design — the store is a cache of the server,
so it is rebuilt by resyncing. Deleting it also means deleting `blobs/`, which
is addressed by rows in the database and is otherwise several hundred orphaned
files.

It also means **local drafts are lost**, since those are not on the server.
That is not an argument for a migration here — no build can read the file, so
they were already unreachable — but it is the sentence to say out loud before
deleting somebody's store, rather than "no mail has been lost".
