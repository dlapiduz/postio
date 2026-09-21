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
    /// How many times a command has asked for the keyboard — see
    /// `ToolbarFieldFocus` for why a count and why AppKit.
    let focusAsks: Int
    /// Bumped by the engine whenever a search ran anywhere — a refine chip,
    /// a saved search, `o` — so this field and the readout follow.
    let searchStamp: Int
    /// Whether the field should take the keyboard.
    ///
    /// Driven from the engine so that `/` and `⌥⌘F` land here: the field is
    /// always on screen now (canvas screen 25 puts it in the toolbar), so
    /// "open search" means "focus this" rather than "reveal something".
    @Binding var wantsFocus: Bool

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
            // The placeholder names the key that focuses it, from the
            // keymap: #1260's last line is that the application teaches its
            // own keyboard, and `/` was in `docs/keybindings.md` and nowhere
            // anybody would see it.
            TextField(SearchHint.placeholder(bindings: session.bindings(for: "search")), text: $query)
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
            if session.isSearching {
                // "Relevance ▾" (canvas 05). The word is the boundary's, and
                // clicking runs the same command `o` does — so the control
                // and the key cannot drift, which is the whole reason this
                // is a command rather than a local flip.
                Button {
                    session.toggleResultOrder()
                    ran += 1
                    reload()
                } label: {
                    HStack(spacing: 2) {
                        Text(session.resultOrderLabel)
                        Image(systemName: "chevron.down")
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .help("Read the results the other way round")
                .accessibilityLabel("Sorted by \(session.resultOrderLabel). Change the order.")
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
        // The refine chips and the footer hints are NOT here, and may never
        // be again: this view is hosted in an `NSToolbar` item, which cannot
        // grow downward — a `safeAreaInset` on it overflows the toolbar and
        // floats over the window as a detached blob. Anything below the
        // field's own row belongs to `SearchRefineBar`, mounted under the
        // toolbar where layout is allowed to happen.
        .onChange(of: wantsFocus) { _, wanted in
            if wanted { focused = true }
        }
        .onChange(of: focusAsks) { _, _ in focused = true }
        // A refine chip runs the search through the engine, not through this
        // field — so the field has to adopt the query that actually ran, or
        // editing it and pressing Return would re-run the unrefined one and
        // silently drop the narrowing.
        .onChange(of: searchStamp) { _, _ in
            query = session.searchQuery ?? ""
            ran += 1
        }
        // AppKit's push, because SwiftUI's is dropped in a toolbar: focus
        // driven from AppKit (a click) reports into `@FocusState` fine, but
        // `focused = true` pushed the other way never crosses into the
        // `NSToolbar` — `/` ran its command and the keyboard went to the
        // sidebar instead. The `focused = true` above is kept for the day
        // that stops being true; this is what moves the keyboard today.
        .background(ToolbarFieldFocus(asks: focusAsks, wanted: wantsFocus))
        .onChange(of: focused) { _, has in
            // **Both directions.** Gaining the keyboard by *clicking* is
            // asking the same question `/` asks, and until this said so the
            // mouse path and the key path left the application in two
            // different states: the engine went on believing the list had
            // the keyboard, so `Save search as folder` and `Toggle result
            // order` were drawn disabled — their registry contexts are
            // `Context::Search` — and `Escape` found nothing to leave.
            //
            // Losing it is leaving search as far as the *keyboard* is
            // concerned; the results stay on screen until they are cleared,
            // which is what a search field on a toolbar means.
            wantsFocus = has
        }
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
