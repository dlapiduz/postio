# Git history was rewritten in place once, before any remote existed

*Archived 2026-09-14: a one-time event from before the repository had a remote; it explains why pre-rewrite commit SHAs in very old notes do not resolve and is not a recurring risk.*

The rewrite used
`git filter-repo --replace-text` to scrub personal addresses from every
commit. Every commit SHA changed as a result. Old notes citing pre-rewrite
SHAs no longer resolve. `git-filter-repo` is not packaged by default —
install with `pip install --user`. Deliberately *not* rewritten: `LICENSE`
(copyright holder), `Design/*.dc.html` (rewriting it would churn the design
canvas through history), and provider hostnames in old fixture blobs
(published server names, not personal data). This is history, not a
recurring risk — but it explains why very old references to commit SHAs may
not resolve.
