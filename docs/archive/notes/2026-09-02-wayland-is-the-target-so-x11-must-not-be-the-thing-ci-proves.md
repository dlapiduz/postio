# Wayland is the target, so X11 must not be the thing CI proves (2026-09-02, #830)

Maintainer, asked directly: *"I don't mind Wayland only. There is no plan to
support x11."* That settles a question the test harness had been quietly
answering the other way.

`scripts/headless-runner.sh` fails open by design — anything wrong and it
execs the binary unchanged, because a broken runner must never be a broken
suite. CI leaned on that: it started an Xvfb display, and if the nested
`mutter --headless` did not come up the suites landed on X11 instead and
passed. A green tick, on a configuration Postio does not ship.

The part worth remembering is not the Xvfb dependency, it is that **nothing
in the log said which of the two had happened.** The runner announced one
fallback and took the other in silence, so grepping a full CI job log for
`postio runner:` returned zero lines — equally consistent with "mutter worked
perfectly" and "mutter never started". Two opposite configurations, one
silence. A fallback that does not name itself turns a green run into an
unfalsifiable claim, which is the same failure as the sixty tests that
skipped themselves for want of a display and reported success (#114).

So: every fallback path says which display it chose and why, and there is no
X11 to fall back to. If the compositor does not come up, `gtk_display_required.rs`
fails the run loudly rather than sixty suites skipping quietly.

The `.unavailable` marker from #794 had the same shape of bug in a smaller
way. It exists so twenty test binaries do not each pay ten seconds
rediscovering that the compositor will not start — sound as a per-run
shortcut, except `XDG_RUNTIME_DIR` outlives the run and nothing ever removed
the file. The nested compositor here exited nine hours into a session; every
suite afterwards would have been demoted to the session's display until
someone deleted a file they did not know existed. **A cache of a negative
result needs an expiry, or it stops being a cache and becomes a decision.**
