# Contract: the engine's search requests

What `postio-focus`'s `perform` asks of `postio-client`, how the host answers,
and what stays as it is. Types are in [data-model.md](../data-model.md).

## Unchanged (GTK's path, FR-046)

`Req::Search`, `Req::SearchHits`, `Req::Facets`, `Client::search_hits`,
`Client::facets`, `postio_session::search::{execute, execute_with_snippets,
facets}`, `postio_index::executor::{search, facets}`. The only change they
see is `label:` and `has:action` in `filter_condition` (S2).

## New requests (`postio-client/src/protocol.rs`, `api.rs`)

| `Req` | `Resp` | `Client` method | Host (`postio-host/src/search.rs`) → session |
|---|---|---|---|
| `Conversations { account, query: ParsedQuery, order, offset, limit }` | `Conversations(Option<Box<ConversationResults>>)` | `conversations(..)` | `postio_session::search::conversations` → `executor::search_conversations` |
| `Passages { query: ParsedQuery, hits: Vec<(MessageId, Vec<Source>)> }` | `Passages(Vec<(MessageId, Vec<Match>)>)` | `passages(..)` | `postio_session::search::passages`: body via `indexable_text`, quote stretches, `passage::cut`; attachment passages from `attachment_passages` |
| `ConversationMatches { key: ConversationKey, query }` | `Matches(Vec<Match>)` | `conversation_matches(..)` | every match in one conversation, for Quick Look |
| `Suggest { account, prefix: String, field: Option<Field> }` | `Suggestions(Box<Suggestions>)` | `suggest(..)` | `executor::completions` |
| `Relaxations { account, query, today }` | `Relaxed(Vec<(Relaxation, u64)>)` | `relaxations(..)` | `postio_search::relax` + `executor::relaxation_counts`, zero counts dropped, sorted |
| `Files { account, query, offset, limit }` | `Files(Vec<FileHit>)` | `files(..)` | `executor::files` |
| `People { account, query, offset, limit }` | `People(Vec<Person>)` | `people(..)` | `executor::people` |
| `RecentSearches` | `Recent(Vec<RecentSearch>)` | `recent_searches()` | `postio_storage` `recent_searches`, newest first, ≤ 20 |
| `RememberSearch { query, hits }` | `Done` | `remember_search(..)` | upsert, trim to 20 |
| `ForgetSearch(String)` | `Done` | `forget_search(..)` | delete |
| `SavedCounts(Vec<(String /*key*/, String /*query*/)>)` | `SavedCounts(Vec<(String, u64 /*total*/, u64 /*new*/)>)` | `saved_counts(..)` | one capped count each, and "new" since `seen_up_to` |
| `MarkSeen(String)` | `Done` | `mark_seen(..)` | `seen_up_to = now` |
| `AttachmentCopy(AttachmentId)` | `Path(Option<PathBuf>)` | `attachment_copy(..)` | blob → the app's temp dir, for system Quick Look and ⌘↓ |

`RecentSearch { query, last_run_at, hits }`.

`Req::family()` gains each name, so `postio-client/src/counting.rs` counts
them and the ffi_suite can assert how many reads a keystroke makes.

## Cancellation (D9)

`Req::cancellable(&self) -> bool` is true for `Conversations`, `Passages`,
`Suggest`, `Relaxations`, `Files`, `People`. In `Local::call`
(`postio-host/src/lib.rs`) a cancellable request is answered as

```rust
tokio::select! {
    _ = answer.closed() => tracing::debug!(family, "a search was abandoned"),
    resp = inner.answer(client, request) => { let _ = answer.send(resp); }
}
```

so dropping the client's future (the driver's abort) stops the host's
work at its next await. Every other request keeps today's behaviour.

## Background work

`postio_session::spawn_attachment_indexer` (beside `spawn_body_indexer`,
`postio-session/src/lib.rs`), started by the host after the body indexer:
batches of `index::attachments_missing_text(…, 8)`, each blob read and
`postio_extract::extract` run on `spawn_blocking`, results written with
`index::index_attachment_text`. It yields to sync like the body indexer,
never fetches a blob, and wakes on the event that says a blob was stored.
Logs carry attachment ids, outcomes and durations only (FR-051).

## Statement budgets (asserted with `counting`)

| Request | Statements, whatever the match size |
|---|---|
| `Conversations` | ≤ 5 (research R2) |
| `Suggest` | ≤ 4 (metadata words, labels, lists, people; bodies only when metadata gave < 3) |
| `Relaxations` | ≤ 8, one count per variant |
| `Passages` for one page | 1 body read per hit plus 1 for attachment passages |
