# The Postio Architecture Manual

### For someone who has never written Rust and has never built a desktop app

---

## How to read this

This document explains one real, working piece of software — Postio, a local-first
email client — from the ground up. It assumes nothing: not what a "crate" is,
not what Rust is for, not what makes a program feel like a native desktop app
instead of a website in a window. Every concept is introduced before it is
used.

It is built around one true story that runs through the whole thing: Postio
chose a specific way of writing its database to disk called Write-Ahead
Logging, or **WAL**, for very good reasons — and then spent weeks discovering
what that choice actually cost, including two real crashes that only showed up
after the database was encrypted. That story is Part Five. Everything before
it is the background needed to understand why the story is interesting, and
everything after it is the sibling story — how the database got encrypted at
all, and what encryption cost. If you only read one section, read Part Five.
But the earlier parts make it make sense.

A note on evidence: everything below is drawn from the actual code, commit
history, architecture decision records, and a long-running engineering
journal this project keeps for exactly this reason — so that lessons learned
the hard way don't have to be relearned. Specific numbers (milliseconds,
megabytes, issue numbers) are measured, not estimated, and are cited by their
GitHub issue number so a curious reader can find the original discussion.

---

## Part One: What Postio is, in one paragraph

Postio is an email program for Linux desktops. Its pitch is "read less, find
anything, act faster" — it keeps a complete copy of your mail on your own
machine, indexes it for instant search, and makes every action (archive,
flag, delete, undo) feel immediate because it never waits for the network to
respond before updating what you see. The network happens quietly, in the
background, and catches up later. That single design choice — local first,
network second — is the gravitational center that almost every other decision
in this document orbits.

One more thing worth knowing up front, because it explains why this document
exists at all: Postio is written almost entirely by AI coding agents (Claude,
running as many parallel sessions) directed by a human maintainer who sets
scope and makes product calls. Every change starts as a GitHub issue, gets
built test-first on its own private copy of the code, and only merges after
an automated gate — tests, linting, architectural rule-checking, a scan for
accidentally-leaked personal data — all pass. As of this writing the project
is six days old and has 840 commits. The reason a document like this one can
be written accurately at all is that the project keeps an unusually thorough
paper trail of *why* — architecture decision records, an engineering-notes
journal, issue discussions — precisely because the people (and agents) doing
the work know they won't remember, and the next session (human or AI) needs
to be able to pick up the reasoning rather than rediscover it.

---

## Part Two: The tools underneath — Rust, Cargo, and crates

### What "Rust" is, and why it's relevant that Postio is written in it

Rust is a programming language. Like C or C++, and unlike Python or
JavaScript, it's **compiled**: before the program runs, a separate program
(the *compiler*, called `rustc`) translates the human-written source code
into machine code that the processor can execute directly. This is different
from an *interpreted* language, where another program reads and executes your
source code line by line while the program runs. Compiled programs generally
start faster and run faster, because none of that translation work happens
while the user is waiting.

