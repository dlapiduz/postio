# Storyboard catalogue seed

Sources: closed/open issues (numbers) and, for Focus, fix commits on origin/feature/postio-focus (no issue; cited as commit sha / task id). Each issue was read from its body; each commit from its message.
Checks: F=focus region, C=cursor, V=view, O=overlay, T=toast/notice, S=scroll.

| issue | one-line defect | app | storyboard sketch (seed; steps; failing check) | needs |
|---|---|---|---|---|
| 1687, f84d1343, 1fa814a0 | Archive/delete under the cursor scrolled the list to top and moved the cursor elsewhere | classic | inbox of 30 threads, cursor on row 12, scrolled; `a`,`a`,`d`; after each: C = row below the removed one (last row: previous), S unchanged, F = list | keyboard-only |
| 1474, 1011, db9e5423 | Escape did nothing once the finder closed (or after a search), list stuck on hits | classic | inbox; `/`, type "inv", Return (finder closes, list shows hits), Escape; V = the folder, not hits, F = list | keyboard-only |
| 6eadd8e2 | Escape from search closed it but did not return the keyboard to the row it left | classic | cursor row 5; `/`, type, Escape; F = list, C = row 5 | keyboard-only |
| 1473, 33b7fd4c | Window opened with keyboard in the search field, so j/a/? typed letters | classic | cold launch; press `j`; F = list, C = row 2, search text empty | keyboard-only |
| 693, 1252, 1034 | Return or Tab from the search box should move keyboard to the list; flake where Tab claimed but list did not get focus | classic | `/`, type, Tab; F = list. Repeat with Return after results land | keyboard-only (timing for flake) |
| 79b1cd8a | Return on a search mode hint row did nothing (ran empty search, fell back to folder) | classic | `/`, Down onto the "#" hint, Return; V/O = that mode entered, not folder | keyboard-only |
| 494, 437 | Tab/Shift-Tab had no deliberate pane cycle; Shift-Tab from reader landed on the last row, not the current one | classic | sidebar focus; Tab, Tab, Shift-Tab; F sequence sidebar>list>reader>list, C = current row | keyboard-only |
| d2be7412 | Sidebar keyboard walk stuck on Snoozed (view rows share id 0) so j never reached Outbox | classic | focus sidebar on Inbox; j x N; F/C visits Flagged, Snoozed, Outbox in order | keyboard-only |
| 455, 471 | Saved searches and account actions had no keyboard path (j/k, enable/remove) | classic | sidebar with 2 saved searches; j into them, Return; V = that search, F = list | keyboard-only |
| 813 | Folder reload (resync, rename) threw the list out of Flagged back to the inbox | classic | open Flagged; inject MailboxesChanged (wait); V still Flagged, C kept | timing |
| 1177, 1212, 1444 | First screen showed a draft/reply then jumped away; pane offered Reply on an unsent draft; draft verbs lost | classic | seed: draft + reply in inbox; cold launch; wait; V = first real message, cursor never lands on draft; on a draft: no Reply, Continue offered | timing |
| 601, 1414 | Opening the app leaves reading pane empty for the selected row | classic | cold launch; wait settle; pane shows row 0 body (not blank/plate) | timing |
| 491, 1196, 1240, 426 | Cold start opens composer from a left-over draft; `c` opens last draft not blank; Reply shows stale composer text; two unsaved drafts treated as same | classic | seed: left-over draft; launch (V = inbox); `c` (blank); type; Escape; `e` on message B (seeded from B, not draft) | keyboard-only |
| 1195 | Composer drew in a quarter of the window instead of taking over the reading pane | classic | `c`; V = compose owns reading pane; Escape returns V = reader | keyboard-only |
| 602, 73 | Single-key bindings eat typing in composer body (e opens another reply); single keys stop working at random | classic | `e` on message; focus body; type "reply here"; field text intact, no second composer; later `?` still opens cheatsheet | keyboard-only |
| 690, 325 | Forward opens focus in body not To; Reply does nothing unless the row was activated | classic | cursor on row (not opened); `e`; F = body, composer seeded. Forward `f`; F = To field | keyboard-only |
| 1481 | `u` does not undo a send, no Undo toast | classic | compose, send; T = Undo toast; `u`; V = composer back with draft | keyboard-only |
| 629, 68 | Return did nothing in onboarding fields (name, manual server fields) | classic | fresh store; wizard; type in each field; Return; V advances / submits from every field | keyboard-only |
| 67, 404 | Failed credential store leaves dead end; locked keyring only a toast | classic | seed: account row without credential; launch; V = setup/recovery screen, not empty inbox | real-routing |
| 1016 | Tabbing the keybinding rebind list leaks bare-letter bindings | classic | Settings > Keys; Tab to list; type `a`; no archive fires; context = Keys | keyboard-only |
| 756 | Toggle sidebar does nothing from palette or Ctrl+B | classic | `ctrl+k`, type "toggle sidebar", Return; V = sidebar hidden; `ctrl+b` shows it | keyboard-only |
| 825 | Narrow window: sidebar preference lost to breakpoint, one-pane mode has no way into reader | classic | 1100x700 window; Return on row; V = reader pane, Escape/back returns list | keyboard-only |
| 1402, 1431, 438, 0b808ee8 | No key scrolled the (one-document) conversation pane: space/PageUp/PageDown/J/K no-op | classic | open long thread; PageDown, space, PageUp, `J`; S moves by a page, marker advances | keyboard-only |
| 1398 | View original (Ctrl+O) did nothing in the one-document pane | classic | open bulk mail in reader view; `ctrl+o`; V = original layout | keyboard-only |
| 1386, 1365, 5621ef0f | Per-message focus and per-message reply do nothing in one-document pane; conversation bar must answer the latest message | classic | 6-msg thread, J x3 (F on msg 4); `e` replies to msg 4; conv bar `e` replies to latest | keyboard-only |
| 1385, 1372 | One-document pane opened on oldest message, not newest; rail mark and scroll fight | classic | thread with 6 msgs (some unread); open; S = newest visible; scroll then J, mark follows once | keyboard-only |
| d1ddd2dc, 69d0d56a, 1433 | Opening a conversation scrolled it / reloaded the document (a flash); scrolling navigated to "The URL can't be shown" | classic | j/k through list holding key; render count == keystrokes; V never error page | timing |
| 749, 947 | Reading pane flashes black between messages (reopened as maybe regression) | classic | hold `j` across 20 rows during backfill; per-frame check: no black frame / one render per step | timing |
| 1679, 2209a7dc | Showing images moved reader scroll 4px / redrawn document lost reader's place | classic | open mail scrolled to 1500px; click Show images; S within 0px of before | pointer |
| 797, 1400 | Opening a conversation marked newest message read without focus reaching it; a read-mark tore down conversation | classic | thread of 6 unread; open, wait dwell; only focused message read; V not redrawn | timing |
| 1173, 822 | Duplicate Reply buttons both bound to `e` in one-message thread | classic | open single message; V = exactly one reply bar; `e` opens one composer | keyboard-only |
| 468, 1701, 811, 1300 | Multi-select/folded unified row/select-all act on the wrong messages (newest only, one account's copy, nothing) | classic | unified inbox; `x` on 3 rows; `a`; all conversations gone in both accounts; T names count | real-routing |
| 753, 750 | Cursor vs selection look identical; inbox does not scroll to reveal new mail | classic | at top, inject new mail; S reveals new row; `x` row and cursor elsewhere show distinct states | timing |
| 1475, 499 | Folder shows Relevance sort chip; chevron does nothing | classic | open inbox; Tab to sort chip; Return; O = menu with date options | keyboard-only |
| 961, 767, 1526 | Search ignores account scope; OpenMessage from search preview reaches unwired dispatcher; hit shown once per folder | classic | Unified scope; `/`, query, Return, `o` on hit; V = reader on that message | real-routing |
| 1523, 1524, 1525 | Outbox reader offers Reply/Forward/Unsubscribe on own mail | classic | Outbox; open item; verbs absent; `e` shows T notice instead of composing | keyboard-only |
| 2af808b1 | Maximising window moved focus to search field and opened the finder | classic | focus on list; maximise then restore; F = list, O none | pointer |
| 92a093b8 | Focus left on a removed widget so j, z, Esc did nothing until click (row left with its message / list redrew) | focus | list cursor row 3; `d` (row removed); then `j`; C moves, F = list (key delivered, not dropped) | real-routing |
| 0cbbd3d8 | Rules list rebuilt rows, key path lost: Delete/Escape did nothing about half the time | focus | Settings > Rules open; wait for rules to land; `Delete` removes focused rule; Escape closes | timing |
| a19c4bbb, de495089 | Keys never reached the window under a dialog: j/k dead in open message, Up/Down stepped the list behind and jumped viewport | focus | open message dialog; `j`, `k`, Down, PageDown, space; list cursor unchanged behind, message scrolls, F in dialog | real-routing |
| 2ba0e97c | Redraw (toggle O) pulled reader back to marked sentence; first open did not scroll there | focus | open long message, scroll down; `O`; S unchanged; reopen: S at marked sentence once | keyboard-only |
| 8ab954bf | List cursor ring animated and trailed j/k | focus | hold `j` x5; cursor ring on target row at every frame (no transition) | timing |
| 88c1f0f7, 63641d47 | Dialog Del key and More menu popover: Del acts, More lets go of its menu on close | focus | message dialog; `Delete`; T/V gone; open More, Escape; F returns to More button | keyboard-only |
| 15192fb5 | Caret offset "none" and "at start" were the same number; typing goes where you look | classic | compose reply; check caret above quote; type "x"; text lands above quote | keyboard-only |
| 1687, 1609 | Rapid `a a a` / `d d d` working through the list must be one step per key | both | 20 rows; press `a` 5 times quickly; V: 5 rows gone, C on 6th original row | timing |
| 56 | Mail notification click should focus specific message; save search as folder | classic | notification activation for msg 7; C/F on msg 7 and pane shows it | pointer |
| 40 | Drag and drop as complete story (selections, dragging out) | classic | select 3 rows; drag to folder; V list drops them, T undo | pointer |

## Outcome (T078–T086, 2026-10-02)

What each row became. **Pinned** storyboards pass on Classic with checks that
name the exact outcome the fix established; **open** ones are red on purpose
until their issue lands; the rest say why they cannot be filmed yet. Focus
rows live on `feature/storyboards-focus` (T085).

| Row(s) | Storyboard | Proof |
|---|---|---|
| 1687 (+f84d1343, 1fa814a0) | `list/archive-walks-down`, `list/archive-keeps-walking-down`, `list/delete-keeps-walking-down` | pinned |
| 1474, 1011, db9e5423 | `search/escape-leaves-search` | pinned |
| 6eadd8e2 | `search/escape-returns-to-the-row` -- **found #1744** | open |
| 1473, 33b7fd4c | `list/launch-keyboard-on-first-row` | pinned |
| 693, 1252, 1034 | `search/return-hands-keyboard-to-results` (Tab now refines; Return is the handoff) | pinned |
| 79b1cd8a | `search/return-on-a-mode-hint` | pinned |
| 494, 437 | `sidebar/tab-cycles-the-panes`; `sidebar/shift-tab-from-reader-returns-to-row` (`routing = "real"`, not covered by chain delivery) | pinned |
| d2be7412 | `sidebar/walk-reaches-outbox` | pinned |
| 455, 471 | not expressible yet: no seed holds saved searches | -- |
| 813 | `sidebar/folder-reload-keeps-flagged`; filming it **found #1747** (`sidebar/go-to-flagged-lists-flagged`) | pinned / open |
| 1177, 1212, 1444 | `compose/launch-does-not-jump-to-a-draft`, `compose/draft-has-no-reply` | pinned |
| 601, 1414 | `reader/pane-is-filled-on-launch` | pinned |
| 491, 1196, 1240, 426 | `compose/blank-compose-after-a-left-over-draft` | pinned |
| 1195 | `compose/composer-takes-the-reading-pane` | pinned |
| 602, 73 | `compose/body-typing-is-not-eaten` | pinned |
| 690, 325 | `compose/reply-and-forward-focus` | pinned |
| 1481 | `compose/send-can-be-taken-back` (pinned); `compose/send-offers-an-undo-toast` -- **found #1752** | pinned / open |
| 629, 68 | not expressible yet: the first-run seed shows no onboarding form in Classic | -- |
| 67, 404 | `onboarding/locked-keyring-shows-recovery` | pinned |
| 1016 | not expressible yet: keys do not reach the settings window's rebind list by the runner | -- |
| 756 | `sidebar/toggle-sidebar-from-palette` | pinned |
| 825 | not expressible yet: `narrow` (900 px) is above the one-pane breakpoint (720 px) | -- |
| 1402, 1431, 438, 0b808ee8 | `reader/page-keys-scroll-the-pane` | pinned |
| 1398 | `reader/view-original-in-the-pane` | pinned |
| 1386, 1365, 5621ef0f | `conversation/j-k-walk-the-messages` -- **found #1748** | open |
| 1385, 1372 | `reader/open-lands-on-the-newest-message` | pinned |
| d1ddd2dc, 69d0d56a, 1433 | `reader/no-flash-between-messages` (the scroll half of #1433 is pointer, below) | pinned |
| 749, 947 | covered by `reader/no-flash-between-messages` (a blank frame fails the run) | pinned |
| 1679, 2209a7dc | not expressible yet: pointer (Show images by click) | -- |
| 797, 1400 | `conversation/read-mark-follows-focus` (only "no redraw" is pinned; the read mark itself is not observed) | pinned |
| 1173, 822 | `conversation/single-message-has-one-reply-bar` | pinned |
| 468, 1701, 811, 1300 | `list/selection-archives-the-selected` (account scope; the unified variant needs a unified start) | pinned |
| 753, 750 | `list/cursor-and-selection-look-different`; revealing new mail needs the `new_mail` event, not yet supported | pinned |
| 1475, 499 | `search/result-order-toggles` (search); the folder chip awaits #1475's decision -- evidence commented there | pinned |
| 961, 767, 1526 | `search/open-a-hit-from-the-results` | pinned |
| 1523, 1524, 1525 | `reader/outbox-offers-no-reply` -- **found #1749** | open |
| 2af808b1 | not expressible yet: pointer (maximise and restore) | -- |
| 15192fb5 | `compose/reply-caret-above-the-quote` (prose; the caret is not observed) | pinned |
| 1687, 1609 rapid keys | `list/rapid-keys-one-step-each` | pinned |
| 56 | not expressible yet: pointer (a notification's activation) | -- |
| 40 | not expressible yet: pointer (drag and drop) | -- |
| Focus rows (92a093b8, 0cbbd3d8, a19c4bbb, de495089, 2ba0e97c, 8ab954bf, 88c1f0f7, 63641d47) | on the Focus lane (T085); the shared storyboards already found **#1746** there | -- |

The ux-architect §4 flows are `flows/triage-walk` (passes),
`flows/open-walk-reply-send` (red on #1748) and `flows/search-open-reply-back`
(red on #1750). Reviewing the first runs also found #1745 (the first key
after launch can be lost) and #1751 (leaving the composer strands the
keyboard).
