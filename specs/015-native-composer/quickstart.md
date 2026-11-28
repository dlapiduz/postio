# Quickstart: seeing the native composer work

No step needs a mail account. `postio-demo` seeds a store.

## 1. The algebra (Phase 2)

```bash
scripts/test-fast.sh
cargo nextest run -p postio-body --test body_suite apply projection
cargo test -p postio-body --doc
```

Expect: every `apply` case green, including the 10 000-block count and the
CJK preedit case. Read `research.md` R3 for the gate's result.

## 2. The old surface, routed (Phase 3)

```bash
cargo nextest run -p postio-widgets --test widgets_suite composer_ editor_
scripts/storyboards.sh compose
```

Open the composer, type in a list, press Enter twice, Ctrl+Z twice. It
behaves as before. `POSTIO_LOG=info,postio_widgets=debug` shows `intent`
lines with ids and counts, never text.

## 3. The native surface (Phase 4)

```bash
POSTIO_NATIVE_COMPOSER=1 scripts/run-isolated.sh
```

- Compose, type a nested list, Tab and Shift+Tab. The bullets are drawn, not
  stored.
- Switch the input method to a CJK one (`ibus`), preedit in a list item,
  commit, Ctrl+Z.
- Paste HTML from a web page. It arrives as paragraphs and lists, no colours.
- Misspell a word (if Q3 passed): underline and menu.
- With Orca running: it reads the text and the caret.
- `pgrep -f WebKit` finds nothing.

```bash
cargo nextest run -p postio-widgets --test widgets_suite composer_native
cargo nextest run -p postio-gtk --test focus_suite a11y
```

## 4. One surface (Phase 5)

```bash
python3 scripts/checks/check-no-browser-engine.py
grep -n webkit6 crates/*/Cargo.toml Cargo.lock    # nothing
```

## 5. The Mac (Phase 6), on a Mac

```bash
scripts/macos-test.sh
scripts/macos-shot.sh compose-native --size 1440x900 --light
```

Compose a reply: marked text in a list item, VoiceOver reading the body.

## 6. Spec 014 (Phase 7)

Run 014's own `quickstart.md` on the native surface. Every `⛔ ADR 0039`
marker in its `tasks.md` is gone.
