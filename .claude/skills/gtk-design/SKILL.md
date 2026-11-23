---
name: gtk-design
description: Design and build Postio's GTK4/libadwaita interface consistently: colour and metric roles, Focus's layout language, motion and interaction rules, GTK traps that fail silently, and the render-to-PNG loop. Load before writing or restyling any widget, CSS or screen.
---

# GTK design for Postio

This skill is the implementation layer (tokens, traps, motion, verification). What the experience *should be* (verbs, state behaviour, whether a pattern belongs) is `/ux-architect`: load it first when designing, this one when building.

Postio should look like a premium app that happens to use GTK (`docs/PRODUCT.md` §19). The identity exists; apply it the same way every time.

**Source of truth.** Focus is the one desktop app (ADR 0043). Each screen is specified in `specs/007-postio-focus/contracts/focus-surface.md` and held against its reference image in `specs/007-postio-focus/screens.md`, which records every known difference with its reason and holds the designs without an image (open message treatments, reading pane, sending states, row menu, Settings). Reference images and the message-dialog handoff live in the maintainer's local `Design/` folder, not in the repository; spec decisions C25 (system font) and C26 (system accent) override them.

## 1. Never hard-code a value

Every value is a `--postio-*` role:

- **Colour roles** come from libadwaita's named colours in `crates/postio-gtk/data/focus-colours.css`, so the system accent and light/dark arrive through `AdwStyleManager`. No hex, no `rgba()` literal; the one exception is the message dialog's surface, ink and rules at the end of that file.
- **Metrics** (spacing, radii, chip and type sizes) are **generated** into `crates/postio-widgets/data/metrics.css` and `data/space.rs` by `postio-widgets`' `build.rs` from the Industry design system. Hand edits are a bug (`postio-ui`'s drift tests fail; a build with the design system rewrites them). In Rust use `postio_widgets::widgets::space`.
- **The accent is reserved** (FR-091): action markers, the open message's action card, tag and links, the keyboard focus ring, the has-action toggle when on. Nothing else; `postio_gtk::style`'s test fails on any other rule painting with it. A selected row is a neutral background and a checked box. Raised buttons (Send, Create, Archive all) are plain raised with bold labels, never `suggested-action`.
- **Type is the system's** (C25): Adwaita Sans for chrome, Adwaita Mono for keys, addresses, counts, operators. Barlow only in a message body drawn in app colours (FR-039). Type is in `rem`, never `px`, so GNOME text scaling works.
- **Keycaps come from the registry** through the shared `keyhint` widgets, never a literal (`check-key-hints-are-derived.py`). A key is taught inside the control it runs.

CSS lives in three places; put a rule in the narrowest that can hold it (a control the app window does not own belongs in `postio-widgets`, ADR 0043):
- `widgets.css` in `postio-widgets`: shared controls (key hints, the four button kinds, chips, action bar, notice)
- `focus.css` and `focus-colours.css` in `postio-gtk`: the app's surfaces and colour roles
- the reader's sheets in `postio-ui`'s `data/`

## 2. GTK traps that fail silently

- **Two providers do not compare specificity.** At one priority the later provider wins every property it sets. `focus.css` therefore imports the shared sheet at its top (`@import url("resource:///dev/postio/Widgets/widgets.css");`), making one provider; `postio_widgets::style::register` must run before that import is parsed. Never add a second provider for "just one rule".
- **A media query reads the provider's scheme, not the app's.** GTK 4.20 evaluates `prefers-color-scheme` against the provider's setting. `postio_gtk::style::install` keeps it in step with `AdwStyleManager`, so `focus-colours.css`'s dark block holds exactly when the app is dark. A provider installed any other way sees the system scheme.
- **The person's `gtk.css` sits above the app.** A desktop can set `@define-color`s in `~/.config/gtk-4.0/gtk.css`. The suite and `shot` use a private `XDG_CONFIG_HOME`; a screen wrong on one desktop only may be that file.
- **GTK CSS is a subset of web CSS** (no grid, limited selectors). `GtkCssProvider` logs parse errors instead of failing, so a typo drops the rule. `postio_gtk::style::install` turns one into a `g_critical`, and `widgets_suite`'s `widgets_css.rs` asserts the shared sheet parses; add to it rather than trust the eye.

