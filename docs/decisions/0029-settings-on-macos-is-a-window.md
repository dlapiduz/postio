# ADR 0029 — Settings on macOS is a window, and the settings *language* is shared while the container is not

- **Status:** Accepted (2026-09-05)
- **Date:** 2026-09-05
- **Decision by:** `/ux-architect`, on [#1156](https://github.com/dlapiduz/postio/issues/1156), after the maintainer confirmed the macOS build should get a real settings screen with structured panes rather than a raw config-file editor.
- **Issue:** [#1156](https://github.com/dlapiduz/postio/issues/1156)
- **Related:** [ADR 0019](0019-macos-frontend.md) Q1 (Native: "a real desktop application, not a website in a window"), canvas 3f (settings *is* `config.toml`), [#869](https://github.com/dlapiduz/postio/issues/869)/[#871](https://github.com/dlapiduz/postio/issues/871)/[#873](https://github.com/dlapiduz/postio/issues/873)/[#874](https://github.com/dlapiduz/postio/issues/874) (the GTK panes, all landed), [#679](https://github.com/dlapiduz/postio/issues/679) (one artboard per pane, in the pane's own PR)
- **Decision:** **macOS opens settings in its own non-modal window; canvas 3f's content contract is preserved verbatim.** And the split is drawn at *language versus container*: `config.toml` stays the only store, `postio-config`'s existing `patch_*`/`validate` functions stay the only writer, the section model moves from `postio-gtk` into `postio-ui`, and **Swift never parses or writes TOML.** What differs per platform is the frame the panes sit in, and nothing below it.

---

## Q1 — Window or pane?

`/ux-architect` states an invariant that appears to decide this outright, and
names this very surface while doing it:

> Postio has exactly one modal dialog in the entire app, and everything else —
> the composer, the `Ctrl+K` palette, the `?` cheat sheet, **the settings
> panel** — is an overlay or a pane on the main window. That is the pattern;
> keep it.

> Detached windows are opt-in, never a default.

Taken literally that forecloses the question, and the first draft of this
decision followed it. It is wrong here, and the reason it is wrong is written
down one document over. ADR 0019 Q1 rejected the cheapest possible macOS port
— GTK, compiled for the platform — and the rejection is not about toolkits:

> the two hardest views would need rewriting anyway, on top of an app that has
> **the wrong menu bar, the wrong shortcuts and the wrong window chrome**.

A settings pane inside the main window, reached by `⌘,`, is the wrong window
chrome. It is the GNOME surface transplanted onto a platform where every
application — including every one the user has open next to Postio — puts
settings in a window of its own, and where `⌘,` has meant precisely that since
Mac OS X 10.0. Shipping the pane would satisfy the letter of "keep the
pattern" and cost the Native principle the whole frontend exists to serve.

**The invariant survives its own reasoning.** Read the rule with the
justification `/ux-architect` gives for it and the conflict dissolves:

> A modal is a claim that nothing else in the app matters until this is
> resolved.

A macOS Settings window is not modal. The main window stays live, keyboard and
all; mail keeps arriving; `⌘,` toggles rather than traps. And "detached windows
are opt-in" is a rule about surfaces that have a natural home *in* the window
and might be torn out of it — the composer is the worked example, and it stays
in the reading pane. Settings on macOS has no such home to be torn from. The
rule is a statement about Postio's surface vocabulary on GNOME, and ADR 0019's
whole architecture is that the frontend owns platform convention while the core
owns behaviour.

**Rejected: the in-window pane**, which would have made the two frontends look
alike in screenshots and behave alike in nothing a Mac user has muscle memory
for. Also rejected: **a modal sheet on the main window**, which fails the
reversibility table above — settings has no OK/Cancel to be modal *about*,
because canvas 3f already decided writes land as you make them.

### What does not change

Canvas 3f is a contract about how settings *behave*, and every clause of it
holds on macOS unchanged:

- `config.toml` is the store. There is no second store and no staging copy.
- No OK/Cancel. Edits land on a debounce; the file on disk is the truth.
- Navigation jumps to a section; it does not open a sub-screen.
- A validity line along the foot, always visible, replacing a dialog's buttons.
- "Revert file" restores the last configuration that loaded without error.
- **A structured pane patches its own table and never reserializes** — the
  format-preserving rule `postio-gtk`'s module doc explains at length, and the
  one that a Swift form rebuilding a `Config` would break silently.

## Q2 — Where the seam falls: language shared, container not

The measurement that shaped this: **the settings semantics are already
toolkit-free and already shipped.** `postio-config` owns `patch_ui`,
`patch_keys`, `patch_sync`, `patch_filters` — each `pub fn(text, value) ->
Result<String>` over `toml_edit`'s document model, each with a test asserting
it rewrites only its own table and leaves the rest of the file verbatim — plus
`validate::check_str` for the footer. `postio-config` is already a
`postio-ffi` dependency, so this crosses with no new crate edge.

So #1156's own framing — "it is not 'port 3,836 lines'" — understates how far
it is from that. Of `postio-gtk/src/settings.rs`, the pure half is roughly a
hundred lines (`Section`, `find_section`, `section_at_line`, `header_key`);
the remaining ~2,850 are `SettingsPanel`, which is GTK widget construction and
is *supposed* to be rewritten natively. That is the frontend's job, not
duplication.

**Decision:** the pure half moves to `postio-ui::settings` and `postio-gtk`
re-exports it — the fifth time this initiative has found toolkit-free logic
inside the toolkit crate, after `palette.rs` (#658), sidebar ordering (#1155),
the dwell rule (#1159) and the search chips (#1157). The boundary exposes the
section model, `validate`'s result, and one `patch` entry point per pane.

**Swift never parses or writes TOML, and this is the load-bearing clause.** A
Swift form that serialized its own table would be a second writer of the same
file with a different idea of formatting, comment survival and key order — the
exact trap the GTK module doc says a naive form falls into. Every macOS edit
goes: form → FFI patch call → returned text → same debounced atomic write.

## Q3 — Which pane first, and why not the valuable one

The panes, against what already exists:

| Pane | `postio-config` | GTK | macOS value |
|---|---|---|---|
| Appearance (`[ui]`) | `patch_ui` | structured (#873) | small — but proves the frame |
| Accounts | own path, keyring | structured (#470) | **highest**: no way to add an account on macOS at all |
| Keyboard (`[keys]`) | `patch_keys` | still raw text | high — 31 bindings, all rebindable |
| Sync (`[sync]`) | `patch_sync` | structured (#874) | medium |
| Privacy | none — XDG state, not `config.toml` | structured (#871) | medium, and it is a privacy feature |
| Filters (`[filters]`) | `patch_filters` | structured (#869) | medium |

**Appearance ships first, and it is deliberately not the valuable one.** The
risk in this work is the frame — window, nav, validity footer, debounced
atomic write, the patch round trip — and not the fields. Appearance is the
smallest pane with a landed patch function, so it proves the frame with the
least in the way. Every pane after it is then a form and a `patch_*` call.

Accounts is the highest-value pane and still does not go first: it is the
largest, it is the only one touching the Keychain, and it is the only one that
can make a network request. Landing it on an unproven frame would confound
"does the frame work" with "does Keychain provisioning work" — two risky
things, one PR, no way to read a failure. A Mac user is also not fully blocked
today, because `postio-provision` exists.

Order: **Appearance → Accounts → Keyboard → Sync → Filters → Privacy.**
Privacy is last because it is the one pane with no `config.toml` table at all
— its allow-list lives in `$XDG_STATE_HOME` on Linux, and where that belongs
on macOS is a portability question of its own rather than a form.

Per #679 and canvas 3f, **each pane gets its own artboard in its own PR.**

## Q4 — The menu placement this exposes

`postio_core::menu::section_for` currently answers `MenuSection::Edit` for
`Settings`, `EditConfig` and `AddAccount`. On macOS all three belong in the
**application menu** — "Postio → Settings…" is where `⌘,` is discoverable, and
an Apple reviewer would call its absence from there a defect.

This is the same shape as the decision above one level down: `MenuSection` is a
shared vocabulary, and "which physical menu" is a platform mapping. Either
`MenuSection` gains an `App` variant that freedesktop folds into Edit, or the
Swift menu builder maps those three. **Decided: add the variant**, because the
alternative puts a hand-maintained list of command ids in Swift, which is the
thing #1158 was about removing.

## Consequences

- The `⌘,` binding needs no change: `CommandId::Settings` already carries
  `default_binding: "mod+comma"`, and `expand_mod` resolves it to `cmd` on
  Apple.
- `⌘E` (`EditConfig`) keeps its meaning — hand the file to the user's editor.
  It is not the settings window and does not open it.
- A second frontend now reads `postio-config`'s patch functions, so their
  format-preservation tests protect two surfaces. They were already the right
  tests; they are now load-bearing twice.
- **The six states**, decided here so no pane re-decides them:
  *Empty* cannot occur — a missing file shows defaults, footer says it is
  created on first edit. *Loading* cannot occur; a local file read is instant
  and a spinner here would be a bug. *Offline* is the normal case: everything
  works, because the only network act in settings is Accounts' test-connection,
  which is user-initiated and stays so. *Failing* means invalid TOML — the
  footer names the error and its line, and a pane whose table will not parse
  disables its controls rather than showing values it had to guess. *Partial*
  applies only to Accounts, whose credentials may not be in the Keychain yet.
  *Dense* honours the same three densities the list does.
