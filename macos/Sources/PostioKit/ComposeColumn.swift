import CoreGraphics
import PostioFFI
import SwiftUI

public enum ComposeColumn {
    public static func width(window: CGFloat) -> CGFloat {
        guard window > 0 else { return .infinity }
        let column = focusMessageColumn(windowWidth: Int32(window.rounded()), treatment: .appColours)
        return CGFloat(column.columnWidth)
    }
}

public enum OlderReplyActions {
    public static let commands = ["show_newer", "reply_to_latest"]

    public static func command(at index: Int) -> String? {
        commands.indices.contains(index) ? commands[index] : nil
    }
}

public struct OlderReplyBar: View {
    let bar: OlderReplyFfi
    let run: (String) -> Void

    public init(bar: OlderReplyFfi, run: @escaping (String) -> Void) {
        self.bar = bar
        self.run = run
    }

    public var body: some View {
        HStack(alignment: .top, spacing: 16) {
            VStack(alignment: .leading, spacing: 2) {
                Text(bar.title)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.primary)
                Text(bar.detail)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            HStack(spacing: 14) {
                ForEach(Array(bar.actions.enumerated()), id: \.offset) { index, action in
                    Button {
                        if let command = OlderReplyActions.command(at: index) { run(command) }
                    } label: {
                        HStack(spacing: 5) {
                            Text(action.label)
                                .font(.system(size: 12, weight: .semibold))
                                .foregroundStyle(.primary)
                            if let key = KeyCapSpelling.cap(action.key) { KeyCap(key) }
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(action.label)
                }
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.accentColor.opacity(0.10))
        .accessibilityElement(children: .contain)
    }
}

public struct NewerMessagesView: View {
    let messages: [NewerMessageFfi]

    public init(messages: [NewerMessageFfi]) {
        self.messages = messages
    }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(messages, id: \.id) { message in
                    VStack(alignment: .leading, spacing: 3) {
                        Text("\(message.who) \u{b7} \(message.when)")
                            .font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(.secondary)
                        Text(message.text)
                            .font(.system(size: 13))
                            .foregroundStyle(.primary)
                            .textSelection(.enabled)
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(RoundedRectangle(cornerRadius: 6).fill(.quaternary.opacity(0.6)))
                    .accessibilityElement(children: .combine)
                }
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 10)
        }
        .frame(maxHeight: 220)
        .accessibilityLabel("Newer messages, read only")
    }
}