What makes Rust distinctive is a design goal: get the speed and control of a
language like C++, without its most common category of bug. Programs written
in C or C++ can accidentally read or write memory they don't own anymore — a
freed piece of memory, an array past its end, a piece of data being read by
one thread while another thread is changing it. These bugs are notoriously
hard to find (they often don't crash immediately, or only crash on someone
else's machine) and are the single largest source of security
vulnerabilities in software history. Rust's compiler tracks, for every piece
of data, who is allowed to read it and who is allowed to change it, and it
refuses to compile a program where those rules could be broken — before the
program ever runs, not by crashing while it runs. This is why the project's
top-level build configuration contains the line `unsafe_code = "forbid"`: it
is telling the compiler to refuse to compile *any* code, anywhere in the
eighteen packages that make up Postio, that opts out of this checking —
except in two narrowly justified places (interfacing with C libraries, which
by definition weren't written under Rust's rules). For a program that parses
untrusted data arriving over the network — and email, from strangers, is
about as untrusted as data gets — that guarantee is not a nicety. It is a
big part of why memory-corruption bugs, historically one of the most
dangerous classes of vulnerability in mail clients, are structurally much
harder to introduce here.

### What a "crate" is

In Rust, a unit of code that can be compiled and shared is called a **crate**.
It's the same idea as a "package" in Python (`pip install requests`) or a
"module" in JavaScript (`npm install express`) — a crate is a folder of
source code plus a manifest file (`Cargo.toml`) describing its name, version,
and what other crates it depends on. A crate can produce a *library* (code
other crates can call into) or a *binary* (a standalone program you can run),
or sometimes both.

Postio is not one crate. It is **eighteen** of them, living side by side in
one repository, each with a narrow, named job — `postio-model`,
`postio-storage`, `postio-search`, `postio-gtk`, and so on. Part Three walks
through what each one is for and, more importantly, *why* the program is cut
into pieces this way rather than written as one big crate. For now, the thing
to hold onto is: a crate is a compilation unit and a dependency boundary, and
Postio uses that boundary deliberately, as an enforcement mechanism, not just
as a filing system.

### What Cargo is

**Cargo** is Rust's build tool and package manager, bundled with the
language itself — the equivalent of `npm` for JavaScript or `pip` combined
with a build system for Python. Three things Cargo does that matter here:

- **It resolves dependencies.** When a crate says "I need library X, version
  2-point-something," Cargo works out a consistent set of versions for every
  dependency of every crate, downloads them, and records the exact versions
  it chose in a lockfile (`Cargo.lock`) so a build is reproducible on another
  machine.
- **It builds.** `cargo build` compiles a crate (and everything it depends
  on) into a runnable program or library. `cargo run` builds and immediately
  runs it.
- **It tests.** `cargo test` compiles and runs a crate's test suite — code
  written specifically to check that other code behaves correctly. This
  matters a lot for how Postio is built, covered in Part Seven.

### What a "workspace" is

When several crates are meant to be developed and built together — which is
Postio's situation, since the eighteen crates all belong to one application —
Cargo lets you declare a **workspace**: one top-level `Cargo.toml` that lists
every member crate, shares one lockfile across all of them, and lets you
build or test the whole set (or any subset) with one command. It's the same
idea as a "monorepo" in other ecosystems — many packages, one repository, one
coordinated build.

Postio's workspace-level `Cargo.toml` does something else worth noting: it
sets rules that apply to *every* crate in the workspace at once — the
`unsafe_code = "forbid"` line above is one; a rule requiring every public
item to be documented is another. This is deliberate: those rules used to be
repeated by hand at the top of each crate's code, and they drifted — some
crates forgot to forbid `unsafe`, some forgot to require documentation. A
rule stated once, at the workspace level, cannot drift, because there is only
one place it could be stated.

---

## Part Three: What a desktop app is made of

Postio isn't a website. It's a **native application** — a program that draws
its own windows, buttons, and lists directly through the operating system's
graphics stack, rather than rendering HTML in a browser. This section
explains the pieces that make that possible on Linux, because the rest of the
document assumes you know roughly what they are.

### The toolkit: GTK4

A **GUI toolkit** is a library that provides the building blocks of a graphical
program — windows, buttons, text fields, lists, menus — and a **main loop**, a
continuously-running piece of code that waits for things to happen (a
keypress, a mouse click, a timer firing, data arriving) and dispatches each
one to the right piece of your program. Every desktop and mobile app is
built on some toolkit: Windows apps typically use WinUI, Mac apps use
AppKit/SwiftUI, and Linux desktop apps in the GNOME family — which Postio
targets — use **GTK4**, paired with **libadwaita**, a companion library
that supplies GNOME's specific visual language (its color tokens, its
window chrome, its standard widgets like the header bar) on top of GTK's more
general-purpose building blocks.

Because the whole GUI toolkit runs on one thread with one main loop, a
program that does something slow — like waiting on a network request —
directly on that thread will freeze the entire interface until it's done: no
redraws, no responding to clicks, nothing. This constraint is central to why
Postio is built the way it is, and comes back throughout this document as
"the UI never awaits the network."

### Rendering mail: WebKitGTK

