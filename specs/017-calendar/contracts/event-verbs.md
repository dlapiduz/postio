# Contract: event verbs, invitations and reminders

The spec's D4 to D8, and FR-010 to FR-022 and FR-040 to FR-043.

## Commands

| Id | Title | Key | Context | Recovery |
|---|---|---|---|---|
| `new_event` | New event | `c` | Calendar | Undo |
| `edit_event` | Edit event | `mod+e` | Calendar, open event | Undo (on save) |
| `delete` (existing) | Delete event | `#`, `Delete` | Calendar too | Undo |
| `move_event` | Move to… | `v` | Calendar, open event | Undo |
| `duplicate_event` | Duplicate | `mod+shift+d` | Calendar, open event | Undo |
| `accept_invite`, `decline_invite` (existing) | Accept, decline | `y`, `Y` | Calendar too | Undo briefly |
| `tentative_invite` | Tentative | `alt+y` | List, thread, reader, Calendar | Undo briefly |
| `save_event` | Save | `mod+s` | Event editor | Undo |
| `save_and_send_event` | Save and send | `mod+Return` | Event editor, with attendees | Undo briefly |
| `email_attendees` | Email attendees | `alt+e` | Calendar, open event | None |
| `snooze_reminder` | Snooze 5 min | the notification action | notification | None |
| `open_reminder` | Open | the notification action | notification | None |

## Rules (one unit case each, in `postio-calendar` or `postio-host`)

| Given | When | Then |
|---|---|---|
| any verb | it runs | the store write and `event_changes` row exist before the repaint. The test's counting transport sees no connection before the repaint |
| a recurring event | `delete`, `move_event` or `save_event` | the scope picker asks This, This and following, or All. This writes an exception, This and following truncates the rule and starts a new event, and All changes the master |
| attendees | `save_event` | nothing is sent and no outbox row exists |
| attendees | `save_and_send_event` | after the undo window: with `calendars.schedules`, a PUT; otherwise an iTIP REQUEST through the outbox |
| an invitation by mail, a non-scheduling account | `y` | `my_partstat` is written, the event lands in "On this computer" or the account's calendar, and one iTIP REPLY is queued, as today |
| an invitation, a scheduling account | `y` | a PUT of the object with the person's `PARTSTAT`, and no mail reply |
| `y` then `mod+z` within the window | — | nothing is sent, and the event is gone |
| an invitation that overlaps a local occurrence | it is filed | its marker chip is `Clashes("Standup")`, read from `event_occurrences`, with no network |
| a `DISPLAY` or `AUDIO` alarm, an app holding the store | the fake clock reaches it | one notification, "Standup · 09:30 · in 10 min" |
| an `EMAIL` alarm | it is due | nothing: no notification and no mail |
| an alarm that fell due while no app ran, under 12 hours ago | start | one notification, "While Postio was closed: …", and `missed`, then `fired` |
| an unmodelled property in the object | any edit | the property's bytes are identical after the edit |

## Privacy

- Logs carry event ids, counts and outcomes only. No title, location, notes,
  attendee or calendar name.
- Postio fetches no URL in an event. A test over hostile event fixtures (URLs
  in notes, `ATTACH;VALUE=URI`, `CONFERENCE`) asserts zero connections.
