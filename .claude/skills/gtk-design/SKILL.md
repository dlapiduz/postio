---
name: gtk-design
description: Design and build Postio's GTK4/libadwaita interface so it stays visually consistent and feels right — the token system, the PLATE layout language, motion and interaction rules, the GTK-specific traps that fail silently, and the render-to-PNG loop that lets you actually look at what you built. Load before writing or restyling any widget, CSS, or screen.
---

# GTK design for Postio

**This skill is the implementation layer.** It covers how to build a surface
correctly in GTK — tokens, traps, motion, verification. What the experience
*should be* — which verbs exist, how states behave, whether a pattern belongs
at all — is `/ux-architect`. Load that one first when designing something new;
this one when building it.

Postio should look like a premium application that happens to be built with
GTK, not like "a GTK developer made an email client" (`docs/PRODUCT.md` §19).
That is a consistency problem more than a taste problem: the identity already
exists, and the job is to apply it the same way every time.

**Before designing a screen, look at the source of truth.** `Design/Mail
Client.dc.html` is the approved canvas; the chosen direction is **PLATE
(option 1b)** — airy native desktop, 40px rows, real breathing room, key hints
revealed on the focused row only. `docs/PRODUCT.md` §19 defers to the canvas on
visual detail rather than restating it, so the canvas is where you look.

---

## 1. Never hard-code a value

Everything comes from tokens. `crates/postio-gtk/data/tokens.css` is
**generated** from the Industry design system by `build.rs` — editing it by
hand is a bug, and your change will vanish on the next build. To change a
value, change the source design system.

The semantic layer is what you write against:

| Role | Tokens |
|---|---|
| Ground / surface | `--postio-ground`, `--postio-hover-bg`, `--postio-active-bg` |
| Text | `--postio-ink`, `--postio-ink-secondary`, `--postio-dim`, `--postio-faint` |
| Accent | `--postio-accent`, `--postio-accent-hover`, `--postio-accent-active`, `--postio-accent-fg`, `--postio-accent-text` |
| Selection | `--postio-selected-bg`, `--postio-selected-fg`, `--postio-selected-border`, `--postio-selected-strong-bg` |
| Rules | `--postio-hairline`, `--postio-hairline-strong` |
| Type | `--postio-font-heading`, `--postio-font-body`, `--postio-font-mono` |
| Space | `--postio-space-1` … `--postio-space-8` |
| Radius / shadow | `--postio-radius-sm/md/lg`, `--postio-shadow-md/lg` |

Raw ramp steps (`--postio-color-accent-700`, `--postio-color-neutral-300`)
exist, but reach for a semantic role first. If no role fits, that is usually a
sign the design needs a new role rather than this widget needing a raw colour.

**Type roles are fixed.** Barlow Condensed for headings, Barlow for body,
IBM Plex Mono for counts, key hints, timestamps and metadata. The mono face is
what makes the interface read as instrument-like rather than generic — use it
for anything numeric or keyboard-related, and nothing else.

**Keep the identity, drop the wireframe chrome.** The Industry system is a
wireframe: its blueprint corner registration marks and transparent
line-drawing cards are drafting notation, not the product. Never port them.
Real `AdwHeaderBar` and window chrome stay, so it reads as a GNOME app.

CSS lives in three layers: `tokens.css` (generated), `shell.css` (the app
chrome and panes), `reader.css` (injected into WebKit for message bodies).
Put a rule in the narrowest layer that can hold it.

---

## 2. Four GTK traps that fail silently

These were each found the hard way. All of them *look* like they work.

**Media queries do not match.** `@media (prefers-color-scheme: dark)` and
`(prefers-contrast: more)` parse fine in an application-priority provider and
then never fire — GTK only evaluates them for the theme provider. Dark and
high-contrast are driven by classes on the window instead:
`style::DARK_CLASS` (`postio-dark`) and `style::HIGH_CONTRAST_CLASS`
(`postio-hc`), kept in step with `AdwStyleManager` by `style::track()`. Write
`:root.postio-dark { … }`, never a media query.

**`@define-color` cannot be scoped.** Overriding libadwaita's *CSS variables*
under a class repaints stock widgets correctly; overriding `@define-color` is
global and leaks across schemes. `tokens.css` already overrides
`--accent-bg-color`, `--card-bg-color`, `--headerbar-bg-color` and friends, so
stock widgets sit on the Industry ground for free — extend that list rather
than restyling each widget.

