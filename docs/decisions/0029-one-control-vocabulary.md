# ADR 0029 — One control vocabulary: segmented for a closed set, a checkbox for a value, a switch for an act

- **Status:** Accepted (2026-09-05)
- **Decision:** The GTK control a setting gets follows from what the setting is, not from what is convenient to build. Rules below hold in settings, onboarding and anywhere a choice is offered.

## Rules
- A closed set of up to about four options is a `postio_widgets::widgets::SegmentedControl`: grouped `gtk::ToggleButton`s with a neutral selected fill (the accent is reserved for markers, the focus ring and the has-action toggle, spec 007 FR-091).
- A boolean that is a `config.toml` value is a `CheckRow` (a `gtk::CheckButton`).
- A `gtk::Switch` is only for an act that takes effect asynchronously elsewhere. The account-enable toggle is the one in settings.
- An open or long list is a `DropDown` (for example the signature picker).
- A spin button is never used. A number a person must type goes in the `Config file` pane. `Check for mail` offers `IMAP IDLE / Every 5 min / Manual`, writes `poll_interval_secs = 300` only when the mode changes to polling, and reports a hand-set interval rather than rounding it.
- `CheckRow::set_active` and `SegmentedControl::set_selected` are silent (no `toggled` echo), so panes hold and update controls rather than rebuild them.
- Section headings use `widgets::kicker`; fact lines use `widgets::stat_line`. Both frames (ADR 0043) draw these shared classes.
- Every control has a hover state and an accent `:focus-visible` ring.
- A control the design asks for and nothing implements is not built (no `Compact index`, no keybinding-set switcher or mutt importer). A button wired to nothing is worse than none.

## Why
- A segmented control is a different widget from a dropdown. A dropdown hides a three-answer vocabulary, and no stylesheet converts one to the other.
- A switch promises an effect elsewhere; a checkbox promises a form value. Settings are the file, so they are the second kind.
- Grouped toggles give arrow-key behaviour and radio-group accessibility free.
- Deciding pane by pane yields three idioms.

## Revisit when
- A choice set exceeds about four and must stay visible.
