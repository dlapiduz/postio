import PostioFFI
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
    /// Bumped when a search runs, so the readout re-reads. `searchOutcome`
    /// is a computed property over a boundary the view cannot observe.
    @State private var ran = 0
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
            // The operators, drawn as pills from `postio-search`'s own parse.
            // Not a second parser: the chips are how somebody learns Postio's
            // query language, so two readings would be two languages
            // (canvas 2b, #1157).
            ForEach(queryChips(query: query), id: \.index) { chip in
                Text(chip.label)
                    .font(.system(.callout, design: .monospaced))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(
                        (chip.complete ? Color.accentColor : Color.secondary)
                            .opacity(chip.negated ? 0.10 : 0.20),
                        in: .rect(cornerRadius: 4)
                    )
                    // A half-typed `from:` is drawn dimmer but still drawn:
                    // it says the parser understood the keyword.
                    .opacity(chip.complete ? 1 : 0.6)
                    .accessibilityLabel(chip.spoken)
            }
            TextField("Search mail", text: $query)
                .textFieldStyle(.plain)
                .focused($focused)
                .onSubmit(run)
                .onKeyPress(.escape) {
                    leave()
                    return .handled
                }
            // "14 hits · 11 ms" — the 100ms budget made visible, which is a
            // claim the application should be willing to make on screen.
            // Its wording, and its caveats, are the core's.
            if let outcome = session.searchOutcome {
                Text(outcome.readout)
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .accessibilityLabel(outcome.spoken)
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
        // Reading `ran` here is what makes the readout above re-evaluate:
        // `searchOutcome` reads through to the boundary, which SwiftUI has no
        // way to observe on its own.
        .id(ran)
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
        ran += 1
        reload()
    }

    /// Leave search, restoring the scope that was open.
    private func leave() {
        session.clearSearch()
        ran += 1
        reload()
        dismiss()
    }
}
