# Tasks: The Conversation Reading Pane

**Input**: Design documents from `/specs/001-conversation-reading-pane/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/](./contracts/)

**Tests**: **Included and non-negotiable.** Constitution Principle IV: the
failing test is written and *observed failing* before the code that satisfies
it. Every `[TEST]` task below must be seen red before the task that follows it.
A test never seen red is tightened until it visibly constrains the behaviour —
never satisfied by re-breaking working code.

**Organization**: grouped by user story. The plan's technical phasing maps onto
these — plan Phase 0/1-core lands in Foundational because several senders
sharing one document cannot be done safely without it.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: parallelizable — different files, no dependency on incomplete work
- **[Story]**: US1–US6 from spec.md
- **[TEST]**: must be observed failing before the next task

## Path Conventions

Rust workspace, 20 crates under `crates/`. Rules that can be proven without a
display live in `postio-body` and `postio-ui`; drawing lives in `postio-gtk`;
wiring is proven in `crates/postio-app/tests/app_suite/`.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: an initiative branch and a worktree, because this feature would
leave `main` half-migrated if landed one issue at a time.

> **`feature/conversation-reading-pane` exists and is the base for everything
> from here.** Claim onto it with
> `scripts/issue-claim.sh --base feature/conversation-reading-pane <n>`; PRs
> target it, not `main`. The one exception already landed: #1285's header
> dedupe went to `main` (PR #1322) because it is an independent `p1` that also
> unblocks the macOS frontend, and holding it inside this initiative would have
> delayed work unrelated to the rail.

- [X] T001 Claim Foundational's header work with `scripts/issue-claim.sh 1285`. **Landed straight to `main`, not onto a feature branch**: #1285 is an independent `ready`/`p1` issue that also unblocks the macOS frontend, so burying it in this initiative's branch would delay work that has nothing to do with the rail. The feature branch is cut when the first phase that would leave `main` half-migrated begins — Phase 2's one-document work
- [X] T002 Install the pinned test runner in the worktree with `scripts/install-nextest.sh` (run by `issue-claim.sh`)
- [X] T003 Verified against the **accumulated branch**, 2026-09-08, 12 commits ahead of `main`: `check.sh` all clean (1023 pub fn, 117 known uncalled), sanity tier 12s, `postio-body` + `postio-ui` 394 tests, and every `gtk_reader` case together. That last one mattered — four sub-cases were added on four separate branches and had never run on one tree until now
- [X] T004 [P] Filed as #1405, late rather than first — and better for it: it records what the phases *produced* rather than what they were planned to, including the two patterns worth carrying forward (controls inert in the one-document pane, and tests that could not fail). The phase order lives in `plan.md`; duplicating it in an epic would have been a second copy to drift

**Checkpoint**: a worktree of your own, on a feature branch, green.

---

## Phase 2: Foundational (Blocking Prerequisites)

**⚠️ CRITICAL**: no user story work begins until this phase is complete. Three
things block everything: one definition of the header, containment for sender
CSS, and the document shape the whole pane is drawn into.

### The two spikes — proof obligations, not experiments

- [X] T005 Spike R2 — **not measured, and not needed for the slice that shipped.** Inline declarations need no scoping: they apply to the element they sit on, already inside `contain_body`'s container. `@scope` matters only for `<style>` blocks (#1326), and even there selector rewriting is preferred — one toolkit-free implementation both frontends inherit, provable without a display, versus a CSS feature needing separate verification against WebKitGTK and WKWebView. Recorded under R2
- [X] T006 [TEST] Spike R3a in `crates/postio-gtk/tests/gtk_reader.rs`: with JavaScript enabled and `enable_javascript_markup(false)`, a fixture carrying inline `<script>`, an event-handler attribute and a `javascript:` href executes none of the three
- [X] T007 [TEST] Spike R3b in `crates/postio-gtk/tests/gtk_reader.rs`: an injected observer runs despite the document's `script-src 'none'`
- [X] T008 If T006 or T007 fails, stop and amend spec.md FR-034/FR-035 to marking by navigation — **never** weaken `crates/postio-body/src/sanitize.rs` to make the rail work. **Both passed; no amendment needed** (#1323, PR #1324)

### One definition of the header (#1285, plan Phase 0)

- [X] T009 [TEST] Add a check to `scripts/checks/` that fails when `postio-gtk` defines any of `address_line`, `address_list`, `subject_text`, `absolute_date`, `MessageHeader::of`, `ReaderAction::ALL` privately
- [X] T010 Create `crates/postio-ui/src/reader/header.rs` with the six shared rules, lifted from `feature/macos` per #1285
- [X] T011 Point `crates/postio-gtk/src/reader/message_header.rs` and `crates/postio-gtk/src/reader/actions.rs` at `postio_ui::reader::header`, deleting the private copies
- [X] T012 Fix #1285's red test in `crates/postio-gtk/src/reader/actions.rs` to assert "never a key that runs something else" rather than "never a key", and add the case where both of Archive's bindings are taken

### Containment for sender CSS (plan Phase 1 core, blocks the shared document)

- [X] T013 [TEST] [P] In `crates/postio-body/src/sanitize.rs`, assert a sender's rule scoped to its own message cannot restyle a sibling message or the document root. Extended by #1326 to `<style>` blocks and asked of the **engine** rather than of the markup: `gtk_reader_styles::one_senders_stylesheet_cannot_restyle_another_message` puts `p`, `body` and `.postio-blocked` rules from one sender against a second sender's message in one document. Its control asserts the sender's own rule reached their own message first — without it a dropped stylesheet passes, and contaminating nobody is not the same as being contained
- [X] T014 [TEST] [P] In `crates/postio-body/src/sanitize.rs`, one named test per refused property — `position: fixed`, `position: sticky`, stacking contexts above the message, viewport sizing beyond the message box, `transform`/`inset` escapes — each asserting the *reason* is containment or privacy
- [X] T015 Admit the `style` attribute in `crates/postio-body/src/sanitize.rs`, and **`<style>` contents too** — #1326 landed the rest. The dependency decision turned out to be nearly free: `cssparser` was already compiled in this tree because ammonia parses `style` attributes with it, so `postio-body` took the same version rather than a second CSS parser. `crates/postio-body/src/styles.rs` rewrites every rule under its own message's container; declarations go through T016's table unchanged. Selector rewriting over CSS `@scope`, per T005
- [X] T016 Implement the refused-property set in `crates/postio-body/src/sanitize.rs` as an enumerable table with one entry per property and its reason (FR-019b)
- [X] T017 [TEST] `url()` in any property names no loadable resource, asserted in `crates/postio-body/src/sanitize.rs`. **`@import` done with #1326** — now that a `<style>` block is admitted there is something to catch, and `styles.rs` refuses `@import` and `@font-face` by name with a reason each in `REFUSED_AT_RULES`, walked by a test that fails if a listed name is not actually refused
- [X] T018 **Already narrow; the work was proving it, and the premise was wrong** (#1383). `style-src 'unsafe-inline'` names no host, so an `@import` cannot fetch — but a sender has no route to one anyway: the sanitizer drops `<style>` elements whole and `@import` is valid only inside a stylesheet, while #1325 admitted the `style` *attribute*. The live-socket test written for this passed with `style-src` deliberately loosened to `http:`, which is how the vacuum was found; it is not in the branch. What landed: the sanitizer boundary asserted with the inline attribute as its control, and `style-src` locked against gaining a source, watched failing against that one-line edit. **#1326 is what makes the CSP layer matter** — admitting `<style>` blocks is when it stops being unexercised, and that has now happened: `content_security_policy` says so at the code, and the layer stands behind `styles.rs` rather than in front of nothing

### The conversation document (plan Phase 2)

- [X] T019 [TEST] [P] In `crates/postio-ui/src/reader/document.rs`, assert a conversation document contains one `.postio-body` container per message, each with a distinct scope identity. **Delivered by the merged one-document work: `postio_ui::reader::thread::conversation_document` composes one `.postio-body` per message, each with its own scope** (merged from `feature/one-document-conversation`, #1316)
- [X] T020 [TEST] [P] In `crates/postio-ui/src/reader/document.rs`, assert Postio's own words (absent, empty, downloading) stay outside every sender container. **Delivered — absent and empty states bypass `contain_body` as before** (merged from `feature/one-document-conversation`, #1316)
- [X] T021 Add the conversation assembly entry point to `crates/postio-ui/src/reader/document.rs` beside `document_for`, composing per-message containers, per-message chrome, scroll markers and the hardened wrapper. **Delivered — `thread::conversation_document`, with `Entry` per message** (merged from `feature/one-document-conversation`, #1316)
- [X] T022 [TEST] Assert `cid:` resolves per message by token in `crates/postio-gtk/tests/gtk_reader.rs` — not against "whichever message is open". **Delivered — `sanitize_body_in` stamps the scope and `reader/scheme.rs` routes on it** (merged from `feature/one-document-conversation`, #1316)
- [X] T023 Route `postio-cid:` by per-message token in `crates/postio-gtk/src/reader/scheme.rs`. **Delivered — per-message `cid` tokens, and `postio-cid:` dropped from the sender-writable scheme list** (merged from `feature/one-document-conversation`, #1316)
- [X] T024 [TEST] Assert in `crates/postio-gtk/tests/gtk_reader.rs` that a thread of any length renders in exactly one web process, counted as `each_reader_costs_a_web_process_of_its_own` counts. **In progress under #1316 on `feature/one-document-conversation`** — PRs #1320, #1321 landed there. **Delivered — `a_whole_thread_costs_one_web_process` in `gtk_reader.rs`, and #1348 measured it at 2, 10 and 50 messages** (merged from `feature/one-document-conversation`, #1316)
- [X] T025 Render the conversation into one reused surface in `crates/postio-gtk/src/reader/view.rs`. **Delivered — `Reader::render_thread` into one reused surface, behind `POSTIO_ONE_DOCUMENT`** (merged from `feature/one-document-conversation`, #1316)

### Shared infrastructure the stories need

- [X] T026 [TEST] [P] Done, and the answer was not three new commands. Only **dismiss-rail** needed one (`ToggleRail`, `⇧I`, #1375, asserted end to end by `gtk_toggle_rail` driving a real window and a real key). Reply and forward to a specific message already exist: `Command::Reply { message: Option<MessageId> }` carries its target, so a `ReplyToFocused` would be a second way to say the same thing and ADR 0029 is one control vocabulary. What was genuinely missing was that `e` **answered nothing at all** in the one-document pane -- the opening path set `imp.focused` directly and told no listener, so the `showing` cell the composer reads stayed empty. Fixed with #1316 and asserted by `conversation_reply_target`
- [X] T027 [P] `ToggleRail` added to `crates/postio-core/src/registry.rs` with its key chosen there and not from the brief -- `⇧I`, because `R` is `Refresh`'s alternate on every message surface (#1375, maintainer's call). The other two commands the task names are not added, deliberately: see T026
- [X] T028 [P] `docs/keybindings.md` regenerated (`POSTIO_UPDATE_DOCS=1`) for both #1375 and #1402, drift test green. The golden Linux binding table was updated **by hand** each time, which is what that file asks for: it exists to catch bindings that *moved*, so a wholesale regeneration would let the change it guards against through as a diff nobody reads
- [X] T029 [TEST] [P] In `crates/postio-ui/src/test_support/`, assert the render counter reports renders issued, surfaces created and bytes per document
- [X] T030 [P] Add the render counting facility in `crates/postio-ui/src/test_support/`, modelled on `postio_storage::test_support::counting` and sitting beside `postio_core::perf_budget` rather than inside it
- [X] T031 [TEST] [P] In `crates/postio-index/src/index.rs`, assert message length is counted from the sanitized plain-text body at index time
- [X] T032 [P] Store message line count at index time in `crates/postio-index/src/index.rs` and its migration, taking a plain default and rebuilding old rows by reindex

**Checkpoint**: header shared, sender CSS contained, one document per
conversation in one surface, registry and counters in place. Stories can start.

### Progress, 2026-09-08

**T009–T012 are landed** — PR [#1322](https://github.com/dlapiduz/postio/pull/1322),
auto-merge armed, closing #1285. Three commits: the shared module and the
check, the `To:`/`Cc (n)` wiring, and the lockfile churn.

Two things worth carrying forward:

- **The check does not forbid the name `MessageHeader`.** Both crates define
  one and that is correct — `postio-ui`'s carries rendered strings,
  `postio-gtk`'s is the widget that draws them. The first draft of the check
  flagged it, which would have pushed a frontend into renaming its widget to
  satisfy a rule about content. A name in common is not a rule in common.
- **The gate caught `to_line` and `cc_toggle_label` arriving uncalled**, which
  is `check-uncalled-pub-fn`'s shape exactly: written, tested, documented,
  wired to nothing, green the whole time. They had a caller here all along —
  `postio-gtk` was writing `format!("To: {}")` itself. Landing them into the
  baseline would have preserved the duplication this issue exists to remove.

**#1285's premise did not hold on `main`.** Its red test
(`a_key_lost_to_another_command_hides_the_hint_rather_than_showing_a_wrong_one`)
passes here, because Archive gains its `mod+shift+a` alternate on
`feature/macos` and that has not reached `main`. The assertion was rewritten
anyway — it asserted "never a key" where the guarantee is "never a key that
runs something else" — so it now survives that merge instead of breaking on it.
Both new tests were written green, which the constitution permits only with
this said out loud: they encode a guarantee for a change that has not arrived,
and neither was ever seen red.

---

## Phase 3: User Story 1 — Know who wrote this, and act on it (Priority: P1) 🎯 MVP

**Goal**: an open message says who it is from, to whom, about what and when,
and offers reply, reply all, forward and archive to the mouse.

**Independent test**: open any single message; the four facts are drawn and the
four verbs are clickable. Delivers a usable reading pane with no thread
behaviour at all.

- [X] T033 [TEST] [P] [US1] **Already asserted**: `header.rs` carries 24 tests, and `a_whole_envelope_renders_every_line_a_reader_asks_for` is this one exactly, with `a_message_with_no_recipients_offers_neither_line` as its edge. Verified rather than rewritten
- [X] T034 [TEST] [P] [US1] In `crates/postio-ui/src/reader/header.rs`, assert a recipient list too long for its width shortens and states how many are hidden
- [X] T035 [US1] Done by #1285, which is what moved `postio_ui::reader::header` out of `feature/macos` and pointed the GTK header at it. `message_header.rs` draws from it
- [X] T036 [TEST] [P] [US1] **Already asserted**, in `gtk_suite/gtk_reader_actions.rs::the_action_bar_follows_the_pane_carries_the_keymap_and_runs_registry_commands` — the bar follows the pane, carries the keymap and runs the registry's commands, which is the three claims this task splits into
- [X] T037 [US1] Done, and proven by the case above: the bar runs registry commands by id rather than reimplementing the verbs (FR-007). `header.rs`'s `the_four_verbs_are_reply_reply_all_forward_and_archive_in_that_order` fixes the order
- [X] T038 [TEST] [P] [US1] **Already asserted** in `gtk_reader.rs`, and further than this task asks: the banner is checked against a real socket in both directions (#1336), the `Show` verb is proven to actually grant consent (#1363), and consent is per sender rather than per thread (#1353)
- [X] T039 [US1] Drawn per message **in the document** rather than as pane chrome, which is what the brief asks for ("per message, never per pane"). Blocked-images now carries a `Show` verb intercepted by scheme, proven end to end (#1353, #1363). Unsubscribe and decode notices in the one-document pane remain pane-level
- [X] T040 [TEST] [US1] Already covered by `crates/postio-app/tests/app_suite/conversation_recipients.rs`, which asserts on the drawn labels — `postio-message-header-sender`, `-subject`, `-recipients` — rather than on what a layer was handed

**Checkpoint**: a single message is fully usable. This is the MVP.

---

## Phase 4: User Story 2 — Read a conversation without losing the thread (Priority: P1)

**Goal**: every message of a conversation readable in one stack, nothing
hidden, landing on the most recent message.

**Independent test**: open a ten-message thread; every body is present in one
scroll and the pane opens on the most recent.

- [X] T041 [TEST] [P] [US2] Already built — `conversation.rs` has `participants`, `message_count` and `date_span`. Verified rather than rewritten
- [X] T042 [US2] Already built (see T041)
- [X] T043 [TEST] [P] [US2] Asserted in `gtk_suite/gtk_rail.rs` (#1408), and watched failing against the refactor it exists to catch — the header appended to the stack instead of the root. The subject is checked through `ellipsize`/`wraps` rather than a measured height, because this display lays nothing out (#1307); the fixture's subject is forty repetitions long so "it fits" cannot be the reason it stayed one line. **US2 is complete**
- [X] T044 [US2] Done, in `conversation.rs` rather than `reader/mod.rs` — the two-row header belongs to the conversation pane (screen 30), and it is built outside the scroller so it cannot scroll away from the thread it names
- [X] T045 [TEST] [P] [US2] Asserted in `crates/postio-gtk/src/conversation.rs` rather than `document.rs` (#1389), because the decision was there: the pane drew a message open only if it was unread, focused or newest, so a conversation you had already read opened as **one body and five headers** — and since FR-015 the pane opens on the newest, that was the common case. The rule is now `expanded_in_document`, and `Expand all` went with it: FR-013 says there is nothing to expand, so a control that would do nothing is worse than none
- [X] T046 [TEST] [P] [US2] Asserted in `document.rs` and `thread.rs` (#1406). The gap was never whether folding *works* — `quote.rs` has fourteen tests — but whether it is **called**: drop either call in `body_html_in` and all fourteen still pass. Watched failing with the call removed. The thread assertion adds the claim a document of several senders needs: the fold is inside the message that owns it, with the neighbour as its control
- [X] T047 [US2] Already implemented: `body_html_in` folds each message's quotes through `quote::fold_html_quotes`, `<details>` and no script. Verified while surveying US2 for #1386
- [X] T048 [TEST] [P] [US2] Already asserted in `crates/postio-ui/src/reader/thread.rs` — that exactly one `postio-latest` marker is emitted, and none for a single-message conversation
- [X] T049 [US2] Already implemented in `crates/postio-ui/src/reader/thread.rs`: the newest entry carries `<span class="postio-latest">latest</span>`
- [X] T050 [TEST] [US2] Asserted in `crates/postio-gtk/tests/gtk_suite/gtk_rail.rs` rather than `app_suite` (#1385) — it is a pane rule and both panes had to be checked against it in one place. **Both were wrong, in opposite directions**: the one-document pane opened on the oldest, the stacked pane on the first unread. The case seeds unread messages early in the thread, because a first-unread rule would otherwise satisfy it by accident
- [X] T051 [US2] Fixed in `crates/postio-gtk/src/conversation.rs`, not `reading.rs` (#1385): `opening_focus` is where both panes ask. Two consequences followed, both direction rather than policy — `expanded_on_open` now walks back from the focus (forwards from the last message expands nothing), and the warm spare looks behind first (#1216's flash had returned for the first `K`)

**Checkpoint**: a whole conversation reads top to bottom, nothing hidden.

---

## Phase 5: User Story 3 — Act on the right message (Priority: P2)

**Goal**: the conversation bar acts on the latest message and archives the
whole conversation; each message offers its own actions.

**Independent test**: reply from the conversation bar quotes the latest
message; reply from an older message's own actions quotes that one.

- [X] T052 [TEST] [P] [US3] In `crates/postio-ui/src/reader/header.rs`, assert reply/reply all/forward resolve to the most recent message and archive resolves to the whole conversation
- [X] T053 [US3] Implement the action scoping rule in `crates/postio-ui/src/reader/header.rs`
- [X] T054 [TEST] [P] [US3] Scope stated in words, asserted in `crates/postio-ui/src/reader/header.rs` (#1338) — including the three wordings the obvious implementation gets wrong: "Forward **the** latest message" (not "to", which names a different action), "both messages" for two, and the bare verb for one
- [X] T055 [US3] Produce scope-stating tooltips and accessible names in `crates/postio-ui/src/reader/header.rs`
- [X] T056 [US3] Drawn in the conversation header's second row (#1351) — `latest · all N`, only where there is a distinction to draw. The header became two rows with a trailing element each, because canvas screen 30 puts the cluster beside the subject and the note beside the meta line
- [X] T057 [TEST] [P] [US3] In `crates/postio-gtk/tests/gtk_reader.rs`, assert a message's own actions appear on hover or keyboard focus and reserve no space when idle — nothing shifts. **Asserted in `crates/postio-gtk/tests/gtk_reader.rs` (#1365) on the scope the reader reported, not on the link being present — a link that names the wrong message passes a markup test and fails a person**
- [X] T058 [US3] Draw per-message actions in `crates/postio-gtk/src/reader/actions.rs`, appearing on hover and focus only. **Drawn in the document, out of flow so they reserve no space and nothing shifts when they appear; `visibility` rather than `display` so they stay focusable**
- [X] T059 [TEST] [US3] `crates/postio-app/tests/app_suite/conversation_reply_target.rs` (#1394), asserting `Draft::in_reply_to` rather than "a composer opened" — a composer opening is what the wrong-message failure also looks like. Three messages, because with the pane opening on the newest (FR-015) a two-message fixture cannot tell the thread-level rule from the per-message one. **It found a real defect** (T060)
- [X] T060 [US3] Fixed in `crates/postio-gtk/src/conversation.rs`, not `reading.rs` (#1394): the per-message targets were already wired: what was wrong was the **conversation bar**, which passed its command through for the application to resolve against the focus — so it answered the focused message while the header beside it said `latest · all 6`. FR-008 says the bar acts on the most recent; it now does
- [X] T061 [TEST] [P] [US3] **Already asserted**, in `crates/postio-core/tests/core_suite/command_registry.rs::destructive_commands_offer_a_way_back`, which walks `registry::all()` and refuses `Recovery::None` on anything destructive. Verified rather than rewritten: all six destructive commands carry `Undo` or `Confirm`, and the pane's own — `Archive` and `ArchiveThread` — are both `Undo`

**Checkpoint**: no action can be applied to a message the user did not mean.

---

## Phase 6: User Story 4 — See the message as it was sent (Priority: P2)

**Goal**: a formatted message renders as its sender built it, with the privacy
posture unchanged.

**Independent test**: render a multi-column newsletter from the corpus and
compare against a reference client.

- [X] T062 [P] [US4] **The fixture already existed** — `html-newsletter.eml` has the `width="600"` shell, three `width="70"` columns, `cellpadding` and `valign` that #1396 was about. What was missing was any assertion its *layout* survives; the suite used it only for reduction and `reads_as_bulk`. Added in #1410 and watched failing with `TABLE_LAYOUT` emptied
- [X] T063 [P] [US4] Half existed — `html-tracking-pixel-remote-images.eml` covers the fetching, including `url()` inside a stylesheet. `html-escaping-styles.eml` is new (#1410) and covers the containment half: `position` fixed and absolute, a viewport-sized overlay, maximal `z-index`, viewport units and a `transform`, beside styling that must survive. **Writing it caught me re-making a mistake #1326 had already corrected** — `transform` is not refused, it is contained by overflow, and the test now says so
- [X] T064 [TEST] [P] [US4] Asserted in `crates/postio-body/src/sanitize.rs`, not `reader_view.rs` (#1396) — the question is what the *sanitizer* lets through. **Five of the six survived; tables did not**, and table markup is how email actually builds columns, so every newsletter collapsed into one. `width`, `bgcolor`, `cellpadding`, `cellspacing` and `border` are admitted now, none of them a URL
- [X] T065 [US4] **Task was wrong as written and is not done as written.** Reader view must *keep* dropping `style`, `bgcolor`, `width` and `class`: it is the simplified presentation for bulk mail, and the sender's styling is what `View original` goes back to. Dropping them there is now the *control* rather than defence in depth, since the sanitizer no longer strips them — that status change and the stale doc comment landed with #1327
- [X] T066 [TEST] [P] [US4] Measured rather than re-derived (#1412). The heuristic has one real blind spot — a table-free campaign with six links reads as correspondence — and two answers worth keeping: a reply quoting one table, and a heavily styled personal note, are both correctly *not* bulk. The corpus newsletter still is. All three now asserted
- [ ] T067 [US4] Update `reads_as_bulk` signal counting. **Deliberately not done** (#1412): the blind spot is mild — a campaign opening in its sender's own layout is what FR-019a asks for, and the heuristic's own comment says the cost of error is small in both directions. A style signal would close it and risk the worse error, reducing styled *correspondence* to prose. Changing it needs evidence a miss actually hurts; the guard rails from T066 are what make the change hard to get wrong when that evidence exists
- [X] T068 [TEST] [P] [US4] Asserted in `crates/postio-gtk/tests/gtk_reader.rs` against a rendered thread (#1398), because the question turned out to be whether the key reaches a message at all — it did not. The case brackets its own control: no `<table>` in the reduced campaign, one after `⌃O`, and still none in the plain message beside it
- [X] T069 [US4] Per-message rendering was already applied from each message's own content; what was missing was any way to **overrule** it (#1398). `view_original` read state only the single-message path fills, so `⌃O` was a silent no-op in the one-document pane. The reader now keeps the scopes it has been asked to show whole, and the conversation supplies the focused one
- [X] T070 [TEST] [P] [US4] A message wider than the pane scrolls within its own block and never widens the pane — asserted against the engine in `crates/postio-gtk/tests/gtk_reader.rs` rather than in `document.rs`, because a stylesheet rule proves the file says something and only a laid-out document says what happened (#1334, PR #1335)
- [X] T071 [US4] Already implemented, at `crates/postio-ui/data/reader.css` rather than the path this task names — it moved with the `postio-body` split. `.postio-body` carries `overflow-x: auto`, and #1396 made the test of it real: it fed `document_for` raw markup, so it asserted containment of a `width` the sanitizer stripped before it arrived
- [X] T072 [TEST] [US4] A style-borne beacon is blocked while unconsented and **arrives on consent**, over a real socket in `crates/postio-gtk/tests/gtk_reader.rs` (#1336). Written as a two-directional proof against a live listener rather than a corpus sweep: a sweep counts requests that did not happen, which a silent-but-unreachable listener also produces

**Checkpoint**: mail looks like mail, and still phones nobody.

---

## Phase 7: User Story 5 — Never lose your place in a long thread (Priority: P2)

**Goal**: a rail that marks what you are actually reading.

**Independent test**: scroll a six-message thread; the marked row tracks the
screen. Click a row; the pane moves to it.

**Depends on**: T006 and T007 green.

- [X] T073 [TEST] [P] [US5] In `crates/postio-ui/src/reader/rail.rs`, assert the current message is the one with the greatest visible **area** — a fully-visible two-line reply must lose to an eighty-line message filling most of the viewport. **Asserted in `crates/postio-ui/src/reader/rail.rs` (#1359) as arithmetic over given extents, so it is provable without a display — which matters because this suite's display produces none**
- [X] T074 [US5] Create `crates/postio-ui/src/reader/rail.rs` with the current-message rule and the row model built from the thread, independent of body preparation. **Landed with #1359** — `current`, `Row` and `rows` are built from senders and stored lengths, so a row exists before its body does
- [X] T075 [TEST] [P] [US5] In `crates/postio-ui/src/reader/rail.rs`, assert a length is shown only above the threshold and comes from stored data, never from measurement. **Length only above the threshold, and `None` rather than zero when the body has not arrived**
- [X] T076 [TEST] [P] [US5] In `crates/postio-ui/src/reader/rail.rs`, assert rail activation and keyboard navigation resolve through one entry point and cannot disagree (#1372). **Seen red against the naive state machine** — both paths setting the mark directly — which is exactly the defect, so the tests failed by landing the mark on the message the scroll was passing over
- [X] T077 [US5] Implement the single entry point in `crates/postio-ui/src/reader/rail.rs`. `Rail::mark` is private; `activate`, `next_message`, `previous_message` and `observed` all end there (#1372)
- [X] T078 [US5] Enable JavaScript while keeping `enable_javascript_markup(false)` in `hardened_settings()` in `crates/postio-gtk/src/reader/view.rs`, with a comment naming the two proofs from T006 and T007. **Landed in #1367 with the three document amendments (ADR 0003 needed a note, not a correction — it already stated the principle)**
- [X] T079 [US5] Inject the visible-area observer in `crates/postio-gtk/src/reader/view.rs` and report positions to the rail model. **The observer is injected after the load, reports through a script message handler, and the payload is treated as untrusted — a scope the document never rendered is dropped (#1370)**
- [X] T080 [TEST] [P] [US5] Asserted in `crates/postio-ui/src/reader/rail.rs`, not `gtk_reader.rs` (#1372): the property is about the state machine, and this suite's display lays nothing out, so a scroll asserted against a rendered pane would prove nothing. Also covers the stale-settle case — two `J` presses where the first scroll's `scrollend` arrives while the second is in flight
- [X] T081 [US5] Suppression lives in `crates/postio-ui/src/reader/rail.rs` rather than the frontend (#1372), so both frontends get it and it is provable without a display. Each scroll is named by a `Settle` token; `set_conversation` clears suppression, because a scroll belonging to the conversation that went away must not silence the observer in the one that replaced it
- [X] T082 [TEST] [P] [US5] In `crates/postio-ui/src/reader/rail.rs` (#1372): every entry point returns an `Effect`, and re-reporting the marked message is `Effect::Nothing`. That is where *never animate it* is enforceable — a widget told to move only when the value differs has nothing to animate between. The rate limit itself is the observer's 100 ms debounce (#1370)
- [X] T083 [US5] Create `crates/postio-gtk/src/reader/rail.rs` drawing the rail, its heading, rows and footer (#1374). A `ListBox`, so FR-046's "a list with the current row marked" is what GTK announces rather than something hand-rolled
- [X] T084 [TEST] [P] [US5] Asserted in `crates/postio-gtk/tests/gtk_suite/gtk_rail.rs`, not `gtk_reader.rs` — the rail is a GTK column and needs the shared display binary, not the WebKit one (#1374). Three steps by width request and class, plus the floor: below 1100 the rail unmounts rather than the body narrowing
- [X] T085 [US5] One component, **three** presentations — screen 29 counts the narrowed column as its own step (#1374, popover in #1380). The ladder is `postio_ui::reader::rail::presentation`, because it is not only about width: a single-message thread and a rail put away have none at any width, and an `AdwBreakpoint` can know neither. The popover holds the *same* `RailColumn`, moved rather than duplicated — two would be two marked rows, and the second would be wrong exactly when someone scrolled with the index open
- [X] T086 [TEST] [P] [US5] Asserted in `crates/postio-gtk/tests/gtk_accessibility.rs` against the tree with `GTK_A11Y=test` (#1378), which meant opening a conversation there for the first time — the rail was in no tree anything walked. Both instruments were shown able to fail first, and one could not: `test_accessible_has_state` answers whether a state is *set*, and an unselected `ListBox` row sets `selected` to false, so it returned true for a marked row and an unmarked one alike. `check_state` compares the value
- [X] T087 [US5] Add the accessible names in `crates/postio-gtk/src/reader/rail.rs` (#1374, finished in #1378) — "Message 3 of 6, Tessa Vaughn, 21 Aug, 84 lines". The visible row is deliberately terse, so the label carries what the eye gets from position and from the message header
- [X] T088 [TEST] [P] [US5] Done in #1374, where the widget gives it a caller — asserted twice: as a rule in `postio-ui` (no rail at any width for one message, none when hidden) and on a real pane in `gtk_rail.rs` (hiding survives both a resize and a new conversation). Held out of #1372 deliberately: `set_hidden` had no caller there, and `check-uncalled-pub-fn.py` is right that a mechanism wired to nothing is this project's characteristic bug (#327, #416)
- [X] T089 [US5] Amend `docs/decisions/0003-rich-text-compose.md`: sender script refused, application script permitted. **Landed with #1367** — the ADR needed a note rather than a correction, because it already stated the principle as *script that arrived in a message never executes*; what changed was the mechanism behind it, not the rule
- [X] T090 [P] [US5] Correct `docs/PRODUCT.md` §21 and `CLAUDE.md`. **Landed with #1367.** Both now say the view *refuses script that arrived in a message* rather than that it has JavaScript off, and §21 names Postio's own observer as the reason the distinction exists

**Checkpoint**: a 400-line message no longer costs you your place.

---

## Phase 8: User Story 6 — The pane stays quiet (Priority: P3)

**Goal**: the regression locks for #749 and #946.

**Independent test**: move through a long thread and observe; separately, let a
message be marked read while staying on it.

- [X] T091 [TEST] [P] [US6] Asserted end to end in `crates/postio-app/tests/app_suite/render_dedup.rs` (#1340) rather than in `postio-ui`'s own tests: the defect #749 found was in the *wiring*, which a unit test over the counter cannot reach
- [X] T092 [US6] Already deduplicated — #749's fix holds. T091 is the lock that keeps it, not a fix
- [X] T093 [TEST] [P] [US6] Asserted in `crates/postio-ui/src/reader/document.rs` (#1341): the faces are named and not carried, with a ceiling calibrated against the measured 16 KB document and the 1.21 MB regression. Also asserts the faces are still *named*, so deleting the fonts cannot satisfy it
- [X] T094 [TEST] [P] [US6] In `crates/postio-ui/src/test_support/`, assert moving between messages and conversations creates zero additional rendering surfaces over a corpus walk. **Already asserted in `crates/postio-gtk/tests/gtk_reader.rs` by #1328's case: rendering a second message into the same reader creates no second surface**
- [X] T095 [TEST] [P] [US6] Asserted in `gtk_reader.rs` (#1414) as what this display can honestly carry: the `WebView` carries the theme ground from construction, before any document exists, and still does after a render — so a load has nothing black to show through. Watched failing with `paint_ground` commented out. **What it cannot see is written into the test**: not "no frame was black", because this display paints nothing and that assertion would be fiction (#1307). **US6 is complete**
- [X] T096 [US6] `paint_ground` already sets it; #1343 locks the **silent** failure — a generated palette value gdk cannot parse leaves the view unpainted with only a warning, which is #749's black frame restored while every test stays green
- [X] T097 [TEST] [P] [US6] Asserted in `crates/postio-gtk/tests/gtk_suite/gtk_rail.rs` rather than `app_suite` (#1400) — the counter is the pane's, and the case needs to drive body arrivals directly. **The property held by accident**: until #1389 `expanded` was `!seen || focused || newest`, so marking read did tear the thread down; FR-013 took the flash with it without anyone deciding to. The control matters — the first version passed because the pane had never drawn at all
- [X] T098 [TEST] [P] [US6] In `crates/postio-ui/src/test_support/`, assert a body arriving for the displayed conversation costs one render and does not move the scroll position. **Already covered by `crates/postio-app/tests/app_suite/body_arrives.rs`, which asserts twenty `BodyLoaded` events for the shown message produce **zero** further paints. Scroll follows transitively: `page.set(0)` lives in `load_document`, so no load means no reset**
- [X] T099 [US6] Update only what changed when content arrives, in `crates/postio-app/src/reading.rs`. **Already implemented — `repaint_if_waiting` narrowed the unconditional repaint #749 found**
- [X] T100 [TEST] [P] [US6] In `crates/postio-ui/src/test_support/`, assert arriving at an absent body and having it arrive costs one render, not two. **Covered, and the requirement it cited was wrong — see FR-064's correction in spec.md. `body_arrives.rs` asserts the first arrival is exactly one paint**
- [X] T101 [TEST] [P] [US6] In `crates/postio-ui/src/test_support/`, assert navigating faster than the pane renders coalesces to the settled selection. **Already covered by `body_arrives.rs`'s burst case**
- [X] T102 [US6] **Already implemented.** `body_arrived` coalesces onto the next turn of the main loop (twenty arrivals for one message are one store read and one repaint), the conversation side coalesces per message via `conversation_queued`, and the `document_signature` guard skips a render of a document already on screen. Verified rather than rewritten
- [X] T103 [TEST] [P] [US6] Asserted in `gtk_reader.rs` rather than `test_support` (#1412) — the counters live there but only a real reader can move them. Fifty conversations hold what one holds, on `surfaces_held` rather than `surfaces_created`, watched failing against a leak of exactly one surface per conversation. The control runs first: forty-nine conversations must have moved `renders_issued`, or the ceiling was dodged rather than measured
- [X] T104 [US6] Release what a conversation held on leaving it, in `crates/postio-gtk/src/reader/view.rs`. **Already asserted by #1328's `gtk_reader.rs` case: `surfaces_held` returns to baseline once the reader is dropped**

**Checkpoint**: #749's four mechanisms cannot return.

---

## Phase 9: Polish & Cross-Cutting Concerns

- [X] T105 [TEST] Delete `collapsed_runs()` and its tests from `crates/postio-ui/src/conversation.rs`. **Done** with #1426 (1634f5f3): the stacked pane is retired, so `collapsed_runs` -- which decided which runs became dividers -- and `run_summary` -- which wrote what a divider said -- both went with it, along with their unit tests. The maintainer made the retirement call on 2026-09-10 without waiting on #1424, which is now decoupled: the Orca pass is owed on the one-document pane itself, not as a gate on keeping a fallback
- [X] T106 Amend `docs/decisions/0015-threaded-list.md` (#1392). Recorded at each superseded claim rather than edited away, with the reasoning kept: FR-015 supersedes first-unread for both panes, and FR-013 **scopes** the collapsing rule to the stacked pane rather than retiring it — every open message there is a `WebKitWebView`, which is why that half still stands
- [X] T107 ADR 0032 moved to **Accepted** (2026-09-09), and the one-document pane is the default -- `POSTIO_ONE_DOCUMENT` is gone. **The screen-reader gate the ADR named as deciding it was not met**, on the maintainer's explicit decision. That is written into the ADR's Status rather than folded away, the superseded paragraph is quoted so a reader can see what changed under it, and the pass itself is filed as **#1424** so it is findable rather than buried in a document marked Accepted. Flipping the default surfaced three real defects the flag had been hiding (#1316): `is_expanded` read `entries` and answered false for a message on screen, the opening path notified no focus listener, and the pane says nothing about recipients (**#1427**)
- [X] T108 [P] Written as `docs/notes/2026-09-09-whose-script-runs-in-the-reader.md` and indexed (#1392) — dated the day it was written rather than 2026-09-08. It is aimed at whoever finds `set_enable_javascript(true)` and reads it as a weakening: two settings not one, the sender's script refused exactly as before, and the two proofs that keep it honest including why the spike deliberately carries no CSP
- [X] T109 [P] #1316 has #1348's measurements, what the experiment did not price (containment, once #1325 admitted inline styling), and the inert-control pattern it left behind. #1259 has what `postio_ui::reader::header` gives the macOS side, and the FR-008 scoping bug GTK had for months so that frontend does not repeat it
- [X] T110 [P] #1285 was already closed. **#946 turned out to be answerable rather than closable**: it asked whether marking a message read repaints the pane, and #1400 had just settled it — no in both panes, and in the one-document pane only since #1389, by accident. Closed with the answer and the assertion that now holds it
- [X] T111 Run over the whole feature branch at `a4521996` (~15 landed commits): **every invariant clean**, nothing to fix. Including the two this work strained — `uncalled-pub-fn` (1035 pub fn, 117 known uncalled, nothing added to the baseline all initiative) and `one-gtk-test-per-binary` across 410 test files
- [X] T112 Run `--workspace --all-targets -- -D warnings`, not only the changed crates: clean. Worth the wider net once at the end, because a shared type's blast radius is wider than the crate list describes (#419)
- [X] T113 Not an agent's to run, and not a duplicate to keep: #749 is closed and the confirmation is tracked as **#947**, whose PR #1313 is merged. Its Tests failure turned out to be nothing to do with the branch -- `gtk_reader` timed out at 241s because the branch predated `e0c1f949`, which fixes exactly that and says it 'was the whole of the Tests failure on four unrelated pull requests and on main'. Rebased, 8.1s, merged. The 60fps recording remains a person's, on #947
- [ ] T114 Merge `feature/conversation-reading-pane` to `main` when whole. **Rebasing is current** -- `origin/main` merged in on 2026-09-09 (merge, not rebase: `/initiative` says the branch is shared), 0 behind after. A traceability map of the spec's FRs is on the epic (#1405). **Now waits on three things the maintainer's 2026-09-09 decisions created**, none of which existed this morning: **#1424** ADR 0032's screen-reader gate, accepted-without and owed after the fact; **#1426** the stacked pane, which the application no longer builds and which is what would be restored if #1424 goes badly; and **#1427** the recipients regression the default flip exposed (landing). T105 becomes correct as written the day #1426 says retire

---

## Dependencies

```
Phase 1 Setup
    ↓
