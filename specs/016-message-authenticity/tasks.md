---
description: "Task list for spec 016, message authenticity"
---

# Tasks: Message authenticity

**Input**: `specs/016-message-authenticity/`: [spec.md](spec.md),
[plan.md](plan.md) (Phase 0 research is R1 to R8),
[data-model.md](data-model.md), [contracts/](contracts),
[quickstart.md](quickstart.md)

**This file is the tracker.** These tasks get no issues (CLAUDE.md,
spec-driven work). Tick a box in the commit that finishes it. The branch
lands once, with `scripts/issue-land.sh --detach --full-suite --title
'feat: message source, verification and signatures'`.

**Tests are required** (constitution IV). Each implementation task follows
the task that writes its test, and that test task names what is seen red. A
test that is green on arrival is a guard, and says so.

## Conventions

- `[P]`: the task touches different files and depends on no unfinished
  task. `[USn]`: the spec's user story. `⛔ S1`, `⛔ S2`, `⛔ S3`: the task waits
  on that spike's "go".
- **Commits** end with `Refs: specs/016-message-authenticity` and the task
  ids. Never write a closing keyword.
- **Code comments follow CLAUDE.md's rules.** A comment is a why the code
  cannot show, in at most three lines. It carries **no FR, task, slice or
  decision ids** and no narrative; those belong in the commit body. A bare
  `(ADR 0042)` or `(#1928)` after a why is allowed. Prefer a test name or an
  assert message to a comment. Test files get no `//!`. `POSTIO-CONSENT:`
  lines stay.
- **The fast tier carries most of this.** Run `scripts/test-fast.sh` between
  edits, and `cargo nextest run -p <crate> --test <suite> <case>` to confirm.
- **Swift** tasks run on the Mac (`macos/CLAUDE.md`), and GTK tasks on Linux.
- **Storyboards** (ADR 0044) are seen red on `main` before they are built,
  and `/ux-review` runs before landing.
- **Fixtures** use `example.com`, invented names and keys generated for the
  corpus, through `/add-fixture`. No test reaches the network, and none reads
  the real `~/.gnupg`.
- **Traps** found along the way go in `docs/gotchas/<area>.md`.

---

## Phase 1: Setup

- [x] T001 Update `docs/PRODUCT.md` §21 for the maintainer's decisions: the "Signed" mark, keys from this machine only, and no DKIM re-check
- [x] T002 [P] Add the corpus fixtures through `/add-fixture`, each a row in `crates/postio-model/tests/corpus/README.md` and `crates/postio-model/src/test_corpus.rs`, under a new `signature` category:
  - `auth-results-arc-list.eml`: an ARC chain through a list;
  - `auth-results-two-dkim.eml`: one signature aligned with From, one not;
  - `pgp-signed-good.eml` and `pgp-signed-tampered-body.eml`;
  - `pgp-signed-tampered-header.eml`, a byte changed in the signed part's MIME headers;
  - `pgp-signed-qp.eml`, a quoted-printable signed part;
  - `pgp-signed-no-key.eml`;
  - `pgp-signed-other-key.eml`, the same From with a second key;
  - `pgp-signed-sha1.eml`;
  - `pgp-inline-signed.eml`;
  - `pgp-signed-list-footer.eml`, `multipart/mixed` around the signed part with a footer;
  - `autocrypt-header.eml`, carrying the good fixture's key;
  - `pgp-key-attached.eml`.

  The keys are generated once by `crates/postio-model/tests/corpus/keys/generate.sh`, which is checked in. The README says the keys are test-only.
- [x] T003 [P] Write `storyboards/reader/source-surface.toml` from US1 and US2: `V` from the list, `2`, `3`, `Esc` to the same row; `V` from the open message, then `Esc` twice. See it red on `main` with `scripts/storyboards.sh`
- [x] T004 [P] Write `storyboards/reader/signed-mark.toml` from US3: open the good fixture and see "Signed", press it and see Verification, accept the key and see "Signed · accepted", then `z`. See it red on `main`

## Phase 2: Spikes (gating)

Each spike ends with a one-paragraph verdict, "go" or "no-go", appended to
`plan.md`'s research section that it settles (R2, R3, R6, R7). A spike's
throwaway code is not merged.

