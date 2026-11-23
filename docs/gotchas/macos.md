# Gotchas: macOS and the FFI

Read before touching `crates/postio-ffi/src/`, `macos/` or anything platform-conditional. `macos/CLAUDE.md` has the build loops.

- **A change under `crates/postio-ffi/src/` has Swift callers Linux cannot compile.** Grep `macos/` for every boundary type and field the diff changed.
- **Pass a platform difference as a `Platform` parameter, not `#[cfg]`.** Assert both values from either host, including cross-crate compositions (what `expand_mod` writes must parse in `Modifiers::parse`). `cfg!` in an assertion checks only the host.
- **`Engine.swift`, `Shell.swift` and `FolderRow.swift` are in the untested executable target.** Put the rule in `PostioKit` as a small type, leave one call, then launch the bundle; `sample` names a blocked main thread.
- **`KeyMonitor` sees keys before the responder chain.** `.onKeyPress` on a resolver-claimed key never runs. Swallow only keys something acted on.
- **Never read `\.openSettings` inside the `WindowGroup`.** The window never finishes first layout and the app draws nothing. Open Settings with `openWindow(id:)`.
- **`ScenePhase.background` means the window closed or hid, not quit.** Shut down in `applicationWillTerminate`.
- **`LazyVStack` pools platform views.** A `WKWebView` inside it frees only when the stack's identity changes; capture platform views weakly in work that can outlive them.
- **A `@MainActor` test suspends with `Task.sleep`, never by spinning `RunLoop.main`.** Spinning starves main-actor work and looks like a leak.
- **SwiftUI `.tag()` compiles against any type, and a nearly-matching `WKNavigationDelegate` method is never called.** Grep every `.tag(` when a selection type changes; use the `async` delegate form and treat Swift 6's near-match warning as an error.
- **A macOS test reaching the platform keyring prompts and hangs.** Give it a `MemorySecretStore`. If every test says `ok` but no summary prints, run `sample <pid>`.
- **`ReaderEgressTests.swift` opens a loopback listener on purpose.** It alone fails if the reader starts fetching. Keep it; create no `NSWindow` in a test.
- **Moving code out of a crate this host cannot compile leaves stale imports.** Split each moved symbol's remaining uses at the last `mod tests`. Rustdoc cannot link up from a dependency to its dependent; name it in prose.
