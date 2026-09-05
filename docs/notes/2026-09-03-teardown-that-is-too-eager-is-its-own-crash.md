# Teardown that is too eager is its own crash (2026-09-03, #794)

Once the reference cycles were broken, destroying a window actually released
it — so the remaining half of #794 was to make something call `destroy()`. A
`GtkWindow` joins the toplevel list when it is **constructed**, so a test that
builds one and drops the handle leaves a WebProcess attached until `exit()`.

The obvious shape — sweep every toplevel after every case, in both harnesses —
**segfaulted `app_suite`**. Every one of its 53 cases passed and the process
died on the way out, in `Error releasing name …WebProcess…`. Each of those
cases stands up a full window over a live engine, and tearing WebKit down
between every one of them left the exit path crashing on connections that were
already closed.

So the sweep is not one policy:

| where | when | why |
|---|---|---|
| `gtk_suite` | after every case | 152 cases, and toolkit state left by one case is what fails the next — its own header says so |
| `app_suite` | once, after all cases | per-case teardown crashes it; once is enough for the thing #794 is about |
| single-test binaries | at the end of the test | no harness to hang it on |

**The lesson is that "release resources promptly" is not free when the
resource is a subprocess.** WebKit tolerates being torn down at a boundary it
expects and not at one it does not, and the difference between the two
harnesses is not something a reader would predict from their names. Anyone
tempted to unify them should run `app_suite` first.

Worth stating plainly: the segfault this issue is named for has never
reproduced on this workstation, so none of the above is validated by a green
local run. What is validated is the mechanism — the windows are released, and
`gtk_window_teardown.rs` asserts it deterministically.
