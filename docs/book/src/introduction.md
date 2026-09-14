# What Postio is

Postio is a local-first, keyboard-first email client built for people who
have too much email.

**Read less. Find anything. Act faster.**

If you've used a mail client that makes you wait — for the inbox to load,
for search to come back, for a click to register — Postio is built to never
do that. It keeps a complete copy of your mail in an encrypted local store
with a built-in search index, so opening the app, searching, and moving
around never touch the network. Every action you take — archive, flag, move,
delete, snooze, undo — applies instantly to that local copy and is sent to
the server in the background. You are never staring at a spinner waiting for
your own mailbox to respond.

## The three things it has to be better at

**Speed.** Startup, navigation, and search are held to a real budget —
under half a second to a usable inbox, under 100 ms for a search — and the
work behind each budget is counted in the test suite, not just claimed.

**Search.** Search isn't a box in the corner; it's a primary way to move
through your mail. `from:ada after:2026-01-01 has:attach` is a query you can
type, save, or pin as a folder — one language, everywhere it appears.

**Keyboard.** Every action has a shortcut. `j`/`k` move, `e` replies, `a`
archives, `u` undoes anything, `/` searches, `Ctrl+K` opens the command
palette, `?` shows the full cheat sheet. The mouse works too, and is never
required.

## What 0.4 does

Several accounts with one unified inbox, over IMAP and SMTP, the Gmail API,
or JMAP, signed in with a password, an app-specific password, or OAuth 2
through your browser. Folders, threads and labels. Read/unread, archive,
delete, flag, move, snooze. HTML and plain-text reading, attachments,
quoted-message folding. Rich-text compose, reply, reply-all, forward,
drafts, scheduled send, and an outbox that never sends twice. Local
full-text search with operators, saved searches pinned as folders, and a
suggestion when a query finds nothing. An address book with groups, grown
from the people in your mail. Vim-style navigation and a command palette,
every binding rebindable. Background sync with IDLE, full offline use, and
undo.

## What it deliberately doesn't do yet

Postio 0.4 is an alpha. It runs on Linux (GTK4/libadwaita); a native macOS
frontend over the same engine reads mail today but is not yet released.
Filters and rules are designed and not built. There are no AI features —
not because they aren't planned, but because shipping AI over a mediocre
mail client would just produce a mediocre mail client with AI in it. Core
mail, search, and the keyboard come first. PGP/S-MIME, phishing warnings and
vCard import are also still to come, each with its own tracked issue.

Ready to try it? See [Installing Postio](install.md).

This is the reference documentation. The [Postio home page](../) is
the wider tour: what it looks like, what it is for, and where the
project stands.
