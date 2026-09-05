# Cross-platform dependencies and what a Linux box can prove (2026-08-28, #642)

`main` spent a day unbuildable on Linux because a macOS dependency section
swallowed fifteen entries of `postio-account`'s `[dependencies]` (#642). The
diff looked tidy — `security-framework` sorts between
`rustls-platform-verifier` and `secrecy`, so it read as an alphabetical
insert — and a TOML table runs until the next header, so everything below it
became macOS-only. On Linux the crate lost `postio-model`, `tokio`, `serde`
and twelve more, and produced 219 errors.

Nothing caught it because nothing built it: CI is `workflow_dispatch`-only and
the reconcile pass had not run since it landed.

Postio is one workspace targeting Linux and macOS (ADR 0019), so this class
recurs by construction. **A Linux box cannot build or test the macOS half**,
which is true and is also where the reasoning usually stops. It can do rather
more than nothing. Three layers, cheapest first:

### 1. Placement, enforced (every machine, instant)

`check-target-sections-last.py`: a `[target.'cfg(...)'...]` table must come
after every plain `[dependencies]`, `[build-dependencies]` and
`[dev-dependencies]` table. Platform sections live at the foot of the
manifest.

This is a placement rule, not a correctness proof, and the distinction is
worth keeping straight: TOML has no notion of a table somebody *meant* to keep
going, so the swallowing is not detectable. The position that makes it
possible is. **A platform section at the foot of the file cannot swallow
anything, because there is nothing below it to swallow.**

### 2. Cross type-checking, as far as the C dependencies allow

`scripts/cross-check.sh [triple]` runs `cargo check --target` over every
workspace member. Measured on this workstation against
`aarch64-apple-darwin`:

| | |
|---|---|
| **6 checked** | postio-model, postio-config, postio-core, postio-body, postio-search, postio-ui |
| **12 skipped** | everything else |

Every skip is a **C build script** wanting a cross-toolchain this machine has
not got — `ring` (via rustls), `zstd-sys` and `openssl-sys` (postio-storage),
the GTK sys crates. Never Rust. The script reports `skipped` separately from
`FAILED` for exactly that reason: a crate whose C dependency would not build
taught us nothing about its Rust, and saying "ok" there would be a lie.

The six are not a consolation prize. `postio-config` is where Apple's
directory layout lives, and `postio-ui` is ADR 0019's shared frontend logic —
the two crates most likely to carry macOS-only code that a Linux build never
compiles. Verified by planting `#[cfg(target_os = "macos")]` code that calls a
function that does not exist: `cargo check -p postio-config` reports **0
errors**, and `cross-check.sh` reports `FAILED postio-config` with the missing
function named.

Setup, once — it is a large download and deliberately not in `mise.toml`:

```sh
rustup target add --toolchain "$(rustup show active-toolchain | cut -d' ' -f1)" \
  aarch64-apple-darwin
```

Not wired into `check.sh`: it compiles a second copy of the dependency graph,
which is minutes on a cold target directory, and `check.sh` runs on every land
across every session. It belongs in CI and in the reconcile pass.

**The skipped twelve could shrink.** `cargo-zigbuild` supplies a cross
compiler that can build C for Apple targets, which would bring `ring`,
`zstd-sys` and `openssl-sys` into reach for `cargo check`. Not tried; worth it
if the macOS port grows and this layer starts feeling thin.

### 3. A macOS runner, for everything else

Linking, the Apple frameworks, `security-framework` actually resolving, the
Swift half, and **running any test at all**. There is no substitute and no
approximation. Whatever CI eventually looks like, a macOS job is what the
other twelve crates get.

### The rule of thumb

Each layer catches a strictly cheaper class than the one below it, and the top
two run on any developer's machine. When adding a platform-conditional
anything, the question is not "can I test this here" — usually no — but "which
of these three is the cheapest thing that would have caught me getting it
wrong". For #642 it was the first, and it costs milliseconds.
