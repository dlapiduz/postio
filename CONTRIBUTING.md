# Contributing to Postio

Postio is written almost entirely by AI coding agents under a human
maintainer's direction, and the contribution process is designed around
that fact. It gives you two ways in, and they are equally welcome:

1. **Contribute code yourself** — the ordinary fork-and-PR path.
2. **Contribute a well-shaped issue** — possibly with the prompt an agent
   could run to fix it. In this repository a precise issue *is* a
   contribution: agents claim `ready`-labelled issues and work them to
   merged PRs, so a good report often becomes a fix without another human
   touching a keyboard.

Either way, start by knowing how the project thinks: the product spec is
[`docs/PRODUCT.md`](docs/PRODUCT.md), the architecture and its reasoning
are [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and the ADRs in
[`docs/decisions/`](docs/decisions/), and the accumulated lessons live in
[`docs/engineering-notes.md`](docs/engineering-notes.md). The agent-facing
workflow (worktrees, claim/land scripts, session rules) is
[`CLAUDE.md`](CLAUDE.md) — worth skimming even as a human, because it is
where most conventions are defined.

## Ground rules — these are enforced, not suggested

CI runs every one of these; a PR that fails them will not merge.

- **Test-driven development is mandatory.** Write the failing test first,
  then the code. An issue is not done until its acceptance criteria are
  covered by tests. No test in the default suite may touch the network —
  live-server tests are `#[ignore]`d.
- **The gate chain**: `cargo test`, `cargo clippy --all-targets -- -D
  warnings`, `rustfmt --check`, plus the repository's own checks —
  crate-boundary enforcement, the personal-data scanner, the tracking
  guard, the lint floor, and cargo-deny for licenses and advisories.
- **Architectural invariants**: `postio-core` never links GTK;
  `postio-gtk` never links SQL or the IMAP protocol crate; `postio-sync`
  talks to the `MailBackend` trait, never `io-imap` types; every mutating
  action is local-first and the UI never awaits the network.
- **Privacy is a feature.** Nothing leaves the user's machine that they
  did not ask for. No telemetry, no phoning home, no logging of message
  content — bodies, subjects and recipient addresses never appear in a
  log at any level.
- **No personal data, ever.** Every email address in the repository uses
  a reserved domain (`example.com`/`.net`/`.org`, `.test`, `.invalid`,
  `.example`, `.localhost`), and no fixture names a real person. This
  applies to issues and PR text too: never paste real mail — subjects,
  bodies, or recipients — into a report. Describe the shape of the
  message and use the corpus in `crates/postio-model/tests/corpus/`.
- **Providers are data, not code.** Server settings live in the preset
  table; no named constants for a vendor, no special-cased branches.

## Commits

The history is part of the product. Small, focused commits, conventional
subjects (`feat(gtk): …`, `fix(sync): …` — the scope is the crate without
its `postio-` prefix), imperative mood, and a body that explains **why**
wrapped at 72 columns. `git config commit.template .gitmessage` sets up
the template. Every commit is green for the crates it touches.

## Contributing code

```bash
git clone https://github.com/dlapiduz/postio && cd postio
# system dependencies: see README.md — Fedora and Ubuntu lines are there
cargo test -p <the-crate-you-are-changing>   # the inner loop
cargo clippy -p <crate> --all-targets -- -D warnings
```

Fork, branch, and open a PR that says which issue it closes
(`Closes #N`). Verify the crates you touched rather than the whole
workspace on every edit — `cargo test --workspace` builds nine crates
including GTK and is the expensive way to find out what `-p` would have
told you. If your change builds a user-facing surface, confirm a person
can actually reach it in the running app; a green suite proves the
widget works, not that the application does.

If you use an AI coding agent yourself: excellent — so does this
repository. Hold its output to everything above, especially TDD; the
gates do not care who typed the code.

## Contributing an issue an agent can act on

The issue templates ask for the usual things — what happened, what you
expected, how to reproduce. The optional field worth your time is the
**agent prompt**: a self-contained prompt someone could hand to an AI
coding agent (Claude Code or similar) running in this repository to fix
the issue. The maintainer's agents work from exactly such framing, so a
good prompt can be the whole distance between "filed" and "merged".

What makes a prompt runnable:

- **Name the observable behaviour and the expected one**, precisely
  enough to write a failing test from. "Archiving from search results
  jumps the selection to the top; it should stay on the next result."
- **Point at the seam if you know it** — a file, a type, a command id.
  Guessing is fine; say it is a guess.
- **State the acceptance criteria** — what test would prove it fixed,
  what must not regress.
- **Respect the ground rules in the prompt itself**: fixture addresses on
  reserved domains, no real mail, no network in tests.

A worked example:

> The reading pane shows a blank body when a message's blob is missing
> instead of saying it is offline. Write a failing integration test in
> `crates/postio-app/tests/` that opens a message whose body blob is
> absent while the connection state is Offline, and assert the pane shows
> the offline notice rather than empty content. Then make it pass —
> likely in the reading-pane feed where BodyLoaded events are applied.
> Do not load the mailbox into memory; keep the fix event-driven.

## Logs in bug reports

`POSTIO_LOG=debug` turns up logging (`RUST_LOG` does nothing). Postio's
logs never contain message content by design, so they are safe to paste —
but read anything you paste anyway, and redact anything that identifies
you. The personal-data scanner cannot see an issue comment; that part is
on you.

## What not to bother with

- PRs that reformat, rename, or "clean up" without an issue behind them.
- Features outside the current scope (see `docs/PRODUCT.md` §2 and the
  [roadmap](https://github.com/users/dlapiduz/projects/2)) — file the
  idea as an issue instead; scope decisions are the maintainer's.
- Anything that adds a network request the user did not ask for. It will
  not merge, whatever else it does.
