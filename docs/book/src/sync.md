# How sync works

Postio keeps a complete local replica of your mail: a SQLite database for
everything listable and searchable, plus a content-addressed blob store for
raw messages and attachments. Every screen you look at — the inbox, a
thread, a search result — is read from that local copy, never fetched live
from the server. That's what makes the app instant: there's nothing to wait
for.

## What happens in the background

A sync engine, running separately from anything you're looking at, keeps
that local copy up to date:

- **New mail arrives quickly.** Where the server supports IMAP `IDLE`,
  Postio holds a connection open on your inbox for push delivery. Where it
  doesn't, Postio polls at an interval you can configure.
- **Message text backfills to completion, not just the newest few
  hundred.** Every message in every folder you haven't excluded eventually
  gets its full text pulled down, in the background, so search and offline
  reading cover your whole mailbox — not just what's recent. You can
  exclude a folder explicitly; nothing is excluded by default.
- **Attachments are lazy.** Attachment bytes are typically nine-tenths of a
  mailbox by weight and contribute nothing to search but their filename, so
  they download when you open or save one, not proactively. (Small inline
  images used for rendering HTML mail are the exception — those come with
  the text, so HTML mail reads correctly offline.) You can ask Postio to
  fetch attachments eagerly if you want a complete offline archive, or turn
  attachment fetching off entirely on a metered connection.
- **A dropped connection reconnects on its own**, backing off if the server
  or network keeps failing, and picks up where it left off.

## Every action is instant, and queues afterward

When you archive, delete, flag, move, or send something, Postio doesn't
wait for the server before showing you the result. The sequence is always:
write to the local database, add the change to a queue, update what you
see — in that order, and all of it happens before anything touches the
network. The sync engine drains that queue in the background and
reconciles with the server afterward.

That's also why undo is instant: pressing `u` reverses the local change
right away, without waiting for a round trip. A burst of actions — say,
archiving twelve messages in a row — counts as one undoable unit, not
twelve.

## Fully usable offline

Because reading, search, and every mutating action work against the local
copy first, Postio works fully offline after its first sync: read, search,
compose, reply, forward, archive, delete, move, label, and mark
read/unread all work with no connection. It's not a special "offline mode"
— it's the same code path either way, which is exactly why it's reliable.
Anything you do offline queues locally and reconciles automatically once
the connection comes back.
