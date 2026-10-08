---
name: gtk-design
description: Design and build Postio's GTK4/libadwaita interface so it stays visually consistent and feels right — the colour and metric roles, Focus's layout language, motion and interaction rules, the GTK-specific traps that fail silently, and the render-to-PNG loop that lets you actually look at what you built. Load before writing or restyling any widget, CSS, or screen.
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

**Before designing a screen, look at the source of truth.** Postio's one
desktop app is the Focus design (ADR 0043). Each screen is specified in
`specs/007-postio-focus/contracts/focus-surface.md` and held against its
reference image in `specs/007-postio-focus/screens.md`, which records every
known difference and its reason and holds the designs that have no image
(the open message's treatments, the reading pane, sending states, the row
menu, Settings). The reference images and the message-dialog handoff are
the maintainer's local `Design/` folder, which is not in the repository;
the spec's decisions C25 (the system font) and C26 (the system accent)
override them where they differ.

---

## 1. Never hard-code a value

Every value is a `--postio-*` role, and there are two kinds:

- **Colour roles** are defined by the app from libadwaita's own named
  colours, in `crates/postio-gtk/data/focus-colours.css`, so the system's
  accent and its light and dark arrive through `AdwStyleManager` with
  nothing of Postio's in between. No hex and no `rgba()` literal; the one
  exception is the message dialog's surface, ink and rules, at the end of
  that file.
- **Metrics** (spacing, radii, chip and type sizes) are **generated** into
  `crates/postio-widgets/data/metrics.css` and `data/space.rs` by
  `postio-widgets`' `build.rs` from the Industry design system. Editing them
  by hand is a bug: `postio-ui`'s drift tests fail, and a build with the
  design system present rewrites them. In Rust, spacing comes from `postio_widgets::widgets::space`.

**The accent is reserved** (FR-091): action markers, the open message's
action card, its tag and its links, the keyboard focus ring, and the
has-action toggle when it is on. Nothing else. A selected row is a neutral
background and a checked box, never the accent. `postio_gtk::style`'s test
reads the sheets as GTK would and fails on any other rule that paints with
it. "Raised" buttons (Send, Create, Archive all) are plain raised buttons
with bold labels, never `suggested-action`.

**Type is the system's** (C25): Adwaita Sans for the chrome, Adwaita Mono
for keys, addresses, counts and operators. Barlow appears only in a message
body drawn in app colours (FR-039). Type is in `rem`, never `px`, so
GNOME's text scaling moves it.

**Keycaps come from the registry** through the shared `keyhint` widgets,
never from a literal (`check-key-hints-are-derived.py`). A key is taught
inside the control it runs.

CSS lives in three places: `widgets.css` in `postio-widgets` (the shared
controls: key hints, the four kinds of button, chips, the action bar, the
notice), `focus.css` and `focus-colours.css` in `postio-gtk` (the app's own
surfaces and its colour roles), and the reader's sheets in `postio-ui`'s
`data/`. Put a rule in the narrowest one that can hold it: a control the
app's window does not own belongs in `postio-widgets` (ADR 0043).

---

## 2. GTK traps that fail silently

These were each found the hard way. All of them *look* like they work.

**Two providers do not compare specificity.** Across two providers at one
priority, the one added later wins every property it sets, whatever its
selectors. So `focus.css` imports the shared sheet at its top
(`@import url("resource:///dev/postio/Widgets/widgets.css");`), making both
one provider; `postio_widgets::style::register` has to run before that
import is parsed. A second provider for "just one rule" breaks this.

**A media query reads the provider's scheme, not the app's.** GTK 4.20
evaluates `prefers-color-scheme` against the provider's own setting.
`postio_gtk::style::install` keeps it in step with `AdwStyleManager`, which
is what makes `focus-colours.css`'s dark block hold exactly when the app is
dark. A provider installed any other way sees the system's scheme instead.

**The person's own `gtk.css` sits above the app.** A desktop can write its
palette as `@define-color`s in `~/.config/gtk-4.0/gtk.css` whatever the
scheme. The suite and `shot` run with a private `XDG_CONFIG_HOME` for that
reason; a screen that looks wrong only on one desktop may be that file.

**GTK CSS is a subset of web CSS.** No grid, limited selectors.
`GtkCssProvider` logs parse errors rather than failing, so a typo silently
drops the rule. `postio_gtk::style::install` turns one into a `g_critical`,
and `widgets_suite`'s `widgets_css.rs` asserts the shared sheet parses —
add to it rather than trusting the eye.

---

## 3. The layout language

