# Project instructions for AI agents

Claim an issue, work it test-first in a worktree of your own, land it, take
the next one. Hooks and `scripts/check.sh` enforce most rules when you break
them, and each says what to do instead; this file is what they cannot check.
Traps worth knowing first are in `docs/gotchas.md`.

## The loop

```bash
scripts/issue-claim.sh                  # next ready issue -> its own worktree
cd ~/src/postio-worktrees/issue-<n>     # work there, never in ~/src/postio
scripts/issue-land.sh --detach          # gates, commit, push, PR, auto-merge
scripts/issue-land.sh --status          # what the landing did
scripts/issue-claim.sh                  # from inside the worktree: the next one
```

`/issue` has the rest: claim flags, several small issues on one branch,
feature branches for interdependent issues, red PRs, releasing a claim.

**Keep going.** Finishing an issue is not finishing a session; do not ask
whether to continue. Stop only when the claim finds nothing ready (say so),
when a decision is the maintainer's (label it `needs-maintainer` and comment
with the question and the options), or when context is nearly gone. Commit
as you go — a WIP commit beats loose files — and land or commit before you
stop.

**Spec-driven work has no issues.** A feature under `specs/<nnn>-<name>/`
that has been through `/speckit-specify` and `/speckit-plan` runs on its own
branch: `git worktree add ~/src/postio-worktrees/<name> -b feature/<name>
origin/main`, then `scripts/worktree-seed.sh` on it, then `printf
'main\n' > "$(git rev-parse --git-dir)/postio-base"`. Work `tasks.md` top to
bottom, a commit per task ending `Refs: specs/<nnn>-<name>` and the task id,
and land with `scripts/issue-land.sh --detach --full-suite`. Do not file an
issue per task. Work the spec does not cover is still filed.

A spec records its decisions, so it gets no parallel ADR. Write an ADR only
for a rule that outlives the feature and other work must obey; fold a
feature-only ADR into the spec and delete it.

## Tests

**Red, green, repeat.** Write the failing test and watch it fail before the
fix. Never re-break working code to test a test; if one was never seen red,
tighten its assertion instead. An issue is done when its acceptance criteria
are covered by tests.

| Tier | Command | When |
|---|---|---|
| fast | `scripts/test-fast.sh` | between edits: changed crates, `--lib` |
| suite | `cargo nextest run -p <crate> --test <suite> [case]` | to confirm what you touched |
| doctests | `cargo test -p <crate> --doc` | nextest does not run them |
| landing | `issue-land.sh` runs the sanity tier and `check.sh` | always |
| CI | every suite, nightly; on the PR with `--full-suite` | new features, big changes |

- Nothing in the default suite touches the network. `#[ignore]` means only
  "this machine may lack what the test needs" (a live server, D-Bus, a
  keyring). A slow test goes in the nightly tier instead: a
  `//! POSTIO-MEASUREMENT:` marker and `.config/nextest.toml`'s
  `default-filter`.
- Assert on what a person would see, not on what a layer was handed. The
  bugs here live between layers that each pass their tests. `focus_suite`
  (`crates/postio-gtk/tests/focus_suite/`) opens the real window and
  presses real keys.
- A change to how an app behaves under the keyboard ships with its
  storyboard in `storyboards/`, written from the acceptance first and red
  on the base, and `/ux-review` before landing.
- A test that needs a display lives in `tests/`, never `src/`. Tests run on
  a private headless compositor; `POSTIO_HEADLESS=0` to watch one.
- Protocol code tests against the `MailBackend` mock and the `.eml` corpus
  in `crates/postio-model/tests/corpus/` (`/add-fixture`).
- Put logic where it is cheap to test: a rule as a function in
  `postio-core`, `postio-ui` or `postio-body` is red in a second.
- Logging is `POSTIO_LOG` (an `EnvFilter`, e.g. `info,postio_sync=debug`),
  not `RUST_LOG`. To see the app at a pinned commit,
  `scripts/run-isolated.sh [commit] [--shot]`.

**Landing.** Use the default tier; `--full` needs a specific reason. A CI
failure on your PR is yours, on the same branch (`issue-claim.sh --resume
<n>`). A failure in code your diff does not touch is probably someone
else's: reproduce it alone and search the issues before re-running. While
the last nightly is red, every PR re-runs it, and fixing it on your branch
is how to clear `Nightly is green`.

## What the code must do

