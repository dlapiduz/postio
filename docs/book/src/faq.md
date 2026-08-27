# FAQ

## Is Postio ready to use as my daily mail client?

Postio is pre-release and under active development. v1 supports a single
IMAP + SMTP account with a password or app-specific password. If that
covers your setup and you're comfortable building from source, it's usable
today — but treat it as early software, and keep your existing client
around until you're confident in it.

## Why Linux only?

v1 targets GTK4/libadwaita on Linux because that's where the team could
build something excellent fastest, not because other platforms are ruled
out. The engine underneath the UI has no GTK in it and no SQLite in the
view layer — that boundary is enforced automatically, specifically so a
macOS or Windows frontend over the same engine stays possible later.
Neither is currently scheduled.

## Does Postio support multiple accounts?

Multiple accounts are in scope for Postio but not yet built. Today's v1
is single-account.

## Does Postio support OAuth (Gmail, Outlook, etc.)?

OAuth 2 is in scope and being worked on, but v1 ships first with password
and app-specific-password authentication. If your provider requires
OAuth, wait for that support to land, or use an app-specific password if
your provider offers one (Gmail and iCloud both do).

## Why no AI features yet?

Deliberately, not accidentally. Postio's founding bet is that a mail
client has to be excellent at the fundamentals — speed, search, and
keyboard control — before AI has anything worth being layered onto.
Shipping AI over a mediocre mail client would just produce a mediocre mail
client with AI in it. AI is planned for after v1, with two constraints
already fixed before a line of it is built: it must never silently modify
or send mail, and every design has to treat mail as attacker-controlled
text an AI agent could be tricked by.

## How does search work?

Locally and fast — a full-text index built on your own machine, never a
server-side search. One query language works everywhere it shows up: typed
in the search bar, saved to the sidebar as a named search, or pinned as a
virtual folder. `from:ada after:2026-01-01 has:attach` is the kind of query
you can type, and results begin appearing as you type it.

## What happens if I lose access to my keyring?

Postio stores your mail credentials in your OS keyring and encrypts your
local mail store with a key that also lives there — never in a plain
config file. If the keyring entry is lost, you lose the local copy and
need to re-sync from the server: annoying, but you don't lose any mail,
since the server is still the source of truth.

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

The project's GitHub issue tracker. See the repository's
`CONTRIBUTING.md` for how to file an issue that's actionable.

## Is my data ever sent anywhere Postio doesn't tell me about?

No. See [Privacy and security](privacy.md) for the specifics — remote
images, read receipts, unsubscribe links, telemetry, and this documentation
site itself are all covered.
