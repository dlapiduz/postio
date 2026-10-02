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

**Count:** 52 capabilities. 24 are covered. 16 have a gap, closed by T233–T248;
four of those are already partly covered. 12 are dropped, and one of them
(flagging) is decided: Flag stays, on `*` (C13).

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
| 22 | Dragging messages out as `.eml` files | `postio-gtk::drag_out`, `postio-app::export` (`drag_out_wiring`) | **Gap → T245** |
| 23 | Empty, offline and failing list states | `postio-gtk::list_state` | **Covered by** `empty.rs` and `postio_ui::focus_state` (`InboxSaying`: empty, syncing, offline, failed; T220) |
| 24 | A unified view names an account it cannot reach, and the reason | `list_state.rs` (`derive_aggregate`), ADR 0005 Q10 (`degraded_unified`) | **Gap → T248.** Focus names an account only when its password is refused. Any other failure is "Sync failed", with no account, no reason and no retry |

### Reading

| # | Capability | Where in the classic app | Verdict |
|---|---|---|---|
| 25 | A thread stacked as one document, with the conversation rail; fold or unfold one message (`z`), expand all (`O`), hide or show the rail (`I`) | `postio-gtk::conversation`, `reader/rail.rs`; `ToggleFold`, `ExpandAll`, `ToggleRail` | **Dropped**, because Focus shows one message at a time and steps the thread with `[`/`]`, a deliberate departure from ADR 0032 (C2, FR-037). Maintainer: no (Clarifications) |
| 26 | The body on the hardened renderer, with quoted text folded | `postio-widgets::reader` (shared) | **Covered by** Focus's open message (`open.rs`, the same `Reader`; FR-033, FR-034) |
| 27 | Reader view and View original (`mod+shift+o`, `mod+o`); Darken this message (`alt+d`) | spec 006; `ToggleReaderView`, `ViewOriginal`, `DarkenMessage` | **Dropped**, because Focus's two treatments do the job: app colours or the original (`O`, T210–T213). Paper is never inverted or recoloured (T212), which rules out darkening. Maintainer: no |
| 28 | Zoom in, out and reset (`mod+plus`, `mod+minus`, `mod+0`), kept in `[reader] zoom` | spec 006 FR-021 (`zoom_persists`) | **Gap → T242.** `docs/PRODUCT.md` §20 promises scalable text. Paper's fit-to-column zoom is automatic, not the person's |
| 29 | Find in the message | `FindInMessage` | **Covered by** `reading_key` (`mod+f`, `mod+g`, `mod+shift+g`; T203) |
| 30 | View source | the reader | **Covered by** `v` (`source.rs`, `focus_suite::view_source`) |
| 31 | Remote images blocked; allowed once, or always for this sender | `postio-widgets::reader::banner` (shared); `ShowImages`, `AlwaysShowImages` | **Gap → T247.** The banner is shared and `focus_suite::remote_images` proves the block, but nothing proves that allowing works in Focus, and both commands are in `NOT_YET` |
| 32 | Unsubscribe on deliberate activation | the reader's notice | **Covered by** the reader's notice and the digest window (`Unsubscribe`, `window.rs`) |
| 33 | Reading a message marks it read, after a short dwell | `list_view.rs` and `reading.rs` (`MarkReadOnDwell`; `dwell_wiring`) | **Gap → T237.** Focus never sends `MarkReadOnDwell`: a message opened and read stays unread until `r`. FR-016 covers only moving the cursor |
| 34 | A body that did not decode cleanly says so | `postio-widgets::reader` (`decode_notice`) | **Covered by** the shared reader's notices, which Focus's open message mounts |
| 35 | Desktop notifications for new mail | `postio-app::notifications` | **Covered by** Focus's notifier (`startup.rs`, `host.focus_notification`), following `[sync]` |

### Attachments

