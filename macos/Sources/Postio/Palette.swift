import PostioFFI
import PostioKit
import SwiftUI

/// The command palette: type, and run what you find.
///
/// Every row comes from `session.paletteEntries`, already ranked and already
/// filtered to what the focused surface can run. **Nothing here sorts or
/// filters** — the ranking is `postio_ui::palette`'s, shared with the GTK
/// frontend, and a second one would mean the same query offering different
/// things on each platform.
struct Palette: View {
    let session: PostioSession
    let context: UiContext
    let run: (String) -> Void
    let dismiss: () -> Void

    @State private var query = ""
    @State private var highlighted = 0
    @FocusState private var focused: Bool

    private var rows: [PaletteEntryFfi] {
        session.paletteEntries(query, in: context)
    }

    var body: some View {
        VStack(spacing: 0) {
            TextField("Run a command", text: $query)
                .textFieldStyle(.plain)
                .font(.title3)
                .padding(12)
                .focused($focused)
                // Typing has to win here more than anywhere: this *is* a text
                // field, and the resolver is told so by `KeyMonitor.isTyping`.
                // What is left is the two keys a list in a field needs.
                .onKeyPress(.upArrow) { move(-1) }
                .onKeyPress(.downArrow) { move(1) }
                .onKeyPress(.return) { activate() }
                .onKeyPress(.escape) {
                    dismiss()
                    return .handled
                }
            Divider()
            list
        }
        .frame(width: 560)
        .background(.regularMaterial, in: .rect(cornerRadius: 12))
        .onAppear { focused = true }
        .onChange(of: query) { _, _ in highlighted = 0 }
    }

    @ViewBuilder
    private var list: some View {
        let found = rows
        if found.isEmpty {
            // A palette that draws nothing looks broken. Saying so is the
            // difference between "no command matches that" and "this window
            // has stopped working".
            Text("No command matches")
                .foregroundStyle(.secondary)
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(found.enumerated()), id: \.element.id) { index, entry in
                        row(entry, isHighlighted: index == highlighted)
                            .contentShape(.rect)
                            .onTapGesture {
                                run(entry.id)
                                dismiss()
                            }
                    }
                }
            }
            .frame(maxHeight: 380)
        }
    }

    private func row(_ entry: PaletteEntryFfi, isHighlighted: Bool) -> some View {
        HStack {
            // The matched characters, emphasised from the offsets the shared
            // matcher returned — the same numbers GTK turns into Pango bold.
            Text(PaletteRow.highlighted(entry))
            Spacer()
            if let binding = entry.binding, let shortcut = MenuPlan.accelerator(from: binding) {
                Text(shortcut)
                    .font(.system(.body, design: .monospaced))
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .background(isHighlighted ? Color.accentColor.opacity(0.18) : .clear)
    }

    private func move(_ delta: Int) -> KeyPress.Result {
        let count = rows.count
        guard count > 0 else { return .handled }
        highlighted = (highlighted + delta).clamped(to: 0...(count - 1))
        return .handled
    }

    private func activate() -> KeyPress.Result {
        let found = rows
        guard highlighted < found.count else { return .handled }
        run(found[highlighted].id)
        dismiss()
        return .handled
    }
}

/// The cheat sheet: every command reachable here, with the key in force.
///
/// The same list the palette reads, unfiltered — *"they are the same list read
/// two ways"* (#658). Built separately they would be two places deciding what
/// "available here" means, and they would disagree.
///
/// It is also the only surface that can describe a **sequence**: `g g` has no
/// accelerator spelling, so a menu cannot show it and this can. That is why
/// the cheat sheet is the Help menu's item.
struct CheatSheet: View {
    let session: PostioSession
    let context: UiContext
    let dismiss: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Keyboard").font(.title2.bold())
                Spacer()
                Button("Done", action: dismiss).keyboardShortcut(.defaultAction)
            }
            .padding(16)
            Divider()
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(session.cheatSheet(in: context), id: \.id) { entry in
                        HStack(alignment: .firstTextBaseline) {
                            Text(entry.title)
                            Spacer()
                            // The binding as written, not as glyphs: this is
                            // the one surface that has to be able to print
                            // `g g`, which no accelerator spelling can hold.
                            Text(entry.binding ?? "—")
                                .font(.system(.body, design: .monospaced))
                                .foregroundStyle(entry.binding == nil ? .tertiary : .secondary)
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 5)
                    }
                }
            }
        }
        .frame(width: 520, height: 560)
    }
}

private extension Int {
    func clamped(to range: ClosedRange<Int>) -> Int {
        Swift.min(Swift.max(self, range.lowerBound), range.upperBound)
    }
}