- [x] T005 **S1, byte-faithful part offsets and rPGP (gates Phase 5 onward).**
  - In a scratch crate outside the workspace, take each of T002's PGP fixtures. Extract the signed part as a byte range with Pimalaya's `mime-rs`, and again with `mail-parser`'s part offsets.
  - Verify each with `pgp` 0.21, against the generated keys.
  - **Go** when every good fixture is `Good`, every tampered one is `Bad`, and the extracted bytes are identical to the wire bytes (CRLF-normalised).
  - Record which parser is chosen and why in plan R3.
  - Check that `pgp`'s resolved graph has no `-sys` crate (`cargo tree -e normal`).
- [x] T006 **S2, S/MIME path validation (gates Phase 8).**
  - In a scratch crate, build test chains with `rcgen`: good, expired, wrong EKU, unknown anchor, address mismatch.
  - Verify a CMS SignedData over a test message with `cms` 0.2 and `x509-cert`.
  - Validate the path with `rustls-webpki`, requiring `id-kp-emailProtection`.
  - **Go** when all five outcomes are right with no C in the graph. **No-go** files S/MIME as its own spec (spec D8), and Phase 8 is struck from this file in the same commit.
- [x] T007 [P] **S3, keyboxd's database (gates T046 only).**
  - Create a keyboxd store in a scratch `GNUPGHOME` with GnuPG 2.4 (`use-keyboxd`).
  - Read `public-keys.d/pubring.db` with the store's engine, read-only. Confirm the table and column that hold keyblocks, and that the open writes nothing (the file's and `-wal`'s mtime and size are unchanged).
  - Record the shape in plan R7. **No-go** leaves that format on the unreadable state.

## Phase 3: Foundational (blocks every story)

- [x] T008 Write failing cases in `crates/postio-core/tests/core_suite/` for the new `Context::Source` and the commands in `contracts/source-surface.md`:
  - each has a key, a command-bar row and an accessible action;
  - none collides in `Context::Source`;
  - `view_source`'s title is "Source and verification".
- [x] T009 Add `Context::Source` and the commands to `crates/postio-core/src/context.rs`, `crates/postio-core/src/command.rs` and `crates/postio-core/src/registry.rs`. Regenerate `docs/keybindings.md` (`POSTIO_UPDATE_DOCS=1 cargo test -p postio-ui`) and `crates/postio-core/tests/golden/linux-bindings.txt`
- [x] T010 Write failing unit cases in `crates/postio-ui/src/reader/authenticity.rs` for the report model in `data-model.md`:
  - every method's domain, selector and policy;
  - relaxed alignment;
  - ARC sets named by instance;
  - the trusted header chosen exactly as today (the existing cases stay green).
- [x] T011 Implement `AuthReport`, `MethodResult`, `ArcSet` and `Verification` in `crates/postio-ui/src/reader/authenticity.rs`, keeping `warning()`'s behaviour unchanged

## Phase 4: User Story 1 and 2, the Source surface (P1) 🎯 first slice

**Goal**: `V` opens Verification, Headers and Raw over what the store holds.
**Independent test**: quickstart "Slice A".

- [x] T012 [P] [US1] Write failing cases in `crates/postio-ui/tests/ui_suite/source_view.rs`, one per row of the spec's "Every state" table and per Verification section:
  - the provider named;
  - the not-believed headers listed;
  - "not checked on this computer";
  - the fixed line.
- [x] T013 [US1] Implement `SourceView` and `VerificationView` and every string in a new `crates/postio-ui/src/source.rs`, registered in `crates/postio-ui/src/lib.rs`
- [ ] T014 [P] [US2] Write failing cases in `crates/postio-ui/tests/ui_suite/source_view.rs` for the Headers tab and the Raw tab:
  - Headers: unfolding, RFC 2047 decoding with the raw form kept, and `Received` hops with delays;
  - Headers: the boundary under the trusted header;
  - Raw: folds over 64 KiB, with a part label and size.
- [ ] T015 [US2] Implement the headers rows, the hops and `RawView` in `crates/postio-ui/src/source.rs`
- [ ] T016 [US1] Write failing cases in `crates/postio-host/tests/host_suite/source_view.rs`:
  - `Req::SourceView` with local bytes: a counted, bounded number of statements;
  - absent bytes: `Fetching` at once, then `SourceReady`;
  - offline: `Offline`.
