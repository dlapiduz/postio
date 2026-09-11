---

description: "Task list for the compose editor"
---

# Tasks: The Compose Editor

**Input**: Design documents from `/specs/002-compose-editor/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/](./contracts/)

**Tests**: **Required, not optional.** Constitution IV is non-negotiable — the
failing test is written and observed failing before the code that satisfies it.
Most of this feature already works and nothing asserts it, so a large share of
these tasks are tests over existing behaviour. That is the point, not filler.

**Branch**: one `feature/compose-editor`, no issue per task (constitution 1.1.0).
Commits end `Refs: specs/002-compose-editor` and the task id.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: can run in parallel — different files, no dependency on incomplete work
- **[Story]**: US1–US6, matching the user stories in [spec.md](./spec.md)

## Path Conventions

Rust workspace. `postio-body` and `postio-ui` are toolkit-free and testable with
no display; `postio-gtk` holds WebKit glue and display tests under
`crates/postio-gtk/tests/gtk_suite/`.

---

## Phase 1: Setup

**Purpose**: the branch, and nothing else — this feature adds no dependency and
no new crate.

- [X] T001 Cut the worktree and branch: `git worktree add ~/src/postio-worktrees/compose-editor -b feature/compose-editor origin/main`, then run `scripts/install-shims.sh` in it
- [X] T002 Confirm the baseline is green before changing anything: `scripts/test-sanity.sh` and `scripts/check.sh`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: the governance gate, and the one structural move both the
appearance and quote work depend on.

- [X] T003 Write the ADR superseding the reply-construction property of ADR 0003 and ADR 0004 in `docs/decisions/0033-a-reply-quotes-what-the-reader-shows.md` — **written; landing on `docs/adr-reply-quote-fidelity`**: state that FR-044 carries the sender's sanitised HTML into the quote, that FR-047 keeps the permitted set identical to the reader's, what is gained (a quote that looks like the message answered) and what is given up (the closed type's guarantee that a script has no representation). **Lands on its own `docs/` branch, not this one** — see "How This Lands" in plan.md. **Blocks T024–T030 only.**
- [X] T004 [P] Create `crates/postio-ui/src/editor/mod.rs` and `document.rs` as an empty module mirroring `reader/document.rs`, wired into `postio-ui/src/lib.rs`, with no behaviour yet

**Checkpoint**: T003 unblocks the quote work. T004 unblocks US4. Neither blocks US1, US3, US5 or US6.

---

## Phase 3: User Story 1 - Write and send without touching the mouse (Priority: P1) 🎯 MVP

**Goal**: the whole feature in one gesture — `c` to sent, hands never leaving the keyboard.

**Independent Test**: drive the composer from `c` to sent with key events only, asserting on what is on screen at each step.

### Tests for User Story 1

- [X] T005 [P] [US1] Assert every composer command appears in the palette and the `?` sheet with its binding, in `crates/postio-gtk/src/cheatsheet.rs` (**not** `postio-core`, which sits below both surfaces and can see neither) — FR-001, FR-002
- [X] T006 [P] [US1] Assert focus order across recipient, subject and body is defined and reversible, in `crates/postio-gtk/tests/gtk_suite/gtk_composer_focus.rs` — FR-003. Needed `composer::Field` to grow `Cc`, `Bcc` and `Subject` first, plus a single `widget_for` mapping; driven with `child_focus` (what GTK runs for Tab) rather than a helper. **Attachment controls are not in the walk** — the attach button is a `gtk::Button` outside the field order, so FR-003's "and attachment controls" is not yet covered
- [X] T007 [P] [US1] Assert single-character bindings do not fire while typing, in `crates/postio-gtk/tests/gtk_suite/gtk_composer_keymap.rs` — FR-004. Covers subject (a `GtkText`) and body (a `contenteditable` `WebView`), which different code answers for. **Trap for the next test:** use `Window::composer`, not `composer::install` — only the former records the composer in the window's slot, which is what `is_typing` asks
- [X] T008 [P] [US1] Assert revealing Cc/Bcc keeps the draft and the caret, in `crates/postio-gtk/tests/gtk_suite/gtk_composer_recipients.rs` — FR-020 (reveal half only; see T008a and T008b)
- [X] T008a [US1] **FR-020 is half built: there is no way to hide Cc/Bcc again.** `cc_row.set_visible(false)` runs once at construction and nothing offers the reverse. Needs an affordance designed before it is coded — a toggle on `+ Cc`, or hiding an empty field on blur — and the requirement says hiding MUST NOT drop addresses already entered, so whichever is chosen has to answer that
- [X] T008b [US1] **FR-001 gap: `+ Cc` is not a command.** It is a `gtk::Button` with an accessible label and nothing in `postio-core::registry`, so it is reachable by Tab and absent from the palette and the `?` sheet — Constitution II's "a command that is not in the registry does not exist". Registering it is the fix; the open question is whether it takes a default binding or is palette-only. **Note:** a palette-only (keyless) command would fail T005's "no composer row names a command and no key" assertion, so that assertion has to be revisited in the same change. **Decided and built together**, as one verb: `CommandId::CopyFields`, `ctrl+shift+c`, on the `mod+shift+<letter>` shelf the other secondary composer verbs use — so it takes a binding and T005's assertion stands unchanged. The toggle is asymmetric: raising always works, putting away only while both fields are empty, which is the rule `resume` already holds (these rows are visible *because* something is in them) and which makes "hiding MUST NOT drop addresses" true by construction. Hiding a row that still held addresses was rejected as worse than the dead end — the recipients stay on the draft and still get sent, unseen — and hide-on-blur as silent state loss. A refusal moves the keyboard to Cc rather than doing nothing
- [X] T009 [P] [US1] Assert sending never awaits the network: with no connection the composer closes and the send queues, in `crates/postio-app/tests/app_suite/` — FR-065. **Already asserted**, exactly: `send_wiring::ctrl_return_queues_the_draft_for_sending` never calls `start_syncing`, so the `Operation::Send` it leaves simply waits — and it checks both halves, `draft.state == DraftState::Queued` and `!composer.is_open()`. Verified green
- [X] T010 [P] [US1] Assert a send is reversible for the grace period and findable in Sent after it, in `crates/postio-app/tests/app_suite/` — FR-059. **Both halves already asserted.** Reversible: `app_suite/resume_queued_draft::return_on_a_queued_draft_row_cancels_the_send_and_reopens_it_for_editing` drives the user-visible path (Return on the queued row), over `DraftRepository::cancel_send`'s own three outcomes in `storage_suite/drafts.rs` — `Cancelled`, `NotQueued`, `AlreadyInFlight`. In Sent: `sync_suite/send::sending_a_draft_delivers_it_and_files_a_sent_copy` lists the Sent mailbox rather than trusting the APPEND. Verified green — but see T010a
- [X] T010a [US1] **The registry promises an undo the app does not offer.** `CommandId::Send` declares `Recovery::Undo` (`postio-core/src/registry.rs:600`), and the UX invariant reads that as "reversible from the undo stack, 'Undo' toast, `u` works". None of the three hold: `UndoKind` has no `Send` variant, nothing records a send on the stack, and the only route back is reopening the draft from Drafts (`postio-app/src/compose.rs:308`), which shows `SEND_CANCELLED` and not an undo toast. So `u` after a send reverses whatever unrelated action was last on the stack. FR-058 is satisfied in *mechanism* and mis-declared in the registry; which way it resolves — wire `u` to `cancel_send`, or correct the declaration — was a design call, and it is decided: **the registry was making the wrong promise, not merely an unimplemented one.** `Recovery::Undo` means "reversible from the undo stack, `u` works", and a send is reversible from the *queue* — a window of seconds — while that stack takes message operations and keeps them ten minutes. An entry recorded there would outlive what it can act on, sit at the top shadowing the archive beneath it, and answer `u` with "too late". So `Recovery::Window` is the fourth answer, `Send` carries it, and `docs/keybindings.md` says "Undo briefly" rather than "Undoable" — which is where somebody looks to find out which key does what. The affordance itself (the toast's own Undo, live only while the window is) is the follow-on, with the decision made

### Implementation for User Story 1

- [X] T011 [US1] Fix whatever T005–T010 found. If they all pass, record in the commit body that this story was already satisfied and the tests are the deliverable — that is a legitimate outcome here, not a skipped task. **Outcome: mostly the predicted one, with three exceptions.** Six of the eight requirements were already satisfied and are now asserted — the tests are the deliverable. The conformance pass found three real gaps: T008a and T008b were in scope, decided and built (`CommandId::CopyFields`); T010a is a declared-vs-actual mismatch in the undo stack that reaches past this feature, so it is filed as #1481 and **US1 ships with it outstanding**. Worth naming: every one of the three was a seam rather than a layer — a verb a widget owned and the registry had not heard of, a recovery the registry declared and nothing implemented. That is the bug this project says it has, found exactly where it said it would be

**Checkpoint**: the MVP. A person can compose and send by keyboard, and it is asserted.

---

## Phase 4: User Story 2 - Reply with the original quoted correctly (Priority: P1)

**Goal**: replies address, thread and quote correctly — including replies to rich HTML.

**Independent Test**: reply to corpus messages and assert recipients, threading, quote and cursor position, without sending.

**⚠️ T024–T030 require T003 (the ADR) to have landed.**

### Tests for User Story 2 — addressing and threading (not blocked)

- [X] T012 [P] [US2] Assert reply recipients, and that reply-to-all excludes the sender's own addresses and aliases, in `crates/postio-model/tests/` — FR-040. Recipients and the primary address were covered; the **alias** half was not, and `owns_address`'s `identities` arm could have been deleted with the suite still green while every alias holder Cc'd themselves on every reply. `reply_all_drops_an_alias_of_ours_as_readily_as_the_primary_address` closes that, addressing the alias in mixed case so it can only pass through `identities`
- [X] T013 [P] [US2] Assert the subject prefix is applied exactly once however deep the thread, in `crates/postio-model/tests/` — FR-018. **Already asserted** twice over: `model_suite/reply::replying_to_an_already_prefixed_subject_does_not_stack_re` counts occurrences against a corpus message, and `subject.rs`'s own units cover non-English prefixes, a subject that is only a prefix, and a word that merely starts like one
- [X] T014 [P] [US2] Assert a reply inside a thread answers the focused message, in `crates/postio-gtk/tests/gtk_suite/gtk_conversation.rs` — FR-042. **Already asserted**, and deliberately in two halves: `app_suite/conversation_reply_target.rs` proves a per-message verb reaches the composer carrying *that* message while the bar's thread-level Reply carries the latest, and `gtk_conversation.rs` proves the pane raises the right one. #1394 is why
- [X] T015 [P] [US2] Assert the attribution line is present and the quote is folded with the cursor above it — FR-041. **Split by the survey: two thirds built, one third not built at all.** `reply::attribution` exists and `a_reply_with_no_fetched_body_still_gets_an_attribution` covers it; the caret lands above the quote because `plain_quote` leads with a blank line, and `replying.rs` says so. **Folding does not exist** — nothing in `editor.js` or the composer folds anything, so this is real work rather than a conformance test. **Moved after T029**: a fold is a natural affordance over the `<blockquote>` T029 produces and an awkward one over the `> `-prefixed plain text it replaces, so building it first would be building it twice. **Done with T045**, and it cost almost nothing there: the reader already folds quotes with `details.postio-quote`, closed by default and opening on click with no script, so the editor uses the same vocabulary rather than inventing a second way to say "this is quoted" — and `<details>` is applied at document-assembly time and never reaches the wire, because mail clients disagree about it and a recipient whose client ignores it would get the summary line as stray text
- [X] T016 [P] [US2] Assert a reply carries threading and lands in the right conversation for the recipient, in `crates/postio-model/tests/` — FR-026. **Already asserted** across five corpus-driven cases in `model_suite/reply.rs`: a thread root, a deep reply carrying the whole chain, `In-Reply-To` with no `References`, neither header at all, and broken `References` threading on whatever survived parsing
- [X] T017 [P] [US2] Assert the reply appears in Postio's own list without waiting for the server, in `crates/postio-app/tests/app_suite/` — FR-027. Written in `sync_suite/send.rs` instead, where the filing actually happens. **Found a bug**: the copy is filed before the socket opens (the promise holds) but with `thread_id = None`, so a sent reply shows up in Sent as a brand-new conversation rather than in the one it answers. The headers are right and `ThreadingRepository::thread` is right — threading the same message again immediately afterwards joins it correctly — so what is wrong is *when* `file_sent_locally` calls it, inside the drain, with the error swallowed by a `let _ =`. **Then fixed.** The cause was a stale copy, not the threading: `file_sent_locally` threads the row and returns the `Message` struct it threaded *from*, and `confirm_sent_copy` later sets the server's coordinates on that same struct and calls `update`, writing the whole row back from a copy whose `thread_id` was still `None`. So the reply was filed correctly, threaded correctly, and then un-threaded by the confirmation. The thread id comes back onto the struct now, and the swallowed `let _ =` says something when it fails
- [X] T018 [P] [US2] Assert threading survives save/reopen and a failed-then-retried send — FR-028. Save/reopen **is** asserted (`storage_suite/drafts.rs` round-trips `in_reply_to` and `thread_id`); the failed-then-retried half is now asserted too, and in the place that decides it: a retry rebuilds the outgoing bytes from the draft row, so what makes the second attempt land where the first would have is that the row keeps `in_reply_to` across the failure. `storage_suite/drafts.rs` checks it against a real parent rather than comparing two `None`s
- [X] T019 [P] [US2] Assert a fresh message starts its own conversation and a forward is not threaded into its source — FR-029. **Already asserted**: `model_suite/reply::forwarding_carries_no_threading_headers_at_all` takes the deepest corpus reply and proves the forward carries none of it
- [X] T020 [P] [US2] Assert a forward carries the original's attachments and inline images — FR-043. Attachments **are** asserted (`reply.rs::a_forward_carries_attachments_with_fresh_local_ids`, which also pins the fresh-local-id part); **inline images** are now asserted too, and they do travel: `carried_attachments` copies the part with its `Content-ID` and `Disposition::Inline` intact. Worth asserting because the two ways to get it wrong are silent — a carry that filtered on `Disposition::Attachment` would drop them, and one that reassigned content ids the way it reassigns row ids would leave the body's `cid:` references pointing at nothing, with every attachment test still green

### Tests for User Story 2 — signatures (not blocked)

- [X] T021 [P] [US2] Assert a draft opens carrying the signature of the identity it starts as, in `crates/postio-model/tests/` — FR-030. Asserted by `starting_as_an_identity_records_it_and_signs_the_body_once` and by the first half of `changing_identity_never_touches_the_body`, which checks the opening signature before it checks that a later switch leaves it alone
- [X] T022 [P] [US2] **Red first**: assert changing identity does not alter the body, including a hand-edited signature — this reverses current behaviour and must fail before T023 — FR-031. Seen red (`no method named start_as`), then green
- [X] T023 [US2] Change `Draft::use_identity` in `crates/postio-model/src/draft.rs` to update the sender without touching the body; assert no path produces two signatures — FR-031, FR-032. Done as a split rather than a deletion: `use_identity` is the header change, `start_as` is what a *new* draft does, and the same split runs through `Composer::apply_identity`/`apply_signature` — the GTK composer had its own body path (`postio_body::apply_signature`) that the model change alone would not have reached

### Tests for User Story 2 — the quote (blocked on T003)

- [X] T024 [P] [US2] **Red first**: assert an HTML original's structure and styling survive into the quote, in `crates/postio-body/tests/` — FR-044
- [X] T025 [P] [US2] **Red first**: assert a plain-text-only original falls back rather than producing an empty quote — FR-045
- [X] T026a [P] [US2] **Red first**: assert a quote is sanitised with remote images blocked even when the original is displayed with them allowed — a per-sender allowance is the reader's own privacy decision and must not travel to a recipient (ADR 0033 Q2) — FR-047
- [X] T026 [P] [US2] **Security test, corpus-wide**: assert zero scripts, zero remote-loading references and zero tracking pixels are re-emitted across every HTML message in `crates/postio-model/tests/corpus/` — FR-047
- [X] T027 [P] [US2] Assert a sender's CSS in a quote cannot restyle the user's own text, a nested earlier quote, or Postio's chrome — FR-078
- [X] T028 [P] [US2] Assert what is in the editor is what is sent — FR-046

### Implementation for User Story 2 — the quote

- [X] T029 [US2] Change `quoted_reply` in `crates/postio-body/src/replying.rs` to build the quote from the reader's sanitised rendering rather than from the closed `Document`, reusing `sanitize.rs` and `styles::Scoped` — no second sanitiser (see [contracts/quote-construction.md](./contracts/quote-construction.md))
- [X] T030 [US2] Make T024–T028 green; if `styles.rs` needs a nesting fix for quotes inside quotes, do it here. `styles.rs` needed nothing — scoping already applied at every level. What did need work was everything *after* `replying.rs`: `harden` narrowed the quote straight back out again (its tag list is defined as the closure of what `to_html` emits, and `to_html` now emits more), `parse` had to recognise a reply quote and rebuild it through `quote_of` rather than narrow it, and the composer had to keep `format=flowed` unwrapping alive on the fallback path (#456). Three product questions came out of it and are filed rather than decided: #1483 (a forward still flattens), #1484 (src-less images in a quote), #1482 (table cells in the text half)

**Checkpoint**: replies are correct, and the quote change is contained by a security test that names its numbers.

---

## Phase 5: User Story 3 - Links, images and attached files (Priority: P2)

**Goal**: three outcomes the recipient can tell apart, and a size check that happens before anything is queued.

**Independent Test**: add and remove links, inline images and files on a draft; assert the draft and the visible rows without sending.

### Tests for User Story 3

- [X] T031 [P] [US3] Assert the three outcomes are distinguishable in the composer — FR-049. **Only two of the three existed.** A link and an attachment each had a command, a binding and a toolbar button; an image in the body was reachable by pasting or dropping and by nothing else — absent from the palette and the `?` sheet, out of reach for anyone who does neither, and the same shape of gap as `+ Cc` in US1. `CommandId::InsertImage` closes it, with a button beside the link one because what those two share is that they put something *into* the text
- [X] T032 [P] [US3] Assert a link can be inserted, edited and removed on a selection, and its target is visible before committing — FR-050. `gtk_editor_format.rs` covers inserting one on a selection and refusing a scheme that does not belong in mail. The target *is* visible before committing — `request_link`'s dialog is an entry you type into and can read — and editing or removing one is done by selecting it and linking again, which is the same path
- [X] T033 [P] [US3] Assert an image dropped or pasted into the body appears at that point and travels as a real part, visible to a recipient blocking remote content — FR-051, FR-052. `gtk_composer_inline_image.rs` covers the whole route: the pasted bytes become an inline attachment with a `Content-ID`, pixels actually render from the blob store, and the wire form carries the `cid:` reference that `outgoing.rs`'s multipart emission inlines — which is FR-052's "a recipient whose client shows no remote content still sees it". It now also covers an image *chosen* rather than pasted, including that the type is sniffed from the bytes and that a file which is not an image is refused with somewhere else to go
- [X] T034 [P] [US3] Assert attachments are listed with name and size and are removable — FR-053, FR-054. **Already asserted** by `gtk_composer_attachments.rs`, which drives the same `add_file` the chooser and a drop both converge on, checks the row appears with its name and size, and removes it again
- [X] T035 [P] [US3] **Red first**: assert inline images and attached files count against one size total — FR-055
- [X] T036 [P] [US3] **Red first**: assert an oversize message is refused before queueing, naming the limit, the overage and the largest items — FR-056
- [X] T037 [P] [US3] Assert a message whose text mentions an attachment with none present asks first — FR-057

### Implementation for User Story 3

- [X] T038 [US3] Add the size total across inline images and attachments, sourced from account configuration and never from the SMTP `SIZE` capability (see [research.md](./research.md) §3), in `crates/postio-gtk/src/composer.rs` and the model
- [X] T039 [US3] Decide and implement what happens when no limit is configured — check nothing rather than invent a number; record the choice in the commit body. **Decided: check nothing.** A guessed ceiling refuses mail the provider would have taken, and the person cannot tell Postio's opinion from their provider's rule. `Account::max_message_size` is `Option<u64>` and `None` is the ordinary state, not a gap to fill

**Checkpoint**: nothing leaves without its attachments, and nothing is silently too big.

---

## Phase 6: User Story 4 - The editor looks like the rest of the application (Priority: P2)

**Goal**: the editing surface stops rendering in WebKit's defaults.

**Independent Test**: open the composer in each colour scheme and compare typeface, size, foreground and background against the reader's, with no message involved.

### Tests for User Story 4

- [X] T040 [P] [US4] **Red first**: assert `postio-ui::editor::document` produces a document carrying a stylesheet, in `crates/postio-ui/src/editor/document.rs` unit tests — FR-073
- [X] T041 [P] [US4] Assert the ground colour resolves in both schemes and a scheme change produces a different sheet from the same input — FR-074, FR-075
- [X] T042 [P] [US4] Assert the quote treatment is present and distinguishable from the user's own text — FR-076
- [X] T043 [P] [US4] Assert text-size and density settings are honoured — FR-077
- [X] T044 [P] [US4] Display test: the editing surface is dark in dark mode with no light frame before or after it draws, in `crates/postio-gtk/tests/gtk_suite/` — FR-074

### Implementation for User Story 4

- [X] T045 [US4] Build `postio-ui/src/editor/document.rs` against [contracts/editor-document.md](./contracts/editor-document.md): stylesheet, ground colour, scheme, quote treatment, density — sharing the reader's tokens, keeping its own CSP
- [X] T046 [US4] Replace the inline shell in `crates/postio-gtk/src/editor.rs::seed` with the new assembly, and apply the ground to the view as well as the document, as `paint_ground` does for the reader
- [X] T047 [US4] Re-apply on a scheme change **without reloading** — a reload loses the caret and undo history — FR-075
- [ ] T048 [US4] **Not done — needs a person at a display.** Look at it: `cargo run -p postio-app`, press `c`, compare with a message body, switch schemes with a draft open. No test here runs on the accelerated path (#1307), so this step is the evidence

**Checkpoint**: the composer reads as part of Postio in both schemes.

---

## Phase 7: User Story 5 - Never lose a draft (Priority: P2)

**Goal**: typed text survives everything except a deliberate discard.

**Independent Test**: type into a composer, trigger each way of leaving it, assert the text is recoverable.

### Tests for User Story 5

- [X] T049 [P] [US5] Assert unsent work is saved without being asked, and survives navigating away, closing the window and an unexpected stop — FR-063. **Already asserted** across the two layers it spans: `gtk_composer_autosave.rs` proves typing coalesces into one debounced save and that closing never leaves an edit sitting in a timer, and `storage_suite/drafts.rs` proves the row survives on disk — which is what "an unexpected stop" reduces to once the edit has left the timer
- [X] T050 [P] [US5] Assert reopening restores text, formatting, recipients and attachments — FR-064. Text and recipients were asserted; **formatting and attachments were not**, and they are the two most worth asserting — the body round-trips through a WebView and a parse, so a draft can survive the disk perfectly and come back with its emphasis flattened and its files gone with every storage test green. It does not; now something says so
- [X] T051 [P] [US5] Assert discarding asks first — FR-061. Both halves: a draft with writing in it survives `request_discard` because the question is still on screen, and an empty one is dropped without a dialog that could only ever be answered one way. Folded into the same case as FR-057 rather than given its own — both ask the same question, and each new composer case is another WebView the suite is already short of (#957)
- [X] T052 [P] [US5] Assert a failed send stays editable and findable and names what went wrong — FR-066. **Two thirds hold and are asserted** (`storage_suite/drafts.rs`): the draft stays editable with its recipients and text, stays in Drafts, and the reason is kept on the queue row. **The third does not reach any surface** — the reason is built by `drain`, written to `last_error`, and carried up through the engine's report, whose own doc says "the reason the user should see", and then nothing reads it. Filed as #1487; the third instance in this spec of a thing computed by one layer and consumed by none
- [X] T053 [P] [US5] **Red first**: assert the reading pane holds at most one draft and that starting another detaches the first rather than refusing or discarding — FR-010, FR-011. Written and **red**, in `gtk_suite/gtk_composer_many.rs`, failing on exactly the clause it names: "asking for a second draft gave back the first, which is the quiet refusal FR-011 forbids". Held out of the default run by `IGNORED` with T056 named, so the suite stays green while the acceptance is in the repository
- [X] T054 [P] [US5] **Red first**: assert a draft is editable in exactly one surface, and asking for an open one brings it forward — FR-013. Same case as T053, same red
- [X] T055 [P] [US5] Assert draft save and load carry `postio_storage::test_support::counting` assertions on statements and rows — Constitution V. **Loading is 3 statements flat** — the row, its recipients, its parts — and stays 3 for a draft with sixteen recipients and eight attachments, which is the N+1 this exists to catch. Saving is ~4 statements per row written and is budgeted as a rate, since autosave runs it on a debounce for the life of every draft

### Implementation for User Story 5

- [X] T056 [US5] Implement the one-in-the-pane, many-detached rule in `crates/postio-gtk/src/shell.rs` and `composer.rs` — FR-010, FR-011, FR-013, FR-014. **Built.** [ADR 0034](../../docs/decisions/0034-one-composer-in-the-pane-many-in-windows.md) has the decision and the analysis that got there. The routing turned out smaller than the ADR first feared and larger than its second guess: not a `connect_command` signature change across 17 subscribers, but `Window::command_for_key_in` — resolve without dispatching — so a detached window's controller acts on *its own* composer, while the broadcast stays the pane's and detached composers skip it. Two feared obstacles were not: `register_reader_occupant` and the `compose` action both replace by key, which is the right outcome once the outgoing composer is in a window. FR-014 needed the window title to become the *subject*: it was the draft kind, which identifies nothing once two replies can be open at once — and the existing detach test had pinned `"Compose"` back when "which composition" had one answer
- [X] T057 [US5] Fix whatever T049–T052 found. **What they found was not fixable here, and that is the finding.** T049–T051 were satisfied and are now asserted. T052 turned up #1487: a failed send's reason is built, persisted and carried up through the engine's report — whose own doc says "the reason the user should see" — and read by no surface. Wiring it needs a decision about *where* it belongs (a toast, the reopened draft, or both), which is why it is `needs-architecture` rather than a patch on this branch

**Checkpoint**: no gesture loses typed text.

---

## Phase 8: User Story 6 - Markdown while typing (Priority: P3)

**Goal**: type `**bold**`, `# `, `- ` and get formatting, with no mode and no new command.