## 3. The layout language

- **One list, no folder sidebar.** A 46 px top bar (Compose, command bar field, sync label, main menu, close button); a 36 px header strip ("Inbox ▾" `g l`, counts, "Has action" toggle `!`, filtered and digest-rule counts); one banner slot; the list under day headings; a bulk bar while anything is selected. The folders popover (`g l`) and `in:` in the command bar reach everything but the inbox.
- **Rows are one `snapshot()` each, at two heights fixed by kind**: 40 px for a conversation with no marker or a digest delivery; 72 px for one with a marker or a fired reminder, whose second line carries the marker's chip, its date or quoted sentence, and its answering actions with keycaps. No density setting.
- **The open message is a dialog over the dimmed list**, its frame computed from the window, never the message (FR-039); after `F8`, a pane beside the list (FR-038). Below 980 px always the dialog. The composer takes its place. `Esc` returns to the same row.
- **One of each**: close button (an X at the right end of a header), icon button, keycap, dialog pattern for every surface over the list, picker pattern (`screens.md`, "Interaction rules").
- **Cursor and selection are different states.** The cursor is where the keyboard is; the selection is what an action hits. Conflating them is the usual bulk-action bug.

## 4. Motion: snappy or nothing

Transitions are **≤100 ms or absent**; pane switches have none. Honour `prefers-reduced-motion`. Budgets are in CLAUDE.md ("What the code must do"). For widgets:
- **One custom `snapshot()` per row**, not nested `GtkBox`es (the usual cause of sluggish GTK lists).
- **Never materialise a mailbox.** The list is a windowed `GListModel` over the paged store.

## 5. Accessibility is part of the design (`docs/PRODUCT.md` §20)

- Every custom widget, rows included, gets an accessible name and role
- Visible focus ring from the accent token; logical focus order
- Full keyboard operation; mouse excellent but never required
- Works at 200% text scaling and in high contrast
- Orca smoke test before calling a screen done

## 6. Look at what you built

```sh
cargo run -p postio-gtk --example shot -- /tmp/01.png 01            # the inbox, light
cargo run -p postio-gtk --example shot -- /tmp/01-dark.png 01 dark
cargo run -p postio-gtk --example shot -- /tmp/04.png 04            # a message opened
cargo run -p postio-gtk --example shot -- /tmp/narrow.png 01 900x700
```

Arguments: `<png> <screen> [light|dark] [WxH]`. Screen numbers are tabulated in the doc comment atop `crates/postio-gtk/examples/shot.rs`. Size defaults to the references' 1440x900; the headless monitor is 1280x800, so a full-size shot needs `POSTIO_TEST_DISPLAY=focus-shot POSTIO_TEST_GEOMETRY=1920x1200`. The window is built over `postio_storage::seed::seed_small` with the day's rows filed the way sync files mail, read through the client the app reads through, so content is what the store produces.

- **Read the PNG back.** The command exits non-zero with `NO IMAGE WAS WRITTEN` when there is nothing to see; an unchecked shot is a claim you cannot make (#809). It renders on the private headless compositor, so a locked screen does not stop it.
- Compare with the screen's row in `screens.md` (and the reference image if you have `Design/`), and name the differences. A difference you keep is written into `screens.md` with its reason (FR-095).
- **A sequence, not a screen.** Keyboard behaviour is a storyboard (`storyboards/`, ADR 0044): `scripts/storyboards.sh run --only '<surface>/*'` films one with the keyboard's place outlined; `scripts/storyboards.sh screens` shows every screen beside its design. `/ux-review` reviews both, by an agent that did not build it.
- Check light and dark, at a laptop width and GNOME's minimum window size: narrowing drops the first line, then the labels, before sender, subject and time.

## Before you call a screen done

- [ ] Every value came from a role; no literal colour, no `px` type size
- [ ] The accent is on nothing but what FR-091 reserves
- [ ] Rendered with `shot` and **looked at**, light and dark, and at a narrow width
- [ ] Cursor and selection visually distinct
- [ ] No transition over 100 ms; none on pane switches
- [ ] Keyboard-only operation works, focus always visible
- [ ] `cargo nextest run -p postio-gtk --test focus_suite` green for the cases you touched; `cargo test -p postio-gtk --lib` for the accent and stylesheet checks
