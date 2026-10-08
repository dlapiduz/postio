import Foundation
import PostioFFI
import SwiftUI

/// What the action bar says while anything is selected
/// (specs/009-focus-macos T052, screen 01): the controller's count on the
/// left, the verbs with the user's keys, and the selection's own keys on
/// the right.
///
/// The verbs are `postio_ui::focus_dialog::BULK`'s, which GTK's bulk bar
/// draws, less Delete, which screen 01 does not offer; the boundary does not
/// export that table, so this is its one Swift copy, in its order. Task is
/// there only with a vault (C9). The words for the count are the
/// controller's (`FocusSelection.summary`), never composed here.
public struct ActionBarWords: Equatable, Sendable {
    /// One verb: the command a press runs, its words and its keycap.
    public struct Action: Equatable, Sendable {
        public let command: String
        public let label: String
        public let cap: String?
        /// Drawn on the raised fill: the verb a selection is usually for.
        public let isDefault: Bool
    }

    /// One of the selection's keys on the right: "x toggle".
    public struct Hint: Equatable, Sendable {
        public let caps: [String]
        public let label: String
    }

    /// The registry's ids for what the bar offers and hints at.
    public enum Command {
        public static let archive = "archive"
        public static let snooze = "snooze"
        public static let markRead = "toggle_read"
        public static let digest = "digest_rule"
        public static let task = "capture_task"
        public static let label = "add_label"
        public static let move = "move"
        public static let toggle = "toggle_selection"
        public static let extendDown = "extend_selection_down"
        public static let extendUp = "extend_selection_up"
        public static let clear = "back"
    }

    public let summary: String
    public let actions: [Action]
    public let hints: [Hint]

    /// The bar for a selection the controller summarised as `summary`, or
    /// `nil` while nothing is selected. `vault` says whether capture can
    /// write anywhere (C9).
    public init?(
        summary: String?, hasSelection: Bool, vault: Bool, binding: (String) -> String?
    ) {
        guard hasSelection, let summary else { return nil }
        self.summary = summary
        var verbs: [(String, String)] = [
            (Command.archive, "Archive"),
            (Command.snooze, "Snooze"),
            (Command.markRead, "Mark read"),
            (Command.digest, "Digest these\u{2026}"),
        ]
        if vault { verbs.append((Command.task, "Task")) }
        verbs += [(Command.label, "Label"), (Command.move, "Move")]
        actions = verbs.map { command, label in
            Action(
                command: command, label: label, cap: KeyCapSpelling.cap(binding(command)),
                isDefault: command == Command.archive)
        }
        // GTK's bulk bar's hints, in its words (`postio-gtk` bulk.rs).
        let said: [([String], String)] = [
            ([Command.toggle], "toggle"),
            ([Command.extendDown, Command.extendUp], "extend"),
            ([Command.clear], "clear"),
        ]
        hints = said.compactMap { commands, label in
            let caps = commands.compactMap { KeyCapSpelling.cap(binding($0)) }
            return caps.isEmpty ? nil : Hint(caps: caps, label: label)
        }
    }
}

/// The action bar, drawn: SwiftUI over `ActionBarWords`, at the bottom of
/// the main window while anything is selected. A press runs the verb's
/// command id through the path its key takes.
public struct ActionBar: View {
    let words: ActionBarWords
    let run: (String) -> Void

    public init(words: ActionBarWords, run: @escaping (String) -> Void) {
        self.words = words
        self.run = run
    }

    public static let height: CGFloat = 44

    public var body: some View {
        HStack(spacing: 6) {
            Text(words.summary)
                .font(.system(size: 14, weight: .bold).monospacedDigit())
                .padding(.trailing, 10)
                .accessibilityAddTraits(.updatesFrequently)
            ForEach(words.actions, id: \.command) { action in
                Button { run(action.command) } label: {
                    HStack(spacing: 6) {
                        Text(action.label).font(.system(size: 13.5, weight: .bold))
                        if let cap = action.cap { KeyCap(cap) }
                    }
                    .foregroundStyle(.primary)
                    .padding(.horizontal, 10)
                    .frame(height: 30)
                    .background(
                        RoundedRectangle(cornerRadius: 6).fill(
                            action.isDefault ? AnyShapeStyle(.quaternary) : AnyShapeStyle(.clear))
                    )
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(action.cap.map { "\(action.label), key \($0)" } ?? action.label)
            }
            Spacer(minLength: 12)
            ForEach(words.hints, id: \.label) { hint in
                HStack(spacing: 4) {
                    ForEach(hint.caps, id: \.self) { KeyCap($0) }
                    Text(hint.label).font(.system(size: 13)).foregroundStyle(.secondary)
                }
                .padding(.leading, 6)
                .accessibilityElement(children: .combine)
            }
        }
        .padding(.horizontal, 20)
        .frame(height: Self.height)
        .frame(maxWidth: .infinity)
        .background(.background)
        .overlay(alignment: .top) { Divider() }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Selection actions")
    }
}
