# The classic app against Focus: parity and retirement

Focus is the one desktop app, and the classic three-pane app is being retired
(spec decision C27,
[ADR 0043](../../docs/decisions/0043-focus-is-the-one-desktop-app.md)). This
file says what the classic app does, whether Focus does it, and the order the
classic app goes in. **Nothing of the classic app is removed while a row
below is a gap**, unless the maintainer has accepted it.

When a gap closes, its row changes to **covered by**. Rows are not appended
to, and history stays in git.

**Sources.** The table was built from:

- the registry's commands that `Frontend::Focus` is not offered
  (`THREE_PANE_MAIL` and `THREE_PANE_CHROME` in `postio-core/src/registry.rs`);
- the commands Focus is offered that nothing in Focus answers. These are
  `registry_parity`'s `NOT_YET`, and the arms missing from
  `FocusWindow::act`, whose fallthrough logs "no Focus surface answers this
  command yet";
- the modules of `postio-gtk`;
- what `postio-app` wires (its `src/` and `tests/app_suite/`), checked
  against `postio-focus`'s code and `focus_suite`.

The terminal is out of scope. It keeps every command it has, including the
three-pane ones (`Requirement::ThreePane` is "not Focus", so it covers the
terminal and macOS too).

**Count:** 52 capabilities. 41 are covered and 11 are dropped. Row 18 carries
a smaller gap (T263), found when T252 ported the classic suites. Flagging, once a twelfth, is decided:
Flag stays, on `*` (C13, T257).

## The table

### Window, layout and navigation