Phase 2 Foundational ─── T005..T008 spikes (gate US5)
    │                    T009..T012 shared header (gate US1, US3)
    │                    T013..T018 containment (gate US2, US4)
    │                    T019..T025 one document (gate US2, US5, US6)
    │                    T026..T032 registry, counters, message length
    ↓
    ├── US1 (P1) T033..T040 ── MVP
    ├── US2 (P1) T041..T051
    ├── US3 (P2) T052..T061 ── needs US1's header drawn
    ├── US4 (P2) T062..T072
    ├── US5 (P2) T073..T090 ── needs T006, T007 green
    └── US6 (P3) T091..T104 ── needs US2's document stack
    ↓
Phase 9 Polish
```

**Story independence**: US1, US2 and US4 are independent once Foundational
lands. US3 needs US1's header on screen. US6 measures US2's stack. US5 is the
only story gated on a spike.

## Parallel opportunities

- **Foundational**: T013/T014 (sanitizer tests), T019/T020 (document tests),
  T026/T029/T031 (registry, counter, index tests) — different crates, no
  shared files.
- **US1**: T033, T034, T036, T038 are four independent test tasks.
- **US4**: T062 and T063 (two fixtures) run alongside T064, T066, T068, T070.
- **US5**: T073, T075, T076, T082, T088 are all pure-rule tests in one new file
  — write them together, then implement against them.
- **US6**: T091, T093, T094, T098, T100, T101, T103 are all counter assertions
  and can be written as one batch before any of the fixes.

## Implementation strategy

**MVP is US1 alone** — a single message with a header and working actions.
That is the whole of #1259's complaint and it is shippable without any thread
behaviour.

**Then US2**, which turns the pane into a conversation. US1 + US2 is the
feature a user would recognise.

**US4 can land at any point** after Foundational and is the most visible change
to a person using the app.

**US5 last of the P2s**, because it is the only one carrying a settings change
and three document amendments.

**US6 is not optional despite being P3.** It is the regression suite for
defects that took real effort to find, and this feature replaces the code those
fixes live in.
