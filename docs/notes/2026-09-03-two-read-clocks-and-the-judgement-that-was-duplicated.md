# Two read-clocks, and the judgement that was duplicated (2026-09-03, #945/#797)

Marking a message read on dwell (#71) is measured by two timers, and both of
them are right: `MessageListView`'s, armed when the cursor lands on a row, and
`ConversationView`'s, armed on the focused message inside an open conversation.
ADR 0015 Q4 turns on the difference — a thread row's id is its *representative*,
the newest message in that folder, so the list's clock firing while a
conversation is open reads a message focus never reached. "Opened the thread,
all six read", in Q4's own words.

What was actually duplicated was never the timer. It was the judgement about
when to stop one, and that lived at five call sites, each naming a timer
directly and each responsible for knowing which of the two applied. Every new
surface that took the reading pane had to rediscover the whole table.

Three of the five were wrong, all in the same direction:

- opening a conversation did not stop the list's clock (#797 — it depended on
  `sync_reading_pane`'s `if !self.reading()` happening to run first, which is
  timing rather than a decision, and on a slow machine it did not);
- a single message taking the pane from a conversation stopped neither, so the
  message the reader had just left went read a moment later;
- the composer taking the pane, and the window going inactive, stopped only the
  list's — a conversation left open while its reader was elsewhere still marked
  its focused message read.

**`!self.reading()` cannot be the test, and is wrong in both directions.** It is
equally true when the pane is empty and when a whole conversation is sitting in
it, because `show_conversation` does not set that flag. Stop both clocks on it
and a conversation can never be read at all; stop only the list's — what it did
— and the composer case survives.

So the sites now say what is in front of the reader and one function decides:

| The pane is showing | list's clock | conversation's clock |
|---|---|---|
| a single message | runs — it *is* the row the cursor is on | stopped |
| a conversation | stopped | runs |
| the composer, nothing, or a window the user left | stopped | stopped |

The two timers stay two. Collapsing them would lose the distinction Q4 depends
on, and the cost of the pair was never the pair.

### The negative assertion needs a duration; the control must not

`gtk_dwell_conversation.rs` asserts a clock did *not* fire, which cannot be a
condition wait — the wait is part of what the assertion means. Its positive
controls are the opposite, and writing them the same way made the file fail
twice on a loaded machine and pass once an `eprintln!` slowed it down. Waiting
a fixed 240ms for a 60ms timer looks generous until the machine is busy.

Controls wait on the condition and answer to `POSTIO_TEST_PATIENCE`; the
negative assertions keep their duration. Both halves need a control at all,
because "nothing was marked read" is exactly as true of a build where the
dwell never armed as of one where it was correctly cancelled — without one,
that file passes with the whole mechanism deleted.