**Independent Test**: type each supported sequence and assert the resulting document structure.

### Tests for User Story 6

- [X] T058 [P] [US6] **Red first**: assert each supported sequence produces the formatting its command produces, in `crates/postio-gtk/tests/gtk_suite/gtk_editor_format.rs` — FR-067
- [X] T059 [P] [US6] Assert no sequence produces structure unreachable from an existing command — FR-068
- [X] T060 [P] [US6] Assert the literal markers do not appear in the sent message — FR-069
- [X] T061 [P] [US6] Assert one undo restores the literal characters and leaves them unconverted — FR-070
- [X] T062 [P] [US6] Assert markdown-looking text that was not converted is sent as shown — FR-071
- [X] T063 [P] [US6] Assert the plain-text alternative carries no doubled or stray markers — FR-072

### Implementation for User Story 6

- [X] T064 [US6] State the supported sequence → command mapping in `postio-ui`, so both frontends implement the same set even though the mechanism differs (see [research.md](./research.md) §4). `postio_ui::editor::markdown::SEQUENCES`, and the GTK table is **generated** from it at script-assembly time rather than restated in JavaScript — a hand-written copy is a copy that drifts, and what it would drift from is the contract both frontends implement. **No `# ` sequence**: there is no heading command, and FR-068 forbids a sequence producing structure no command can make. Filed as #1485
- [X] T065 [US6] Implement the input transformation in `crates/postio-gtk/data/editor.js`, reaching the existing commands only; add no registry entry — see [contracts/commands.md](./contracts/commands.md)

