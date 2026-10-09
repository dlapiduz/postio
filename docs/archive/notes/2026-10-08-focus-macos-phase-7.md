# Focus on the Mac, phase 7: the app's states against screens 16 to 19

2026-10-08, specs/009-focus-macos T101 (FR-061). The phase numbering
follows tasks.md: phase 9 of the spec is US7, and this is the seventh Mac
comparison.

**Captures.**
- Made with `scripts/macos-shot.sh` at 1440×900, light, with 18 and 19
  also in dark.
- A demo never syncs, and screens 17 to 19 are what sync says. A demo
  build now takes `POSTIO_DEMO_STATE`, which calls the new `demo_state`
  export once the session opens: it emits, for the demo's account, the
  engine events sync would have emitted (`emit_for_test`, which the `demo`
  feature already carried). `offline` is `ConnectionChanged(Offline)`;
  `auth` is `Failing(Auth)`; `first-sync` is `Online` then
  `SyncProgress(12,408 of 18,204)`, the pack's numbers; `synced` is a pass
  that finished. A build without demos answers `false` and emits nothing.
  - `16-empty`: `--seed empty`, `POSTIO_DEMO_STATE=synced`;
  - `17-first-sync`, `18-offline`, `19-sign-in`: `--seed small` with
    `first-sync`, `offline` and `auth`.
- Captures live in the main checkout's untracked
  `Design/review/focus-macos/`.
- References: `Design/focus-macos-design/screens/16-state-empty-inbox.png`,
  `17-state-first-sync.png`, `18-state-offline.png` and
  `19-state-sign-in-error.png`.

Every word on these screens -- the banner's heading, sentence and button,
the toolbar's label, the empty page's heading, lines and shortcuts -- is
`postio_ui::focus_state`'s, said by `postio-focus`'s controller (slice 10)
as `FocusBanner`, `FocusSyncLabel` and `FocusEmpty`. The Mac draws them.

## What matches

- **16, the empty inbox.** The list gives way to a centred page: the tray
  symbol, "Inbox is empty" in bold, "Next digest: Weekly · Newsletters,
  <day> 16:00", and the shortcuts as keycaps before their words --
  `g f` 186 filtered today, `g r` archive, `c` compose. The toolbar says
  "Synced HH:MM" with its check mark; the strip keeps Inbox ▾ 0 and
  Has action · 0.
- **17, first sync.** One strip under the header strip: "First sync" in
  bold, "12,408 of 18,204 messages, newest first. You can read and search
  what's here.", and a progress line at about two thirds. The toolbar
  says "Syncing 12,408 of 18,204" with the circular arrows. The list stays
  live under it.
- **18, offline.** "You're offline", "Everything you do is saved here and
  syncs when you're back.", and "Retry now"; the toolbar says "Offline"
  with the no-network symbol.
- **19, sign-in error.** The strip in `systemRed` at 10%: "Can't sign in to
  <server>", "The server rejected the password for <address>. Mail on this
  computer is still available.", and "Update password…". The toolbar says
  "Sync failed" in red with the warning triangle. Dark appearance tints the
  strip the same way over the dark list.

## Differences, each fixed or explained

1. **Fixed: "Update password…" drew a `c` keycap.** The controller asked
   the keymap for `update_credential`'s key without asking where it
   applies; `c` is that command's binding in the settings window's
   accounts list, and over the list `c` composes. The banner now names a
   button's key only when the command is available in the list's context
   (`postio-focus` `states.rs`; its test had pinned the wrong key).
2. **"Retry now" draws an `F5` cap; the pack draws none.** `refresh` is
   bound to `F5` and works from the list, and every action has one key
   shown beside the button that does the same (SPEC, global rules). Kept.
3. **The empty page has a "Synced HH:MM" line the pack does not.** It is
   the controller's `detail` (`inbox_saying`: a pass has finished, and
   when), which GTK draws too. The pack's page has room for it; kept as
   the shared words.
4. **"1 digest rule", "9 filtered today", the senders, the day.** The
   demo store's fixture, not the pack's.
5. **The plain strip is a touch greyer than the pack's.** `.quinary` at
   half opacity; the pack's fill is not a named colour, and the contract
   allows semantic colours only.
6. **17 to 19 in the pack show three selected rows and the action bar.**
   A state of the list, not of these screens (phase 2's comparison).
7. **The toolbar's label is blank until the controller first speaks.**
   Before the first sync event there is nothing to say; the Swift copy of
   "Not synced yet" went with the Swift copy of `sync_label` (T099). Not
   visible in these screens, which are all after a sync word.

## Also built

- **"Update password…"** opens a sheet on the main window for the account
  the banner names (`banner.account`), through `AccountRepair` -- its own
  instance, since the settings window presents whenever its own is asking
  -- so the password goes to the Keychain by the same route as the
  settings row's. An OAuth account goes to the browser instead, as there.
  While the sheet is up every key is its own: Return saves, Escape
  cancels.
- **The store refusal** (T100) is not on these screens: a store from
  another build is the ffi_suite's to make, and was not photographed.
