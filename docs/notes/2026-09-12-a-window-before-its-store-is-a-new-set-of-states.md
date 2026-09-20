# A window before its store is a new set of states (2026-09-12, #1114)

Postio opened its store before `app::build_with` was called at all, so there
was no application — let alone a window — until the store had succeeded or
been refused. Presenting the window first is a small change to `run` and a
large change to what states the application can be in. Four of them did not
exist before, and each had to be decided rather than discovered.

## The window is not the budget any more

`Phase::FirstFrame` meant "the compositor showed the first frame" and that was
also the moment the mail arrived, so one mark answered both questions. It does
not any more: pixels arrive early and mail arrives when the store opens. A
timeline that closed at the first frame would report **200 ms, ok** on a start
that took twelve seconds to show anything a person could read.

So `shell` is the pixels and `FirstFrame` is the frame with mail in it, marked
by whoever fed the panes. **When a budget's subject moves, the mark has to
move with it** — a phase list that is merely in the wrong order produces a
wrong number rather than an obvious failure.

## Saying nothing takes code

A list pane with no sync status derives `Offline` from its own defaults, so a
window presented ahead of its store drew a full-pane "Offline — reading local
mail" over mail it had not opened, and then replaced it. The absence of a
plate is not the absence of a decision.

`State::Opening` therefore answers *ahead* of every other state rather than
being one more arm of the match: with no store there is no connection worth
describing, no mailbox to be empty and no query to have matched nothing.

## A verb that cannot run must not be offered — but silence is not a refusal

The registry already had the mechanism (`CommandSpec::requires`) and its docs
already named the extension point. Two things it did not have:

- **One slot is not enough.** `Move` needs a single account *and* a store, and
  `Option<Requirement>` could express either. It is a set now.
- **`registry::available` had no callers.** The one function documented as
  "the one place a surface asks can the user do this right now" was dead:
  nothing filtered key dispatch, only the palette and the cheat sheet
  filtered at all. Worth knowing before assuming a requirement is enforced
  anywhere.

The keyboard refuses out loud, and the refusal is gated on there being a wait
to *name* rather than merely on the store being shut. Without a sentence there
is nothing to say, and blocking silently would be the bug it exists to fix
wearing a different hat. It also keeps every widget test — hundreds of windows
built with nothing behind them and keys pressed at them — meaning what it
meant before.

## A guard that is set at the far end of a chain guards nothing at the near end

`fed` stops a second `activate` re-wiring the window, and it is set once the
store has landed and the panes are being fed. A second launch arriving *during*
the open finds it false: a second thread, a second runtime and a second set of
engines over one file. Anything that can now take twelve seconds needs its own
"already in progress" flag, at the point the work starts rather than where it
finishes.

## The waits are four, not one

`open_store` and `Database::open` grew reporting variants because only they
know which wait is running. From the outside they are not the same promise:
reading a file, rewriting a schema, and rebuilding an index are three
different sentences, and the keyring read before them is a fourth and the one
least under Postio's control. The instrument fires for every stage including
the instant ones — whether a stage is worth mentioning is a question about how
long it takes, which the caller finds out by still being in it.