**Fonts must be installed before the first widget.** A `PangoContext` caches
the family it resolved, so `fonts::install()` after any widget exists means the
fallback font is baked in for the session. `style::install_for_application()`
does the ordering; anything driving the app directly (a test, a bench, an
example) has to do it too.

**GTK CSS is a subset of web CSS.** No grid, no flex-gap in older versions,
limited selectors. `GtkCssProvider` logs parse errors rather than failing, so a
typo silently drops the rule. `crates/postio-gtk/tests/gtk_style.rs` asserts
zero parse errors — add to it rather than trusting the eye.

---

## 3. The layout language

Three-pane PLATE, collapsing through two-pane to message-focused via
`AdwBreakpoint`. Sidebar, list, reader.

Row anatomy, from canvas 1b: avatar initials chip, sender, time, subject,
snippet, thread-count badge, unread and attachment indicators. Key hints
(`e reply`, `a archive`, `t thread`) appear on the **focused row only** — that
is the PLATE signature and how the app teaches its own keyboard without
permanent clutter.

**Selected and focused are different states.** Focused is where the keyboard
is; selected is what an action will hit. They need distinct treatments —
selection uses `--postio-selected-bg` with the 3px `--postio-selected-border`
left edge. Conflating them makes bulk actions feel unpredictable, and it is the
usual bug.

**Density is three row heights**, driven by `[ui].density` and applied as CSS
classes — never a rebuilt widget tree. Airy for reading, compact for triage.
Check any new widget at all three.

---

## 4. Motion: snappy or nothing

Transitions are **≤100ms or absent**. Pane switches and thread drill-in use
*no* transition at all — instant. Honor `prefers-reduced-motion` everywhere.

The budget is a functional requirement, not a preference: <500ms to usable UI,
<16ms for ordinary interaction. Two implications for how you build widgets:

- **Row widgets use a single custom `snapshot()`**, not nested `GtkBox`. Nested
  boxes per row are the usual reason GTK lists feel sluggish, and they would
  break 40px rows at scroll speed.
- **Never materialise a mailbox.** The list is a windowed `GListModel` over
  paged SQLite. Any design that needs "all the rows" needs rethinking.

---

## 5. Accessibility is part of the design

Not a later pass (`docs/PRODUCT.md` §20):

- Every custom widget gets an accessible name and role — including list rows
- Visible focus ring from the accent token; logical focus order
- Full keyboard operation, always; mouse stays excellent but never required
- Works at 200% text scaling and in high contrast
- Screen-reader smoke test with Orca before calling a screen done

---

## 6. Look at what you built

This is the part that actually produces consistency. "Matches the canvas" is
not checkable by squinting at a running app.

```sh
cargo run -p postio-app --example shot -- /tmp/plate.png            # light
cargo run -p postio-app --example shot -- /tmp/plate.png dark
cargo run -p postio-app --example shot -- /tmp/plate.png dark hc
cargo run -p postio-app --example shot -- /tmp/narrow.png 900x700
cargo run -p postio-app --example shot -- /tmp/plate.png demo
```

It asks GTK for the exact render node it would put on screen and writes a PNG,
so spacing, weight and colour become something you can look at, diff, and
attach to a review. `demo` fills the panes from `postio_storage::seed` — a
migrated in-memory database with a real folder tree and corpus-derived
messages, read back through the store the running application reads through —
so what you are looking at is content the store actually produces rather than
content that was written to match the drawing.

It is `-p postio-app`, not `-p postio-gtk`: reading a store means `rusqlite`,
which the view layer may not have at any depth, dev-dependencies included.

**Read the PNG back.** Rendering it and not looking is the same as not
rendering it. Then compare against the artboard in `Design/Mail Client.dc.html`
and name the differences.

Check every screen in **light, dark, and high contrast**, and at the narrow
breakpoint. Dark is not an afterthought here: it follows canvas 3c, where steel
goes light-on-dark and hairlines *lift* rather than darken.

---

## Before you call a screen done

- [ ] Every value came from a token; nothing hard-coded
- [ ] Rendered and **looked at** in light, dark, and high contrast
- [ ] Checked at all three densities and at the narrow breakpoint
- [ ] Focused and selected are visually distinct
- [ ] No transition over 100ms; none at all on pane switches
- [ ] Keyboard-only operation works, focus always visible
- [ ] `cargo test -p postio-gtk` green, including the CSS parse assertions
