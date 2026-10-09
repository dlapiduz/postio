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
type, save, or pin — one language, everywhere it appears.

**Keyboard.** Every action has a shortcut. `j`/`k` move, `e` replies, `a`
archives, `Ctrl+Z` undoes anything, `/` searches, `Ctrl+K` runs any command,
`?` shows the key map. The mouse works too, and is never required.

## What the desktop app looks like

One dense inbox, newest first, that shows mail as it arrived: sender,
subject and first line, verbatim. A message opens over the list, one at a
time, and `Esc` puts you back on the same row; if you prefer, `F8` opens it
beside the list instead. There is no folder sidebar: `g o` lists every
mailbox, folder and label, and the command bar goes anywhere.

Beside reading, writing and filing mail, it does four things to it. It
calls out real actions on the row — an invitation to accept or decline, a
direct question, a to-do — quoting the sentence that raised it. It holds
some mail back into digests on a cadence you choose. It files spam and
automated updates into Filtered, each with its reason and one key from
restored. And it captures a message into an Obsidian vault as a task or a
note. A local model you run yourself can write digest summaries and spot
questions and to-dos; without one, a built-in detector does the second.

## What 0.4 does

Several accounts with one unified inbox, over IMAP and SMTP, the Gmail API,
or JMAP, signed in with a password, an app-specific password, or OAuth 2
through your browser. Folders, threads and labels. Read/unread, archive,
delete, flag, move, snooze. HTML and plain-text reading, attachments,
quoted-message folding. Rich-text compose, reply, reply-all, forward,
drafts, scheduled send, and an outbox that never sends twice. Local
full-text search with operators, pinned saved searches, and a suggestion
when a query finds nothing. An address book with groups, grown from the
people in your mail. Vim-style navigation and one command bar, every
binding rebindable. Background sync with IDLE, full offline use, and
undo.

## What it deliberately doesn't do yet

Postio 0.4 is an alpha. It runs on Linux (GTK4/libadwaita); a native macOS
frontend over the same engine reads mail today but is not yet released.
Filters and rules are designed and not built. Beyond the optional local
model above there are no AI features — not because they aren't planned, but
because shipping AI over a mediocre mail client would just produce a
mediocre mail client with AI in it. Core mail, search, and the keyboard come
first. PGP/S-MIME, phishing warnings and vCard import are also still to
come, each with its own tracked issue.

Ready to try it? See [Installing Postio](install.md).

This is the reference documentation. The [Postio home page](..) is
the wider tour: what it looks like, what it is for, and where the
project stands.
