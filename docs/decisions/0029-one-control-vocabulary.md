# ADR 0029 — One control vocabulary: segmented for a closed set, a checkbox for a value, a switch for an act

- **Status:** Accepted (2026-09-05)
- **Date:** 2026-09-05
- **Decision by:** the maintainer, reporting against the running app, with new flow screens from the designer (`Design/screens/21`, `22`, `23`).
- **Issue:** [#1179](https://github.com/dlapiduz/postio/issues/1179)
- **Related:** `PRODUCT.md` §19 (visual identity), `Design/Mail Client.dc.html` (PLATE 1b), [#1174](https://github.com/dlapiduz/postio/issues/1174) (action-bar buttons, same root cause), [#873](https://github.com/dlapiduz/postio/issues/873) / [#874](https://github.com/dlapiduz/postio/issues/874) (the structured panes these controls replace the insides of)
- **Decision:** **which GTK control a setting gets is decided by what the setting *is*, not by what is convenient to build.** Four rules, below, and they hold across settings, onboarding, and anywhere else a choice is offered.

---

## Why this needed deciding at all

The settings view was built from 11 `Entry`, 8 `DropDown`, 6 `Switch` and 6
`SpinButton`. The design specifies, for the same settings, segmented controls
and square checkboxes, and no spin button anywhere. That is not a styling gap:
`shell.css` cannot turn a `DropDown` into a segmented control, because a
segmented control is a different widget with different behaviour.

It is also not only a visual mismatch. A switch and a checkbox make different
promises to the reader. A switch reads as *this takes effect somewhere else,
possibly in a moment* — the GNOME convention, and the reason a switch has an
animation. A checkbox reads as *this is a value in the form I am filling in*.
Every boolean in Postio's settings is the second kind: it is a key in
`config.toml`, and the window and the file are the same thing. Six switches
were making the first promise about the second kind of thing.

Likewise a dropdown hides its own vocabulary. `System / Light / Dark` behind a
dropdown means a person cannot see that there are three answers without
opening it first, and three answers is exactly the case where seeing them all
is the point.

Filing one decision rather than six pane-sized ones, because fixing one pane
to match while the others stay is how a settings view ends up with three
idioms.

## Q1 — A closed set of three or four options is a **segmented control**

`System / Light / Dark`. `Airy / Snug / Compact`. `IMAP IDLE / Every 5 min /
Manual`. `Above the quote / Below the quote`.

Joined, square-cornered buttons with the chosen one filled in the accent.
Built from grouped `gtk::ToggleButton`s, so GTK supplies the keyboard
behaviour and the accessibility for free: arrow keys move within the group,
exactly one member is ever active, and a screen reader announces a radio
group rather than three unrelated buttons.

`postio_gtk::widgets::SegmentedControl` is the one implementation.

**The bound is roughly four.** Past that the row stops fitting and the
labels start abbreviating, and an abbreviation is a worse dropdown.

## Q2 — A boolean is a **checkbox**, and a switch is for an act

Square indicator, filled when checked, label beside it. `gtk::CheckButton`
under `postio_gtk::widgets::CheckRow`.

A `gtk::Switch` stays legitimate for a control that *does* something when
flipped, asynchronously and elsewhere — enabling an account, which reconnects
it. It is not legitimate for a value that is written to a file. As of this
ADR the settings window has no switch, and the account-enable toggle is the
only switch in the application that would qualify.

### Why `CheckRow` wraps `CheckButton` at all

The guard. Setting a `CheckButton`'s state fires `toggled`, so a pane
redrawing itself from a fresh read of the file writes the value straight back.
The old panes worked around this by connecting the handler *after* setting the
state, and rebuilding the whole row on every change — which works right up
until something needs to update a control it did not itself just build.
`CheckRow::set_active` and `SegmentedControl::set_selected` are silent by
construction, and that is what lets the panes hold their controls and update
them instead of rebuilding them.

## Q3 — An open or long list is a **dropdown**; a number is neither

A dropdown is right when the options are not knowable in advance or will not
fit on a line: the account's signature picker has as many entries as the
account has signatures. That one stays a `DropDown`.

**A spin button is never right in this application.** Six existed; the design
has none. A number that a person genuinely needs to type — a poll interval in
seconds — is not a choice between three things, and the surface for typing a
number into this file is the `Config file` pane, which the footer names from
every other pane. So `Check for mail` offers `IMAP IDLE / Every 5 min /
Manual`, writes `poll_interval_secs = 300` when and only when the mode
*changes* to polling, and states the interval it actually found underneath —
an interval somebody set by hand survives pressing the segment it is already
on, and is reported rather than rounded away.

## Q4 — Chrome: kickers and stat lines are shared, not re-typed per pane

A section heading is letterspaced small caps in the heading face
(`widgets::kicker`); a line of facts under a group is mono
(`widgets::stat_line`). Neither is a control and neither is worth a struct,
but the *classes* are the shared thing — a kicker that is 0.7rem in one pane
and 0.75rem in the next is the drift `postio_gtk::widgets` exists to stop.

## Consequences

- `postio_gtk::widgets` gains `SegmentedControl`, `CheckRow`, `kicker` and
  `stat_line`, beside the `KeycapButton`, `ActionBar` and `NoticeBar` already
  there for the same reason.
- Every control needs a hover state and a `:focus-visible` ring drawn from
  the accent token. No control relies on the default ring.
- **A control the design asks for and nothing implements does not get built.**
  Two from the new screens are deliberately absent: `Compact index`, because
  no command compacts an index, and the `Mnemonic / Vim / Emacs` keybinding-set
  switcher with `Import mutt bindings`, because this build has one set of
  defaults and no importer. A button wired to nothing is worse than a button
  that is missing — it is `check-uncalled-pub-fn`'s rule, applied to the
  surface rather than to the code.
- Onboarding uses the same controls for the same kinds of choice, which is
  the third box on #1179 and the reason this is an ADR rather than a comment
  in `settings.rs`.
