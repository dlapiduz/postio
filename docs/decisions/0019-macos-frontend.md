# ADR 0019 — A native macOS frontend over `postio-session`

- **Status:** Accepted — maintainer-directed (2026-08-27)
- **Date:** 2026-08-27
- **Issue:** [#557](https://github.com/dlapiduz/postio/issues/557), under
  [#15](https://github.com/dlapiduz/postio/issues/15) and epic
  [#23](https://github.com/dlapiduz/postio/issues/23)
- **Related:** ADR 0010 (the MCP surface, which is why `postio-session`
  exists), ADR 0013 (event fan-out by subscriber name), ADR 0004 (the
  composer's document is Postio's own), ADR 0003 (the hardened WebView),
  ADR 0014 (the store's key comes from the OS keyring), ADR 0006 Q3 (the
  consent screen opens in the user's browser), `docs/ARCHITECTURE.md` §9
  (the two enforced crate boundaries)
- **Decision:** Postio grows a **native Swift (SwiftUI + AppKit) frontend**
  over the same Rust engine, through a **UniFFI boundary in a new
  `postio-ffi`**, with the toolkit-free presentation logic currently
  trapped in `postio-gtk` extracted into a new shared **`postio-ui`**.
  The first shipping slice is **read-only** — sign in, sync, three-pane
  shell, list, reader, search, keyboard — with compose deferred. The
  architectural model is [Ghostty](https://github.com/ghostty-org/ghostty),
  which solves the same problem and has lived with the consequences.

---

## Why now

`docs/PRODUCT.md` §2 has said since the beginning that a macOS frontend
"remains possible, and that possibility is the reason for two CI-enforced
boundaries rather than an aspiration in a document." The boundaries have been
paid for on every commit since. This calls in the debt.

Nothing about the product changes. The three things Postio must beat the
alternatives at — speed, search, keyboard — are properties of the engine, and
the engine is what gets reused. What a second frontend risks is the fourth
principle, **Native**: *"a real desktop application, not a website in a
window."* A GTK application on macOS would satisfy the letter of the port and
none of that, which is Q1.

## What we measured first, and what it corrected

The plan for this work assumed the workspace did not compile on macOS —
`tracing-journald` needing `memfd_create`, `oo7` needing the Secret Service,
`zbus` needing a system bus — and budgeted a phase for making them
target-conditional. **That was an inference nobody had compiled.** Measured on
`aarch64-apple-darwin`, rustc 1.98.0:

```
cargo build -p postio-session -p postio-runtime -p postio-imap   exit 0
cargo test  -p postio-session -p postio-runtime -p postio-imap   exit 0   500+ tests
cargo check --workspace --all-targets                            exit 101
    error: failed to run custom build command for `glib-sys v0.21.5`
cargo check --workspace --all-targets \
    --exclude postio-gtk --exclude postio-app                    exit 0
```

**Thirteen of the fifteen crates build and test on macOS today with no changes
at all.** `tracing-journald`, `oo7` and `zbus` all compile and link there; they
fail at *runtime*, gracefully, which they were already written to do. The only
boundary is `glib-2.0` via `pkg-config`, and it falls exactly on `postio-gtk`
and `postio-app`.

Two consequences. The port has no porting phase — it begins at the extraction
and the boundary. And the enforced crate boundaries turn out to have done
precisely what they were kept for: the engine was portable before anyone tried.

Issues #553 and #554 were closed as invalid on this evidence.

## Q1 — Native Swift, or GTK4 on macOS?

**Native Swift.**

GTK4 and libadwaita have arm64 Homebrew bottles, so a GTK-on-macOS build is
not obviously absurd. **`webkitgtk` has none** — a 41-dependency source build,
and macOS is not a supported upstream configuration for the GTK port of
WebKit. Both surfaces that matter are WebKit views: the reader
(`postio-gtk/src/reader/view.rs`) and the composer (`editor.rs`). The shortcut
breaks precisely where the application lives, so the saving is illusory: the
two hardest views would need rewriting anyway, on top of an app that has the
wrong menu bar, the wrong shortcuts and the wrong window chrome.

Rejected for the same reason as a web view in a window, and by the same
principle.

## Q2 — What shape is the seam?

**Two tiers: a small required floor, and a large optional surface.**

Ghostty's `src/apprt/` is the reference. Its required set — the calls made
directly on the runtime, where a missing one is a compile error — is **sixteen
functions**, and its do-nothing frontend (`apprt/none.zig`) is nineteen lines.
Everything ambitious is an *optional, one-way action* in a union whose handler
returns `bool` meaning "did you handle it", so a frontend that implements none
of them still compiles and runs.

Postio adopts both halves. `postio-ffi` exposes a floor of about fifteen
functions, and a `UiEvent` surface that is append-only, ignorable variant by
variant, with the rule for adding one written at the definition site rather
than in a wiki. The Swift dispatcher ends in a `default:` that logs and returns
false, so an app built against an older boundary degrades to a log line instead
of a crash.

The point is not elegance. It is that the macOS app can be *running and
useless* early, and grow — rather than being unrunnable until it is complete.

## Q3 — What crosses the boundary?

**Commands cross as name strings; the registry is the vocabulary.**

`CommandId` already serialises stably — `[keys]` in `config.toml` is a file
format built on that. So Swift calls `commands()` once and derives its palette,
cheat sheet, menu bar and key hints from the registry, exactly as the GTK side
does. A new command reaches the macOS UI with **no boundary change and no Swift
change**, and *"a command that is not in the registry does not exist"*
(`PRODUCT.md` §8) stays true on both platforms. Mirroring the `Command` enum
into Swift would have created a second vocabulary, and a second vocabulary
drifts.

**The list is pull, not push, and never materialised.** `NSTableView` asks for
a row count and then for rows, synchronously, in microseconds. So the boundary
offers `row_count()`, `request_page(generation, page)` and a synchronous
`take_page()` that reads a bounded resident cache and does no I/O. A miss
returns a placeholder and asks; `UiEvent::PageReady` reloads exactly that range.
*"A mailbox is never loaded into memory"* holds on both sides of the FFI, and
there is deliberately no API that would let `await` appear inside a table
delegate.

**The reader receives a finished document.** `reader_document()` returns the
whole thing — CSP, embedded font faces, tokens, sanitised body, scroll markers
— built by the same Rust function the GTK reader calls. Swift never composes
reader HTML. This is what makes Q6 provable rather than aspirational.

**UniFFI rather than a hand-written C ABI.** Ghostty hand-rolled because
Zig↔Swift forced it, and paid for it with a `wakeup` callback plus a tick
function to fake async. Rust does not have that problem: `async fn next_event()`
becomes Swift `async`, so the drain loop is
`Task { @MainActor in while let e = await session.nextEvent() { apply(e) } }` —
the same shape as the GTK side's `glib::spawn_future_local`. UniFFI also keeps
`Result<_, SessionError>` as distinguishable Swift `throws` cases, which
ADR 0014 needs: `SecretError::Locked` must reach the surface that asks the user
to unlock, not onboarding. A C ABI flattens that to an int and a string, and
the routing decision degrades to matching on message text.

**And it costs the workspace no `unsafe`.** The plan expected UniFFI's
generated scaffolding to force `postio-ffi` into `check-lint-floor.py`'s
exception list at `deny`, the way `postio-gtk`, `postio-app` and `postio-imap`
sit there. Tested on uniffi 0.29.5 rather than assumed: the crate compiles with
`uniffi::setup_scaffolding!()` and `#[uniffi::export]` under the workspace's
`unsafe_code = "forbid"`, and `forbid` is genuinely in force — a hand-written
`unsafe` block in the same crate is rejected. **No exception is needed, and the
invariant is not narrowed.**

`postio-ffi` is **private to the macOS app and promises no stability.** Ghostty
says the same of `include/ghostty.h` in its own header, and their genuinely
public library turned out to be a separate artifact. A frontend seam that
accretes general-purpose API obligations stops being able to change.

## Q4 — Who owns the keyboard?

**The core does. Swift owns no keymap.**

This replaces what the plan originally proposed — a `primary+key` spelling in
`[keys]` resolving to Control on freedesktop and Command on Apple — which would
have made the config file mean different things on different platforms.

Ghostty's arrangement is better and costs less. The core owns the keycode
table, the modifier semantics, the binding trie, sequences and leaders. Each
frontend does three small things: send a reduced key event in; ask
`trigger_for_command(id)` when it needs to draw a native accelerator; and own a
small renderer from that trigger into its platform's accelerator format —
`<Ctrl>N` for GTK, `⌘N` for `NSMenuItem`.

So `[keys]` and `docs/keybindings.md` do not change at all, and there is one
binding table rather than two that agree until they do not.

Dispatch on macOS is a window-level `NSEvent` monitor, not SwiftUI's
`.keyboardShortcut`. The latter is a menu-accelerator model: it cannot express
`gg` sequences, cannot express a context-dependent `Esc`, and cannot express
"typing always wins". Menu items take their accelerators from
`trigger_for_command` but are given no key equivalent, so they never race the
monitor.

## Q5 — What is shared, and what is rewritten?

A new **`postio-ui`** takes the toolkit-free logic currently inside
`postio-gtk`: `selection.rs` (352 lines, no toolkit references at all),
`tokens.rs` (965), `palette.rs` (461), `keymap.rs` (1,471, whose only real
toolkit reference is one constructor), the list's paging and generation
bookkeeping, and — most importantly — the reader's document assembly.

Its boundary rule lands in `check-crate-boundaries.py` in the same commit that
creates it, so *dependency* leakage is impossible from the first day. Ghostty's
experience says the other kind is not: their shared action union still carries
a `show_gtk_inspector` variant, and pulling logic back out of frontends is an
ongoing effort rather than a finished one. Expect maintenance, not a cleanup.

What is genuinely rewritten is `postio-gtk`'s 34k lines of widgets and
`postio-app`'s 8.5k of glue. There is no way around that and no attempt to
pretend otherwise.

**`postio-app` is not made cross-platform.** Nothing depends on it; it is the
GTK binary, and it names `gtk4`, `libadwaita` and `postio-gtk` directly. It
stays Linux-only, and `postio-ffi` sits on `postio-session` instead.

## Q6 — How do the privacy invariants survive two frontends?

**By being one implementation, not two that agree.**

This is the highest risk in the whole undertaking: two readers, two content
security policies, two link policies, and the drift is invisible until somebody's
mail phones home. The structural answer is that the CSP string, the document
wrapper, the `@font-face` data URIs, the scroll markers and the absent-state
HTML all move into `postio-ui`, and both frontends call them. The CSP is
asserted byte-for-byte, for blocked and allowed remote images, in `postio-ui`'s
own tests.

Swift's entire responsibility for the reader is: build a hardened
`WKWebViewConfiguration`, hand it a string, and refuse navigations. Every
setting `hardened_settings()` applies has a WebKit counterpart, with one honest
exception recorded here rather than left to be assumed: **WebKit exposes no
public toggles for WebRTC, WebGL or WebAudio.** JavaScript is off and
`default-src 'none'` closes the rest, so the effect is the same while the
mechanism is weaker. That belongs in a comment at the call site too.

The proof is a test, not a review: a loopback listener bound in-process, its
URL placed in a message body as a remote image, asserting **zero accepted
connections** with remote images blocked and exactly one when allowed. It is
the only thing that demonstrates the CSP does what its comment claims.

Link clicks leave the pane and open the user's browser, as they do on GTK —
and both call sites carry the `POSTIO-CONSENT:` marker
`check-no-silent-tracking.py` looks for. That check must also learn to scan
`macos/Sources/**/*.swift`; without it the entire Swift half sits outside the
privacy guard, which is the worst available outcome and lands with the first
Swift file.

## Q7 — How does Linux stay green?

Two mechanisms, because the exposure runs both ways.

**`postio-ffi` builds and tests on Linux.** It contains no macOS-specific code,
so `cargo test -p postio-ffi` runs in the ordinary gate. This is Ghostty's
highest-leverage CI job — `zig build -Dapp-runtime=none test` compiles the
macOS shim on a cheap Linux runner — and Postio gets it for the price of
remembering to wire it in. A Linux session cannot silently break the macOS seam.

**`issue-land.sh` refuses to land a crate the host cannot build** (#555,
already landed). It probes `pkg-config`, not `uname`, so a Linux box without
the `-dev` packages is caught identically. A changed crate the host cannot
build is a hard stop; a changed crate the unbuildable ones depend on lands with
`needs-linux-verify` on the PR. The doctrine, which is the existing display
rule aimed at the other axis: **a crate the host never compiled is not a crate
that passed.**

Queue separation is the third leg: macOS issues carry `ready-mac` and not
`ready`, so an ordinary claim skips them by construction (#552).

## Q8 — How is it built and shipped?

`macos/` at the repository root, beside `crates/` and `flatpak/`. **SwiftPM,
not a checked-in `.xcodeproj`** — this is the one place we deviate from
Ghostty, and deliberately: their `.pbxproj` is maintained by humans, whereas
this repository is script-driven and largely agent-written, and a merge-hostile
XML blob is the worst possible artifact for that. The packaging *is* theirs:
cargo → staticlib → `.xcframework` consumed as a SwiftPM binary target, with a
checked-in module map and the framework gitignored as a build product.

Distribution is **unsigned, from source**, with ad-hoc signing and a separate
entitlements file carrying `com.apple.security.cs.disable-library-validation` —
Ghostty's `ReleaseLocal` configuration, which exists precisely so a contributor
needs no Apple account. Two entitlement files from the start, so adding a
Developer ID later is a flag rather than a restructure.

Two build loops, written into `macos/CLAUDE.md`: Rust changed → rebuild the
library; Swift changed → `swift build`, seconds, no cargo. Ghostty documents
exactly this and contributors still rebuild the world without it.

## Consequences

**Good.** The engine is proven portable by measurement rather than by
assertion. `postio-ui` makes the GTK frontend smaller and its presentation
logic testable without a display — a win on Linux whether or not the macOS app
ever ships. The privacy invariants get *stronger*, because moving them into
shared code turns "two implementations agree" into "one implementation".

**Costs.** Roughly 43k lines of frontend are rewritten, not ported. The
extraction touches `postio-gtk`, the most-edited crate in the repository, and
**cannot be verified on a Mac** — it needs a Linux host in the loop, which is
an open question rather than a solved one. Leakage into `postio-ui` will be
permanent maintenance. And shared core does not mean shared behaviour: anything
a frontend *interprets* will drift, which is why Q4, Q6 and the list windowing
push interpretation down into Rust wherever it is cheap.

**The risk nobody can design away.** Ghostty's Windows frontend has stalled
three times — PRs #10857, #11660, #12403, all closed unmerged — *despite* the
seam existing and compiling in CI, and their glfw frontend was deleted
outright. **The abstraction makes starting cheap; it does nothing for
finishing.** The mitigation is sequencing rather than optimism: every piece of
this that lands before the first Swift file — `postio-ui`, the boundary, the
land-script guard, the two workflow fixes — is independently valuable to the
Linux application. If the initiative stalls, the repository is better off
rather than half-migrated. That is the only insurance available.

## Amendment to ADR 0006 Q3

ADR 0006 rejects a custom URI scheme for the OAuth redirect because it "needs a
desktop-file registration and hands the callback to whatever else claimed the
scheme". That reasoning is freedesktop-specific: on macOS, `CFBundleURLTypes`
in `Info.plist` registers a scheme with no user action.

The decision does not change — **the loopback redirect stays**, on both
platforms, because one flow with one set of tests is worth more than a
per-platform optimisation. But the *reasoning* is now recorded as
platform-scoped, so a future reader does not mistake a freedesktop constraint
for a universal one.
