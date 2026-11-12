import CoreGraphics
import PostioFFI
import SwiftUI

/// The composer's body column at the window's own width: spec 009 M1's
/// rule, `min(960, w - 80)`, extended from the message window to the
/// composer. It is the same shared geometry the message window asks
/// (`focusMessageColumn`), not a second copy of the numbers.
public enum ComposeColumn {
    /// The widest the body may be in a window `window` wide; unbounded
    /// until the window has a width.
    public static func width(window: CGFloat) -> CGFloat {
        guard window > 0 else { return .infinity }
        let column = focusMessageColumn(windowWidth: Int32(window.rounded()), treatment: .appColours)
        return CGFloat(column.columnWidth)
    }
}

/// The older-reply bar's two actions, by their place on the bar, as the
/// registry's commands: what a click on each runs.
public enum OlderReplyActions {
    /// *Show them*, then *Reply to latest*.
    public static let commands = ["show_newer", "reply_to_latest"]

    /// The command for the action at `index`; `nil` past the bar's two.
    public static func command(at index: Int) -> String? {
        commands.indices.contains(index) ? commands[index] : nil
    }
}

/// The bar under the composer's title on a reply to a message that is not
/// the latest (spec 013 FR-022, screen 09): accent-tinted, the title and the
/// line under it on the left, Show them and Reply to latest, each with its
/// key, on the right.
public struct OlderReplyBar: View {
    let bar: OlderReplyFfi
    let run: (String) -> Void

    /// `run` is a click on an action: the command it stands for.
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

/// The messages newer than the one answered, read-only under the bar:
/// each headed by who and when.
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