- [ ] T017 [US1] Implement `Req::SourceView` and `Resp::SourceView` in `crates/postio-client/src/protocol.rs` and `crates/postio-client/src/api.rs`, and the handler in `crates/postio-host/src/lib.rs` and a new `crates/postio-host/src/source.rs`, over `parts::raw_source` and `postio_session::reading`'s account lookup
- [ ] T018 [US1] Write a failing case `source_surface_makes_no_connection` in `crates/postio-host/tests/host_suite/source_view.rs`. It opens all three tabs for every corpus fixture in the `hostile` and `prompt-injection` categories, through the counting transport, and asserts zero connections. No renderer is reachable from the surface at all: `crates/postio-ui/src/source.rs` and the widgets dialog name no `postio_render` item, which a `check-crate-boundaries.py`-style source check asserts
- [ ] T019 [US1] Write failing behaviour cases in `crates/postio-focus/src/` (a new `source.rs`, beside `filtered.rs`): `V` from the list and from the message, `1` to `3`, `Esc` one rung at a time, and the cursor and scroll kept
- [ ] T020 [US1] Implement the Source surface's behaviour in `crates/postio-focus/src/source.rs`, and wire it in `crates/postio-focus/src/surfaces.rs`
- [ ] T021 [US1] GTK: replace `crates/postio-gtk/src/source.rs` with a `postio-widgets` dialog (a new `crates/postio-widgets/src/source/`) drawing the view model. Use `/gtk-design`, the one close and non-editable text with no link detection. Make `crates/postio-gtk/tests/focus_suite/view_source.rs` assert the tabs, with one case per path. T003 goes green
- [ ] T022 [P] [US2] GTK: `copy_source` and `save_source` (through `postio-host/src/export.rs`), with the case "the saved file equals `Client::raw_source`" in `crates/postio-gtk/tests/focus_suite/view_source.rs`
- [ ] T023 [P] [US1] Terminal: draw the three tabs as frames in `crates/postio-tui/src/view/open.rs`, replacing its single raw frame, with cases in `crates/postio-tui/tests/tui_suite/`
- [ ] T024 [P] [US1] Mac: expose `SourceView` in `crates/postio-ffi/src/focus_reader.rs`, and draw it in `macos/Sources/PostioKit/MessageWindow.swift`, with `macos/Tests/PostioKitTests/MessageWindowModelTests.swift` cases. The FFI command sweep `crates/postio-ffi/tests/ffi_suite/command_coverage.rs` stays green
- [ ] T025 [US1] Run `/ux-review` on T003's storyboard, and fix or contest its verdicts

## Phase 5: User Story 3, OpenPGP verification and the mark (P2) ⛔ S1

**Goal**: signed mail says "Signed", "Signed · accepted" or the warning line.
**Independent test**: quickstart "Slice B".

- [ ] T026 [US3] Create the pure leaf `crates/postio-signature/` (`Cargo.toml` with `pgp` pinned exactly and default features trimmed, and the parser S1 chose). Add its entry to `scripts/checks/check-crate-boundaries.py` beside `postio-calendar`'s, with no store engine, toolkit, async runtime or network crate, and see the check fail on a deliberately added `tokio` dev-dependency, then remove it
- [ ] T027 [P] [US3] Write failing cases in `crates/postio-signature/src/scope.rs` for `signed_scope` over T002's fixtures: whole, part (the list footer), inline, encrypted, and none
- [ ] T028 [US3] Implement `signed_scope` and the exact-bytes slice of `contracts/verdict.md` in `crates/postio-signature/src/scope.rs`
- [ ] T029 [P] [US3] Write failing cases in `crates/postio-signature/src/policy.rs`, one per row of `contracts/verdict.md`'s policy table
- [ ] T030 [US3] Implement `verify` and `Policy` in `crates/postio-signature/src/verify.rs` and `crates/postio-signature/src/policy.rs`
- [ ] T031 [US3] Write the sweeps SC-003 and SC-004 as `crates/postio-signature/tests/signature_suite/sweeps.rs`: every fixture, against every key subset
- [ ] T032 [P] [US3] Write failing cases, then implement `autocrypt`, in `crates/postio-signature/src/autocrypt.rs`: `addr` equals From, `keydata` parsed, and a mismatched `addr` refused
- [ ] T033 [US3] Write failing schema cases in `crates/postio-storage/tests/storage_suite/`, then add `public_keys`, `public_key_sources`, `public_key_addresses`, `contact_accepted_keys` and `signature_verdicts` to `crates/postio-storage/src/schema.rs` (`data-model.md`), with repository functions in a new `crates/postio-storage/src/repository/keys.rs`
- [ ] T034 [US3] Write failing cases in `crates/postio-sync/tests/sync_suite/backfill.rs`:
  - a message whose `content_type` is signed or encrypted is fetched whole, and keeps `raw_blob_id`;
  - other mail is unchanged.

  Implement it in `crates/postio-sync/src/backfill.rs`.
