# The classic app against Focus: where each capability went

The classic three-pane app (`postio-gtk` and `postio-app` as they were) was
removed in T256, approved by the maintainer on 2026-10-02 (spec decision
C27, [ADR 0043](../../docs/decisions/0043-focus-is-the-one-desktop-app.md)).
Postio is Focus, in the crate `postio-gtk`. This file says, for each thing
the classic app did, where Postio does it now or why it was dropped.

The table was built from the registry's commands the classic app was
offered and Focus was not, the commands Focus is offered that nothing in
Focus answered (`registry_parity`'s `NOT_YET`), the classic crates' modules,
and what the classic composition root wired. The terminal and macOS keep
their own commands; `Requirement::ThreePane` is macOS's.

**Count:** 52 capabilities. 41 are covered and 11 are dropped.

## The table

### Window, layout and navigation

| # | Capability | In the classic app | Where it is now |
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
| 11 | Single instance: a second launch raises the window | `postio-app` (`second_activate_wiring`) | **Covered by** `postio_gtk::app`'s `activate`, which presents the window it already has |
| 12 | First-run keyboard orientation strip | `postio-gtk::orientation`, `postio-app::orientation` | **Dropped**, because every Focus control carries its key inside it (FR-092; T219's rule that a key is taught inside the control it runs), so a strip would teach what every control already shows. Maintainer: no |

### The list

| # | Capability | In the classic app | Where it is now |
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

| # | Capability | In the classic app | Where it is now |
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

| # | Capability | In the classic app | Where it is now |
|---|---|---|---|
| 36 | Opening an attachment, or a link, from the keyboard | `parts.rs` (`OpenPart`, `OpenPartExternally`) | **Covered by** `o` (`chooser.rs`, `open_choice`): each target is shown before anything opens, and a part is written and handed to the default app |
| 37 | Saving one attachment, or all of them, to a folder; clicking a chip | `parts.rs` (`SavePart` `s`, `SaveAllParts` `S`); `Reader::connect_attachment` | **Covered by** the `o` chooser's Save and Save all through the file-chooser portal (`FocusWindow::set_file_picker` is the test seam), written by the host as the classic app's are; a chip opens the same chooser at its part (`save_attachments`) |
| 38 | The MIME tree: walking every part, and rendering one once (`p`, `j`/`k`, `H`) | `postio-gtk::parts`; `OpenParts`, `NextPart`, `PrevPart`, `RenderPartOnce` | **Dropped**, because `v` (the raw source) checks what a message is made of, and `o` reaches every attachment. Maintainer: no |

### Search

| # | Capability | In the classic app | Where it is now |
|---|---|---|---|
| 39 | Live search with the query language as chips, and `in:` to complete a folder | `postio-gtk::search`, `postio-app::search` | **Covered by** the command bar (`bar.rs`, `postio_search::natural::lower`; US4) |
| 40 | The misspelling suggestion ("Search instead for…", ADR 0037), and the result order (relevance or date, `O`) | `search.rs`, `list_view.rs` (`set_result_order`); `ToggleResultOrder` (`search_instead`) | **Covered by T241.** The bar says "Showing results for …" with a runnable "Search instead for “…”" row (the word quoted, exactly), lists "Sorted by relevance"/"Sorted by date" as a row, and `O` switches it once the arrows have chosen a result; before that `O` is a letter |
| 41 | Refinements with counts (the facet column) and the search scope column | `search.rs` (`set_facets`), `widgets/nav_row.rs` | **Dropped**, because the bar's chips and operators (`in:`, `from:`, `account:`) narrow a search in the box itself, and the bar draws no columns (screens 07–09). Maintainer: no |
| 42 | Saved searches: save, run (`alt+1`–`alt+4`), and rename, reorder and delete in the sidebar | `sidebar.rs` (`connect_saved_search_action`), `config.rs`; `RenameSavedSearch`, `MoveSavedSearchUp`, `MoveSavedSearchDown`, `DeleteSavedSearch` | Saving and running are **covered by** `SaveSearch` and `SavedSearch1`–`4` (`window.rs`). Renaming, reordering and deleting are **covered by** Settings' Filters section (`postio_widgets::settings`, T233, T234; `settings::a_saved_search_deleted_in_settings_leaves_alt_1_to_the_next`) |

### Compose

| # | Capability | In the classic app | Where it is now |
|---|---|---|---|
| 43 | Rich-text composer: Cc and Bcc, identities, recipient completion, attachments, inline images, detach, send later, autosaved drafts | `postio-widgets::composer` (shared); `postio-app::compose` | **Covered by** Focus's composer dialog (`compose/`, T221) over the same composer and `present::compose` |
| 44 | Signatures: per account, placed above or below the quote | `[compose]`; Settings' Composing section | Inserting a signature is **covered by** the shared composer (`compose/seams.rs`). Editing one, and making it the default, are **covered by** Settings' account detail (T233, T234; `settings::a_signature_made_in_settings_signs_the_next_message`); placement by Composing, applied live (T235) |
| 45 | Sending states: cancel a queued send, retry a stopped one, settle an unconfirmed one (mark sent or retry) | `row.rs` (unconfirmed mark), the reader's `Verbs::STANDARD`; `CancelSend`, `RetrySend`, `MarkSent` (`unconfirmed_send`, ADR 0021) | **Covered by** the row's state word and the open message's send verbs (`focus_dialog::send_verbs`): Cancel send, Retry send, Mark as sent and Edit, in the dialog or the pane; the Outbox in `g o` (T239; `focus_suite::sending_states`) |
| 46 | `mailto:` links open a composer | `postio-app` (`mailto_uri`), the desktop entry's `MimeType` | **Covered by** `FocusWindow::open_link`, which takes a `mailto:` URI through `postio_model::mailto` into the composer (`focus_suite::desktop`). The desktop entry's `MimeType` claims `mailto:` since T253 (`packaging::postio_handles_mailto_and_postio_links`) |

### Accounts, settings and configuration

| # | Capability | In the classic app | Where it is now |
|---|---|---|---|
| 47 | First run: the account form, OAuth, then how much history to sync | `postio-app::onboarding` (`Status::SyncWindow`, `write_sync_window`; `sync_window`) | The form and sign-in are **covered by** Focus's first run (`window.rs`, `postio_widgets::present::onboarding`; `focus_suite::first_run`). The sync-window step is **covered by** `Presenter::ask_sync_window`, which every first run uses (`focus_suite::first_run`) |
| 48 | Adding another account; updating a credential | `add_account.rs`, `settings_credential.rs` | **Covered by** `AddAccount` and `UpdateCredential` in `FocusWindow::act`, and by the sign-in banner's button, which an account with no stored credential gets too: the sync blocks on a missing password (`BackendError::needs_a_password`) and the banner opens the credential form (`focus_suite::startup_repair`, T262) |
| 49 | The settings window. Accounts: edit, test the connection, token expiry, enable or disable, remove, rebuild the index, set the default, map mailbox roles, weights. Also Filters, Composing, Appearance, Keyboard, Sync and storage, Privacy (the remote-image allow list, the unsubscribe log, the read-receipt count, the connection log) and the config file | `postio-gtk::settings`, `widgets/`; `postio-app::settings_*`, `sidebar_backfill.rs`; `ToggleAccountEnabled`, `RemoveAccount`, `RebuildAccountIndex`, `SetDefaultAccount`, `MapMailboxRole` | **Covered by** the shared settings window in a dialog (T233, T234): `mod+comma` and the main menu's Settings open it (`FocusWindow::act`). Every section but Appearance (rows 18, 19), with the classic app's wiring ported (`settings_wiring.rs`). The account verbs are reached from each row's menu and its detail view, and from the keyboard (T258): with the keyboard on an account row, `Return`, `Delete` (undone by `mod+z`), `r`, `m` and `M` (which opens the account's roles) act on that row, and the command bar lists them (with Settings shut they open it to pick a row). |
| 50 | Excluding a folder from backfill (ADR 0016) | `sidebar.rs` (`connect_backfill_exclusion_changed`) | **Covered by** Sync & storage's "Back up locally", a check per folder (T234; `settings::a_folder_left_out_of_backfill_is_written_and_shown`) |
| 51 | Edit configuration (`mod+e` opens `config.toml` in the person's editor) | `postio-gtk::config`; `EditConfig` | **Covered by** `FocusWindow::act` through `postio_widgets::editor`, the shared launcher (T235; `settings::mod_e_opens_config_toml_in_the_persons_editor`) |
| 52 | `config.toml` applied live: keys, filters, sync, storage ceiling, reader zoom, compose, ui | `postio-gtk::config` (`storage_ceiling_wiring`) | **Covered by** `Session::follow_config`: keys, filters, sync, `[focus]`, and since T235 `[compose]`, `[reader]` and `[storage]` (`reload.rs`'s case per section) |

**Checked and absent from both apps:** printing (neither app had a print
operation), and creating, renaming or deleting folders.
