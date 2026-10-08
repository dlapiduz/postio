# Contract: what `postio-ffi` exports for Focus

UniFFI 0.32, proc-macro. Every engine payload crosses as an `*Ffi` mirror
(data-model.md, conventions). Blocking reads follow the existing pattern:
synchronous exports wrapped in `blocking(..)`, which Swift calls off the main
actor.

## Session lifecycle (changed)

| Export | Change |
|---|---|
| `Session::open_at(store_path)` | After the host starts and before `start_syncing`, it calls `Host::enable_focus(FocusSetup::from_config(..))`. It starts `ConfigService::watch`: on `keys` it rebuilds the resolver and emits `KeymapChanged`; on `focus` it re-runs `enable_focus`; on `filters` it refreshes saved searches |
| registration | `Frontend::Focus` on `Platform::Apple` for the resolver, availability and settings sections |
| `start_over(store_path)` (new, free fn) | Over `postio_session::start_over_at`, for `SessionError::StoreFromAnotherBuild` |

## Driving the controller

| Export | Returns |
|---|---|
| `key(char, name, ModifiersFfi, in_text_entry) -> KeyPressFfi` | `{handled, pending, intents: Vec<IntentFfi>}`. It replaces today's `KeyOutcomeFfi`; the context comes from the controller's stack, not from Swift |
| `command(id: String, origin: OriginFfi) -> Vec<IntentFfi>` | for buttons, menus and the bar |
| `ui_fact(FactFfi) -> Vec<IntentFfi>` | window size, cursor placed by the pointer, row pick, surface closed, typed text |
| `next_event() -> UiEvent` | unchanged, with appended variants below |
| `row_at(position) -> Option<FocusRowFfi>`, `row_count()` | over `ListWindow<FocusRow>` |

**`IntentFfi`** mirrors `Intent` with ids as `i64`, command ids as registry
strings, and words already composed in Rust (toast text, strip labels,
banner text, keycap spellings).

**`UiEvent` (append-only)** gains:
- `SurfacedChanged`
- `BackfillProgress{account, done, total}`
- `KeymapChanged`
- `Intents{intents: Vec<IntentFfi>}`, the controller's output for replies,
  timers and engine events.

## Rows

`FocusRowFfi` carries:
- **Identity:** `id`, `thread`.
- **Day:** `day_heading: Option<String>`.
- **Columns** (from `postio_ui::focus_row`): `sender`, `subject`, `preview`,
  `time`, `unread`, `count_badge`, `attachments`, `pills: Vec<LabelPillFfi>`
  (at most 2).
- **Marker line:** `marker: Option<MarkerLineFfi{kind, date, quote, action_id, action_label, second_action}>`.
- **Digest:** `digest: Option<DigestRowFfi>`.
- **Task:** `task_line: Option<String>` (M3, later).

## Reads

| Export | Over |
|---|---|
| `focus_counts()` | `Client::focus_counts` |
| `filtered_tabs()`, `filtered(reason, offset, limit)`, `sweep_preview()` | Filtered |
| `held(ids)`, `delivery_messages(delivery)`, `digest_summary(delivery)`, `digest_preview(queries, since)`, `digest_like_this(message)`, `save_digest_rule(replacing, draft)`, `delete_digest_rule(name)`, `digest_waiting(names)` | digests and rules |
| `raw_source(message) -> Vec<u8>` | `v` |
| `vault(subject)`, `capture_task(project, task)`, `capture_note(path, entry)` | capture (fails with a sentence when no vault is configured, C9) |
| `thread_labels(threads)`, `label_counts(account)`, `move_recent()`, `note_move(mailbox)` | pickers |
| `undo_description() -> Option<String>` | the new `Req::UndoTop` |

Most of these are reached through controller `Request`s and need not be
exported. An export exists only where Swift draws something the controller
does not hold: the raw source bytes, capture previews, and the digest
summary's text.

## Reader document (changed)

`reader_document(message)` moves to `postio_ui::reader::document`'s treated
path, and `ReaderDocumentFfi` gains:

- `treatment_shown`
- `treatment_classified`
- `render_mode_words`
- `sender_choice`
- `column_width` (from M1 geometry for the window's width)
- `paper_floor`

New exports:
- `switch_treatment(message)` for ⇧O.
- `always_treatment(sender, treatment)` for "Always for this sender", via
  `postio_ui::allowlist`.
- `treatment_css() -> String`, returning `postio-ui/data/treatment.css`.

## Links and contacts

- `message_link(id) -> String`
- `parse_message_link(uri) -> Option<i64>`
- `link_unknown() -> String` and `link_gone() -> String`
- `recipient_suggestions(account, prefix, limit, extra: Vec<ExternalContactFfi{name, address}>) -> Vec<RecipientCandidateFfi>`,
  ranked by `postio_ui::recipients::suggest`.

## Removed

Everything only the three-pane Mac used:

- `RailFfi`
- `rail_presentation`, `next_pane`
- the sidebar, folder and parts exports
- `thread_document` (Focus shows one message)
- `HANDLED_HERE`
- the classic cursor and selection exports, including `settleCursor` and
  the selection verbs

Coverage tests: `command_coverage.rs` owes every command
`offered_on(.., Apple)` for `Frontend::Focus`. `INTERCEPTED` and its Swift
mirror list only the commands Swift answers itself: window close, find in
page, zoom, and open attachment externally.
