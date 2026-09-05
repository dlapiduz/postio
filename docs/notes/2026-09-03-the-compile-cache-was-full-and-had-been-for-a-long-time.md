# The compile cache was full, and had been for a long time (2026-09-03)

`sccache --show-stats` on the development box:

```
Cache size        10 GiB
Max cache size    10 GiB      <- the default; nobody had ever set it
```

11G on disk against a 10 GiB ceiling is a cache in permanent eviction. Nine
worktrees live here, each holding ~2.1 GB of dependency artifacts, so the
sessions were continuously throwing out each other's entries — and, it
turned out (#1101), each worktree's entries were keyed by its own linker
path, so none of them could have served another worktree anyway. From inside a
session that presents as "the compile cache died and fell back to compiling
locally" and as a unit tier that takes 43 minutes instead of seconds -- which
is how it was noticed, by a session on another machine saying so out loud.

`scripts/rustc-wrapper.sh` now defaults it to 30G, keeps the server up
(`SCCACHE_IDLE_TIMEOUT=0`) and writes `SCCACHE_ERROR_LOG`, because there was
no evidence of any kind when it went wrong.

**These are read when the server starts, not per invocation** -- the same
hazard as the TMPDIR pinning beside them (#359). Changing them needs one
`sccache --stop-server`; `sccache --show-stats` prints "Max cache size" and is
how you tell which server you are talking to.

Worth knowing generally: a profile or linker flag change creates an entirely
new key space in that cache and evicts whatever was there. On a box this
close to its ceiling, landing a profile change is not free for the other
sessions, which is an argument for doing it rarely and in one commit.
