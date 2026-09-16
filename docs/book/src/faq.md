# FAQ

## Is Postio ready to use as my daily mail client?

Postio 0.4 is an alpha. It supports several IMAP, Gmail or JMAP accounts
with a password, an app-specific password or OAuth 2, and covers the
everyday work — reading, searching, replying, filing, snoozing, undo — with
full offline use. If that covers your setup it's usable today, but treat it
as early software: keep your existing client around until you're confident
in it, and expect to file the rough edges you find.

## Why Linux only?

v1 targets GTK4/libadwaita on Linux because that's where the team could
build something excellent fastest, not because other platforms are ruled
out. The engine underneath the UI has no GTK in it and no database code in
the view layer — that boundary is enforced automatically — and a native
macOS frontend over the same engine already reads mail, searches and pages.
It is built and tested but not yet released. Windows is not scheduled.

## Does Postio support multiple accounts?

Yes. Each account is synced by its own engine, and the unified inbox groups
their threads together at read time, so a slow or unreachable server never
holds the others back. `g a` switches between an account and the unified
view; `account:` scopes a search to one of them. Add an account with
`Ctrl+Shift+N`.

## Does Postio support OAuth (Gmail, Outlook, etc.)?

Yes. Providers that require OAuth 2 — Gmail and Microsoft 365 among them —
open your system browser to sign in; the token goes into your keyring and
Postio re-authenticates on its own when a refresh grant expires. Providers
that offer app-specific passwords (iCloud, Fastmail, Gmail too) work with
those as well.

## Why no AI features yet?

Deliberately, not accidentally. Postio's founding bet is that a mail
client has to be excellent at the fundamentals — speed, search, and
keyboard control — before AI has anything worth being layered onto.
Shipping AI over a mediocre mail client would just produce a mediocre mail
client with AI in it. AI is planned for after v1, with two constraints
already fixed before a line of it is built: it must never silently modify
or send mail, and every design has to treat mail as attacker-controlled
text an AI agent could be tricked by.

## Can I make rules that file mail as it arrives?

Not yet. Rules are designed — the same search language you type in the
search bar, reused as the condition — and not built. Saved searches are:
`Ctrl+S` on a search pins it to the sidebar as a folder that re-runs when
you open it.

## How does search work?

Locally and fast — a full-text index built on your own machine, never a
server-side search. One query language works everywhere it shows up: typed
in the search bar, saved to the sidebar as a named search, or pinned as a
virtual folder. `from:ada after:2026-01-01 has:attach` is the kind of query
you can type, and results begin appearing as you type it. When a query
finds nothing, Postio suggests the spelling that would.

"All mail" means every folder except drafts, junk and trash — the three a
search almost never means, and the ones that turn "66 hits" into a number
nobody can act on. Sent is included. Name one of the three with `in:`
(`in:trash invoice`) and the search reaches it.

## What happens if I lose access to my keyring?

Postio stores your mail credentials in your OS keyring and encrypts your
local mail store with a key that also lives there — never in a plain
config file. If the keyring entry is lost, you lose the local copy and
need to re-sync from the server: annoying, but you don't lose any mail,
since the server is still the source of truth. Drafts and messages waiting
in the outbox are the exception, so send or save them elsewhere before
resetting a keyring.

## Is Postio really written by AI?

Yes — Postio is written by AI coding agents under a human maintainer who
sets scope, reviews the results, and makes the product calls. It isn't a
disclaimer so much as the actual experiment behind the project: not
whether an agent can write code, but whether a *process* — test-driven
development, machine-checked invariants, and a public issue tracker as the
paper trail — can make agent-written software trustworthy. Read the code
with the same skepticism you'd give any project, and if you find something
wrong, the issue tracker is exactly where that gets fixed.

## Where do I report a bug or request a feature?

The project's [GitHub issue tracker](https://github.com/dlapiduz/postio/issues).
See the repository's `CONTRIBUTING.md` for how to file an issue that's
actionable. Postio's logs never contain message content, so
`POSTIO_LOG=debug` output is safe to attach — read it before you paste it
anyway.

## Is my data ever sent anywhere Postio doesn't tell me about?

No. See [Privacy and security](privacy.md) for the specifics — remote
images, read receipts, unsubscribe links, telemetry, and this documentation
site itself are all covered.
