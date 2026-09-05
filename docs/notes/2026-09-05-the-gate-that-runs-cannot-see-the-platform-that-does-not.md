# The gate that runs cannot see the platform that does not (2026-09-05, #656/#1146)

Two defects were sitting in `feature/macos`. Both were reachable from `main`,
both had passing tests over them, and neither could be found by anything the
project ran. They failed in different layers and they are the same bug.

## One: every `mod+…` binding was dead on macOS

`postio_config::keys::expand_mod` resolves the `mod` token when the keymap is
built -- `ctrl` on freedesktop, `cmd` on Apple (#669). `MODIFIERS` accepts
`cmd` and `command`, so `chord_problem` validates them and a hand-written
`[keys]` entry passes.

`postio_ui::keymap::Modifiers::parse` knew `ctrl`, `control`, `alt`, `shift`,
`super` and `meta`. Not `cmd`.

So on macOS the expansion produced a spelling the only thing that matches
chords could not read, and **thirty-one default bindings were unparseable at
once** -- the command palette, settings, select-all, send, every composer
verb. Not silently: `Resolver::from_commands` reported each one as a keymap
problem. But a problem list is not where anyone looks when a key does nothing,
and it is a long way from the layer that produced the spelling.

Every layer passed its own tests. `postio-config` tested that `mod` expands to
`cmd` on Apple. `postio-ui` tested that `ctrl+k` parses. Nothing tested that
the output of the first is an input the second accepts, because on the
platform the gate runs on, `expand_mod` writes `ctrl`.

## Two: the application drew no window

`Session::open`'s doc comment says, in as many words, that it blocks on the OS
keyring and *"belongs in a launch task and never on the main actor."*
`Engine.init` called it on the main actor, and `@State private var engine =
Engine()` made that happen inside SwiftUI's `App.init()` -- earlier than any
scene exists.

`sample Postio` named it in one stack: the main thread parked in
`store_key_blocking`, under `NSApplication` not yet running. The application
appeared in the Dock and drew nothing.

What made it more than a slow start is what the block waits *for*. macOS
raises a Keychain prompt in front of the asking application's window, and there
was no window -- so the machine asked a question about an application that was
not on screen to be asked about, and the application could not draw itself
until the question was answered. An ad-hoc-signed build gets a new code
identity on every rebuild (`macos/CLAUDE.md`), so this was the **normal** path
after any build.

## What they have in common

ADR 0019 Q7 asks how Linux stays green and answers it well: `postio-ffi` has
no macOS-specific code, so `cargo test -p postio-ffi` proves the seam on a
cheap runner, and `issue-land.sh` refuses to land a crate the host cannot
build. Both mechanisms work. Neither addresses the other direction.

A shared layer's contract has two ends, and a gate that only ever runs one
platform sees only one of them. The `cmd` bug is that exactly: two correct
functions whose *composition* is only ever evaluated on the platform nobody
tests. The main-actor bug is the frontend version -- a rule written down in the
place that must not be violated, violated by the only caller, on the only
platform that has one.

## What to do about it

**Assert the composition, on both platforms, from either host.** The fix for
the first was not teaching `Modifiers::parse` one more word. It was this, in
`postio-ui`'s suite:

```rust
for platform in [Platform::Freedesktop, Platform::Apple] {
    let keymap = postio_core::Keymap::resolve_on(&Default::default(), platform);
    let (_, problems) = Resolver::from_commands(&keymap);
    assert!(problems.is_empty(), "{platform:?} could not resolve its own defaults");
}
```

A Linux runner fails that the day a platform-conditional spelling stops
round-tripping. It is the same discipline `postio-config`'s path resolution
already uses and says why: *"taking the platform as a parameter rather than
reading a `cfg` is what lets either host assert both answers."* Anywhere a
`Platform` is threaded through, both values belong in the test.

**A `cfg!` in an assertion is not the same thing.** `ffi_suite/keys.rs` has one
-- it asserts the *host's* primary modifier and reads the same on both -- and
it is worth having, but it only ever checks the platform it is compiled for.
The both-platforms loop is what a Linux gate can fail on.

**And run the application.** The second bug survived a suite of 97 passing
Swift tests, because nothing in it constructs an `Engine` -- everything
assertable was deliberately factored into pure types (`MenuPlan`,
`Announcements`, `PaletteRow`, `KeyEvent`) so it could be tested without a
session. That factoring is right and it has a blind spot exactly the size of
the composition root, which is the same thing `postio-app`'s `app_suite`
exists for and the macOS side does not have yet. Until it does, launching the
bundle and reading `sample` is the check -- and `sample` is very good at this:
one command named the blocked call, its caller, and the SwiftUI entry point
that made it happen.
