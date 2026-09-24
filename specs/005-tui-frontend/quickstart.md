# Quickstart: validating Postio in the terminal

How to prove the feature works end to end. Each scenario names the spec
criterion it proves. The shapes are in [contracts/](./contracts/) and
[data-model.md](./data-model.md); this guide does not repeat them.

## Prerequisites

- A worktree of `feature/tui-frontend` with its target dir seeded
  (`scripts/issue-claim.sh` seeds one; see `CLAUDE.md`).
- `scripts/install-nextest.sh` has been run.
- For the by-hand scenarios: a terminal with mouse reporting (any modern
  one), and optionally one without the kitty keyboard protocol for the
  fallback checks.

## Automated: what CI and the landing gate run

```bash
cargo nextest run -p postio-tui                      # screens, input, paste, layout (TestBackend)
cargo nextest run -p postio-body --test body_suite   # markdown mapping + corpus privacy (SC-005)
cargo nextest run -p postio-client                   # protocol round-trips, handshake refusals
cargo nextest run -p postio-host --test two_clients  # US6: two clients, one daemon (SC-007)
cargo nextest run -p postio-app --test app_suite     # GTK unchanged (FR-005)
cargo nextest run -p postio-ffi                      # macOS facade unchanged, now with writes
python3 scripts/checks/check-crate-boundaries.py     # no GTK/WebKit/turso in postio-tui (FR-051)
```

What each must show:

| Suite | Proves |
|---|---|
| `postio-tui` `registry_parity` | Every command has a chord a legacy terminal delivers and a palette entry (SC-001) |
| `postio-tui` `triage` | US1 scenarios 1–6, asserted on the rendered buffer; the budget test allows at most one `Page` request per keystroke |
| `postio-tui` `reader` | US2 scenarios 1–5; a plain-text `#` line stays literal |
| `postio-tui` `composer` | US3 scenarios 1–11; the queued bytes have HTML equal to GTK's and a text part equal to the Markdown |
| `postio-body` corpus test | No tag, script, remote image or control character from any corpus message (SC-005) |
| `two_clients` | An archive in one client is gone from the other within 1 s, and a send is submitted once to the mock (SC-007) |
| `app_suite` | Passes with no edits beyond import paths |

## By hand: the scenarios a person should see

### 1. Two frontends, one store (US6)

```bash
cargo run -p postio-app            # GTK; spawns postio-daemon
cargo run -p postio-tui            # in a terminal; joins the same daemon
```

Archive a conversation with `a` in the terminal. Expected: it leaves the GTK
list within a second. Start a reply in GTK, close the composer, then open
Drafts in the terminal. Expected: the draft is there. Quit both. Expected:
`postio-daemon` exits about 30 s later (`pgrep postio-daemon` is empty).

### 2. Reading (US2)

Open an HTML newsletter from the corpus fixture store. Expected: headings,
lists and links are styled; quoted history is folded; images show as
`[image: …]` placeholders; the remote-image count is in the header, and no
connection is made (`ss -tp | grep postio` stays empty).

### 3. Writing, drop and paste (US3)

Press `e` on a message, type `**bold** and a [link](https://example.com)`,
drag a file from the file manager onto the terminal, copy a screenshot and
press the paste key, then send with `Ctrl+Enter`. Expected: the file and the
image are listed, and the message is in the Outbox. Its `.eml` (via
`export`) has a text part holding the Markdown as typed, an HTML part with
`<strong>` and the link, and the image as an inline `cid:` part.

Then invoke "edit in external editor". Expected: `$EDITOR` opens the Markdown,
and on exit the terminal redraws with the edited body.

### 4. Small terminal, no colour, no mouse

```bash
NO_COLOR=1 postio-tui
```

Resize to 60×20, then to 40×10. Expected: a single pane, then "Terminal too
small". Unread and flagged rows are still marked with `●` and `⚑`.

### 5. Hostile content

Send yourself (via the fixture store) a message whose subject contains
`\x1b]0;pwned\x07` and `\x1b[2J`. Expected: the terminal title does not change
and the screen does not clear. The subject shows the escape sequences as
visible replacement glyphs.

### 6. Size (SC-004)

```bash
scripts/measure-package-size.sh     # new; compares tarball vs GTK binary+libs, TUI flatpak vs desktop flatpak
```

Expected: each terminal package is under half the desktop equivalent, and
the terminal frontend's RSS with the same mailbox open is under half of GTK's
(`/proc/<pid>/status` VmRSS, daemon counted for neither).