**One list, no folder sidebar.** The window is a 46 px top bar (Compose,
the command bar's field, the sync label, the main menu, the close button),
a 36 px header strip ("Inbox ▾" `g o`, the counts, the "Has action" toggle
`!`, the filtered and digest-rule counts), one banner slot, then the list
under day headings, and a bulk bar while anything is selected. The folders
popover (`g o`) and `in:` in the command bar are how a person gets anywhere
but the inbox.

**Rows are one `snapshot()` each, at two heights fixed by kind**: 40 px for
a conversation with no marker or a digest delivery, 72 px for one with a
marker or a fired reminder, whose second line carries the marker's chip,
its date or quoted sentence, and its answering actions with their keycaps.
There is no density setting in this app.

**The open message is a dialog over the dimmed list**, its frame computed
from the window and never from the message (FR-039), or, after `F8`, a pane
beside the list (FR-038). Below 980 px it is always the dialog. The composer
takes the open message's place. `Esc` returns to the same row.

**One of each**: one close button (an X at the right end of a header), one
icon button, one keycap, one dialog pattern for every surface over the
list, and one picker pattern (`screens.md`, "Interaction rules").

**The cursor and the selection are different states.** The cursor is where
the keyboard is; the selection is what an action will hit. Conflating them
makes bulk actions feel unpredictable, and it is the usual bug.

---

## 4. Motion: snappy or nothing

Transitions are **≤100ms or absent**. Pane switches use *no* transition at
all — instant. Honor `prefers-reduced-motion` everywhere.

The budget is a functional requirement, not a preference: <500ms to usable UI,
<16ms for ordinary interaction. Two implications for how you build widgets:

- **Row widgets use a single custom `snapshot()`**, not nested `GtkBox`. Nested
  boxes per row are the usual reason GTK lists feel sluggish, and they would
  break 40px rows at scroll speed.
- **Never materialise a mailbox.** The list is a windowed `GListModel` over
  the paged store. Any design that needs "all the rows" needs rethinking.

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

This is the part that actually produces consistency. "Matches the design"
is not checkable by squinting at a running app.

```sh
cargo run -p postio-gtk --example shot -- /tmp/01.png 01            # the inbox, light
cargo run -p postio-gtk --example shot -- /tmp/01-dark.png 01 dark
cargo run -p postio-gtk --example shot -- /tmp/04.png 04            # a message opened
cargo run -p postio-gtk --example shot -- /tmp/narrow.png 01 900x700
```

The arguments are `<png> <screen> [light|dark] [WxH]`. The screen is a
reference's number, and the table of them is the doc comment at the top of
`crates/postio-gtk/examples/shot.rs`. The size defaults to the references'
1440x900; the headless compositor's monitor is 1280x800, so a full-size
shot wants `POSTIO_TEST_DISPLAY=focus-shot POSTIO_TEST_GEOMETRY=1920x1200`
in front of it. The window is built over `postio_storage::seed::seed_small`
with the day's rows filed on top the way sync files mail, read back through
the client the running app reads through, so what you see is content the
store produces rather than content written to match the drawing.

**Read the PNG back.** Rendering it and not looking is the same as not
rendering it — and check the command succeeded: it exits non-zero and says
`NO IMAGE WAS WRITTEN` when there is nothing to look at, so a shot you did
not check is a claim you cannot make (#809).

It renders on the private headless compositor, not on your session, so a
locked or blanked screen does not stop it.

Then compare against the screen's row in `specs/007-postio-focus/screens.md`
(and the reference image, if you have the maintainer's `Design/` folder),
and name the differences. A difference you keep is written into
`screens.md` with its reason (FR-095).

**A sequence, not just a screen.** `shot` is one picture; how a screen
behaves under the keyboard is a storyboard (`storyboards/`, ADR 0044). Run
`scripts/storyboards.sh run --only '<surface>/*'` to film one with the
keyboard's place outlined on every frame, and `scripts/storyboards.sh
screens` for every screen beside its design. `/ux-review` is the review of
both, by an agent that did not build it.

Check every screen in **light and dark**, and at a laptop's width and
GNOME's minimum window size: as the window narrows, the first line and then
the labels give way before the sender, subject and time.

---

## Before you call a screen done

- [ ] Every value came from a role; no literal colour, no `px` type size
- [ ] The accent is on nothing but what FR-091 reserves it for
- [ ] Rendered with `shot` and **looked at** in light and dark
- [ ] Checked at a narrow width
- [ ] The cursor and the selection are visually distinct
- [ ] No transition over 100ms; none at all on pane switches
- [ ] Keyboard-only operation works, focus always visible
- [ ] `cargo nextest run -p postio-gtk --test focus_suite` green for the
      cases you touched, and `cargo test -p postio-gtk --lib` for the
      accent and stylesheet checks