**Checkpoint**: markdown input works and has changed nothing about what a draft is.

---

## Phase 9: Polish & Cross-Cutting Concerns

- [X] T066 [P] Assert a Bcc recipient is never disclosed to another recipient, including a Bcc-only message, in `crates/postio-model/tests/` — FR-021
- [X] T067 [P] **Red first**: assert a Bcc-only message goes out with `undisclosed-recipients:;` in To, never a real address and never absent, in `crates/postio-model/src/outgoing.rs` — FR-022
- [X] T068 [P] Assert the composer makes no network request the user did not ask for while editing — FR-079. **Already asserted**, and with a real socket rather than an inspection: `gtk_editor_profile.rs` binds a loopback listener this process owns and proves nothing connects to it while content laden with remote references loads and is edited. The other half of FR-079 — no address or body text leaving — holds by construction: recipient completion reads the local contacts store and nothing else, and `check-no-silent-tracking.py` is in the invariants
- [X] T069 [P] Assert recipient count is shown before sending, and a malformed address is reported before queueing — FR-023, FR-024. **Both were half-built.** The warning counted wrong addresses without naming them — useless to somebody with nine recipients across three fields — so it now names the address and its field, and summarises past three rather than reciting. And there was no count at all: `recipient_summary` reports per field, shown on the same line when there is no problem to report, and silent for one recipient because a banner that is always there is a banner nobody reads
- [X] T070 [P] Assert an empty subject asks first, and no recipients is refused — FR-017, FR-062. No recipients was already refused by name (`NO_RECIPIENTS`). The empty subject was not asked about at all (FR-018), and adding it as a second dialog would have stacked with FR-057's — so both are now one question that names whatever applies, because two dialogs in a row about one message is how somebody learns to answer the second without reading it
- [X] T071 Run the full gate: `scripts/check.sh`, `scripts/test-sanity.sh`, `gtk_suite` and `app_suite` under nextest. **Green.** `cargo nextest run --workspace --no-fail-fast`: **4,087 tests, 4,087 passed**, 19 skipped. Two flaky and both are #957's shared-binary WebView contention (`gtk_editor_format`, `gtk_composer_toolbar`), passing on retry and passing alone in seconds. Plus `check.sh` clean on every invariant, `test-sanity.sh` in 31s, `cargo check --workspace --all-targets` clean, and the doctests nextest does not run — 25 across the seven crates this branch touches
- [~] T072 Walk [quickstart.md](./quickstart.md) end to end, including the by-eye checks that no test covers. **Every runnable step walked and green** — section 1 (8 + 38), section 2 (6), section 3 (25 + 42 + 1), section 4 (sanity tier, gtk_suite 275, app_suite 89), and `check.sh`. **One real defect found and fixed in the walking**: the command labelled "the security assertion, which is the one that matters" was `cargo test -p postio-body corpus_quote`, which matches no test — it printed `test result: ok` having run nothing. Every command in the file now carries the count it should select, because `ok. 0 passed` and `ok. 1 passed` look alike at a glance and mean opposite things. **What remains is the by-eye half only**, which is T048 and needs a person at a display
- [ ] T073 Land: `scripts/issue-land.sh --detach` — one pull request, reviewed against spec.md, closing no issue

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)** → everything
- **Phase 2 (Foundational)**: T003 blocks **only** T024–T030; T004 blocks **only** US4
- **Phases 3–8**: otherwise independent of each other and may be done in any order
- **Phase 9**: after the stories it polishes; T071–T073 last

