# ADR 0031 — The settings window is one model in two frames

- **Status:** Accepted (2026-09-06)
- **Decision:** The settings model is shared in `postio_ui::settings`; each platform owns its frame. Sections, order, headings, labels, descriptions, the table each writes and every rule about what a change does to the file are shared. The widget tree and icon set are not. Swift parses and writes no TOML.

## Rules
- Frames: macOS opens the SwiftUI `Settings` scene (`Cmd+,`). Focus opens a dialog over the list (`postio_widgets::settings::SettingsPanel`, spec 007 screens.md "Settings"). A frame may omit a section whose keys it does not honour (Focus shows no Appearance).
- Settings semantics live in `postio-config` (`patch_ui`, `patch_keys`, `patch_sync`, `patch_filters`, `validate::check_str`). Each rewrites only its own table and leaves the rest of the file verbatim. A comment attached to the patched table does not survive.
- ADR 0029 binds every frame. In SwiftUI set `.pickerStyle(.segmented)` for closed sets and `.toggleStyle(.checkbox)` for values explicitly, because the defaults are a popup and a switch.
- The middle density is "Snug" on screen and `comfortable` in the file, on both platforms.
- A control's change is a mutation applied to freshly read values: read the file, change one field, write, read back. Never patch a copy remembered from drawing the view, because `config.toml` is hand-edited.
- Open Settings on macOS with `NSApp.sendAction(Selector(("showSettingsWindow:")))`, never `\.openSettings`. Reading it inside the `WindowGroup` stops the main window completing first layout (`docs/notes/2026-09-05-the-app-that-ran-logged-and-drew-nothing.md`).
- `MenuBar.install` replaces `NSApp.mainMenu`, so the menu model needs an `App` section (folded into Edit on freedesktop) that carries Settings (#1207).
- Pane states: Empty cannot occur (defaults, file created on first change). Loading cannot occur. Offline is normal (only Accounts' test-connection touches the network, user-initiated). Failing means invalid TOML: the footer names the line and controls disable. Partial applies to Accounts only. Dense honours the list's three densities.
- A pane must not write settings its frontend ignores. The macOS list reads none of `[ui]` yet, so Appearance is a parity gap.

## Why
- The semantics were already toolkit-free. A Swift form serializing its own table would be a second writer of a hand-edited file.

## Revisit when
- A frontend honours a different set of keys than the shared model lists.
