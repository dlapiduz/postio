# mold is wired in, for memory -- and `-fuse-ld` order is why it took three tries (2026-09-03)

Re-measured after the debug info came out of the profile, because the
2026-09-01 verdict was reached on a link that carried 162 MB of DWARF and the
profile no longer does. Interleaved, six rounds, idle box, timing the whole
`rustc` invocation for `app_suite`:

```
rust-lld  1.30 1.24 1.23 1.23 1.25 1.23   median 1.235s   maxRSS 734 MB
mold      1.16 1.21 1.13 1.75 1.16 1.12   median 1.16s    maxRSS 469 MB
```

**Time is a wash and always will be**: there is only ~1.2s of compile-and-link
to contest, so no linker choice can matter. Memory is not: mold's peak is
~265 MB lower, and on a workstation where four sessions link at once that is
the scarce resource -- it is why `jobs = 2` and the linker thread cap exist.
That is the reason it is wired in; it is not a speed change, and `scripts/
linker.sh` says so.

**The trap, which cost three wrong answers in a row.** rustc appends its own
linker selection to every link on this target, *even with*
`-C linker-flavor=gcc`:

```
-B <sysroot>/lib/rustlib/x86_64-unknown-linux-gnu/bin/gcc-ld
-fuse-ld=lld
```

`gcc` honours the **last** `-fuse-ld`. So every natural spelling fails
silently:

* `-C link-arg=-B<mold>/libexec/mold` -- ignored, LLD links it.
* a wrapper running `cc -fuse-ld=mold "$@"` -- ignored, because the
  `-fuse-ld=lld` inside `"$@"` comes later. The wrapper genuinely runs, mold
  is genuinely on PATH, and LLD genuinely does the link.

What works is putting ours last: `cc "$@" -fuse-ld=mold`. It also means the
no-mold path needs nothing at all -- rustc's own arguments already select the
toolchain's lld, so plain `cc` reproduces current behaviour exactly, which is
what makes the wrapper safe on a runner with no mold.

**Verify with `readelf -p .comment <binary>`, always.** It names the linker
that actually ran. There is no error, no warning, and no behavioural
difference between "mold linked this" and "you thought mold linked this" --
only that string. Note mold's output is slightly larger here (88.7 MB against
lld's 74.3 MB for `app_suite`), which is itself a quick way to notice.
