# What only the Mac can tell you (2026-09-21, #15, #668)

A parity pass over the macOS frontend found forty-nine commands that were
drawn in a menu, bound to a key, offered in the palette, and answered by
nobody — plus about twenty defects that a green suite had been sitting beside
for months. Almost none of them were hard. What they had in common is more
useful than any one of them: **they were all in the half of the application
that this project's tooling cannot reach.**

This note is the list of what that half is, so the next session knows where
to look first.

## `Engine` and `Shell` are untestable by construction

`macos/Package.swift` puts `Engine.swift`, `Shell.swift` and `FolderRow.swift`
in the **executable** target, and the only test target is declared against
`PostioKit`. So every decision that ends up in those three files is a decision
with no test, and cannot have one without moving it.

That is where the defects were:

- the sidebar refreshed its folder counts on one event out of five, so reading
  a message did not decrement Inbox;
- `Flagged` and `Snoozed` opened *mailbox zero*, because a view row's id is
  `MailboxId::UNASSIGNED` and the open path asked for `.mailbox(row.id)`
  whatever the row was;
- the key context was never once `Composer`, so every composer binding
  resolved to nothing **and** `a` in a compose window archived mail from the
  list behind it;
- the conversation pane kept drawing the previous thread under a message the
  store had not threaded, which also made the single-message pane beside it
  unreachable after the first conversation opened;
- `⌘W` ended the session and nothing reopened it.

Each fix in that pass has the same shape: the *rule* moved into `PostioKit` as
a small type with tests — `SidebarCounts`, `SidebarScope`, `SidebarRowId`,
`KeyboardContext`, `KeyDisposition`, `Notice`, `SessionLifetime` — and what
stayed in the executable target is one call. That is the only way to have a
test at all, and it is worth doing *before* writing the logic rather than
after finding the bug.

## The key monitor runs ahead of the responder chain

`KeyMonitor` is `NSEvent.addLocalMonitorForEvents`. Whatever it returns `nil`
for never reaches AppKit — not the view under the pointer, not SwiftUI's
`.onKeyPress`, not a menu item's own equivalent.

Two consequences, both of which bit:

1. **A `.onKeyPress` on a key the resolver claims is dead code.** The search
   field's `Escape` handler was never once called; the monitor had already
   taken it. Reading the file gives no hint of this, because the handler looks
   exactly like one that runs.
2. **Claiming a key and acting on it are different things.** The monitor
   swallowed every key the resolver named, so a command this frontend had not
   built yet was *worse* than missing: `space` in the reading pane resolved to
   `scroll_reader_down`, was eaten, reached nothing, and never got to the
   scroll view that would have paged it natively.

`KeyDisposition` is the rule now — only a command something acted on earns the
swallow — and a half-typed sequence is still always swallowed, because `g`
must not also be typed into whatever takes text next.

## Host script runs in a reader with content script off

The reading pane sets `allowsContentJavaScript = false`, and the paging
mechanism needs to move the scroll position — which, with script off, no
WebKit exposes as a call to the host. Both frontends solve it the same way:
the shared document lays down sixty invisible anchors and a page turn is a
same-document fragment jump, performed by **host-evaluated** script.

That works: `allowsContentJavaScript` gates the *page's* own script and leaves
`evaluateJavaScript` alone. It is a claim about WebKit rather than about
Postio, though, and if it were ever wrong the key would go on doing nothing
while every unit test passed — so `ReaderPagingTests` loads a tall document,
pages it, and reads `window.scrollY` back. Assert the scroll position, not the
call.

## `ScenePhase.background` is `⌘W`, not "quitting"

On this platform, closing the window, hiding the application and minimising
all produce `.background`. Ending the session there stops every sync, every
IDLE connection and every notification — the exact gesture a person makes to
leave a mail client *collecting mail*. Quitting is
`applicationWillTerminate`, and that is where an orderly shutdown belongs.

The comment justifying the old placement had expired without anybody
noticing: it cited SQLCipher and libcrypto, and the store is Turso now. A
reason that names a dependency is worth re-reading when the dependency goes.

## Two test binaries share `$TMPDIR`

A session with no store on disk fell back to `$TMPDIR/postio-drafts-out` for
editor hand-off, and a hand-off file is named for the draft id — which starts
at `1` in every fresh store. Two sessions handing out their first draft wrote
to the same file.

In the suite that is two binaries racing, so the case passed alone, passed in
its own crate, and failed when `postio-session` happened to run beside it: a
red that reads as noise. It was not noise, and it was not only a test problem
— that fallback is what a Postio with no store yet uses.

Any shared path under `$TMPDIR` wants the process id **and** a per-session
counter, not one or the other.

## A tag of the wrong type is a row `List` can never select

SwiftUI's `.tag()` is generic and compiles against anything. When the
sidebar's selection became a `SidebarRowId` — three rows are queries with no
id of their own — the folder rows went on tagging with `Int64`, and nothing
anywhere said so. Selection simply stopped working for those rows.

There is no compiler help here. When the selection type changes, grep for
every `.tag(`.

## What to reach for

`crates/postio-ffi/tests/ffi_suite/command_coverage.rs` is the sweep that
found the forty-nine. It is the macOS counterpart of
`postio-app`'s `app_suite/command_wiring.rs`, whose own orphan list is empty
because it has existed long enough to empty it. Its `KNOWN_ORPHANS` list is
debt and may only shrink: a command that gains a handler and stays listed
fails the same way a new orphan does.

If you are about to build a surface on macOS, read that list first. It is the
honest answer to "how far along is this".
