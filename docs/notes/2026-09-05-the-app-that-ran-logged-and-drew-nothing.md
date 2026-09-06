# The app that ran, logged, and drew nothing (2026-09-05, #1156)

Adding a SwiftUI `Settings` scene to the macOS app cost the **main window**.
Not the settings window — the main one. `Postio` launched, wrote
`postio starting version="0.2.0"` to the log, reached `-[NSApplication run]`,
spun its event loop happily, and created zero windows. No crash, no error, no
warning, nothing in `sample` but a healthy run loop.

## What did it

Not the scene. The scene was innocent, and so was the `@State` store beside
it. It was this, applied to the `WindowGroup`'s root view:

```swift
private struct SettingsOpener: ViewModifier {
    @Environment(\.openSettings) private var openSettings
    let engine: Engine
    func body(content: Content) -> some View {
        content.onAppear { engine.openSettings = { openSettings() } }
    }
}
```

**Reading `\.openSettings` from a view inside the `WindowGroup` prevents that
window group from ever completing its first layout.** The environment value
resolves against the `Settings` scene, and doing that from inside another
scene's content is a cycle SwiftUI does not report — it simply never finishes.

The first hypothesis was wrong and worth recording as wrong: `Engine` is
`@Observable`, so assigning a tracked property from `onAppear` looked like an
invalidate-during-render loop. `@ObservationIgnored` on the property changed
nothing. The cause is the environment read, not the write.

## What to do instead

`NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)`.
It is a stringly-typed selector and it is the right answer anyway: it is what
the menu item SwiftUI installs already sends, so the keyboard path and the
menu path end up at one place instead of two.

Worth knowing, because it removes the reason to reach for `openSettings` at
all: **a `Settings` scene puts "Settings…" in the application menu with `⌘,`
by itself**, correctly placed, without `MenuBar` doing anything. The platform
gives you the native placement ADR 0029 wanted for free.

## Why nothing caught it

The Swift suite is 110 tests and every one of them passed throughout. They
are pure types by design — `MenuPlan`, `PaletteRow`, `SettingsStore` — and
the composition root is in the executable target, which has no tests at all
and cannot easily have any. This is the second time in two days that a bug
lived exactly there (#1146 was a main-actor block in `App.init()`), and both
times the thing that found it was launching the bundle and looking.

## How to look, when the screen is locked

The display was asleep, so `screencapture` returned black and System Events
reported zero windows — that last one is a *lie* on a locked session, not a
finding, and it is the same shape as the skip-that-looks-like-a-pass this
repo already guards against. What works regardless:

```swift
// windows.swift — CGWindowList sees windows a locked session will not enumerate
let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] ?? []
for w in list where (w[kCGWindowOwnerName as String] as? String) == "Postio" { … }
```

And menus *are* readable and clickable when locked, even though windows are
not — so `click menu item "Settings…" of menu 1 of menu bar item "Postio"`
drove the whole path end to end with the machine locked. Check
`CGSSessionScreenIsLocked` in `ioreg -n Root -d1` before trusting a zero.

The bisect that found it was four builds: baseline without the change (window
appears), scene removed (still gone), opener removed (window appears). Each
build is about a minute. Guessing was not cheaper.
