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

## Getting mail into a fresh build

A newly built Postio.app opens an empty store, because **there is no way to
configure an account from the macOS UI yet.** Onboarding — autodiscovery, the
preset table, credential capture, the OAuth flow — is GTK top to bottom, and a
Mac has no Linux Postio to have set things up with. #649 decided the first
slice is a headless helper, with native onboarding deferred behind it.

The helper makes the same two writes the onboarding screen makes: an account
row in the encrypted store, and a password in the login Keychain.

```bash
export POSTIO_ADDRESS='you@your-provider.example'
read -rs POSTIO_APP_PASSWORD && export POSTIO_APP_PASSWORD   # not echoed, not in history
cargo run -p postio-session --bin postio-provision
```

It prints the servers it resolved before it writes anything, so a mistyped
host is visible then rather than as a failed sync later. Then open the app and
it syncs on launch.

**Do not put the password in a file, and do not pass it on the command line.**
`argv` is readable by every process on the machine through `ps`. The helper
reads exactly one variable and hands it to the Keychain; it is never printed,
never logged, and there is no field on the account row that could hold it.

**Not `postio-app`.** The helper lives in `postio-session` because
`postio-app` links GTK, which is precisely the crate a Mac cannot compile
(ADR 0019). Its ancestor was a `postio-app` example, so the only platform
without an onboarding screen was the only platform that could not run the
stand-in for one.

### What to expect from the Keychain

Two prompts, not one, and possibly more on later runs. The helper and the app
are separate binaries with separate code identities, and a Keychain item's ACL
is bound to whatever created it — so granting access to the helper says nothing
about the app. An unsigned build's identity also changes on every rebuild, for
the reason under *Things that will bite* below, which is why "Always Allow"
stops sticking as soon as you rebuild.

### When it will not resolve the servers

The provider preset table is consulted by domain, and it is the same table the
onboarding screen reads rather than a second copy — a provider added for the
screen is available here on the same commit (#69 is what two copies cost). A
domain the table does not publish settings for is **refused rather than
guessed**: `imap.<your-domain>` resolves for a great many hosts that are not
your mail server, and pointing an account at one of those means typing a
password into somebody else's machine. Give the servers instead:

```bash
export POSTIO_IMAP_HOST='imap.example.com'
export POSTIO_SMTP_HOST='smtp.example.com'
export POSTIO_USERNAME='...'      # only if the login is not the address
```

`POSTIO_IMAP_PORT` and `POSTIO_SMTP_PORT` override a preset's ports the same
way, field by field — overriding one setting keeps the rest of the row.

An iCloud custom domain wants `imap.mail.me.com` and `smtp.mail.me.com`, with
`POSTIO_USERNAME` set to the Apple ID address rather than the custom one. And
iCloud needs an **app-specific password** — an Apple ID password will not
authenticate. Create one at appleid.apple.com under Sign-In and Security, with
two-factor authentication on, and revoke it there when you are done testing;
that takes effect immediately.

### Re-running it

Safe, and deliberately inert: an address already in the store is reported and
left alone. It will not write a second row for one address, and it will not
overwrite a password that is already working — a re-run from a shell whose
environment had drifted would otherwise break an account that was syncing
perfectly well. Repairing an account is onboarding's job, where there is a
person to confirm it.

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
