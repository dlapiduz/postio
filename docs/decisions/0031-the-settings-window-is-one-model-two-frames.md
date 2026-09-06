# ADR 0031 — The settings window is one model in two frames

- **Status:** Accepted (2026-09-06)
- **Date:** 2026-09-06
- **Decision by:** `/ux-architect`, on [#1156](https://github.com/dlapiduz/postio/issues/1156), after the maintainer confirmed the macOS build should get a real settings screen with structured panes rather than a raw config-file editor.
- **Issue:** [#1156](https://github.com/dlapiduz/postio/issues/1156)
- **Supersedes earlier drafts** of this decision numbered 0029 and then 0030. Both numbers were taken by ADRs that landed on main while this branch was open ([ADR 0029](0029-one-control-vocabulary.md), [ADR 0030](0030-a-rule-stages-where-it-can-be-carried-out.md)), and the draft also argued from a premise that stopped being true while it was being written. Q1 has that correction; the renumbering is just what parallel sessions cost.
- **Related:** [ADR 0029](0029-one-control-vocabulary.md) (which control a setting gets — binding here too), [ADR 0019](0019-macos-frontend.md) Q1 (Native), canvas 3f, `Design/screens/22`, [#1179](https://github.com/dlapiduz/postio/issues/1179) (the GTK window this follows)
- **Decision:** **the settings *model* is shared and the *frame* is each platform's own.** The eight sections, their order, their two headings, their labels, their descriptions and the table each one writes all live in `postio_ui::settings`; so does every rule about what a change does to the file. What each frontend owns is the widget tree and the icon set. **Swift parses no TOML and writes none.**

---

## Q1 — Window or pane, and two corrections

The first draft of this ADR argued at length that macOS should use a window
*because* GTK used an in-window pane, and that ADR 0019 Q1's "wrong window
chrome" outranked `/ux-architect`'s "settings is an overlay" invariant.

**The conclusion was right and the argument was obsolete before it was
written.** `c70a830d` — landed the same day — made the GTK settings surface a
real `AdwWindow`, for the maintainer's own reason: the panes "render as
floating cards with no window frame — no way to tell whether they are tabs,
panes or a dialog." So this is not a platform divergence to be justified. Both
frontends put settings in a window, and they did so independently, which is
better evidence than the argument was.

What survives is the narrower claim, and it still matters: `⌘,` opens a window
on macOS, and the `Settings` scene is how SwiftUI provides one. The rest of
that draft's reasoning is withdrawn rather than restated.

The second correction is bookkeeping with teeth: the draft was numbered 0029,
which was already taken by *One control vocabulary* — decided from the same
screens, on the same day, by the maintainer. That ADR is not a neighbour of
this one, it is **binding on it** (Q3).

## Q2 — Where the seam falls

The settings semantics were already toolkit-free and already shipped, which
made this far smaller than [#1156](https://github.com/dlapiduz/postio/issues/1156)
assumed. `postio-config` owns `patch_ui`, `patch_keys`, `patch_sync`,
`patch_filters` and `validate::check_str`, each with tests asserting it
rewrites only its own table and leaves the rest of the file verbatim.

So the move was the *navigation model*, not the semantics: `Section`, `Group`,
`find_section`, `section_at_line` and `humanize_interval` go to
`postio_ui::settings`, and `postio-gtk` re-exports them. Two things stay
behind, and the line between them is the useful part:

| Shared | Each frontend's own |
|---|---|
| Which sections exist, their order, their two headings | The widget tree |
| Their labels, descriptions, and the table each writes | The icon set — a GTK symbolic name is not an SF Symbol |
| Every rule about what a change does to the file | — |

**Swift never parses or writes TOML.** A form that serialized its own table
would be a second writer of a file people edit by hand, with its own idea of
key order and comment survival.

## Q3 — The controls are ADR 0029's, and SwiftUI's defaults fight it

ADR 0029 decides which control a setting gets: segmented for a closed set of
three or four, a checkbox for a value in a form, a switch only for something
that *acts* when flipped, a dropdown only for an open list, a spin button
never. **That holds here.** It is not a GTK decision that macOS may reinterpret
— it is about what a setting *is*.

This needs saying because SwiftUI's defaults break it silently. `Picker`
renders as a popup on macOS unless told `.pickerStyle(.segmented)`, and
`Toggle` renders as a switch unless told `.toggleStyle(.checkbox)`. Theme and
Row density are closed sets of three; the three message-list settings are
values in a form. Every one of them is styled explicitly for that reason, and
a pane added later that forgets will look native and be wrong.

The middle density is labelled **"Snug"** on screen and `comfortable` in the
file, on both platforms. Changing one alone makes the other a lie.

## Q4 — What the platform gives for free

A SwiftUI `Settings` scene puts **"Settings…" in the application menu with
`⌘,`** by itself, correctly placed, without `MenuBar` building anything. So the
macOS half of the menu-placement problem is not a problem;
[#1207](https://github.com/dlapiduz/postio/issues/1207) is now only about
`MenuSection`'s shared vocabulary.

Opening it goes through `NSApp.sendAction(Selector(("showSettingsWindow:")))`
rather than the `openSettings` environment value. That is not a preference:
reading `\.openSettings` from a view inside the `WindowGroup` stops the **main**
window ever completing its first layout, and the app then launches, logs, runs
its event loop and draws nothing. See
`docs/notes/2026-09-05-the-app-that-ran-logged-and-drew-nothing.md`.

## Q5 — The file is the store, so read it at the moment of the write

Canvas 3f's "no second store" is usually read as "there is no staging copy to
save from". It has a second, sharper consequence that a real edit found:
**a settings window may not patch a copy of the file it is remembering.**

`config.toml` is a file people edit by hand, and `⌘E` is a command Postio
offers for exactly that. A window open across such an edit is holding a stale
copy; patching it writes that copy back over everything the editor added. In
the case that found this, one click on a segmented control destroyed an
unknown key and an entire `[sync]` table.

So a control's change is applied as a *mutation to freshly-read values*, not as
a whole struct assembled when the view was drawn: read the file, change one
field, write, read back. The read-back is the same argument once more — the
footer describes the file as it is, not as the window believes it left it.

**What is deliberately not promised:** a comment attached to the patched table
does not survive, because `patch_*` rewrites that table wholesale.
`postio_config::ui` calls this "the deliberate half of the promise". Everything
outside the patched table is promised, and that is what the tests assert.

## Consequences

- Two frontends now read `postio-config`'s patch functions, so their
  format-preservation tests protect both. They were already the right tests.
- `⌘,` needed no change: `CommandId::Settings` already carries `mod+comma`.
- **The panes will outrun what this frontend honours.** The macOS message list
  reads none of `[ui]` today — not density, not theme, not the three
  message-list settings — so the Appearance pane writes settings this
  application ignores. That is tracked, and it is the real parity gap: a pane
  wired to a file nothing reads is the same defect as a button wired to
  nothing, one layer further back.
- **The six states**, decided once so no pane re-decides them: *Empty* cannot
  occur — a missing file shows defaults and is created on the first change.
  *Loading* cannot occur; a local file read is instant and a spinner here would
  be a bug. *Offline* is the normal case, since the only network act in
  settings is Accounts' test-connection, which stays user-initiated. *Failing*
  means invalid TOML — the footer takes over from the table name and says which
  line, and the pane disables its controls rather than showing values it had to
  guess. *Partial* applies only to Accounts, whose credentials may not be in
  the Keychain yet. *Dense* honours the same three densities the list does.