- [ ] T035 [US3] Write a failing case in `crates/postio-storage/tests/storage_suite/blob.rs` for the eviction order: unsigned raw first, then payloads, then signed raw. Implement it in `crates/postio-storage/src/blob.rs` (`evictable`). Rewrite `docs/decisions/0017-backfill-cost-attachments-memory-disk-encryption.md` in place to the new keep rule and order
- [ ] T036 [US3] Write failing cases in `crates/postio-session/tests/session_suite/signature.rs`:
  - a signed fixture arrives and a verdict row is written off the UI path;
  - an Autocrypt key is recorded;
  - a key change for an address re-verifies its rows;
  - evicted raw is `not_here_yet`, and resolves after the fetch.

  Implement the verdict job in a new `crates/postio-session/src/signature.rs`, called from the body stage.
- [ ] T037 [US3] Write `contracts/header-mark.md`'s table as failing unit cases in `crates/postio-ui/src/reader/authenticity.rs`, then implement `HeaderAuthenticity` and `Mark`. Replace `Reading.sender_warning` with `Reading.authenticity` in `crates/postio-client/src/protocol.rs` and every reader of it (`grep -rn sender_warning crates/ macos/`)
- [ ] T038 [US3] Draw the mark:
  - GTK: in `crates/postio-widgets/src/reader/message_header.rs` and `crates/postio-gtk/src/open_header.rs`, a neutral chip that runs `view_source`, with no icon and no accent;
  - terminal: in `crates/postio-tui/src/view/open.rs`;
  - Mac: through `crates/postio-ffi/src/focus_reader.rs`.

  Add one `focus_suite` case, `signed_mark`. T004 goes green.
- [ ] T039 [US3] Add the Verification tab's signature section (the source of the key, D10's lines, and "Accept this key for …") to `crates/postio-ui/src/source.rs`, with cases in `crates/postio-ui/tests/ui_suite/source_view.rs`
- [ ] T040 [US3] Show the encrypted state ("This message is encrypted. Postio can't decrypt mail yet.") in `crates/postio-ui/src/reader/document.rs`, and drop the signature part from the attachment list in `crates/postio-ui/src/reader/parts.rs`, with cases in `crates/postio-ui/tests/ui_suite/`
- [ ] T041 [US3] Add the refused mechanisms (`/.well-known/openpgpkey/`, `hkps://`, `_domainkey`, and spawning `gpg`) to `scripts/checks/check-no-silent-tracking.py`, with a self-test file showing each refused

## Phase 6: User Story 5, managing keys (P3)

**Goal**: import, accept and forget keys from the card and the tab.
**Independent test**: US5 scenarios 1 and 2.

- [ ] T042 [US5] Write failing registry and host cases, then add `import_key`, `accept_key` and `forget_key` (with `Recovery::Undo` for accept and forget) to `crates/postio-core/src/registry.rs`, and their verbs to `crates/postio-host/src/` against `contact_accepted_keys` and `public_keys`
- [ ] T043 [P] [US5] Show a contact's keys, with their sources, on the card in `crates/postio-ui/src/` (the contact card view model) and its frontends, with one case per frontend
- [ ] T044 [US5] Offer "Import key" for an `application/pgp-keys` part and for a `.asc` file in `crates/postio-gtk/src/` (the file chooser) and the terminal's command bar, with cases

## Phase 7: User Story 5, the GnuPG keyring (P3)

**Goal**: keys from `~/.gnupg`, read-only, when the person turns it on.
**Independent test**: quickstart "Slice B2"; US5 scenarios 3 to 5.

- [ ] T045 [P] [US5] Write failing cases in `crates/postio-signature/src/gnupg.rs` with byte fixtures built in the test:
  - `pubring.gpg` with trust packets between keys;
  - a keybox with header, OpenPGP and X.509 blobs;
  - a truncated blob;
  - a blob whose stated length overruns.

  Implement `keyblocks_from_pubring_gpg` and `keyblocks_from_keybox` there. They are bounds-checked, and never panic.
