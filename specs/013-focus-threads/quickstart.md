# Quickstart: validating threads in Focus

How to prove the spec's acceptance scenarios on both platforms, and where
each proof lives. What is promised is in [contracts/](contracts); the types
are in [data-model.md](data-model.md).

## Prerequisites

- A worktree of `feature/focus-threads`, seeded (`scripts/worktree-seed.sh`).
- The demo seed `threads` (research R14). It is in memory, invented, and
  uses reserved domains only.
- The design pack in the maintainer's checkout:
  `~/src/postio/Design/focus-macos-threads/screens/01`–`11`. Read it there
  and never copy it in.
- For Linux: CI's `Frontends` check, and for screens the Linux box (at most
  two agents at a time; nothing else running a full suite).

## Automated

| What | Command | Proves |
|---|---|---|
| Grouping rules | `cargo nextest run -p postio-model threading` | R2/R3: subject window, list mail, split joins without merging (SC-005) |
| Threading corpus | `cargo nextest run -p postio-storage --test storage_suite threading` | Replies, forks, cross-folder, subject reuse, lists, digests over `.eml` fixtures |
| Scope table | `cargo test -p postio-ui --lib scope` | Every cell of D2 (SC-004) |
| Words | `cargo test -p postio-ui --lib thread_bar thread_list focus_row` | Participants, "2 new", chips that fit, kicker, banners |
| Budgets | `cargo nextest run -p postio-storage --test storage_suite thread_budget` | `Thread` is constant in statements at 2 and 500 members (SC-002) |
| Controller | `cargo nextest run -p postio-focus --test controller_suite threads` | Expand/collapse, cursor through children, walk keys, new reply never moves the reader (SC-006), outline, search children, ⌘G across messages |
| Forward bodies | `cargo test -p postio-body --lib forward_thread` | Oldest first, headers, attachments once; not-local refused |
| FFI | `cargo nextest run -p postio-ffi --test ffi_suite focus_threads` | The demo's views as Swift receives them |
| GTK | `cargo nextest run -p postio-gtk --test focus_suite threads` (CI or Linux box) | One case per path a person takes, real keys in the real window |
| Swift | `scripts/macos-test.sh` | Expansion focus, window keyboard paths, chooser keys |
| Storyboards | `scripts/storyboards.sh storyboards/threads` | Each keyboard path, red on `main` first |

## By hand: the issue's reproduction (SC-008)

On the Mac:

```sh
scripts/macos-build.sh && scripts/macos-bundle.sh
POSTIO_DEMO=threads open macos/build/Postio.app
```

On Linux:

```sh
POSTIO_DEMO=threads cargo run -p postio-gtk
```

1. Find "Harbor API draft v3" and press `Right`. Six children show their own
   text, and the draft is last. **#2099 reproduces no longer.**
2. Press `j` until "Ben Adeyemi ↳ to Ana" is focused. The footer reads
   "Message 4 of 6 · Ben Adeyemi".
3. Press `Return`. The window shows message 4, with the older-message banner
   and "Reply to: Ana Ruiz … (message 1)".
4. Press `]`, then `}`, then `g u`, then `[`. Each moves inside the thread;
   `j` moves to the next thread.
5. Press `alt+t`, type `Ben`, press `Return`. The window shows Ben's message.
6. Press `f`, then `2`. The subject reads "Fwd: Harbor API draft v3 (6
   messages)".
7. With the demo's live-reply key, deliver a reply. The banner appears and
   nothing else moves.
8. Search `rate limit headers`, press `j`, `Return`, `mod+g` twice, then
   `Escape`.

## Screens

| Screen | Mac | Linux |
|---|---|---|
| 01–11 at 1440×900 | `scripts/macos-shot.sh` with `POSTIO_DEMO_KEYS` per screen | `scripts/run-isolated.sh <commit> --shot` on the Linux box |
| 02 dark | `scripts/macos-shot.sh --dark` | dark style |

For each screen, list in the commit body or PR what differs from the PNG
and why (FR-027). The PNGs carry no real data, but stay in `Design/` and
never enter the repo.

## Review

Before landing, every interaction change goes through `/ux-review` over
`storyboards/threads/`.