| # | Capability | Where in the classic app | Verdict |
|---|---|---|---|
| 36 | Opening an attachment, or a link, from the keyboard | `parts.rs` (`OpenPart`, `OpenPartExternally`) | **Covered by** `o` (`chooser.rs`, `open_choice`): each target is shown before anything opens, and a part is written and handed to the default app |
| 37 | Saving one attachment, or all of them, to a folder; clicking a chip | `parts.rs` (`SavePart` `s`, `SaveAllParts` `S`); `Reader::connect_attachment` | **Gap → T240.** `docs/PRODUCT.md` §11 promises "save as". Focus draws the chips but leaves `connect_attachment` unwired, and nothing saves |
| 38 | The MIME tree: walking every part, and rendering one once (`p`, `j`/`k`, `H`) | `postio-gtk::parts`; `OpenParts`, `NextPart`, `PrevPart`, `RenderPartOnce` | **Dropped**, because `v` (the raw source) checks what a message is made of, and `o` reaches every attachment. Maintainer: no |

### Search

| # | Capability | Where in the classic app | Verdict |
|---|---|---|---|
| 39 | Live search with the query language as chips, and `in:` to complete a folder | `postio-gtk::search`, `postio-app::search` | **Covered by** the command bar (`bar.rs`, `postio_search::natural::lower`; US4) |
| 40 | The misspelling suggestion ("Search instead for…", ADR 0037), and the result order (relevance or date, `O`) | `search.rs`, `list_view.rs` (`set_result_order`); `ToggleResultOrder` (`search_instead`) | **Gap → T241.** Focus's bar asks for relevance only and drops `SearchResults::instead` |
| 41 | Refinements with counts (the facet column) and the search scope column | `search.rs` (`set_facets`), `widgets/nav_row.rs` | **Dropped**, because the bar's chips and operators (`in:`, `from:`, `account:`) narrow a search in the box itself, and the bar draws no columns (screens 07–09). Maintainer: no |
| 42 | Saved searches: save, run (`alt+1`–`alt+4`), and rename, reorder and delete in the sidebar | `sidebar.rs` (`connect_saved_search_action`), `config.rs`; `RenameSavedSearch`, `MoveSavedSearchUp`, `MoveSavedSearchDown`, `DeleteSavedSearch` | Saving and running are **covered by** `SaveSearch` and `SavedSearch1`–`4` (`window.rs`). Renaming, reordering and deleting are a **gap → T233**, in Settings' Filters section |

### Compose

| # | Capability | Where in the classic app | Verdict |
|---|---|---|---|
| 43 | Rich-text composer: Cc and Bcc, identities, recipient completion, attachments, inline images, detach, send later, autosaved drafts | `postio-widgets::composer` (shared); `postio-app::compose` | **Covered by** Focus's composer dialog (`compose/`, T221) over the same composer and `present::compose` |
| 44 | Signatures: per account, placed above or below the quote | `[compose]`; Settings' Composing section | Inserting a signature is **covered by** the shared composer (`compose/seams.rs`). Editing one is a **gap → T233** |
| 45 | Sending states: cancel a queued send, retry a stopped one, settle an unconfirmed one (mark sent or retry) | `row.rs` (unconfirmed mark), the reader's `Verbs::STANDARD`; `CancelSend`, `RetrySend`, `MarkSent` (`unconfirmed_send`, ADR 0021) | **Gap → T239.** Focus's reader is built with `Verbs::NONE` and its rows draw no draft state, so an unconfirmed send cannot be settled in Focus |
| 46 | `mailto:` links open a composer | `postio-app` (`mailto_uri`), the desktop entry's `MimeType` | **Covered by** `FocusWindow::open_link`, which takes a `mailto:` URI through `postio_model::mailto` into the composer (`focus_suite::desktop`). The desktop entry's `MimeType` still moves with T253, and `packaging.rs` still asserts it leaves `mailto:` to the classic app until then |

### Accounts, settings and configuration

