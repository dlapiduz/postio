import SwiftUI

/// The find bar above the reading pane (spec 006 FR-018, #1705).
///
/// Every gesture here is the command's: the field types into
/// `FindInMessage`, Return is `find_next`, the arrows are `find_next` and
/// `find_previous`, and Done and Escape close it. The bar decides nothing.
public struct FindBar: View {
    private let find: FindInMessage
    private let setQuery: (String) -> Void
    private let next: () -> Void
    private let previous: () -> Void
    private let close: () -> Void

    @State private var text: String
    @FocusState private var focused: Bool

    public init(
        find: FindInMessage,
        setQuery: @escaping (String) -> Void,
        next: @escaping () -> Void,
        previous: @escaping () -> Void,
        close: @escaping () -> Void
    ) {
        self.find = find
        self.setQuery = setQuery
        self.next = next
        self.previous = previous
        self.close = close
        _text = State(initialValue: find.query)
    }

    public var body: some View {
        HStack(spacing: PostioTokens.space2) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
            TextField("Find in message", text: $text)
                .textFieldStyle(.plain)
                .focused($focused)
                .onSubmit(next)
                .onExitCommand(perform: close)
                .onChange(of: text) { _, typed in setQuery(typed) }
                .accessibilityLabel("Find in message")
            if find.missing {
                Text("Not found")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Button(action: previous) { Image(systemName: "chevron.up") }
                .buttonStyle(.borderless)
                .disabled(text.isEmpty)
                .accessibilityLabel("Previous match")
            Button(action: next) { Image(systemName: "chevron.down") }
                .buttonStyle(.borderless)
                .disabled(text.isEmpty)
                .accessibilityLabel("Next match")
            Button("Done", action: close)
                .controlSize(.small)
        }
        .padding(.horizontal, PostioTokens.space4)
        .padding(.vertical, PostioTokens.space2)
        .background(.bar)
        .onAppear { focused = true }
        .onChange(of: find.focusToken) { focused = true }
    }
}