- [ ] T046 [US5] ⛔ S3 Read keyboxd's `public-keys.d/pubring.db` read-only in `crates/postio-host/src/gnupg.rs`, in the shape S3 recorded, with a case over a database the test builds
- [ ] T047 [US5] Write failing cases in `crates/postio-host/tests/host_suite/gnupg_keyring.rs` over a temporary `GNUPGHOME`, one per format. Assert:
  - every path opened, and never `private-keys-v1.d/`, `secring.gpg`, `trustdb.gpg` or a lock file;
  - no process spawned;
  - the switch off and on;
  - reload after a change;
  - the unreadable state.

  Then implement `crates/postio-host/src/gnupg.rs` (read the whole file, retry a torn read once, count what was skipped).
- [ ] T048 [US5] Add `[signatures] gnupg_keyring` to `crates/postio-config/src/` with a `config_suite` case, the Settings → Privacy row and its counts to the settings model in `crates/postio-ui/src/`, and `reload_gnupg_keys` to the registry
- [ ] T049 [US5] Add `--filesystem=~/.gnupg:ro` to `flatpak/dev.postio.Postio.json` and `flatpak/dev.postio.PostioTui.json`, each with a one-line manifest comment saying why
- [ ] T050 [US5] Verification tab: "From your GnuPG keyring". A key GnuPG fully trusts is still "Signed" until it is accepted, with a case in `crates/postio-ui/tests/ui_suite/source_view.rs`

## Phase 8: User Story 4, S/MIME (P3) ⛔ S2

Every task here waits on T006's "go". On "no-go" this phase is deleted, and
S/MIME is filed as its own spec.

- [ ] T051 [US4] ⛔ S2 Add the T006-chosen crates to `crates/postio-signature/Cargo.toml`, keeping the boundary check and the no-`-sys` rule green. Add S/MIME fixtures (good, tampered, expired, unknown anchor, address mismatch) through `/add-fixture`
- [ ] T052 [US4] ⛔ S2 Write failing cases, then implement S/MIME `verify` in `crates/postio-signature/src/smime.rs`: chain to anchors handed in, `id-kp-emailProtection`, the From address against the certificate, and no revocation
- [ ] T053 [US4] ⛔ S2 Read the system trust anchors from disk in `crates/postio-host/src/` (`/etc/pki/tls/certs`, `/etc/ssl/certs`; the Mac's keychain through `postio-ffi`), with a case over a temporary anchor directory
- [ ] T054 [US4] ⛔ S2 Add "Revocation not checked", and the mark for S/MIME, to the view model and `contracts/header-mark.md`'s cases

## Phase 9: Polish and landing

- [ ] T055 [P] Fuzz targets for `verify`, `keyblocks_from_keybox` and `keyblocks_from_pubring_gpg` in `crates/postio-signature/fuzz/`, run in the nightly job beside the measurement tier, never on the merge path
- [ ] T056 [P] Add a "signatures" section to `docs/gotchas/safety.md` for whatever canonicalisation or keybox trap the spikes and tasks found
- [ ] T057 Update `docs/ARCHITECTURE.md`'s diagram and §9 for `postio-signature`, and `docs/decisions/README.md`'s 0017 row
- [ ] T058 Run `/ux-review` on T004's storyboard, then land with `scripts/issue-land.sh --detach --full-suite`

## Dependencies and order

```text
T001-T004 ─▶ T005 S1 ─┐        T006 S2 ─────────────────▶ Phase 8
            T007 S3 ──┼───────────────────────▶ T046
T008-T011 ─▶ Phase 4 (US1, US2) ─▶ Phase 5 (US3) ─▶ Phase 6 (US5) ─▶ Phase 7 (US5 keyring)
                              S1 ─┘                                            └─▶ Phase 9
```

- Phase 4 depends only on Phase 3, and lands value alone.
- Phase 5 needs S1's "go". Phase 7 needs Phase 5's tables. T046 alone needs
  S3. Phase 8 needs S2's "go" and Phase 5.

## Parallel examples

- Phase 1: T002, T003 and T004 together.
- Phase 2: T005, T006 and T007 together.
- Phase 4: T012 and T014, then T023 and T024 once T017 is in.
- Phase 5: T027, T029 and T032 together.

## Implementation strategy

1. **MVP is Phase 4**: the Source surface, with no crypto. It is useful
   alone.
2. Then Phase 5, the mark, which is the maintainer's Q1.
3. Then the key sources (Phases 6 and 7).
4. Then S/MIME only if S2 says go.

Each phase leaves `main`-quality code on the branch, and the branch lands
once.