| # | Capability | Where in the classic app | Verdict |
|---|---|---|---|
| 47 | First run: the account form, OAuth, then how much history to sync | `postio-app::onboarding` (`Status::SyncWindow`, `write_sync_window`; `sync_window`) | The form and sign-in are **covered by** Focus's first run (`window.rs`, `postio_widgets::present::onboarding`; `focus_suite::first_run`). The sync-window step is **covered by** `Presenter::ask_sync_window`, which both apps' first runs use (`focus_suite::first_run`) |
| 48 | Adding another account; updating a credential | `add_account.rs`, `settings_credential.rs` | **Covered by** `AddAccount` and `UpdateCredential` in `FocusWindow::act`, and by the sign-in banner's button |
| 49 | The settings window. Accounts: edit, test the connection, token expiry, enable or disable, remove, rebuild the index, set the default, map mailbox roles, weights. Also Filters, Composing, Appearance, Keyboard, Sync and storage, Privacy (the remote-image allow list, the unsubscribe log, the read-receipt count, the connection log) and the config file | `postio-gtk::settings`, `widgets/`; `postio-app::settings_*`, `sidebar_backfill.rs`; `ToggleAccountEnabled`, `RemoveAccount`, `RebuildAccountIndex`, `SetDefaultAccount`, `MapMailboxRole` | **Gap → T233, T234.** Focus's main menu has "Settings" (`mod+comma`, `contracts/focus-surface.md`), but `FocusWindow::act` does not answer `Settings`, so the item does nothing |
| 50 | Excluding a folder from backfill (ADR 0016) | `sidebar.rs` (`connect_backfill_exclusion_changed`) | **Gap → T234**, as a per-folder control in Settings' Sync and storage section, since Focus has no sidebar to carry the menu |
| 51 | Edit configuration (`mod+e` opens `config.toml` in the person's editor) | `postio-gtk::config`; `EditConfig` | **Gap → T235** |
| 52 | `config.toml` applied live: keys, filters, sync, storage ceiling, reader zoom, compose, ui | `postio-gtk::config` (`storage_ceiling_wiring`) | Keys, filters, sync and `[focus]` are **covered by** `Session::follow_config`. `[storage]`, `[compose]` and `[reader]` are a **gap → T235**: they take effect only on the next start |

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
   until removal. Focus's name and binary name wait for the maintainer (see
   below).
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

## Risks

- **Focus builds from files in `postio-gtk`.** `postio-widgets/build.rs` reads
  `../postio-gtk/data/icons` (T217's single copy of the icon), and
  `postio-widgets/data/metrics.css` is generated by `postio-gtk/build.rs`. The
  Flatpak installs the desktop entry, metainfo and icons from
  `crates/postio-gtk/data/`, and `postio-focus/tests/packaging.rs` reads them
  there. Deleting the crate before T249 would break Focus's build and its
  icon.
- **Some shared code is only exercised by the classic app.** Examples are the
  reader's conversation and rail seams (`Reader::connect_current_message`,
  the per-message reply bars), `Verbs::STANDARD`, `postio_ui::list_state`'s
  aggregate rule, `postio_ui::settings` (until T234), and the
  `ComposerHost` that `postio-gtk::composer` implements. With the classic app
  gone these become untested or dead, and `check-uncalled-pub-fn.py` will say
  so late. T251 lists them first.
- **`app_suite` proves things Focus relies on.** Examples are the reader
  spawning no web process, hostile mail, reclaiming disk on open, startup
  repair, the event fan-out, notifications off the main thread, and `e2e.rs`
  against a real IMAP server. These are host and renderer behaviour, not
  classic surfaces. Deleting `app_suite` without T252 would remove the only
  proof of them at the composition root.
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
- **Benchmarks link the classic app.** `postio-bench`'s `list_scroll`,
  `composer_open` and `conversation_rows` use `postio-gtk` types, and the
  release workflow compares the terminal's binary size with `postio`'s. T250
  and T254 port or retire them.
- **Other lanes on this branch.** T232 (the reading pane) is changing
  `window.rs` and `open.rs`. T236, T237 and T240 touch the same files and
  should rebase onto it.

## Questions for the maintainer

1. **Removal (T256)** waits for your word, once T233–T255 are done.
2. **The constitution amendment** for one desktop app, with the branch's
   existing amendment.
