# Quickstart: Validating the Compose Editor

**Plan**: [plan.md](./plan.md) | **Spec**: [spec.md](./spec.md)

How to prove this feature works, in the order the plan builds it. Every command
here runs from the feature worktree.

## Prerequisites

```bash
git worktree add ~/src/postio-worktrees/compose-editor \
    -b feature/compose-editor origin/main
cd ~/src/postio-worktrees/compose-editor
scripts/install-shims.sh          # the claim script normally does this
```

Tests that need a display are put on a private compositor automatically; see
`scripts/test-headless.sh --status`. `POSTIO_HEADLESS=0 cargo test` to watch a
run on the real display.

## The gate before anything is built

```bash
ls docs/decisions/ | tail -3        # the ADR superseding 0003/0004 must exist
```

FR-044 changes what a reply re-emits. Until that ADR is written, the quote work
does not start — see the Constitution Check in [plan.md](./plan.md). The other
two builds are not blocked by it.

## 1. The editor looks like Postio (Story 4, FR-073 to FR-078)

No display needed for most of it — that is the point of putting it in
`postio-ui`:

```bash
cargo test -p postio-ui editor::document
```

Expect: the generated document carries a stylesheet; the ground resolves in both
schemes; a scheme change produces a different sheet from the same input.

Then the half that needs a display:

```bash
cargo nextest run -p postio-gtk --test gtk_suite composer
```

Expect: the editing surface in dark mode is dark, with no light frame before or
after it draws.

**By eye**, which is the actual acceptance:

```bash
cargo run -p postio-app
```

Press `c`. The text you type should be the same typeface, size and colour as a
message body in the reader. Switch the system to dark and back with a draft
open — the surface follows, the caret stays, undo still works.

## 2. Markdown input (Story 6, FR-067 to FR-072)

```bash
cargo nextest run -p postio-gtk --test gtk_suite editor
```

Expect: each supported sequence produces the formatting its command produces;
one undo restores the literal characters; text that resembles markdown but was
not converted is sent as shown.

By eye: type `**bold**`, `# `, `- `. Then type something markdown-ish you did
*not* mean — one `mod+z` should give you your characters back, not reapply
formatting.

## 3. The quote (Story 2, FR-044 to FR-047) — after the ADR

```bash
cargo test -p postio-body replying
cargo test -p postio-body -- --include-ignored sanitize
```

Expect: an HTML original's structure survives into the quote; an original
carrying a script or a remote image yields neither; a plain-text-only original
falls back rather than producing an empty quote.

The security assertion, which is the one that matters:

```bash
cargo test -p postio-body corpus_quote
```

Expect **zero** scripts, **zero** remote-loading references and **zero**
tracking pixels re-emitted, across every HTML message in
`crates/postio-model/tests/corpus/`.

## 4. The conformance pass (most of the spec)

These assert behaviour that already works and nothing currently checks. Two are
worth running on their own because both fail silently and both are visible to a
recipient:

```bash
cargo test -p postio-model outgoing::   # FR-021, FR-022: Bcc is never disclosed
cargo test -p postio-model draft::      # FR-030 to FR-032: never two signatures
```

Then the rest:

```bash
scripts/test-sanity.sh
cargo nextest run -p postio-gtk --test gtk_suite
cargo nextest run -p postio-app --test app_suite
```

## Before landing

```bash
scripts/check.sh
scripts/issue-land.sh --detach
```

One pull request, reviewed against `spec.md`, closing no issue — see **How This
Lands** in [plan.md](./plan.md).

## What none of this covers

Every WebKit test in this repository runs on the software rendering path:
`scripts/headless-runner.sh` sets `WEBKIT_DISABLE_DMABUF_RENDERER=1`, which is
the mitigation for #272. So a green run says nothing about how the editor paints
on the accelerated path a user actually gets (#1307). For the appearance work in
particular, the `cargo run` check above is not optional politeness — it is the
only evidence of the thing being specified.
