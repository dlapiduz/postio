import PostioFFI

/// Which context the **list** resolves keys as.
///
/// `Context::Search` is not "the query field has focus". Everything scoped to
/// it — `o` for *Toggle result order*, `⌘⇧S` for *Save search as folder* —
/// acts on the *result set*, and every one of them is a key you press after
/// leaving the field: the field is a text field, so `KeyMonitor` refuses bare
/// characters while it has focus, and `o` typed there is the letter o.
///
/// So the list is in `Context::Search` for exactly as long as what it is
/// showing is a set of results. While that was read off the field's focus
/// instead, both commands resolved in the one place they could not be pressed
/// and nowhere they could.
public enum SearchContext {
    /// What the list resolves as, given whether results are what it holds.
    public static func list(showingResults: Bool) -> UiContext {
        showingResults ? .search : .list
    }
}
