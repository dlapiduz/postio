# `debug = "line-tables-only"` was still most of the binary (2026-09-03)

`[profile.dev] debug = "line-tables-only"` was set once as a tuning and never
measured afterwards. Measured on the `app_suite` test binary -- cold builds of
`cargo test --no-run -p postio-app --test app_suite` at `-j6`, sccache
disabled, idle box, baseline reproduced twice at 301s and 307s:

```
debug                cold build   binary   .debug sections
"line-tables-only"     301/307s   239 MB       153.6 MB
0                          282s    70 MB              0
```

Faster **and** smaller, which is not the usual shape of this trade: line
tables are not free to emit across ~470 crates, and `target/debug/deps` holds
~330 executables that each carried them.

**`strip = "debuginfo"` was tried on top and is a pure cost -- do not add it
back.** The theory was that `debug = 0` cannot reach the *precompiled std*
rlibs, which ship with their own DWARF, and that only a link-time strip would.
That theory is wrong: `debug = 0` alone leaves **zero** bytes of `.debug_*` in
the output. Adding `strip` produced a byte-identical 70 MB binary and cost 44
seconds (326s against 282s), because cargo passes `-C strip` to all ~470 units
while only a handful are ever linked.

What the change costs was checked rather than assumed, and it is narrower than
it sounds. `file:line` survives in **panic and assertion messages** -- what a
failing test prints -- because those come from `#[track_caller]` and are
compiled in as data, not read from DWARF. What goes is `file:line` on the
frames of a `RUST_BACKTRACE` dump (function names remain) and variable
inspection in gdb. Nothing in this workspace asserts on a backtrace.
`CARGO_PROFILE_DEV_DEBUG=line-tables-only` restores it for one run, at the
price of a full rebuild.
