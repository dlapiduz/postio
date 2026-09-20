# The Flatpak build is not this workstation, and `.cargo/config.toml` follows it in

*2026-09-19*

The v0.4.1 release reached `flatpak-builder` and died on its first build
script:

```
Running: cargo --offline build --release --package postio-app --bin postio
   Compiling proc-macro2 v1.0.107
   Compiling quote v1.0.47
error: linker `postio-linker` not found
  = note: No such file or directory (os error 2)
```

`postio-linker` is the shim from #1101. The linker and `CC` are bare names
rather than paths precisely so that a worktree path never reaches rustc's
argument list — that was what dropped the sccache hit rate to 1% — and
`scripts/install-shims.sh` puts them on `PATH`. The claim, land and test
scripts run it. **The Flatpak build runs none of those scripts, and is not on
this machine's `PATH`.**

## Why the manifest sees it at all

`flatpak/dev.postio.Postio.json` builds the `postio` module from

```json
{ "type": "dir", "path": ".." }
```

so the whole checkout is copied into the sandbox, `.cargo/config.toml`
included. Cargo reads it from the source root like any other invocation, and
every workstation-shaped setting in it applies inside a sandbox that has none
of the things it names:

| setting | inside the sandbox |
|---|---|
| `linker = "postio-linker"` | not on `PATH` — this is what failed |
| `CC = "postio-cc"` | not on `PATH` — would have failed next, in `ring`, `blake3`, `zstd-sys` |
| `rustc-wrapper = "scripts/rustc-wrapper.sh"` | resolves, being relative to the config, but drives an sccache that is not there |
| `-C link-arg=-Wl,--threads=4` | depends on the binutils on the box, and the SDK is a different box |

Only the first one announced itself. The rest were behind it.

## What was done

Neutralised in the manifest's `build-options.env`, not in
`.cargo/config.toml` — the config is right for the machine it was written
for, and the sandbox is the exception:

```json
"CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER": "cc",
"CC": "cc",
"RUSTC_WRAPPER": "",
"CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS": "-C linker-flavor=gcc"
```

An environment variable beats the config file for each of these, which was
checked rather than assumed: setting the linker variable to a bogus name
reproduces the CI failure exactly, on `quote`'s build script, on this
workstation. Setting the four above builds `postio-body` clean from an empty
target directory with no shims installed and no sccache — which is as close
to the sandbox as this machine gets.

`-C linker-flavor=gcc` is kept because `.cargo/config.toml` sets it
deliberately; `--threads=4` is dropped, being a tuning for the binutils here.

## The rule

**Anything in `.cargo/config.toml` that names a tool by bare name, or a path,
or this machine's capabilities, has to be answered in the Flatpak manifest
too.** The config is not a private arrangement between a developer and their
worktree — it is copied wholesale into every sandboxed build. The existing
rule ("never put a worktree path into anything rustc or a build script sees")
is the same lesson from the other side: the first version of it made the
cache miss, and this one made the release fail.

This was found four blockers deep into cutting 0.4.x, each hidden behind the
last, because no tag had ever run the release path to completion. The Release
workflow's build and bundle steps are **not** gated on `github.ref_type ==
'tag'`, so `gh workflow run Release --ref main` exercises `flatpak-builder`
without cutting a tag or publishing anything. Use it before tagging; the
three tags before this note did not exist to be learned from.
