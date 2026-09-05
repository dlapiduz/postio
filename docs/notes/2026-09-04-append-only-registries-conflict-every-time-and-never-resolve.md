# Append-only registries conflict every time, and never resolve by hunk (2026-09-04, #1000/#1048)

The turn-8 initiative (#1000) ran eight issues across six parallel worktrees
cut from one feature branch. Every rebase conflict it produced — four of them,
plus three more in the final merge to `main` — landed in the same four files:

* `postio-core/src/command.rs` — a new `CommandId` variant and its three
  match arms
* `postio-core/src/registry.rs` — a new `CommandSpec`, in `CommandId` order
* `postio-gtk/tests/gtk_suite/main.rs` — a new row in `CASES`
* `docs/keybindings.md` — generated, so both sides regenerate it differently

That is not bad luck. All four are **append-only registries**: two branches
that each add one command are both inserting at the end of the same list, and
git has no way to know the two insertions are independent. The *content* never
disagrees. Only the insertion point does.

### The trap is that they look mechanical

They read as "both sides added a thing, keep both", which is true — and it is
true at **statement** granularity and false at **item** granularity. A regex
that concatenates each conflict's two hunks produced, silently:

* two `CommandSpec { ... }` bodies fused into one struct literal, with `id`
  and `title` each specified twice;
* two `pub fn` test bodies fused into one function, with the second
  function's signature and doc comment stranded inside the first;
* one `CASES` tuple containing four elements where the type says two.

Each cost a compile cycle to discover and a hand edit to fix. The diff looked
plausible in every case — the conflict markers were gone and the lines were
all lines somebody had written.

**Resolve one item at a time.** Read what each side added, decide where the
new item goes in the existing order, and place it whole. Then `cargo check -p
postio-core` *before* `git rebase --continue`, because a fused struct literal
is a compile error and a fused function is a parse error, and both are
cheaper to find now than after the rebase has finished and you have moved on.

### The cheaper fix is upstream of the conflict

An initiative whose children all extend the same registry does not need
parallel branches. `CLAUDE.md` already blesses several small issues on one
branch; this is that argument at a larger size. #1000 was about 70% sequential
dependencies and still ran as six branches, which bought no parallelism and
four rebases. The decision is worth making once, before the first
`issue-claim.sh`, and the `/initiative` skill is where
it now lives.

### A related one, in the same session

`refs/stash` is one ref per **repository**, not per worktree. `git stash` run
inside a private worktree lands on the same stack every other session on the
machine is using. Commit the work in progress instead — CLAUDE.md already
says uncommitted work is unprotected work, and this is a second reason: a WIP
commit is private to your branch and a stash is not.
