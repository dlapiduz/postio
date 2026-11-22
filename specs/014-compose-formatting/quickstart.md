# Quickstart: validating compose formatting

These scenarios prove the feature end to end. The commands are the tiers in
`CLAUDE.md`. None of them touches the network except the last, which is a
manual send the maintainer runs once.

## 1. The document and the wire (seconds)

```bash
scripts/test-fast.sh                                  # postio-body, postio-ui, postio-core --lib
cargo nextest run -p postio-body --test body_suite formatting_wire
cargo nextest run -p postio-body --test body_suite outgoing   # harden is still a no-op
cargo test -p postio-body --doc
```

Expected:
- the golden document in `contracts/wire.md` renders to exactly its HTML
  and plain text;
- every round trip in SC-002 holds;
- the hostile corpus still narrows.

## 2. The registry and the keys

```bash
cargo nextest run -p postio-core --lib registry
POSTIO_UPDATE_DOCS=1 cargo test -p postio-ui --test ui_suite keybindings_doc
git diff docs/keybindings.md                          # only the new rows
```

Expected: no collision in `Composer` or `Everywhere`, and every new id has
a key a legacy terminal can deliver, or carries `GRAPHICAL_MAIL`.

## 3. Typing, in the real GTK composer

```bash
cargo nextest run -p postio-widgets --test widgets_suite composer_formatting
cargo nextest run -p postio-gtk --test focus_suite formatting_picker
POSTIO_HEADLESS=0 cargo nextest run -p postio-gtk --test focus_suite formatting_picker   # to watch it
```

Expected: each row of `contracts/sequences.md`, typed into the body,
yields its construct. One `ctrl+z` returns the characters. The picker
opens at the caret, filters, runs and closes.

## 4. The storyboards and the review

```bash
scripts/storyboards.sh storyboards/compose/formatting-*.toml
```

Then `/ux-review` on the branch. The storyboards are written from US1,
US2, US4 and US5 and must be red on `main` first.

## 5. The terminal and the Mac

```bash
cargo nextest run -p postio-tui --lib composer
scripts/macos-test.sh                                  # on the Mac: ComposeCommandsTests, the format picker
```

## 6. Look at it

```bash
scripts/run-isolated.sh --shot                         # the composer with the golden document pasted (Paste as Markdown)
```

Compare with spec 007's composer shots at 1440 × 900 and 1024 × 768, light
and dark. The toolbar keeps one row, and the picker fits the column.

## 7. Once, by hand: what recipients see

The maintainer sends the golden document from a test account to their own
test inboxes in Gmail (web), Apple Mail, Outlook for Windows and a plain
client (mutt or aerc). The addresses are `@example.com` placeholders in
anything written down. For each client, record in a note under
`docs/notes/` whether:
- each task's state is legible;
- the table has borders and alignment;
- the struck words are struck;
- the code block keeps its lines.

This confirms research R3. It is not a gate.