- **Instant.** Startup < 500 ms, interaction < 16 ms, local search
  < 100 ms, transitions ≤ 100 ms or none, and honour reduced motion. Never
  load a whole mailbox; lists are windowed over the paged store. Budgets
  are gated as counts (`postio_storage::test_support::counting`), so add a
  count assertion when you touch a read path.
- **Local-first.** Every mutating action is store write, enqueue, emit,
  repaint. The UI never awaits the network.
- **Providers are data.** Server settings live in the preset table, never
  in named constants or special-cased branches.
- **No backwards compatibility.** There are no installs to protect: write
  the clean version and its migration, without shims or deprecation paths.
- **Pimalaya first.** For a protocol or format need, check the Pimalaya
  crates (io-imap, io-smtp, io-oauth, …) before writing wire code.
- **Crate boundaries** are in `docs/ARCHITECTURE.md` and enforced by
  `scripts/check.sh`.

## Privacy

Nothing leaves the machine that the user did not ask for: no telemetry, no
prefetch, no speculative connections, remote images blocked per sender, and
a reader that cannot run script or reach the network. Credentials live in
the OS keyring only. **Logs carry ids, counts and outcomes, never message
content.** The repository is public: fixtures, issues, PRs and commits use
reserved domains (`ada@example.com`) and no real person's name or address,
least of all the maintainer's. Never paste real mail or unread logs.

## Commits and git

- `<type>(<scope>): <summary>`: type from `feat fix docs test refactor
  perf chore ci build revert`, scope the crate without its prefix, summary
  imperative and at most 50 characters. The body says **why**, wrapped at
  72. End with `Refs: #<issue>` (or `Refs: specs/<nnn>-<name>`); the PR's
  `Closes` does the closing. Template in `.gitmessage`.
- **Never write a closing keyword in a commit body**, not even negated:
  GitHub closes the issue for "does not fix #12". Write "leaves #12 open".
- A change too small for an issue gets none: branch it `fix/<slug>`,
  `docs/<slug>` or `chore/<slug>` and land it the same way.
- PRs merge squashed. Fetch before you reason about the tree; other
  sessions land all the time.
- Allowed without asking: committing, pushing your own branch, and
  `--force-with-lease` on it. Ask first: pushing `main`, adding remotes,
  rewriting shared history, bare `--force`.

## One machine, many sessions

`~/src/postio` is for coordination; work in your own worktree, which no
other session may touch. Never put a worktree path into anything rustc or a
build script sees: it breaks the shared compile cache. Never pass `-j`; a
machine-wide jobserver hands out compile jobs. A tree made with plain `git
worktree add` should be seeded with `scripts/worktree-seed.sh`, or it builds
cold.

On this project's Macs, `unset RUSTUP_TOOLCHAIN` before cargo (mise exports
it and it overrides the pin). The Mac app's build loops are in
`macos/CLAUDE.md`.

## Say it where it persists

| What | Where |
|---|---|
| Why the change is shaped this way | the commit body |
| What you found along the way | a comment on the issue |
| Work you found, less than ~10 minutes | fix it now, in its own commit |
| Work you found, more than that | `scripts/issue-file.sh`, which searches for duplicates first; comment on the existing issue if it finds one |
| A design call an agent can make | label `needs-architecture` (`/ux-architect`'s queue) |
| A call only the maintainer can make | label `needs-maintainer`, with the question and options |
| A trap future sessions must know | `docs/gotchas.md`, or a dated note in `docs/notes/` listed in its README |
| A decision other work must obey | an ADR in `docs/decisions/` |

## Skills and sources of truth

Skills: `/issue` (the loop), `/steward` (the periodic health pass),
`/lanes` (who else is working), `/preflight` (the true state of the tree),
`/add-fixture`, `/ux-architect` (designing a surface),
`/gtk-design` (building one), `/ux-review` (filming and reviewing an
interaction change), and the `/speckit-*` set for spec-driven features.

- Principles: `.specify/memory/constitution.md`.
- Product: `docs/PRODUCT.md`. The app says "Flagged", never "Starred".
- Keys: `docs/keybindings.md`, generated from the registry; one keymap for
  every app, all rebindable.
- Crates and boundaries: `docs/ARCHITECTURE.md`.
- Decisions: `docs/decisions/`, then each feature's `specs/`.
- Visual truth: spec 007's `screens.md` against the maintainer's local
  `Design/` references (untracked, never copied in).
- Scope for v1: Linux, IMAP and SMTP with OAuth, one provider preset
  table, no AI in Postio itself (a local model the user connects is
  optional, never required).
- A release is a PR: `scripts/release-prepare.sh X.Y.Z` from a worktree.
