# Contract: Calendar mode, its keys, and pop out

The spec's D1 to D4 and D12, and FR-001 to FR-009, are the behaviour. This
file pins what the tests assert.

## Commands (`postio-core::registry`)

| Id | Title | Key | Context | Recovery |
|---|---|---|---|---|
| `go_to_calendar` | Calendar | `g e` | the `GO_SURFACES` contexts | None |
| `calendar_agenda` | Agenda | `1` | Calendar | None |
| `calendar_day` ⛔ handoff | Day | `2` | Calendar | None |
| `calendar_week` ⛔ handoff | Week | `3` | Calendar | None |
| `calendar_month` ⛔ handoff | Month | `4` | Calendar | None |
| `calendar_previous` | Earlier | `[` | Calendar | None |
| `calendar_next` | Later | `]` | Calendar | None |
| `calendar_today` | Today | `t` | Calendar | None |
| `next_message`, `prev_message` (existing) | Next, previous | `j`, `k` | Calendar too | None |
| `first_message`, `last_message` (existing) | First, last | `g g`, `G` | Calendar too | None |
| `open_message` (existing) | Open event | `Return` | Calendar too | None |
| `search` (existing) | Search events | `/` | Calendar too | None |
| `pop_out` | Open in its own window | `mod+shift+o` | Calendar | None |
| `pop_in` | Back into the main window | `mod+shift+o` | Calendar (own window) | None |

- A new `Context::Calendar`, and a `Context::CalendarWindow` for the
  popped-out window, so that `pop_out` and `pop_in` share a chord without
  colliding.
- Reusing an existing id means adding `Context::Calendar` to its contexts.
  Its title in the command bar comes from a per-context title, where one
  exists ("Open event"). No second command is made for the same verb.
- `docs/keybindings.md` and `crates/postio-core/tests/golden/linux-bindings.txt`
  regenerate in the same commit.

## Behaviour (`postio-focus`)

| Given | When | Then |
|---|---|---|
| mail, anywhere in `GO_SURFACES` | `g e` | Calendar mode on Agenda, today, cursor on the next event that has not ended |
| Calendar | `Esc` at its root | back to where it was opened, with the cursor and scroll kept |
| Agenda | `j` past the loaded page | the next page loads, and the cursor moves on. No jump, no spinner |
| Agenda | `]` | a week later. `t` returns to today |
| Calendar on a date with the cursor on an event | `mod+shift+o` | an own window on the same date and event. The main window is back where Calendar was opened from |
| Calendar popped out, the main window focused | `g e` | the Calendar window is presented and focused. The main window shows no Calendar mode |
| the Calendar window | `g i` | the main window is presented and focused, on the inbox |
| the Calendar window at its root | `Esc` | nothing |
| the Calendar window | `mod+shift+o`, or its X | Calendar is the main window's mode again, with the same view, date and cursor |
| popped out at quit | the next start | the main window is usable first, then the Calendar window opens at its stored size |
| the terminal | — | `pop_out` is not offered (`registry::offered_on`) |

## States

Every row of the spec's "Every state" table is one `postio-ui` unit case on
the strings, plus one frontend case per path a person takes (ADR 0049).

## Reads (counted)

- An Agenda page is a fixed number of statements, independent of the store's
  size. `crates/postio-host/tests/host_suite/calendar_agenda.rs` holds the
  count at 50,000 occurrences.

## Storyboards

- `storyboards/calendar/agenda-walk.toml`: `g e`, `j` `j`, `Return`, `Esc`,
  `Esc` back to the same inbox row.
- `storyboards/calendar/pop-out.toml`: `g e`, `mod+shift+o`, `g i` in the
  Calendar window, `g e` from mail, `mod+shift+o` back.

Both are red on `main` and judged by `/ux-review`.
