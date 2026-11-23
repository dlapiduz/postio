# Quickstart: seeing each slice work

How to see each slice working, and which test proves it. Nothing here touches
the network: sync runs against the fake backend or the in-process wire server.

## Slice A: local calendar and Agenda

```bash
scripts/test-fast.sh                                          # postio-calendar expansion, postio-ui agenda rows
cargo nextest run -p postio-host --test host_suite calendar_agenda
cargo nextest run -p postio-gtk --test focus_suite calendar
scripts/storyboards.sh calendar/agenda-walk
```

In the app (`scripts/run-isolated.sh`, the demo store with its invitations):

1. press `g e`. Agenda opens on today;
2. `j` and `k` walk it, and `Return` opens an event;
3. `Esc` `Esc` returns to the inbox row;
4. press `y` on an invitation in the inbox. Its event appears in Agenda, and a
   clashing invitation's marker says "Clashes with …".

## Slice A2: pop out

```bash
cargo nextest run -p postio-gtk --test focus_suite calendar_pop_out
scripts/storyboards.sh calendar/pop-out
```

In the app:

1. in Calendar, press `mod+shift+o`. Calendar is in its own window;
2. `g i` there focuses mail, and `g e` in mail focuses Calendar;
3. close the Calendar window. Calendar is a mode again, on the same date.

`POSTIO_HEADLESS=0` shows both windows.

## Slice B: management

```bash
cargo nextest run -p postio-host --test host_suite calendar_verbs
```

1. Press `c`, type "Dentist", tab to the time, type "thu 3pm", and press
   `mod+s`.
2. Press `v`, then "Next week".
3. Press `#`, then `z`. Each step repaints at once.

## Slice C: CalDAV sync

```bash
cargo nextest run -p postio-dav --test dav_suite
cargo nextest run -p postio-sync --test sync_suite calendar
```

A live check against a real account is `cargo nextest run -p postio-dav
--run-ignored only live_`, with credentials in the environment. It is never
run in CI.

## Slice D: reminders

```bash
cargo nextest run -p postio-host --test host_suite calendar_reminders   # fake clock, start_paused
```

## Slices E and F: held for the designer handoff

There is nothing to run until the handoff arrives and the held tasks are
released.
