# Quickstart: validating the conversation reading pane

**Feature**: `specs/001-conversation-reading-pane` | **Date**: 2026-09-08

How to prove each phase works. Written so that a session picking up a phase can
verify it without re-deriving what to look at.

## Prerequisites

```bash
scripts/issue-claim.sh --base feature/conversation-reading-pane <n>   # a worktree of your own
scripts/install-nextest.sh                                           # pinned version
```

Work in the worktree, never in `~/src/postio`. This feature is an **initiative**
— several interdependent phases that would leave `main` half-migrated if landed
one at a time — so it runs on a feature branch and merges when whole.

**#1316's branch is checked out in another worktree and assigned.** Do not
work in it; take its measurements as input (R5).

## Iterate at the cheapest layer that can fail

Most of this feature's rules are provable without a display, which is the whole
reason they live in `postio-body` and `postio-ui`:

```bash
scripts/test-fast.sh                                    # changed crates, --lib, seconds
cargo test -p postio-body --lib                         # sanitizer, scoping, refused properties
cargo test -p postio-ui --lib                           # header, rail rule, document assembly
cargo nextest run -p postio-gtk --test gtk_reader       # surfaces and rendering, one binary
cargo test -p postio-app --test app_suite               # wiring — does it reach the running app
```

## Per phase

### Phase 0 — the header is shared (#1285)

```bash
cargo test -p postio-ui --lib reader::header
cargo nextest run -p postio-gtk --test gtk_reader
```

**Expect**: `postio-gtk` holds no private copy of `address_line`,
`address_list`, `subject_text`, `absolute_date`, `MessageHeader::of` or
`ReaderAction::ALL`. #1285's red test
(`a_key_lost_to_another_command_hides_the_hint_rather_than_showing_a_wrong_one`)
is green, rewritten to assert *"never a key that runs something else"* rather
than *"never a key"*.

### Phase 1 — sender CSS, contained

```bash
cargo test -p postio-body --lib sanitize
```

**Expect**, each as a named test:

- A three-column message keeps three columns (FR-019a).
- A rule written to restyle the document affects only its own message (FR-020).
- Each refused property is refused **with its reason** — containment or privacy
  — and nothing is refused for any other reason (FR-019b).
- `background-image: url(https://…)` loads nothing (FR-022); `@import` is
  refused by CSP (`style-src`).

**The `@scope` spike comes first.** Verify against the installed WebKitGTK
(2.52.5) before building on it; selector rewriting is the fallback.

### Phase 2 — one document per conversation

```bash
cargo nextest run -p postio-gtk --test gtk_reader
```

**Expect**: a thread of any length renders in **exactly one** web process,
counted the way `each_reader_costs_a_web_process_of_its_own` counts today.
`cid:` resolves per message, by token, not by "whichever message is open".

### Phase 3 — header and actions

```bash
cargo test -p postio-ui --lib
cargo test -p postio-app --test app_suite
```

**Expect**: the conversation bar's reply quotes the **most recent** message and
an individual message's reply quotes **that** message (SC-007) — asserted on
what the composer opens with, not on what a layer was handed. Tooltips state
scope in words.

### Phase 4 — the cost of moving

```bash
cargo test -p postio-ui --lib counting
cargo nextest run -p postio-gtk --test gtk_reader
```

**Expect** the five assertions in `contracts/render-counting.md`. These are the
regression locks for #749 — a full teardown per switch, no ground colour, 1.2 MB
of inlined fonts per document, and gestures issuing two loads. All four were
fixed; this phase is what stops the rewrite reintroducing them.

### Phase 5 — the rail *(starts with the R3 spike)*

```bash
cargo test -p postio-ui --lib reader::rail
```

**Expect**: given message geometries, the current message is the one with the
greatest visible **area** — a fully-visible two-line reply must lose to an
eighty-line message filling most of the viewport. That rule is pure and is
tested without a display; only its input comes from the engine.

**Run the spike first**, before any rail code:

1. A fixture carrying inline `<script>`, an event-handler attribute and a
   `javascript:` href must execute none of them with JavaScript enabled and
   `enable_javascript_markup(false)`.
2. An injected observer must run despite the document's `script-src 'none'`.

Both are `gtk_reader` cases. If either fails, stop and amend the spec — do not
weaken the sanitizer to make the rail work.

**This phase also owes three documents**: ADR 0003 amended, and the sentence
about the reader having JavaScript off corrected in `docs/PRODUCT.md` §21 and
`CLAUDE.md`. They land with the change, not after it.

### Phase 6 — retire the collapsed conversation

```bash
cargo test -p postio-ui --lib conversation
```

**Expect**: `collapsed_runs()` and its tests are gone, ADR 0015 is amended in
the same change, and a conversation opens on its most recent message every time
(FR-015) including on reopen (FR-018).

## Seeing it

```bash
scripts/run-isolated.sh --inspect      # pinned commit, own target dir, throwaway store
scripts/run-isolated.sh --shot
```

Links `--release`; never run it while other sessions are building.

`POSTIO_LOG=postio_gtk=debug` for the reader's own logging — an `EnvFilter`,
not `RUST_LOG`, and `[logging]` in `config.toml` retunes a running instance
live.

**Reproducing #749's black frame**, if it returns: record at 60 fps while
holding `j`, then press Enter on the row already under the cursor. A single
black frame is easy to miss by eye and obvious frame-by-frame.
`WEBKIT_DISABLE_DMABUF_RENDERER=1` confirms the renderer's role — it is
disabled for tests but never for the app, which is why no test saw it.

## Before landing

```bash
cargo clippy -p <crate> --all-targets -- -D warnings
scripts/issue-land.sh --detach          # sanity tier; CI runs the rest
scripts/issue-land.sh --status
```

Land on the default tier. `--full` needs a specific reason.