### User Story Dependencies

| Story | Depends on | Notes |
|---|---|---|
| US1 (P1) | Setup only | The MVP |
| US2 (P1) | T003 for the quote half only | Addressing, threading and signature tasks are not blocked |
| US3 (P2) | Setup only | |
| US4 (P2) | T004 | |
| US5 (P2) | Setup only | |
| US6 (P3) | Setup only | Independent of everything |

### Parallel Opportunities

Every task marked **[P]** within a phase touches a different file and may run
together. The test blocks are almost entirely parallel: T005–T010, T012–T020,
T024–T028, T031–T037, T040–T044, T049–T055, T058–T063, T066–T070.

Implementation tasks within a story are sequential — they converge on the same
files.

## Parallel Example: User Story 1

```text
T005, T006, T007, T008, T009, T010   # all in parallel — six different test files
T011                                 # then, sequentially, whatever they found
```

## Implementation Strategy

**MVP is User Story 1.** It is the gesture the whole feature exists for, and it
is mostly assertion rather than construction — which makes it the cheapest way
to find out whether the composer is as good as it looks.

**Then by risk, not by priority number.** US2's quote work is the only part that
changes a security property, so it goes next and carries the corpus-wide test
that bounds it. US4 is the most visible defect (a white page in dark mode) and
is independent of everything. US6 is last because it is the only pure addition.

**A story whose tests all pass is a finished story.** Most of this feature is
built. A phase that ends with green tests and no production diff has delivered
what it was for — the assertion that did not exist before — and the commit body
should say so rather than inventing work to look busy.
