import PostioKit
import SwiftUI

/// The query surface over the boundary's search.
///
/// It parses nothing. The whole of the query language — `from:`, `is:unread`,
/// `after:yesterday` and the rest — is `postio-search`'s, behind the boundary,
/// for both frontends; a second parser here would mean the two platforms
/// accepting different queries, which is the drift ADR 0019 exists to prevent.
///
/// Typing wins here, and it has to: this is a text field, `KeyMonitor.isTyping`
/// reports it as one, and the resolver refuses a bare-character binding while
/// it has focus. Otherwise `a` would archive mail while somebody typed
/// "already replied".
struct SearchField: View {
    let session: PostioSession
    /// Called after every run, so the list can reload against the new
    /// generation the boundary answered with.
    let reload: () -> Void
    let dismiss: () -> Void

    @State private var query = ""
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
            TextField("Search mail", text: $query)
                .textFieldStyle(.plain)
                .focused($focused)
                .onSubmit(run)
                .onKeyPress(.escape) {
                    leave()
                    return .handled
                }
            if !query.isEmpty {
                Button {
                    query = ""
                    leave()
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear the search")
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 6))
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .onAppear { focused = true }
    }

    /// Run what has been typed.
    ///
    /// On submit rather than on every keystroke. The budget is under 100 ms
    /// and FTS5 meets it, but a query is *parsed* as a whole — a half-typed
    /// `from:ada` is `from:a` for three keystrokes, and running each of those
    /// spends the budget answering questions nobody asked.
    private func run() {
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty else {
            leave()
            return
        }
        session.search(query)
        reload()
    }

    /// Leave search, restoring the scope that was open.
    private func leave() {
        session.clearSearch()
        reload()
        dismiss()
    }
}
