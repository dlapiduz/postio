# Encrypting the store, and the things it made visible (2026-08-28, #610/#300)

Bodies moved out of the blob store into compressed `messages` columns
(ADR 0020) and the database became SQLCipher (ADR 0014) in one pass. The
encryption itself was uneventful. What it *exposed* was not, and most of it
had been latent for months.

### `exit()` does not stop threads, and now that matters

`Engine::spawn` started a thread and dropped its `JoinHandle`, so nothing
could wait for it even in principle. The application then leaked each engine
on the reasoning — written in the code — that "dropping it at exit would stop
the engine a moment before the process ends anyway".

That was true until the store was encrypted. `exit()` runs the process's exit
handlers and then kills it; every page the sync thread writes now goes through
libcrypto, and libcrypto is torn down by those handlers:

```
thread A: exit() -> __run_exit_handlers -> (libcrypto goes away)
thread B: sqlcipher_page_cipher -> walWriteOneFrame
          -> sqlite3PagerCommitPhaseOne -> SyncStateRepository::observe
```

A coredump, not a theory: `postio-app --test e2e` every run, the engine tests
about one in six, and the application whenever somebody quit mid-sync. No mail
is lost — a torn WAL frame is what recovery is for — but the process dies on
the way out.

**A detached thread that writes to the store is a bug now, whatever it looks
like locally.** `Engine` keeps its handle and is joined: `Drop` for the
ordinary case, `Engine::stop` for the handles the application holds for the
whole session, `stop_retained` called by `run` once the GTK loop returns. The
wait is bounded at five seconds and gives up saying so, because the last handle
usually goes on the main loop and a shutdown that blocks it on a stalled
network read is a worse bug than the one being fixed.

`postio-storage` also asks libcrypto not to register its `atexit` handler.
That is belt to those braces and **does not stand alone** — with a system
libcrypto the DSO is finalized regardless, which is how the remaining crashes
were traced back to the thread rather than the flag.

### `cipher_memory_security` is a correctness setting, not a tuning knob

ADR 0014 lists it as the second performance lever after `cache_size`. It is
not: with it on, Postio segfaults inside a WAL write. The feature `mprotect`s
SQLCipher's internal buffers `PROT_NONE` between uses, and this application
always has two connections writing at once — the sync engine committing a pass
while the UI writes a flag is the ordinary state, and the whole reason
`WriteGate` exists. One connection shields a page another is mid-cipher on.

Off, permanently, and issued before `PRAGMA key` because SQLCipher wants it
there.

### `PRAGMA key` cannot fail, so something must read a page

SQLCipher accepts any key and only discovers a wrong one when a page will not
decrypt — surfacing later, elsewhere, as `SQLITE_NOTADB`: *"file is not a
database"*. That sentence reaches a screen (#404), and it tells somebody their
mail is corrupt when it is intact and merely locked. `configure` reads page 1
immediately and turns the failure into `Error::WrongStoreKey`.

### mmap is gone, and the memory story improved

`PRAGMA mmap_size` is meaningless over encrypted pages — SQLCipher decrypts
each one into the page cache, so there is no version of "the file is the
buffer". Removing it moved memory out of the file-backed half: that row used
to grow 83 → 167 MiB with mailbox size and is now flat at ~121 MiB of shared
libraries, and resident total at 100k messages went from 215 MiB to 177 MiB.

### Measuring an encrypted store: three traps, all of which caught us

1. **A stale binary measures an error screen.** After isolating a cost by
   patching out `PRAGMA key`, `target/release/postio` was still the plaintext
   build; it could not open the freshly-encrypted stores, so the first memory
   run measured a store that never opened — and "flat memory" looked entirely
   plausible. **Verify the store opened before believing any number from it.**
2. **The startup passes are a transient.** Anonymous memory peaks well above
   the settled figure — 86 MiB against 55 MiB on a 400k store — while the
   body-index catch-up and the dictionary trainer run. Sampling at ten seconds
   measures that and calls it the baseline. Wait 45 s.
3. **Two data points cannot show a bound.** 1k → 100k rises; it takes a third
   point at 400k, where it does not move at all, to show the shape is the page
   cache filling rather than the mailbox loading.

**Attribute cost to the cipher by measuring, not by reasoning.** Patching out
`PRAGMA key` in `db::configure` and re-running the same bench against an
equivalent plaintext store takes two minutes and settles it. Done that way:
encryption costs ~5% on the unified page, ~22% on startup — and is *not* why
the unified page is over budget (#619) or why startup drifted (#636). Without
the isolation the cipher would have worn both.

### The gates that nothing runs

Four bench regressions (#619, #622, #636, #638) and a licence drift (#639)
were all found by hand in one session, and they share a cause: `cargo bench`
is not in the steward loop and `cargo deny` was in no gate at all. `deny.toml`
had been a policy in the sense that a sign is a policy, while three crates
declared `GPL-3.0-or-later` in an MIT workspace and stayed green.

`check.sh` now runs `deny.toml` (`check-dependency-policy.py`). Benches
deliberately did **not** get a `--no-run` gate: `cargo clippy --all-targets`,
which `issue-land.sh` already runs, compiles them — verified by breaking one
and watching `--all-targets` catch it and a plain `cargo check` miss it. None
of the four failures were compile errors anyway. Only *running* them catches
those, which CLAUDE.md already asks of the reconcile pass.

### The vendored OpenSSL costs more than the ADR priced

ADR 0014 prices `bundled-sqlcipher-vendored-openssl` as "the heaviest new
compile in the graph", absorbed by sccache. That covers compiling OpenSSL and
not *configuring* it: `Configure` is a perl program, Fedora splits the perl
standard library into packages, and getting it to run took six of them plus
twenty-two transitive, discovered one build failure at a time as
`Can't locate X.pm in @INC` inside a cargo build script. There is no
`perl-core` metapackage in the Fedora 44 repos; the definitive list comes from
grepping `use` statements out of the extracted OpenSSL source. They are in the
README's system deps now.

The `bundled-sqlcipher` + system-libcrypto variant the ADR records as its
alternative needs none of that and built first time. The one thing vendoring
genuinely buys is that a statically linked libcrypto has no DSO to finalize at
exit — belt to the engine join above, not load-bearing now that the join
exists.
