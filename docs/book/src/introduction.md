# What Postio is

Postio is a local-first, keyboard-first email client built for people who
have too much email.

**Read less. Find anything. Act faster.**

If you've used a mail client that makes you wait — for the inbox to load,
for search to come back, for a click to register — Postio is built to never
do that. It keeps a full copy of your mail in a local database with a
built-in search index, so opening the app, searching, and moving around
never touch the network. Every action you take — archive, flag, move,
delete, undo — applies instantly to that local copy and is sent to the
server in the background. You are never staring at a spinner waiting for
your own mailbox to respond.

## The three things it has to be better at

**Speed.** Startup, navigation, and search are held to a real budget —
under half a second to a usable inbox, under 100ms for a search — and it's
checked automatically, not just claimed.

**Search.** Search isn't a box in the corner; it's a primary way to move
through your mail. `from:ada after:2026-01-01 has:attach` is a query you can
type, save, or pin as a folder — one language, everywhere it appears.

**Keyboard.** Every action has a shortcut. `j`/`k` move, `e` replies, `a`
archives, `u` undoes anything, `/` searches, `Ctrl+K` opens the command
palette, `?` shows the full cheat sheet. The mouse works too, and is never
required.

## What v1 does

One IMAP + SMTP account, authenticated with a password or an app-specific
password. Inbox, folders, threads. Read/unread, archive, delete, flag,
move. HTML and plain-text reading, attachments, quoted-message folding.
Compose, reply, reply-all, forward, drafts. Local full-text search with
operators. Vim-style navigation and a command palette, every binding
rebindable. Background sync, full offline reading, undo.

## What it deliberately doesn't do yet

Postio is Linux-only for now (GTK4/libadwaita), and v1 has no AI features —
not because they aren't planned, but because shipping AI over a mediocre
mail client would just produce a mediocre mail client with AI in it. Core
mail, search, and the keyboard come first. Rules, contacts management, and
snooze/scheduled send are also out of v1, each with its own tracked issue.

Ready to try it? See [Installing Postio](install.md).

This is the reference documentation. The [Postio home page](../) is
the wider tour: what it looks like, what it is for, and where the
project stands.
