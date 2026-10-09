# Only `main` writes CI build caches

*2026-09-30, #1710.*

A pull request in this repository builds against caches that `main` wrote.
It never writes one of its own. If you add a job that compiles, restore its
cache with `.github/actions/build-cache` and end the job with
`.github/actions/build-cache-save`. Don't reach for `actions/cache` for
`target/`.

## Why

`actions/cache` saves from whatever ref runs it. A cache saved by a pull
request can be read only by that pull request and its re-runs. So a PR-scoped
save helps no other branch, and it still counts against the repository's
10 GB quota. Once the quota is full, the oldest entries go first, which is
usually `main`'s.

On 2026-09-30, `refs/pull/1306/merge` held 9.4 GB of the 10 GB. `main` held
no build cache at all, so every pull request built cold:

| Job | Warm | Cold |
|---|---|---|
| Tests | 3.5 min | 12.8 min |
| Clippy | 1.8 min | 8.1 min |
| App suite | 6.1 min | 15.9 min |

`main` couldn't refill the quota either. Its runs were cancelled by the next
merge (31 of 66 in one week), and a cancelled job saves nothing.

## What holds it together

- **`main`'s CI is never cancelled.** In `ci.yml`, `cancel-in-progress` is
  false for `main`, so every merge's run completes and saves. GitHub keeps
  one pending run per concurrency group, so the queue does not pile up.
- **The cache key names everything that changes what cargo builds.** That
  covers the compiler, every `Cargo.lock`, the root `Cargo.toml` (which holds
  the profiles) and `.cargo/config.toml`. A setting that lives anywhere else,
  such as a workflow `env:` that changes codegen, needs the action's `epoch`
  bumped. Otherwise `main` gets an exact hit on a cache full of artifacts
  cargo will throw away, and never refreshes it.
- **Restores fall back to the newest cache with the same name and compiler.**
  A branch that changes the lockfile still starts from most of `main`'s
  build.
- **Each job has one build cache.** The pair passes the key and paths through
  the job environment, so a second `build-cache` in the same job would
  overwrite the first.

## Debug info

No workflow sets `CARGO_PROFILE_DEV_DEBUG`. The workspace's `debug = 0`
holds on CI too. The old `line-tables-only` override put debug info back
into every test binary (239 MB against 70) and into every cache.
