# The error log was never switched on (2026-09-05, #1184)

`SCCACHE_ERROR_LOG` was added on 2026-09-03 "because when this does go wrong
there is currently no evidence at all". The daemon then wedged again, and the
log had nothing in it from the server — which read as the instrument being
broken, and was really the instrument never having been switched on.

**`SCCACHE_ERROR_LOG` says where log records go. `SCCACHE_LOG` decides whether
there are any.** Measured with an isolated daemon on its own port and cache
directory, so the shared one was never touched:

| | result |
|---|---|
| `SCCACHE_ERROR_LOG` only | the file is created, **0 bytes** |
| `SCCACHE_ERROR_LOG` + `SCCACHE_LOG=info` | 348 bytes of server lifecycle |

What made it look like it worked: a *client* that cannot start a second server
writes `Address in use (os error 98)` there regardless of level. The live log
had exactly that, 45 bytes of it, so the file existed and had content in it and
nobody looked twice.

`info` rather than `debug`, and the number is why: **four lines per server
start and nothing per compile** — six compiles produced the same 348 bytes as
one. It costs nothing and records what a wedge investigation wants, which is
when the daemon started and how it was configured.

## Two ways to start a daemon, and only one of them is safe

sccache reads its settings **when the server starts**, from whatever
environment that command happened to have. So the obvious next command after
`--stop-server` is the dangerous one: a bare `sccache --show-stats` starts a
server at the **default 10 GiB**, which against a 24 GiB cache directory is
instant permanent eviction — the *other* failure this box has had.

`scripts/sccache-restart.sh` is the safe path, and writing it turned up that
the obvious implementation is also wrong: running `rustc --version` through
the wrapper does **not** spawn a daemon, because `--version` is not a
cacheable compile and sccache just runs it. The server was then started by the
script's own `--show-stats`, outside the wrapper, at 10 GiB. The script's size
check caught it — the first thing that check ever did was fail its own author.

It has to be `rustc-wrapper.sh --start-server`, and the size has to be read
back rather than assumed.

## A trap for anyone testing this in a scratch directory

`SUN_LEN`. The wrapper points `TMPDIR` at `$SCCACHE_DIR/tmp`, and sccache's
control socket lives under it. A unix socket path is capped at 108 bytes, and
a session scratchpad path
(`/tmp/claude-1000/-home-diego-src-postio/<uuid>/scratchpad/...`) is most of
that on its own:

```
sccache: error: failed to start server process
sccache: caused by: path must be shorter than SUN_LEN
```

Which presents as "the restart script does not write a log". Test an isolated
daemon from a short path — `/tmp/scc-iso` — and it behaves exactly as the real
one does.
