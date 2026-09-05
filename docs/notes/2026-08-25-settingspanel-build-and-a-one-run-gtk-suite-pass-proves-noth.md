# `SettingsPanel::build()` and a one-run `gtk_suite` pass proves nothing (#873, #880, #881)

`Window::new` constructs `SettingsPanel` as a hidden overlay child while it
is still wiring up its own overlay siblings and shortcut controllers, and
#873 found a real, deterministic case of that mattering: building a
`gtk::DropDown` (`[sync]`'s structured pane) during that window corrupted
keyboard routing for the rest of it — `gtk_finder`, `gtk_finder_focus`,
`gtk_move_picker`, `gtk_toggle_sidebar` all failed, reliably, and reliably
stopped failing once `redraw_sync`/`redraw_ui`'s calls were removed from
`build()`'s own trailing sequence. That bisection ran the filter multiple
times each side and the signal held. #880's account-detail view hit the same
signature with `gtk::Entry`/`gtk::SpinButton` and the same fix — `OnceCell`-
backed lazy fields built on first `open_account_detail()` — generalized it
one step: any widget with its own internal event controllers, built during
`build()`.

#881 looked like a third confirmation and was not one. Its capture
controller — one plain, hand-written `gtk::EventControllerKey`, added
directly to an already-built `gtk::ListBox` — was built during `build()`,
and removing that one `add_controller` call made a full-`gtk_suite`
segfault disappear on the first retry. Deferring it (an `installed:
Cell<bool>` guard, tripped from `redraw_keys()` instead of `build()`) then
also appeared to fix it, once. Neither observation survived a second look:
the *same* segfault, and a *different* single-test failure
(`gtk_settings::the_settings_panel_edits_the_file_in_place`), each showed up
again in further runs of the exact same commit, deferred controller and
all — and then reproduced identically on `ef1bb529`, the commit immediately
before any of #881's widget code existed at all, run three times. This
machine was carrying heavy concurrent load throughout, and the honest
conclusion is a pre-existing, load-dependent `gtk_suite` flake unrelated to
`SettingsPanel`, not a fourth widget joining the pattern. Filed as #1015
rather than chased further inside #881.

**What #873 and #880 established stands: a widget with its own internal
event controllers, built during `SettingsPanel::build()`, has a real,
reproducible failure mode, and the fix is deferring construction to the
first real interaction after `Window::new` has finished** — the same shape
`redraw_sync`/`redraw_ui`'s removal, and `open_account_detail`'s
`OnceCell`s, both use. What #881 adds is a warning about the *evidence bar*
for a new case: **one clean `gtk_suite` run, or one run that stops
segfaulting after a change, is not confirmation on a machine this loaded.**
Bisect by running each side two or three times, not once each, before
writing up a "confirmed" fix — and when a single-run result doesn't hold up,
correct the write-up rather than leave a false attribution for the next
session to build on. #881's own controller was left deferred anyway, on the
precautionary principle #873/#880 established, even though nothing pinned
this particular crash on it.
