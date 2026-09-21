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
    /// The narrowings worth offering, re-measured after every run.
    ///
    /// Held rather than read inline because measuring them is a second pass
    /// over the index: a run that only draws a list should not pay for it on
    /// every redraw.
    @State private var refinements: [RefinementFfi] = []

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
        .padding(.top, 6)
        .padding(.bottom, refinements.isEmpty ? 6 : 0)
        .safeAreaInset(edge: .bottom, spacing: 0) { refineChips }
        .safeAreaInset(edge: .bottom, spacing: 0) { footerHints }
        .onChange(of: wantsFocus) { _, wanted in
            if wanted { focused = true }
        }
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

    /// The discoverable half of the query language (#1157).
    ///
    /// Four at most, and every one of them measured against the results on
    /// screen — `postio-search` decides which are worth offering, not this
    /// view. Clicking appends the token to the query and runs it, which is
    /// exactly what typing it would have done.
    @ViewBuilder private var refineChips: some View {
        if !refinements.isEmpty {
            HStack(spacing: 6) {
                ForEach(refinements, id: \.token) { refinement in
                    Button {
                        query = query.isEmpty
                            ? refinement.token
                            : "\(query) \(refinement.token)"
                        run()
                    } label: {
                        HStack(spacing: 4) {
                            Text(refinement.token)
                                .font(.system(.caption, design: .monospaced))
                            Text(Int(refinement.hits).formatted(.number))
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 4))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(
                        "Narrow to \(refinement.token), keeping \(refinement.hits) messages"
                    )
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 18)
            .padding(.bottom, 6)
        }
    }

    /// `Ret open · Tab refine · ⌘S save as folder` (canvas 05).
    ///
    /// From the keymap, like the row's own hints: this is the only place
    /// most people will ever read these keys, which is what makes teaching
    /// the wrong one worse than teaching none. Only while results are
    /// showing — over a mailbox two of the three mean nothing.
    @ViewBuilder private var footerHints: some View {
        if session.isSearching {
            let hints = session.searchHints()
            if !hints.isEmpty {
                HStack(spacing: 4) {
                    ForEach(Array(hints.enumerated()), id: \.offset) { index, hint in
                        if index > 0 {
                            Text("·").foregroundStyle(.tertiary)
                        }
                        Text(hint.key)
                            .font(.system(.caption2, design: .monospaced))
                        Text(hint.label)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 18)
                .padding(.bottom, 6)
                .accessibilityElement(children: .combine)
            }
        }
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
        // After the run, not before: they are measured against the results
        // this query found.
        refinements = session.refinements()
        reload()
    }

    /// Leave search, restoring the scope that was open.
    private func leave() {
        session.clearSearch()
        ran += 1
        refinements = []
        reload()
        dismiss()
    }
}