Email arrives as HTML — fonts, colors, embedded images, sometimes deliberately
obfuscated tracking pixels — and rendering arbitrary HTML safely needs a real
browser engine, not a toolkit's basic text widget. Postio embeds
**WebKitGTK**, the same rendering engine that powers Safari, as a component
inside its own window, but locked down hard: JavaScript is switched off,
network access is switched off (so a tracking pixel or a "load remote image"
request can't silently phone home), and images referenced by
`cid:` (a scheme meaning "this image is one of the file attachments in this
same email, not something to fetch off the internet") are read from Postio's
own local storage rather than the network. The rest of Part Six discusses why
this posture — treating a message's own content as fundamentally
untrustworthy — matters so much that it's stated as a top-level architectural
invariant, not a feature toggle.

### Talking to the rest of the desktop: D-Bus and portals

Modern Linux desktop apps, especially sandboxed ones (Postio ships as a
Flatpak, a sandboxed package format), don't get to reach directly into the
filesystem or other running programs. Instead they talk to trusted system
services over **D-Bus**, a message-passing system built into the desktop, and
specifically through **portals** — narrow, permission-gated APIs for things
like "let the user pick a file" or "let the user drag a file out of this app
into another one." One of the case studies later in this document (dragging a
file out of Postio) turned on a subtlety of exactly how one of these
portals — `org.freedesktop.portal.FileTransfer` — actually moves data, which
is a good small example of how much devil lives in these details.

### The pieces assembled

Put together: Postio is a GTK4/libadwaita program (the window, the widgets,
the keyboard handling), with an embedded, locked-down WebKitGTK view for
rendering mail content, talking to the desktop shell through D-Bus portals
when it needs to, all compiled from Rust source organized as an
eighteen-crate Cargo workspace. The next part explains why eighteen crates,
and what each one does.

---

## Part Four: Postio's shape — the crate map

### Why split one program into eighteen crates at all?

The short answer: **so that a machine, not a person's memory, can enforce
which parts of the program are allowed to know about which other parts.**

Here's the concrete problem this solves. Postio's core promise is that it can
grow a second frontend someday — a native Mac app, say — without rewriting
the mail engine underneath. (This isn't hypothetical: as of the most recent
measurement, a native macOS frontend is scheduled, and thirteen of Postio's
fifteen core crates already build and test on macOS with *no changes at
all*.) For that promise to hold, the code that talks to SQLite and the code
that talks to GTK have to be genuinely separable — not "separable if everyone
remembers to keep them apart," but separable in a way a computer can check on
every single change. Rust's crate boundary is what makes that checkable: a
crate can only use code from crates it explicitly depends on, and Cargo will
tell you exactly what any crate's dependency list is. Postio adds a script,
run on every landed change, that inspects the *actual resolved dependency
graph* (not just what a comment claims) and fails the build if, say, the GTK
frontend crate ever ends up depending — even indirectly, through some other
crate — on the SQL library or the mail-protocol library. A rule like "the
view layer must not do its own SQL queries" stops being a convention someone
has to remember and becomes a fact a computer verifies every time.

### The crates, grouped by role

Picture four layers, each one only allowed to depend on the layers below it:

**The pure domain layer** — no networking, no SQL, no GTK, nothing but data
types and the logic that belongs to them:

- **`postio-model`** — the shared vocabulary every other crate speaks:
  what a `Message` is, what a `MailboxRole` is (inbox, archive, sent,
  trash...), how message threads get reconstructed from `In-Reply-To` and
  `References` headers (an algorithm called JWZ threading, named for its
  inventor Jamie Zawinski). Almost every other crate in the workspace depends
  on this one, which is exactly why it's kept so deliberately free of
  outside dependencies — the whole workspace waits for it to compile.
- **`postio-search`** — the query language parser: turning what a user types
  into a search box (`from:alice after:monday "quarterly report"`) into a
  structured, typed representation. It knows nothing about SQL or how a
  query actually gets executed — that's a different crate's job (see
  `postio-index`, below). This split matters: it's what lets the *same*
  parsed query be reused for a live search, a saved search, a sidebar
  "virtual folder," or eventually a filter rule — one matching language,
  used four different ways, rather than four different matching languages
  that could quietly disagree.
- **`postio-body`** — a message's readable content, in both directions:
  turning raw HTML mail into a restricted, safe internal representation on
  the way in (this is the "sanitizing" step — stripping scripts, remote
  images, anything dangerous), and generating outgoing HTML from that same
  safe representation on the way out, so a reply can never accidentally
  smuggle out a sender's malicious script.

**The contract layer** — the seam between "what the application does" and
"what it looks like," deliberately forbidden from ever depending on GTK:

- **`postio-core`** — the UI-agnostic core: a fixed vocabulary of
  `Command`s the interface can send ("archive this message," "mark this
  read") and `Event`s that come back ("this message changed," "sync
  finished"), a single master table of every command that exists (the
  *registry* — more on this below), and the undo system.
- **`postio-config`** — reading and validating `config.toml`, the user's
  settings file, including live-reloading it while the app runs.

**The engine layer** — "the database half": everything that talks to a
server or to disk:

- **`postio-storage`** — the SQLite schema, database migrations, and a
  separate content-addressed store for raw message bytes and attachments
  (the "blob store"). This is where Part Five and Part Six's stories live.
- **`postio-index`** — the full-text search index (built on a SQLite feature
  called FTS5) and the code that actually executes a parsed query against
  it.
- **`postio-imap`**, **`postio-jmap`**, **`postio-gmail`** — three different
  implementations of talking to a mail server, one per protocol (the
  traditional IMAP protocol; the newer JMAP protocol used natively by
  Fastmail; and Gmail's own REST API). All three implement the same trait
  (Rust's word for an interface — a fixed contract of what methods a type
  promises to provide) called `MailBackend`, so the rest of the program
  never has to know or care which protocol a given account actually speaks.
- **`postio-smtp`** — sending mail.
- **`postio-sync`** — the engine that keeps the local copy and the server in
  step: an operation queue, incremental resynchronization, and `IDLE` (the
  IMAP feature that lets a server push "you have new mail" instead of the
  client having to keep asking).
- **`postio-runtime`** — the piece that actually owns a live database
  connection and drives the sync engine over time — draining the operation
  queue, backfilling message bodies in the background, reconnecting after a
  dropped connection.

**The frontend layer**:

- **`postio-gtk`** — the actual GTK4/libadwaita widgets, the keyboard
  handling, the visual design tokens. Forbidden from touching SQL or a mail
  protocol directly, by the boundary rule described above.
- **`postio-ui`** — presentation logic that doesn't need a toolkit at all
  (selection rules, how a reader document gets assembled, keymap
  resolution) split out specifically so a second frontend — the planned
  macOS one — can reuse it instead of reimplementing it.
- **`postio-ffi`** — the boundary a non-Rust frontend (Swift, for the macOS
  case) actually calls into, using a cross-language binding generator called
  UniFFI.

**The composition roots** — the only two crates allowed to know that
*everything else* exists, because assembling the pieces is their entire job:

- **`postio-session`** — builds the local store, starts the background
  runtime, wires up the sync engines, and holds the "verb vocabulary" that
  turns a `Command` into actual rows changing in the database and events
  going out. Notably, this crate *also* has no GTK dependency, which is
  what makes it possible for a future non-GTK frontend to link against it
  directly.
- **`postio-app`** — the actual GTK binary you run. What's left here,
  deliberately, is as small as possible: a window, and the "presenters"
  that connect the two halves together. Each of those presenters names a
  specific widget, which is precisely the dividing line — anything that
  names a widget belongs in `postio-app`; anything that doesn't belongs in
  `postio-session`.

### The rule that makes the split real, not aspirational

It would be easy for this four-layer picture to be true on the day it's
drawn and false a month later, as convenience wins one small case at a time.
Two structural facts keep it honest. First, a script runs on every landed
change and inspects the actual, resolved dependency graph — not source
comments, not intentions — for two rules specifically: the contract layer
(`postio-core`) may never depend on GTK, and the frontend layer
(`postio-gtk`) may never depend on the SQL library or a mail-protocol
library. Second, and more subtly: Cargo's dependency *features* resolve as a
union across everything being built in one program — meaning if the
database code were merely an optional feature of the contract crate, turning
that feature on anywhere in the program would pull SQLite into the graph of
*every* crate depending on the contract crate, including the view layer,
whether or not the view layer wanted it. That's a real trap a less careful
design would fall into silently. It's the specific reason `postio-runtime`
(which owns a database) and `postio-app` (which owns GTK) are their own
separate crates rather than optional add-ons bolted onto `postio-core` — the
contract crate has *no* optional dependencies at all, precisely so nothing
can accidentally widen its graph.

---

## Part Five: Case study — the WAL saga

This is the story the rest of the document was building toward. It's a good
story because it has three acts, each one revealing something the previous
act's fix didn't actually solve, and because the final act — the database
getting encrypted — turned a setting that had been quietly correct for
months into something that made the program *segfault*.

### Act Zero: what a "write-ahead log" even is, and why Postio chose one

SQLite — the database engine Postio stores everything in — has more than one
way of guaranteeing that a crash or power failure can't corrupt your data.
The traditional way is a **rollback journal**: before changing the real
database file, SQLite copies the *old* version of whatever it's about to
change into a separate journal file, so a crash mid-write can be undone by
restoring from that copy. The problem with this scheme for an app like
Postio is that a **writer blocks every reader** — nobody can read the
database while a change is in progress, because the file itself is being
edited in place.

**WAL mode** (Write-Ahead Logging) inverts this. Instead of editing the main
database file directly, a writer appends its changes to a separate log file,
and readers keep reading the old, stable version of the main file until the
log is periodically folded back in. The practical consequence, and the whole
reason Postio uses it, is: **readers never block the writer, and the writer
never blocks readers.** For an application whose entire premise is "the
message list keeps scrolling instantly while mail syncs in the background,"
that property isn't an optimization — it's the mechanism the whole
architecture's central promise depends on. It's turned on with one line,
`PRAGMA journal_mode = WAL`, applied to every database connection Postio
opens.

What WAL mode does *not* give you is multiple simultaneous *writers*. SQLite
still allows only one writer at a time, WAL or not — WAL only frees up
readers. That single sentence is the seed of everything that follows.

### Act One: "the pool wasn't the bottleneck" (issue #425)

Once Postio was actually syncing real mailboxes, an odd, specific complaint
showed up: **archiving a single message took 1.8 seconds.** Not "syncing is
slow" — one keystroke, on one row, taking almost two full seconds to
register. And it wasn't the connection pool: Postio keeps a small pool of
database connections so multiple parts of the program can talk to SQLite
concurrently, and measurement showed the pool handed out a connection in two
*microseconds*. The pool was never waiting for anything.

The actual mechanism: during a first sync, the background sync engine is
committing batches of newly-downloaded messages back-to-back, with almost no
gap between finishing one transaction and starting the next. SQLite's answer
to "someone else is writing right now" is `PRAGMA busy_timeout`: when a write
collides with another write already in progress, the loser doesn't fail
immediately — it sleeps for a little while and tries again, backing off up
to a hundred milliseconds between attempts. This is a *retry loop*, not a
*queue*. There's no ordering to it and no fairness: every retry is a brand
new race against whatever else happens to be writing at that exact moment.
When one side of the race is a background sync engine writing constantly and
the other side is a single interactive keystroke, the keystroke can lose that
coin flip over and over, for as long as nearly two seconds, while the
database itself sat almost entirely idle the whole time.

Two fixes that looked obviously right were tried, and both were wrong, which
is worth recording precisely because they *looked* right:

- **A bigger connection pool** changed nothing, because the pool was never
  the contended resource — SQLite's single-writer rule is enforced far below
  the pool, inside the database file's own locking.
- **Making background write transactions smaller** helped almost not at all
  either: cutting each background transaction to an eighth of its previous
  size still left an interactive write taking half a second, because
  shrinking each race didn't reduce how *many* races there were to lose —
  it just meant losing more of them.

The fix that actually worked was to stop relying on SQLite's own
lock-collision retry entirely, for the writes that matter to a person
waiting on them, and build an application-level traffic light in front of
SQLite: `WriteGate`. It's a queue with two priority levels. The rule is
simple to state and precise in what it buys: a *background* writer is never
allowed to *begin* a new write while an *interactive* writer is waiting for
its turn — meaning a person's keystroke waits, at absolute most, for
whichever single background chunk of work happened to already be
in flight when they typed, not for an unbounded sequence of lost coin flips.
Making that bound tight also required capping how big one background "write
unit" is allowed to be (25 messages, roughly 8 milliseconds) — the gate on
its own would have let a keystroke wait behind a single enormous batch; the
size cap on its own was the "shrink the transactions" non-fix from above.
Both pieces were necessary; neither alone was sufficient. This is a good
example of a broader shape this project's engineering journal calls out
repeatedly: a fix that intuitively "should" work and doesn't is worth writing
down precisely, so the next person doesn't re-spend the time proving the same
wrong hypothesis wrong.

### Act Two: the transaction that took a lock it didn't need to (issue #79)

A related but distinct bug: certain background sync passes, under
concurrency testing, would intermittently lose their *first* batch of
writes entirely — not slowly, just silently gone. The cause was a subtlety
of how SQLite transactions actually acquire their lock. A transaction can be
started in **deferred** mode (the default) or **immediate** mode. A deferred
transaction takes no lock at the moment it starts — it only grabs one the
first time it actually reads or writes something, and if it starts by
*reading* (which almost every write path in Postio's storage layer does — you
look a row up before deciding whether to insert or update it), it grabs a
*read* lock first. The problem comes when that same transaction later tries
to *write*: it needs to upgrade its read lock into a write lock, and SQLite
refuses to let a connection wait for that upgrade, because doing so could
deadlock against exactly the writer it would be waiting on. So instead of
waiting (which is what `busy_timeout` is for), the upgrade attempt fails
outright, on the spot, without even asking whether it might have succeeded a
moment later.

The fix was to make every outermost write transaction start as `BEGIN
IMMEDIATE` instead of the deferred default — meaning it grabs the write lock
up front, honestly, before doing anything else, rather than discovering
partway through that it needs to fail. This sounds like a one-line change,
and the actual code change was small, but it had to happen in *two* separate
places that had grown independently — the general-purpose transaction helper
every repository in the storage crate goes through, and a couple of
sync-engine code paths that had, for their own reasons, opened their own
transactions directly rather than going through that shared helper. Each fix
was independently necessary; reverting either one on its own put the failing
test straight back to failing every single run. It's a small, precise
illustration of a recurring shape in real systems: a correct rule stated in
one place is not the same thing as a correct rule *enforced* in every place
that rule applies.

### Act Three: encrypting the database turns a quiet setting into a crash

This is the part of the story where two of this document's threads —
WAL and the encryption work covered in Part Six — collide directly, and it's
the best illustration in the whole project of a lesson worth stating plainly:
**a change can be entirely correct in isolation and still expose a bug that
was sitting there all along, waiting for exactly this combination of
conditions to occur.**

Once Postio's database was switched over to SQLCipher — meaning every page
of the database is now encrypted and decrypted through a cryptography
library (libcrypto) as it's written and read — a new and genuinely alarming
failure mode appeared: the application would occasionally crash with a full
memory-corruption coredump on ordinary shutdown. Not corruption of the mail
— a torn WAL frame from an interrupted write is exactly the kind of thing
WAL's own recovery mechanism exists to repair — but the *process itself*
dying on the way out, sometimes, unpredictably.

The mechanism, once traced with a debugger attached to the crashing thread,
was this: Postio's background sync engine ran on its own operating-system
thread, and the application had always simply *dropped* the handle to that
thread on exit rather than waiting for it to finish — reasoning, in a
code comment at the time, that the thread would be killed a moment later by
the process exiting anyway, so waiting for it seemed pointless. That
reasoning was true and harmless — right up until the database was encrypted.
Calling `exit()` on a process doesn't just kill every thread instantly; it
first runs the process's registered *exit handlers*, and libcrypto — the
cryptography library doing the actual page encryption — registers one of its
own that frees its internal state. If the sync thread was still mid-write at
that exact moment, encrypting a page as part of committing a transaction, it
could end up trying to use cryptographic memory that a *different* thread had
just freed out from under it, milliseconds earlier, as part of the very same
process shutdown. A coredump, not a hypothesis — reproducible on essentially
every run of the test suite's end-to-end test, and about one run in six of
a narrower engine test. The fix was to stop treating "the thread will die
soon anyway" as equivalent to "the thread has actually stopped": the engine
now keeps its thread handle and is explicitly joined — waited for — with a
bounded timeout, on every path that shuts the application down, rather than
silently abandoned.

A second, independent booby trap surfaced in the very same encryption work,
and it's a good demonstration that a setting can be *documented correctly and
still be wrong for a particular application*. SQLCipher offers a hardening
option called `cipher_memory_security`, which — reasonably, in isolation —
actively scrubs its internal cryptographic buffers by marking their memory
pages `PROT_NONE` (meaning "not accessible at all, to anyone, until further
notice") the instant it's done using them, so that leftover key material
can't linger in memory for something else to accidentally read later. That's
a legitimately good idea for a program with one connection doing one thing
at a time. Postio is not that program: it routinely has *two* database
connections writing concurrently — the background sync engine committing a
pass at the same moment the interface itself is writing a flag change — which
is the entire reason the `WriteGate` from Act One exists in the first place.
With this hardening setting turned on, one connection's cryptographic
cleanup could revoke access to a memory page that a *second* connection was
still in the middle of using to encrypt a different page, and the program
would segfault directly inside a WAL write. The fix here wasn't a redesign;
it was recognizing that a setting the original encryption plan had filed
under "performance tuning, adjust if a benchmark trips" was actually a
**correctness** setting for this specific, concurrent application, and
turning it off permanently, before it ever got the chance to be tuned.

One more small, sharp detail from the same stretch of work, because it's a
nice example of a failure that reports itself as the wrong problem entirely:
SQLCipher's `PRAGMA key` — the statement that actually tells the database
what encryption key to use — **cannot fail on its own.** It will silently
accept *any* key at all, correct or not, and the wrong key only reveals
itself later, elsewhere, the first time SQLite actually tries to read a page
and finds garbage where a database page should be — surfacing as the generic
SQLite error "file is not a database." That error message, reaching an
actual person's screen, tells them their mail is *corrupted*, when in fact it
is perfectly intact and simply locked behind the wrong key. The fix was to
deliberately force the failure to happen early and honestly: immediately
after issuing `PRAGMA key`, the code now reads one page on purpose, purely
to force SQLite to prove the key actually works before the rest of the
program can proceed, and translates a decrypt failure at that specific,
controlled moment into an explicit "wrong key" error rather than letting a
wrong key surface, unpredictably, as an apparently corrupted mailbox.

### What this saga adds up to

Three acts, three different mechanisms — an unfair retry loop, a lock that
couldn't safely wait for itself, a cryptography library's cleanup racing a
second thread it didn't know existed — and one common thread running through
all of them: SQLite's WAL mode gives Postio exactly the concurrency property
its whole design depends on (readers and writers never blocking each other),
and every single one of these bugs was concurrency finding a seam that
hadn't been tested under real, simultaneous load. None of them would have
been caught by a single-threaded test running against an empty, in-memory
database — and in fact, the project's own engineering notes separately
record that an in-memory SQLite database uses a *different* locking model
entirely from a real, file-backed WAL database, which makes it actively
unsuitable for testing exactly this class of bug. Every fix in this section
was found either by deliberately reproducing sustained concurrent load, or
by attaching a debugger to a coredump and reading the actual mechanism off
the stack — not by guessing.

---

## Part Six: Encrypting a mailbox — SQLCipher and what it costs

### The decision: SQLCipher, not filesystem encryption

Postio's stance is that relying on the operating system's own disk
encryption isn't enough — a mail client that promises privacy should encrypt
its own data, so that privacy holds even if the disk encryption is off,
misconfigured, or the threat is a backup or a synced copy of the data
wandering somewhere the user didn't intend. That decision — encrypt at rest,
in the application itself — was made deliberately by the maintainer as an
architecture decision (documented, with its full reasoning, as ADR 0014).

The database engine underneath Postio's SQLite usage is **SQLCipher**, a
well-established fork of SQLite that adds transparent encryption *below*
SQLite's own machinery — meaning every individual page of the database file
is encrypted, but everything built on top (the full-text search index, WAL
itself, every existing database query) keeps working completely unchanged,
because as far as SQLite's own code is concerned, nothing about how pages are
read or written has changed at all. The alternative approaches considered and
rejected are worth naming, because each rejection teaches something: relying
on the Linux filesystem feature `fscrypt` was rejected because the actual
development machine's filesystem (btrfs) doesn't even support it, which would
have made "Postio encrypts your mail" true only on some filesystems by
accident — an unacceptable thing for a stated privacy promise to depend on.
Encrypting message *bodies* but leaving the search index alone was rejected
because the search index is *built from* message content — an unencrypted
index would leak exactly what encryption was supposed to hide. Hand-rolling a
custom encryption layer was rejected on the general principle that
reimplementing an already-solved cryptography problem mostly just
reimplements its mistakes too.

### Being honest about what encryption does and doesn't protect against

The architecture decision is unusually direct about the limits of this
protection, and it's worth repeating precisely because at-rest encryption is
so often oversold. **What it protects against:** a stolen or discarded hard
drive; a backup, a cloud sync, or an rsync copy of the mail folder that ends
up somewhere it shouldn't; another user on a shared machine who defeats the
file permissions; anyone reading the raw files while the encryption key
itself is locked away. **What it explicitly does not protect against:**
someone running as the logged-in user *while the key is unlocked* — they can
read the key exactly the way Postio itself does; the system's administrator
account (root); and the contents of RAM, swap, or a hibernation image, which
is why the design treats full-disk encryption as complementary, not
replaced. Overselling that boundary would be worse than not encrypting at
all, because it would give someone false confidence about a threat the
feature was never built to stop.

### How the key actually works

One 32-byte encryption key is generated per mailbox, the first time it's
opened, using the operating system's random number generator, and it's
stored in the desktop's system keyring (the same secure, OS-managed vault
that already holds the user's mail server password) rather than anywhere in
Postio's own configuration file or logs. From that single key, three
separate, cryptographically distinct sub-keys are *derived* — one for the
database itself, one for the content of attachment and message files, and
one for the *filenames* those files are stored under — using a technique
called `derive_key` from a hash function called BLAKE3, which is explicitly
designed to produce independent-looking keys from one input plus a distinct
"context" string per purpose. The reasoning for one master key plus three
derived subkeys, rather than three separate keys stored in the keyring
directly, is concreteness about failure: three independently-stored keys
would be three separate things that could each go missing, three separate
migration paths, and three chances for a mailbox to end up only partially
re-keyed. One key, derived three ways, is one thing to protect and one thing
to lose.

The consequence taken deliberately, and stated plainly in the decision
record: **a locked keyring means the mail does not open.** There is no
"open it read-only anyway" fallback, because that fallback would itself be
exactly the kind of quiet, undocumented privacy leak the whole feature exists
to prevent. This also means the keyring entry is, functionally, *part of the
mailbox* — copying the on-disk database to another machine without also
transferring the key copies nothing but ciphertext. That's framed in the
project's own documentation not as a limitation to apologize for but as
proof the feature works: a backup that wanders off unintentionally is
supposed to be unreadable. (It's also a lower-stakes situation than it might
sound: apart from drafts and the queue of not-yet-sent server operations,
everything else in the local store is, by design, just a re-downloadable
cache of the actual mail sitting on the server — so losing the key costs a
re-sync, never actual mail.)

Attachments and raw message files, stored separately from the SQL database
in what's called a content-addressed blob store (each file named by a hash
of its own contents, so identical attachments are automatically stored only
once), get their own encryption: each individual file is encrypted with
XChaCha20-Poly1305 (a modern, well-vetted encryption algorithm), with a
fresh random value mixed in for every single file so that encrypting the
same attachment twice never produces identical-looking ciphertext. The
filenames those encrypted files are stored under also had to change: a
plain content hash of a file's *plaintext* would let anyone with just a
directory listing (no key needed) confirm whether a *specific known file*
exists somewhere in this mailbox — for instance, "does this inbox contain
this exact leaked document?" — purely by hashing that file themselves and
checking for a matching filename. Deriving the filename from the content
hash *plus* the mailbox's own secret key closes that specific leak, while
still preserving the useful property that identical files within the *same*
mailbox are still recognized as identical and stored only once.

### What it actually cost, measured rather than guessed

A few honest numbers, each one obtained by actually measuring rather than
reasoning about what should be true, because more than one plausible-sounding
guess in this project's own history turned out wrong once someone measured:

- The most visible casualty was memory-mapping the database file directly
  into the program's address space (`mmap`), a feature the project had
  previously relied on and even documented specific memory numbers for.
  Memory-mapping fundamentally assumes "the file's bytes and the in-memory
  bytes are the same thing" — which stops being true the instant those
  bytes are encrypted on disk and have to be decrypted into memory before
  anything can read them. Losing that feature was priced in advance as a
  known cost of the decision, and — measured after the fact — it actually
  turned out to be a net *improvement*: the equivalent of an 83-to-167
  megabyte-and-growing memory number tied to mailbox size flattened to a
  steady roughly 121 megabytes, and total memory use on a 100,000-message
  mailbox dropped from 215 to 177 megabytes.
- Isolated properly — by measuring an identical database with encryption
  patched out and comparing directly, rather than assuming the difference
  between "before" and "after" was all encryption's doing — the actual cost
  of the cipher itself came out to roughly 5% on typical page access and
  roughly 22% on startup time. Two separate, real performance regressions
  that showed up around the same time turned out to have entirely different
  causes and were *not*, in fact, the encryption's fault — a distinction
  that would have been impossible to make correctly without deliberately
  isolating the variable.
- The engineering journal also records, plainly, three specific ways this
  kind of measurement can fool you if you're not careful — worth listing
  because they're generally useful, not just specific to this project: an
  old, unencrypted build of the program silently failing to open a newly
  encrypted database looks, at a glance, exactly like "flat, low memory
  use" — so the very first thing to check is whether the mailbox actually
  opened at all; the first several seconds after startup are not
  representative, because background catch-up work is still running and
  temporarily inflates memory well above its settled value; and two
  measurements can never prove a trend has leveled off — it took a third
  data point, at a much larger mailbox size, to actually confirm that a
  rising number had stopped rising rather than merely paused.
- Getting the encryption library itself to build turned out to be more
  expensive than the architecture decision anticipated. The chosen build
  configuration compiles its own copy of OpenSSL from source as part of
  building Postio, for reproducibility — but OpenSSL's own build process is
  a Perl program, and the Linux distribution used for development splits
  Perl's standard library across many small packages, several of which
  aren't installed by default. The actual list of missing pieces was
  discovered the hard way, one cryptic "can't locate this module" build
  failure at a time, and had to be worked out by directly reading which
  Perl modules OpenSSL's own build script imports — because no single
  package metadata anywhere listed them all together.

---

## Part Seven: How this all gets built — the AI-agent development loop

This part is shorter, but it's the piece that explains why a document this
detailed and this honest about its own mistakes could exist in the first
place, so it's worth including.

Postio has no single author sitting down and writing code for months. It's
built by AI coding agents working through a fixed loop, over and over: claim
one open, unblocked task from a public issue tracker; do the work in an
isolated copy of the code (so multiple agents can work at once without
stepping on each other); write a failing test *first*, watch it actually
fail, then write the code that makes it pass — never the other way around;
run an automated gate that checks tests, code style, the architectural
boundary rules described in Part Four, and a scanner that specifically
checks that no real person's email address or name has accidentally leaked
into the public repository (since email fixtures, by their nature, look a lot
like real mail); and only then merge. When a decision turns out to need a
human's judgment rather than an engineering default — a genuine product
trade-off, not "which variable name is nicer" — the work stops and asks,
explicitly, rather than guessing.

Two habits from that loop explain why sections like Part Five and Part Six
could be written at this level of specific, mechanistic detail rather than
vague summary. First: every non-obvious lesson — a bug that looked like one
thing and turned out to be another, a setting that was correct until it
suddenly wasn't — gets written down in a running engineering journal, in
enough concrete detail (function names, issue numbers, exact measurements)
that it's *findable* later and distinguishable from something that merely
sounds similar. Second: every real architectural decision, including the
alternatives that were considered and specifically rejected and why, is
written up as a permanent, numbered decision record before or alongside the
work that implements it — so "why is it built this way" never depends on
anyone's memory of a conversation that happened once.

---

## Closing

The thread running through all of this: nearly every distinctive thing about
how Postio is built — eighteen crates instead of one, commands flowing one
direction and events flowing back the other, write-ahead logging, an
application-level write queue in front of SQLite, encryption keys derived
three separate ways from one master key — is not decoration. Each one is a
direct, traceable answer to a specific problem that was measured, not
assumed: a UI that must never freeze waiting on a network it doesn't
control, a search index that would otherwise leak the very content it's
supposed to protect, a keystroke that was losing a race it should never have
had to run, a cryptography library's cleanup routine that didn't know a
second thread existed. The interesting parts of this system, in other words,
aren't the parts that were designed correctly on the first try — they're the
parts where a reasonable, well-argued decision met reality, something broke
in a way nobody had predicted, and the fix (and the reasoning behind it) got
written down clearly enough that it didn't have to be rediscovered.
