# The macOS application

Swift over the same Rust engine, through the UniFFI boundary in
`crates/postio-ffi`. Read `docs/decisions/0019-macos-frontend.md` before
changing the shape of anything here.

## Two build loops, and using the wrong one wastes minutes

**Rust changed** — anything under `crates/`:

```bash
scripts/macos-build.sh --lib-only    # cargo, then regenerate the bindings
```

**Swift changed** — anything under `macos/Sources`:

```bash
cd macos && swift build              # seconds, no cargo at all
```

**Both, or you are not sure:**

```bash
scripts/macos-build.sh               # the whole chain
scripts/macos-bundle.sh              # assemble Postio.app
open macos/build/Postio.app
```

`scripts/macos-test.sh` runs the Swift tests with the library linked.

## The bindings are generated, never edited

`macos/Sources/PostioFFI/` and `macos/Sources/postio_ffiFFI/` are **build
products** and are gitignored. `scripts/ffi-bindgen.sh` writes them from the
Rust crate on every build, using a generator built from this same workspace —
so the generator and the `uniffi` runtime cannot skew. They can: uniffi writes
a per-function checksum into the Swift and verifies it at startup, so a
mismatch is a `fatalError` on launch, a long way from the change that caused it.

Editing them is always wrong. The next build overwrites it.

## What belongs on which side

**Rust**, always: anything that decides something. Which command a key runs,
what a reader document contains, how a list pages, what a body's absence means.
Both frontends share those, and *"shared core does not mean shared behaviour —
anything a frontend interprets will drift"* (ADR 0019).

**Swift**, only: views, and platform observation the platform's own language is
better at. `NWPathMonitor` and `UNUserNotificationCenter` are Swift's, and they
push *down* into the engine through a setter rather than being asked for by it.

If you find yourself writing a rule here, it is on the wrong side.

## Things that will bite

- **A nested `enum State` shadows SwiftUI's `@State`.** The error names
  neither. Call it something else.
- **Opening a session reads the login Keychain.** An unsigned build has a new
  code identity on every rebuild, so macOS asks again each time. Anything that
  can work without a session — the command registry, for one — should.
- **`swift test` needs the library on the linker path.** Use
  `scripts/macos-test.sh` rather than a bare `swift test`.
- **The privacy check reads this directory.** `check-no-silent-tracking.py`
  scans `macos/Sources/**/*.swift` and refuses `URLSession`,
  `NSWorkspace.shared.open` and friends without a `POSTIO-CONSENT:` comment
  saying how the user asked for it. That is not bureaucracy: the reader's whole
  claim is that its web view has no network.
