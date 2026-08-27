-- The indexes the unified thread list orders and groups by (#184, ADR 0005
-- Q2). Same reasoning as migration 0013 gave for unified search: an
-- AccountScope with no account named leaves queries with no account_id
-- predicate, and the per-account indexes cannot supply their ordering with
-- the leading column unpinned.

-- The unified list's own order: newest conversation first, across every
-- account, no sort step.
CREATE INDEX idx_threads_last_at ON threads (last_at DESC, id DESC);

-- The subject fallback's cross-account lookup: "which other account has a
-- conversation with these words". idx_threads_account_subject leads with
-- account_id and cannot answer it.
CREATE INDEX idx_threads_subject ON threads (subject) WHERE subject IS NOT NULL;
