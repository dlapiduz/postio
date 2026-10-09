# A coredump names a worktree, and that work may never have landed (2026-09-05, #1015)

#1015 was reopened around three coredumps that were "byte-identical in
signature" — same crash site, same six frames above it, same `gtk_row` filter
on the command line. That is a strong-looking fact, and the conclusion drawn
from it was that `gtk_suite`'s segfault "is one path in one test, not diffuse
contention damage". Both halves of that were true of *the binary that
crashed*. Neither was true of `main`.

`coredumpctl info` prints the executable path and the timestamp:

```
Timestamp:  Wed 2026-09-02 00:26:37 EDT
Executable: ~/src/postio-worktrees/issue-753/target/debug/deps/gtk_suite-…
```

A worktree path is a *branch*, mid-flight, and this project deletes the tree
when the branch lands. So the second question after "what crashed" is **"was
that code ever merged?"** — one `git log` away:

```
614e5973 Wed Sep 2 00:50:22 2026  fix(gtk): give the cursor and the selection separate devices  (#753)
```

Twenty-three minutes *after* the last of the three cores. And that commit's
own body says what the tree had contained while it was crashing:

> Calling `refresh_focus()` from `connect_bind` was tried first and segfaults
> — it re-enters layout while the factory is still building the widget

The three cores were that experiment. It was diagnosed and removed before the
branch landed, and a later session spent a pass treating them as evidence
about `main`.

## Reading a core whose binary is gone

Worth knowing, because it is what placed these: the binary being deleted with
its worktree does not cost you the crash, only the locals.

- `coredumpctl info <pid>` already carries a symbolised stack for every
  thread — systemd resolves it through `debuginfod` at capture time.
- `coredumpctl dump <pid> --output=core` then `gdb -q -batch -c core -ex
  "info registers" -ex "x/8i $pc-24"` gives the faulting instruction and the
  registers. That is usually the whole diagnosis: here `mov (%rdi),%rax`
  loaded a `GTypeInstance`'s `g_class` and got `0`, which is what
  `g_type_free_instance` leaves behind — the object was **freed**, and the
  argument registers said which one.
- `debuginfod-find debuginfo /usr/lib64/libfoo.so` and `debuginfod-find
  source /usr/lib64/libfoo.so <path>` fetch the symbols and the *actual
  source* of the system library, so a crash inside GTK can be read against
  the code that crashed rather than against memory of it. `nm -a` on the
  fetched debuginfo gives the internal (non-exported) symbol addresses that
  `objdump -d` on the real library then needs.

## And the load you are measuring against may not be the project's

While measuring #1015 under contention, this machine's baseline load of ~20
turned out to be **14 `while :; do :; done` shells that had been spinning for
eleven hours** — leaked by the previous session on the same issue, which had
raised load deliberately and whose `kill $LOADPIDS` never ran because the
tool call was cut off first.

A load generator has to survive its own author being killed: put it under
`timeout`, or in a process group with a `trap`, and never rely on a line at
the end of the script. Otherwise the next session's measurements are taken
against your leftovers, and so are everybody's build times.