| # | Capability | Where in the classic app | Verdict |
|---|---|---|---|
| 1 | Three panes: sidebar, list and reading pane, with draggable dividers; Next and Previous pane (`tab`, `shift+tab`); Toggle sidebar (`mod+b`) | `postio-gtk::shell`, `window.rs`; `CyclePane`, `CyclePaneBack`, `ToggleSidebar` | **Dropped**, because the spec's Out of Scope excludes a three-pane layout and a folder sidebar, and `docs/PRODUCT.md` §2 says Focus has neither. Reading beside the list comes back as Focus's own layout (T232). Maintainer: no (C27) |
| 2 | Folder list with counts, special-use folders first, ordinary folders after, per account | `postio-gtk::sidebar`; `NextFolder`, `PrevFolder` | **Covered by** the folders popover (`g o`, "Inbox ▾"; `places.rs`): mailboxes, folders and labels with counts, filtered by typing. Also by `in:` in the command bar (screen 08) |
| 3 | Expanding and collapsing nested folders and accounts, remembered across restarts | `sidebar.rs` (`set_collapsed`), `state.rs`; `ToggleFolder` | **Dropped**, because it only exists in a sidebar (row 1). The popover is flat and filtered by typing. Maintainer: no |
| 4 | Sync status at the sidebar's foot (`idle · imap`, "last sync 12s") | `sidebar.rs` (`set_status`) | **Covered by** the top bar's sync label and the one banner (`postio_ui::focus_state`, `chrome.rs`) |
| 5 | Go-to keys: Inbox `g i`, Drafts `g d`, Sent `g s`, Archive `g r`, Snoozed `g z`, Flagged `g *` | `window.rs` (`Command::default_for`) | **Covered by** `FocusWindow::act`: Sent and Archive by folder role, Snoozed and Flagged as view scopes; `registry_parity` runs every offered command and fails on one `act` does not answer (T236) |
| 6 | Scope: one account or all of them (`g a`, the sidebar's account rows) | `sidebar.rs` (`set_scope`); `NextScope` | **Dropped**, because Focus is one inbox across accounts (spec Assumptions). `account:` in the command bar narrows to one account in a search. Maintainer: no |
| 7 | Previous view (`Left`) | `PrevView` | **Dropped**, because Focus has no view history: `Esc` leaves every view and `g i` goes home (screen 21's footer). Maintainer: no |
| 8 | Command palette and query bar as one box | `postio-gtk::finder`, `palette.rs` | **Covered by** the command bar (`/` and `ctrl+k`, C24; `bar.rs`) |
| 9 | Cheat sheet `?` | `postio-gtk::cheatsheet` | **Covered by** the key map dialog (`keymap_dialog.rs`) |
| 10 | Window size and maximised state remembered across restarts | `postio-gtk::state` (`window.ini`) | **Covered by** `postio_widgets::state::Geometry`: Focus reopens at its saved size and maximised state, and a missing or unreadable `window.ini` opens at the default (`focus_suite::window_state`) |
| 11 | Single instance: a second launch raises the window | `postio-app` (`second_activate_wiring`) | **Covered by** `postio_focus::app`'s `activate`, which presents the window it already has |
| 12 | First-run keyboard orientation strip | `postio-gtk::orientation`, `postio-app::orientation` | **Dropped**, because every Focus control carries its key inside it (FR-092; T219's rule that a key is taught inside the control it runs), so a strip would teach what every control already shows. Maintainer: no |

### The list

| # | Capability | Where in the classic app | Verdict |
|---|---|---|---|
| 13 | Windowed list over the paged store, one row per thread | `postio-gtk::list`, `list_view.rs`, `row.rs` | **Covered by** Focus's list (`list/`, over `postio-widgets::list_model`) |
| 14 | Cursor separate from selection; extend, toggle and select all as a predicate | `list_view.rs` | **Covered by** `window.rs` (cursor, `x`, `J`/`K`, select all as a predicate, C19) and `bulk.rs` |
| 15 | Archive, delete, mark read or unread, move, label, snooze, with undo | `Command::default_for`, `list_view.rs` | **Covered by** `FocusWindow::act`, the pickers (`move_picker.rs`, `label_picker.rs`, `when.rs`) and the undo toast |
| 16 | Unsnooze (`B`) | `list_view.rs` (`Command::Unsnooze`) | **Covered by** `B` and the row menu in the Snoozed list and the open message, with an undo toast (T238) |
| 17 | Flag (`*`), and the Flagged view | `Flag` (three-pane only), `row.rs`'s flag mark | **Covered by** `*` (Flag, Unflag on a flagged row) from the key, the row menu and the command bar, with Undo; no flag mark on the row; `g *` lists flagged mail (T257, C13) |
| 18 | Row density (`[ui] density`), hover actions, sender avatars | `list_view.rs` (`set_density`), `row.rs`; `[ui]` | **Dropped**, because Focus's row is one fixed design (40 px, or 72 px with a marker, `contracts/focus-surface.md`, "Rows"), and the spec's handoff ranks consistency above preference. Maintainer: no |
| 19 | Theme override (`[ui] theme`: light, dark or system) | `postio-gtk::style` | **Dropped**, because Focus follows the system's scheme and accent (FR-090, C26). Maintainer: no |
| 20 | Row context menu | `list_view.rs` | **Covered by** `row_menu.rs` |
| 21 | Dragging messages onto a folder | `sidebar.rs` (`connect_dropped`), `autoscroll.rs` | **Covered by** the move picker (`m`): Focus has no folder list to drop onto (row 1) |
| 22 | Dragging messages out as `.eml` files | `postio-gtk::drag_out`, `postio-app::export` (`drag_out_wiring`) | **Covered by** `postio_widgets::drag_out` and `present::export`, offered by Focus's rows (T245, `focus_suite::drag_out`) |
| 23 | Empty, offline and failing list states | `postio-gtk::list_state` | **Covered by** `empty.rs` and `postio_ui::focus_state` (`InboxSaying`: empty, syncing, offline, failed; T220) |
| 24 | A unified view names an account it cannot reach, and the reason | `list_state.rs` (`derive_aggregate`), ADR 0005 Q10 (`degraded_unified`) | **Covered by T248.** A refused password is the sign-in banner; any other failure names the account and the reason (the sync's words when it gave them, else the kind of failure) with Retry now, and every account's mail stays listed |

### Reading

| # | Capability | Where in the classic app | Verdict |
|---|---|---|---|
| 25 | A thread stacked as one document, with the conversation rail; fold or unfold one message (`z`), expand all (`O`), hide or show the rail (`I`) | `postio-gtk::conversation`, `reader/rail.rs`; `ToggleFold`, `ExpandAll`, `ToggleRail` | **Dropped**, because Focus shows one message at a time and steps the thread with `[`/`]`, a deliberate departure from ADR 0032 (C2, FR-037). Maintainer: no (Clarifications) |
| 26 | The body on the hardened renderer, with quoted text folded | `postio-widgets::reader` (shared) | **Covered by** Focus's open message (`open.rs`, the same `Reader`; FR-033, FR-034) |
| 27 | Reader view and View original (`mod+shift+o`, `mod+o`); Darken this message (`alt+d`) | spec 006; `ToggleReaderView`, `ViewOriginal`, `DarkenMessage` | **Dropped**, because Focus's two treatments do the job: app colours or the original (`O`, T210–T213). Paper is never inverted or recoloured (T212), which rules out darkening. Maintainer: no |
| 28 | Zoom in, out and reset (`mod+plus`, `mod+minus`, `mod+0`), kept in `[reader] zoom` | spec 006 FR-021 (`zoom_persists`) | **Covered by** `mod+plus`, `mod+minus` and `mod+0` in the open message, dialog and pane alike (`FocusWindow::act`), written to `[reader] zoom` by `postio_config::save_zoom` and kept for the next message (`reader_zoom`). Paper's fit-to-column is automatic and multiplies under it (`open_layout::paper_fit_multiplies_under_the_zoom`) |
| 29 | Find in the message | `FindInMessage` | **Covered by** `reading_key` (`mod+f`, `mod+g`, `mod+shift+g`; T203) |
| 30 | View source | the reader | **Covered by** `v` (`source.rs`, `focus_suite::view_source`) |
| 31 | Remote images blocked; allowed once, or always for this sender | `postio-widgets::reader::banner` (shared); `ShowImages`, `AlwaysShowImages` | **Covered by** the shared banner and `i i` / `i a`, answered by `FocusWindow::act`; `focus_suite::remote_images` proves the block, that Show fetches once and nothing is asked before, and that Always holds for the sender's next message and for no one else |
| 32 | Unsubscribe on deliberate activation | the reader's notice | **Covered by** the open message's notice, in the dialog and the pane (`Reader::set_unsubscribe` from `postio_ui::unsubscribe::offer`), whose button (or `U`) logs the activation under the message's account through `FocusWindow::unsubscribe`; the digest answers `U` the same way (`focus_suite::unsubscribe`, T261) |
| 33 | Reading a message marks it read, after a short dwell | `list_view.rs` and `reading.rs` (`MarkReadOnDwell`; `dwell_wiring`) | **Covered by** the open message's read clock (`open.rs`, `postio_ui::dwell`): open for the dwell, in the dialog or the pane, it is marked read; `r` marks it unread again (T237; `focus_suite::read_on_dwell`) |
| 34 | A body that did not decode cleanly says so | `postio-widgets::reader` (`decode_notice`) | **Covered by** the shared reader's notices, which Focus's open message mounts |
| 35 | Desktop notifications for new mail | `postio-app::notifications` | **Covered by** Focus's notifier (`startup.rs`, `host.focus_notification`), following `[sync]` |

### Attachments

| # | Capability | Where in the classic app | Verdict |
|---|---|---|---|
| 36 | Opening an attachment, or a link, from the keyboard | `parts.rs` (`OpenPart`, `OpenPartExternally`) | **Covered by** `o` (`chooser.rs`, `open_choice`): each target is shown before anything opens, and a part is written and handed to the default app |
| 37 | Saving one attachment, or all of them, to a folder; clicking a chip | `parts.rs` (`SavePart` `s`, `SaveAllParts` `S`); `Reader::connect_attachment` | **Covered by** the `o` chooser's Save and Save all through the file-chooser portal (`FocusWindow::set_file_picker` is the test seam), written by the host as the classic app's are; a chip opens the same chooser at its part (`save_attachments`) |
| 38 | The MIME tree: walking every part, and rendering one once (`p`, `j`/`k`, `H`) | `postio-gtk::parts`; `OpenParts`, `NextPart`, `PrevPart`, `RenderPartOnce` | **Dropped**, because `v` (the raw source) checks what a message is made of, and `o` reaches every attachment. Maintainer: no |

### Search

| # | Capability | Where in the classic app | Verdict |
|---|---|---|---|
| 39 | Live search with the query language as chips, and `in:` to complete a folder | `postio-gtk::search`, `postio-app::search` | **Covered by** the command bar (`bar.rs`, `postio_search::natural::lower`; US4) |
| 40 | The misspelling suggestion ("Search instead for…", ADR 0037), and the result order (relevance or date, `O`) | `search.rs`, `list_view.rs` (`set_result_order`); `ToggleResultOrder` (`search_instead`) | **Covered by T241.** The bar says "Showing results for …" with a runnable "Search instead for “…”" row (the word quoted, exactly), lists "Sorted by relevance"/"Sorted by date" as a row, and `O` switches it once the arrows have chosen a result; before that `O` is a letter |
| 41 | Refinements with counts (the facet column) and the search scope column | `search.rs` (`set_facets`), `widgets/nav_row.rs` | **Dropped**, because the bar's chips and operators (`in:`, `from:`, `account:`) narrow a search in the box itself, and the bar draws no columns (screens 07–09). Maintainer: no |
| 42 | Saved searches: save, run (`alt+1`–`alt+4`), and rename, reorder and delete in the sidebar | `sidebar.rs` (`connect_saved_search_action`), `config.rs`; `RenameSavedSearch`, `MoveSavedSearchUp`, `MoveSavedSearchDown`, `DeleteSavedSearch` | Saving and running are **covered by** `SaveSearch` and `SavedSearch1`–`4` (`window.rs`). Renaming, reordering and deleting are **covered by** Settings' Filters section (`postio_widgets::settings`, T233, T234; `settings::a_saved_search_deleted_in_settings_leaves_alt_1_to_the_next`) |

### Compose

| # | Capability | Where in the classic app | Verdict |
|---|---|---|---|
| 43 | Rich-text composer: Cc and Bcc, identities, recipient completion, attachments, inline images, detach, send later, autosaved drafts | `postio-widgets::composer` (shared); `postio-app::compose` | **Covered by** Focus's composer dialog (`compose/`, T221) over the same composer and `present::compose` |
| 44 | Signatures: per account, placed above or below the quote | `[compose]`; Settings' Composing section | Inserting a signature is **covered by** the shared composer (`compose/seams.rs`). Editing one, and making it the default, are **covered by** Settings' account detail (T233, T234; `settings::a_signature_made_in_settings_signs_the_next_message`); placement by Composing, applied live (T235) |
| 45 | Sending states: cancel a queued send, retry a stopped one, settle an unconfirmed one (mark sent or retry) | `row.rs` (unconfirmed mark), the reader's `Verbs::STANDARD`; `CancelSend`, `RetrySend`, `MarkSent` (`unconfirmed_send`, ADR 0021) | **Covered by** the row's state word and the open message's send verbs (`focus_dialog::send_verbs`): Cancel send, Retry send, Mark as sent and Edit, in the dialog or the pane; the Outbox in `g o` (T239; `focus_suite::sending_states`) |
| 46 | `mailto:` links open a composer | `postio-app` (`mailto_uri`), the desktop entry's `MimeType` | **Covered by** `FocusWindow::open_link`, which takes a `mailto:` URI through `postio_model::mailto` into the composer (`focus_suite::desktop`). The desktop entry's `MimeType` claims `mailto:` since T253 (`packaging::postio_handles_mailto_and_postio_links`) |

### Accounts, settings and configuration

| # | Capability | Where in the classic app | Verdict |
|---|---|---|---|
| 47 | First run: the account form, OAuth, then how much history to sync | `postio-app::onboarding` (`Status::SyncWindow`, `write_sync_window`; `sync_window`) | The form and sign-in are **covered by** Focus's first run (`window.rs`, `postio_widgets::present::onboarding`; `focus_suite::first_run`). The sync-window step is **covered by** `Presenter::ask_sync_window`, which both apps' first runs use (`focus_suite::first_run`) |
| 48 | Adding another account; updating a credential | `add_account.rs`, `settings_credential.rs` | **Covered by** `AddAccount` and `UpdateCredential` in `FocusWindow::act`, and by the sign-in banner's button, which an account with no stored credential gets too: the sync blocks on a missing password (`BackendError::needs_a_password`) and the banner opens the credential form (`focus_suite::startup_repair`, T262) |
| 49 | The settings window. Accounts: edit, test the connection, token expiry, enable or disable, remove, rebuild the index, set the default, map mailbox roles, weights. Also Filters, Composing, Appearance, Keyboard, Sync and storage, Privacy (the remote-image allow list, the unsubscribe log, the read-receipt count, the connection log) and the config file | `postio-gtk::settings`, `widgets/`; `postio-app::settings_*`, `sidebar_backfill.rs`; `ToggleAccountEnabled`, `RemoveAccount`, `RebuildAccountIndex`, `SetDefaultAccount`, `MapMailboxRole` | **Covered by** the shared settings window in a dialog (T233, T234): `mod+comma` and the main menu's Settings open it (`FocusWindow::act`). Every section but Appearance (rows 18, 19), with the classic app's wiring ported (`settings_wiring.rs`). The account verbs are reached from each row's menu and its detail view, and from the keyboard (T258): with the keyboard on an account row, `Return`, `Delete` (undone by `mod+z`), `r`, `m` and `M` (which opens the account's roles) act on that row, and the command bar lists them (with Settings shut they open it to pick a row). |
| 50 | Excluding a folder from backfill (ADR 0016) | `sidebar.rs` (`connect_backfill_exclusion_changed`) | **Covered by** Sync & storage's "Back up locally", a check per folder (T234; `settings::a_folder_left_out_of_backfill_is_written_and_shown`) |
| 51 | Edit configuration (`mod+e` opens `config.toml` in the person's editor) | `postio-gtk::config`; `EditConfig` | **Covered by** `FocusWindow::act` through `postio_widgets::editor`, the launcher both apps share (T235; `settings::mod_e_opens_config_toml_in_the_persons_editor`) |
| 52 | `config.toml` applied live: keys, filters, sync, storage ceiling, reader zoom, compose, ui | `postio-gtk::config` (`storage_ceiling_wiring`) | **Covered by** `Session::follow_config`: keys, filters, sync, `[focus]`, and since T235 `[compose]`, `[reader]` and `[storage]` (`reload.rs`'s case per section) |

**Checked and absent from both apps:** printing (neither app has a print
operation), and creating, renaming or deleting folders.

## The gaps, as tasks

Each gap is a task in `tasks.md`, under "One app: Focus, and the classic app
retired". They are copied here so the table can be read alone.

- **T233** Settings, part one: move the settings window and its presenters to
  `postio-widgets`.
- **T234** Settings, part two: Focus opens it, in Focus's frame.
- **T235** Edit configuration, and `[storage]`, `[compose]` and `[reader]`
  applied live.
- **T236** Every command Focus offers is answered, starting with the four
  go-to keys.
- **T237** Reading a message marks it read.
- **T238** Unsnooze.
- **T239** Sending states in Focus.
- **T240** Saving attachments, and chips that open.
- **T241** The misspelling suggestion and the result order.
- **T242** Zoom.
- **T243** The sync-window step in Focus's first run.
- **T244** `mailto:`.
- **T245** Dragging messages out.
- **T246** Window size remembered.
- **T247** Allowing remote images, proven in Focus.
- **T248** A failing account named, with its reason and a retry.
- **T261** Unsubscribe in the open message.
- **T262** A missing credential offers the credential form.
- **T263** Rows grow with the text scale (done).
- **T264** A destroyed window frees its composer (done).

## The retirement, in order

Each step is a task in `tasks.md`, T249–T256, and none starts before the steps
it depends on. ADR 0043's rules hold throughout: the classic app is kept
building and gets no new work.

1. **Close the gaps (T233–T248).** They are independent of one another, apart
   from T234, which waits for T233. The packaging switch (step 4) waits for
   T233, T234, T236, T237, T239, T243 and T244, because without them Focus
   would be the only app and still be missing settings, read state, sending
   states or `mailto:`.
2. **Move to shared crates what Focus will need once the classic crates are
   gone (T249, T250).** These are the design tokens, the icons and the
   desktop data that `postio-widgets/build.rs` and the Flatpak read from
   `crates/postio-gtk/data/`, and the startup timeline and jank detector.
   T233 and T245 move the settings window and drag-out as part of their gaps.
3. **Account for every test only the classic app runs (T251, T252).** Check
   whether any shared code is exercised only by the classic app, and port or
   retire each `app_suite` and `gtk_suite` case, so that removing the crates
   removes no proof that Focus still depends on.
4. **Switch the package to Focus (T253).** Focus takes the app id
   `dev.postio.Postio`, the launcher, the icon, the metainfo and `mailto:`,
   and the classic binary leaves the Flatpak. It still builds from source
   until removal, as `postio-classic` under `dev.postio.Postio.Classic`.
   Focus is named Postio, its binary `postio` (the maintainer, 2026-10-02).
5. **Point CI and the developer scripts at Focus (T254).**
6. **Rewrite the docs for one app (T255).**
7. **Remove `postio-gtk` and the `postio` binary (T256).** This step **waits
   for the maintainer**.

### Notes for T255 (CLAUDE.md, recorded for the docs step)

These places in `CLAUDE.md` describe the classic app and change with step 6,
not before:

- the tier table and the `app_suite` timings in "Build & test";
- the "iterate at the cheapest layer" paragraph, which cites `postio-gtk`'s
  330 unit tests and `app_suite`'s ~200s;
- "Integration suites run under nextest", which cites `app_suite`'s 200s → 20s;
- "Tests are headless automatically", which names `cargo run -p postio-app`;
- "To see the app", where `scripts/run-isolated.sh` builds the classic app
  unless given `--focus`, and `cargo run -p postio-app`;
- "To prove a change reaches the running app", which names
  `crates/postio-app/tests/app_suite/`, `wiring.rs`, `keystroke.rs`,
  `click_preview.rs`, `CASES`, `IGNORED` and `list_contract.rs`. Focus's
  equivalent is `focus_suite`, with the same harness;
- the invariant "`postio-gtk`: no SQL, no protocol";
- "Keys: `e` reply, `a`/`A` archive, `u` undo…", which is already stale (the
  one keymap moved undo to `mod+z`), plus "Compose takes over the reading
  pane" and "The sidebar says 'Flagged'".

The skills `/gtk-design`, `/issue`, `/initiative` and `/steward` name
`postio-gtk` and `app_suite` too, and so does
`.claude/hooks/test-guard-shared-tree.py`.

## Shared code only the classic app calls (T251)

The `pub` items of `postio-ui` and `postio-widgets` whose only callers
outside their crate are `postio-gtk` and `postio-app`. They were found by
name across every crate's `src`, `tests`, `benches` and `examples`, then
checked by hand. A generic method name (`new`, `set_*`) that Focus also calls
on another type can hide one, so T256 confirms each with the compiler as it
deletes. **Goes** means it is deleted with the classic app in T256, with its
classic-only tests. **Stays** names the caller that keeps it, which also
gives it a test.

**Stays, though the plan expected otherwise:**

- `ComposerHost`: Focus's `compose::host::DialogHost` implements it, and
  `focus_suite::one_composer` has a second implementation. The classic
  `WindowHost` goes. `ComposerHost::composing` and `::adopt` have empty
  bodies in Focus and go with the classic one, with `COMPOSING_CLASS` and
  `Composer::present_surface`.
- The composer's detach seam (`toggle_detached`, `detached_window`,
  `is_detached`, `ComposerHost::{restore, remove, parent}`): Focus's frame
  dispatches `DetachComposer` into it.
- `postio_ui::settings`: Focus's settings dialog, the terminal and macOS.
- `postio_ui::reader::rail` (`Rail`, `rows`, `presentation`): macOS
  (`postio-ffi/src/rail.rs`). Only its `NARROW_BELOW`, `LENGTH_THRESHOLD` and
  `UNMOUNT_BELOW` go.
- `postio_ui::selection::{extend_over, select_only}` and
  `status::Trackers::note_last_sync`: the terminal.
- `postio_widgets::{startup, jank}`: Focus (T250).

**Goes with the classic app (T256):**

| Crate and module | Items |
|---|---|
| `postio_widgets::reader` (`Reader`) | `Verbs::STANDARD` (Focus uses `Verbs::NONE`) and the verb tables only it reaches; `connect_current_message` with `CurrentMessageHandler`, and `BodyView::{connect_current_message, current_message}` (the rail's marks); `connect_message_action` and `BodyView::connect_message_verb` (the per-message reply bars); `scroll_to_message`, `render_thread`, `offer_prepared`, `connect_parts_requested`, `request_parts`, `set_actions_visible`, `darken_title`, `connect_unsubscribe_activated`, `set_unsubscribe`, `zoom_indicator`; the test hooks only classic suites call (`click_*`, `banner_always_allow_label`, `unsubscribe_banner_*`, `reader_notice_visible`, `shows_encoding_problems`, `toggle_reader_view_for`, `view_original_for`, `visible_verbs`, `set_notices_visible`, `actions_visible`, `actions_widget`, `is_reader_view`, `document_for_test`, `test_press`); uncalled anywhere: `would_render_thread`, `forget_originals`, `remember_treatment` |
| `postio_widgets::reader::message_header` | `set_identity_visible`; test hooks (`cc_revealed`, `cc_toggle_visible`, `date_label`, `identity_visible`, `sender_label`, `subject_label`, `to_visible`); uncalled: `add_before_date`, `add_before_sender`, `set_recipients`, `set_subject`, `set_subject_visible` |
| `postio_widgets::body_view` | test hooks only classic suites call (`drag_select`, `evict_tiles`, `focused_link_target`, `release_renders`, `toggle_darken`, `click_select`, `selection_rects`, `find::find_rects`, `zoom::pinching`). They stay if T252 moves the `body_view*` cases that call them |
| `postio_widgets::composer` | the `test_*` hooks only classic suites call; `editor::editing_view`; `web_process::take_death`; uncalled: `editor::format_state`, `web_process::{deaths, last_death}`. The hooks stay with any `gtk_composer_*` case T252 moves |
| `postio_widgets::present::compose` | `install_reply_source` |
| `postio_widgets::onboarding`, `settings` | the `test_*` hooks only classic suites call, `footer_text`, `footer_target_text`, `read_receipt_count_label`, `set_list_viewport_height`, `set_row_height_probe`; uncalled: `settings::BODY_HEIGHT`. Hooks stay with any case T252 moves |
| `postio_widgets::widgets` | the whole of `nav_row` (`nav_row`, `nav_name`, `nav_count`), `chip` (`chip_button`, `filter_chip`) and `screen` (`under_window_chrome`, `showing_in`); `toast::offers_undo`; `NoticeBar::set_icon`; uncalled: `NoticeBar::set_action_sensitive`, `render_mode::press_switch` |
| `postio_widgets` (other) | `state::{open_for_writing, state_dir}`; `list_model::emissions`; `drag_out::Materialise`; `style::ICONS`; uncalled: `style::WIDGETS_CSS_URL`, `startup::{start_at, within_budget}` outside tests |
| `postio_ui::list_state` | `derive_aggregate` and its helper `is_current`, `derive_opening` |
| `postio_ui::keymap` | `Keymap::apply_commands`, `Chord::keysym_name`, `trigger_for_command` |
| `postio_ui::reader` | `document::prepare_message`, `document::LICENSES`, `rail::{NARROW_BELOW, LENGTH_THRESHOLD, UNMOUNT_BELOW}`; uncalled: `document::DOCUMENT_BASE_URI`, `cost::note_waited_out` |
| `postio_ui::search`, `status`, `focus_dialog`, `sidebar` | `search::{NOTHING_MATCHED, NOTHING_TO_NARROW}`, `SyncStatus::refresh_interval`; uncalled: `search::{with_reindexing, with_unreachable}`, `status::detail_in_full`, `focus_dialog::PANE_WINDOW_MIN`, `sidebar::{ancestors_of, attention_for}` |
| `postio_ui::test_support` | `document_bytes`, `documents_built`, `pages_requested`, `redraws_waited_out`, `snapshot_counts`; uncalled: `largest_document` |
| `postio_ui::observe` | all of it but `Tone`, which `postio_widgets::widgets::toast` uses. Its other users are the classic window and `postio-storyboard`, which only `postio-app` depends on (see the questions) |

## What only the classic suites proved (T252)

Every case in `postio-app/tests` and `postio-gtk/tests` was sorted three
ways. **Ported**: it proved host, engine, renderer or startup behaviour, and
now runs against Focus or the crate that owns the behaviour. **Moved**: it
tested a shared crate and only sat in a classic suite; it moved to that
crate's suite. **Goes**: it proves a classic surface (three panes, the
sidebar, the classic list, the rail, the finder, classic-only commands, PLATE
tokens) or is covered elsewhere, and is deleted with the classic app in T256.
Ported cases' classic originals stay until then; moved cases left the classic
suites.

**Ported to `postio-host`** (display-free):

| Classic case | Now |
|---|---|
| `e2e.rs`, `app_suite::attach_account` | `tests/e2e.rs`: a loopback IMAP server's first sync is listed, a flag and an archive reach its copy, a delivery reaches the list; an account added to a running host syncs |
| `oauth_signin.rs` | `tests/oauth_signin.rs` |
| `backend_choice.rs` | `tests/backend_choice.rs` |
| `reclaim_wiring` (2), `reclaim_pages` (2) | `tests/reclaim.rs`, through `Host::start_idle_passes` |
| `search_index` (3 of 5) | `tests/search_index.rs`; one was already covered by the host's own tests, and one is Focus's (`idle_passes`) |
| `postio-app`'s `startup_route` unit tests (4 uncovered), `recover_empty_draft` | `src/tests.rs` |

**Ported to `focus_suite`:**

| Classic case | Now |
|---|---|
| `e2e.rs`, the window half | `e2e`: `a` reaches the server's Archive, a delivery grows the list |
| `reader_spawns_no_web_process` | `reader_spawns_no_web_process`; a move costs one or two snapshots in Focus, which draws a placeholder between bodies |
| `hostile_mail` | `hostile_mail` |
| `notify_off_the_main_thread` | `notify_off_the_main_thread` |
| `autosave_off_the_main_thread` (2), `compose_recipients` (no connections) | `compose_counts` |
| `startup_reads` | `startup_reads`: no statement on the main thread, the same connections at 1,000 and 10,000 messages (five; the classic ceiling was four). Its scan half was the classic list's query and goes |
| `startup_behind_the_window` (2) | `startup_behind_the_window` |
| `search_index`, idle passes after the first frame | `idle_passes` |
| `startup_repair` | `startup_repair` (T262) |
| `gtk_accessibility.rs` | `a11y_sweep`: every Focus surface has roles and names, and 200% text stays usable; rows grow with the type (T263) |
| `window_teardown`, `gtk_window_teardown` | `window_teardown`; a mounted composer is freed with its window (T264) |
| `second_activate_wiring` | `desktop::a_second_activate_has_one_window_and_starts_sync_once` |
| `unsubscribe_wiring` | `unsubscribe`: the digest's `U` is logged and listed; the open message's notice is logged and listed too (T261) |
| `gtk_store_opening` (2 of 3) | `store_opening`; the third ("a key for mail says why it cannot run yet") goes, since Focus's wait plate says it |
| `large_folder_open` | `visible_window::opening_and_switching_large_folders_asks_a_bounded_number_of_pages` |
| `gtk_list_reload` (2) | `list_reload` |
| `add_account_wiring` (the key over a running window) | `add_account_running` |
| `compose_detach`, `composer_warm` | `compose_detach` |
| `startup_timeline` (T250) | `startup_timeline` |

**Moved to the crate that owns the code:**

- `postio-session`: `correlation`, `event_fanout`, and `postio-app::onboarding`'s
  unit tests of `postio_session::onboarding` (`tests/onboarding.rs`).
- `postio-storage`: `glib_main_context`.
- `postio-ui`'s `ui_suite`: `keymap_defaults`, `keymap_live`, `reader_tokens`.
- `postio-widgets`' `widgets_suite`: `list_model` (rewritten over the generic
  model), `list_recycling`, `body_view*`, `reader_*` (anchor, fallback, fonts,
  notices, scroll, teardown), `reader_corpus` (the corpus hardening and the
  counters from `gtk_reader.rs`), `one_allowlist`, `first_frame`, `jank`, the
  small widgets (`small_widgets`, `checkrow`, `segmented`, `toast`,
  `toast_tone_and_undo`, `components`), `composer_*`, `editor_*`,
  `editable_dialect`, `onboarding*` and `settings_*` (accounts, account
  detail, filters, frame, keys, privacy, sync). The composer cases run in a
  minimal `ComposerHost` (`support_compose.rs`) rather than the classic window.

**Goes with the classic app (T256):** the rest of `app_suite` and
`gtk_suite`, and `logic_suite`'s `desktop_entry` and
`gtk_extension_commands`. They are classic surfaces (rows 1, 3, 6, 7, 12, 18,
19, 25, 27, 38 and 41 of the table) or covered by the Focus cases the table
names: the classic list, rows, feeds, sidebar, scopes, panes, rail,
conversation, finder, search panel, cheat sheet, orientation, parts, reading
pane and its dwell, classic keystroke wiring, density and theme, the
composer's classic window joins (`gtk_composer_{header, reply, action_row,
detach, many, keymap, detached_scheme}`), `gtk_settings` and the classic
window's settings key context (`gtk_settings_accounts_keys`,
`gtk_settings_keys_context`, partly covered by
`settings_wiring::the_account_verbs_have_keys_on_the_focused_row`),
`onboarding_probe` (the classic onboarding window), the remaining
`gtk_reader.rs` thread-document cases, and the `postio-gtk`/`postio-app` unit
tests of their own modules. The storyboard runner and its 24 cases are rebuilt over Focus (T265).

## Risks

- **Some shared code is only exercised by the classic app.** It is listed,
  with what happens to each item, under "Shared code only the classic app
  calls" below (T251).
- **`app_suite` proved things Focus relies on.** Where each case went is
  under "What only the classic suites proved" below (T252).
- **The registry names the classic app.** `Frontend::Classic` is the default
  in `Availability::open`, which many tests call, and `Requirement::ThreePane`
  means "not Focus", so the terminal and macOS rely on it. Removal renames or
  narrows these and does not delete them, and the terminal's `registry_parity`
  has to stay green.
- **The parity test cannot see a command that is not answered.**
  `registry_parity` passed four go-to keys that do nothing (row 5). Until
  T236 adds the check, a gap can hide behind a key, a bar row and a control.
- **The constitution names two desktop apps.** Its Scope ("two desktop apps,
  the classic app and Postio Focus") and its boundary paragraph change with
  the amendment that lands with this branch, and that amendment needs the
  maintainer (FR-009).
- **Two benchmarks still link the classic app.** `postio-bench`'s
  `conversation_rows` (the classic thread row) goes with it.
  `action_round_trip` times `postio_gtk::feed::Feed` applying an archive's
  events; Focus's list has its own `Feed`, and the bench is ported to it or
  retired at T256. `list_scroll` and `composer_open` time Focus's row and
  composer. The release workflow's size comparison (`postio-tui` against
  `postio`) follows T254.
- **Other lanes on this branch.** T232 (the reading pane) is changing
  `window.rs` and `open.rs`. T236, T237 and T240 touch the same files and
  should rebase onto it.

## Questions for the maintainer

1. **The constitution amendment** for one desktop app, with the branch's
   existing amendment (drafted in T255).
