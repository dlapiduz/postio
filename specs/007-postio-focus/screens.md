# Screens: each build compared with its PNG

Spec FR-095 and SC-009. Each screen from 01 to 20 is rendered from the demo
store (`cargo run -p postio-focus --example shot -- <png> <screen>`). The
image is read back beside its reference in the maintainer's
`Design/postio-focus-design/screens/`, in light and dark. Every difference is
written down here with its reason. The references are never committed; they
carry a real name until they are re-rendered.

The differences already known before any screen was built come from the
spec's table *Where the inputs disagree* (C1–C23), and they are filled in
below. A phase that builds a screen adds what its comparison finds, and sets
the date.

| Screen | Compared on | Differences | Reason |
|---|---|---|---|
| 01 Inbox, light | not yet | Digest row's first line shows its senders until summaries exist (C5). Task buttons and "Task in … · due …" appear only with Obsidian (C9). Header counts appear only once their features do (C10). Shifted keys drawn as the shared hint code draws them (C22) | Milestone order; the one keymap's hint rules |
| 02 Inbox, dark | not yet | As 01 | As 01 |
| 03 Has-action filter | not yet | As 01 (C9, C22) | As 01 |
| 04 Open email | not yet | The body is rendered HTML, not the plain-text part (C1). The header and marker cards stay while the body scrolls (research R2). Task and Note buttons wait for Obsidian (C9) | Handoff; one reused view; milestone order |
| 05 Compose | not yet | No Markdown toggle (C7). Suggestions open at four characters (C23). Cc and Bcc share one command. "Task after sending" waits for Obsidian (C9) | The existing composer; one completion rule |
| 06 Reply all | not yet | As 05 (C7, C9) | As 05 |
| 07 Search, plain English | not yet | "invoice" stays free text rather than `subject:` | Deterministic lowering (research R5) |
| 08 Go to a folder | not yet | none known | |
| 09 Commands | not yet | "Archive everything read, older than a week" has a key (C11) | Constitution II |
| 10 Folders and labels | not yet | none known | |
| 11 Snooze picker | not yet | Preset wording is shared with the classic app (C14). "Comes back at the top" holds only if `sort_at` lands (T012, T093) | ADR 0029; research R7 |
| 12 Remind picker | not yet | none known | |
| 13 Label picker | not yet | none known | |
| 14 Move picker | not yet | none known | |
| 15 Undo toast | not yet | Shifted keys (C22) | Hint rules |
| 16 Empty inbox | not yet | Lists only what exists (C10) | FR-070 |
| 17 First sync | not yet | none known | |
| 18 Offline | not yet | none known | |
| 19 Sign-in error | not yet | none known | |
| 20 Key map | not yet | The footer names `[keys]` in `config.toml` (C3). No Obsidian group before milestone 3 (C9). Shifted keys (C22) | Constitution II; milestone order |
