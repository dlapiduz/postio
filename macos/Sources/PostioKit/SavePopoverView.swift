import Observation
import PostioFFI
import SwiftUI

public protocol SavePopoverEngine: AnyObject {
    func focusSearchSave(_ name: String, pin: Bool, notify: Bool, rolling: Bool)
}

extension PostioSession: SavePopoverEngine {}

/// Save sends the field and all three switches down as they stand, and the controller takes the
/// popover away. No AppKit (#1264).
@MainActor
@Observable
public final class SavePopoverModel {
    public enum Change: Equatable {
        case open
        case close
    }

    public private(set) var view: SaveViewFfi?
    public var name = ""
    public var pin = true
    public var notify = false
    public var rolling = false

    @ObservationIgnored public var dismiss: (() -> Void)?

    @ObservationIgnored private let engine: SavePopoverEngine
    /// Return reaches both the field and the key monitor; this makes it one save.
    @ObservationIgnored private var sent = false

    public init(engine: SavePopoverEngine) {
        self.engine = engine
    }

    public var isOpen: Bool { view != nil }

    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        guard case let .focusSavePopover(next) = event else { return nil }
        guard let next else {
            guard view != nil else { return nil }
            view = nil
            return .close
        }
        view = next
        sent = false
        name = next.name
        pin = next.pin
        notify = next.notify
        rolling = next.rolling
        return .open
    }

    public func save() {
        guard view != nil, !sent else { return }
        sent = true
        engine.focusSearchSave(name, pin: pin, notify: notify, rolling: rolling)
    }

    public func cancel() {
        guard view != nil else { return }
        view = nil
        dismiss?()
    }

    public func closedByToolkit() {
        view = nil
    }
}

public struct SavePopoverView: View {
    @Bindable var model: SavePopoverModel

    public init(model: SavePopoverModel) {
        self.model = model
    }

    public static let width: CGFloat = 364

    public var body: some View {
        if let view = model.view {
            VStack(alignment: .leading, spacing: 0) {
                Text(view.title).font(.system(size: 14.5, weight: .bold))
                    .padding(.bottom, 14)
                Text(view.nameLabel).font(.system(size: 12)).foregroundStyle(.secondary)
                    .padding(.bottom, 5)
                TextField("", text: $model.name)
                    .textFieldStyle(.roundedBorder)
                    .font(.system(size: 14))
                    .onSubmit { model.save() }
                    .accessibilityLabel(view.nameLabel)
                    .padding(.bottom, 10)
                FlowChips(chips: view.chips)
                    .padding(.bottom, 14)
                toggle(view.pinLabel, note: pinNote(view), isOn: $model.pin)
                toggle(view.notifyLabel, note: view.notifyNote, isOn: $model.notify)
                if let rolling = view.rollingNote {
                    toggle(view.rollingLabel, note: rolling, isOn: $model.rolling)
                }
                HStack(spacing: 8) {
                    Spacer()
                    Button { model.cancel() } label: {
                        Text(view.cancel).font(.system(size: 13, weight: .semibold))
                            .padding(.horizontal, 12).frame(height: 28)
                            .background(RoundedRectangle(cornerRadius: 6).fill(.quaternary))
                    }
                    .buttonStyle(.plain)
                    Button { model.save() } label: {
                        HStack(spacing: 6) {
                            Text(view.save).font(.system(size: 13, weight: .bold))
                            if let cap = KeyCapSpelling.cap(view.saveKey) { KeyCap(cap) }
                        }
                        .foregroundStyle(Color(nsColor: .textBackgroundColor))
                        .padding(.horizontal, 12).frame(height: 28)
                        .background(RoundedRectangle(cornerRadius: 6).fill(.primary))
                    }
                    .buttonStyle(.plain)
                }
                .padding(.top, 8)
            }
            .padding(16)
            .frame(width: Self.width, alignment: .leading)
            .background(Color(nsColor: .textBackgroundColor))
        }
    }

    private func pinNote(_ view: SaveViewFfi) -> String {
        guard let cap = KeyCapSpelling.cap(view.pinKey) else { return view.pinNote }
        return "\(view.pinNote) \(cap)"
    }

    private func toggle(_ title: String, note: String, isOn: Binding<Bool>) -> some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.system(size: 13, weight: .semibold))
                Text(note).font(.system(size: 11.5)).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
            Toggle("", isOn: isOn).toggleStyle(.switch).labelsHidden().controlSize(.small)
                .accessibilityLabel(title)
        }
        .padding(.bottom, 10)
    }
}

private struct FlowChips: View {
    let chips: [String]

    var body: some View {
        WrapLayout(spacing: 6) {
            ForEach(Array(chips.enumerated()), id: \.offset) { _, chip in
                Text(chip)
                    .font(.system(size: 11.5, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 6)
                    .frame(height: 22)
                    .background(RoundedRectangle(cornerRadius: 5).fill(.quaternary))
                    .fixedSize()
            }
        }
    }
}

struct WrapLayout: Layout {
    var spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        var x: CGFloat = 0
        var y: CGFloat = 0
        var line: CGFloat = 0
        var widest: CGFloat = 0
        for view in subviews {
            let size = view.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width {
                y += line + spacing
                x = 0
                line = 0
            }
            x += size.width + spacing
            line = max(line, size.height)
            widest = max(widest, x - spacing)
        }
        return CGSize(width: proposal.width ?? widest, height: y + line)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX
        var y = bounds.minY
        var line: CGFloat = 0
        for view in subviews {
            let size = view.sizeThatFits(.unspecified)
            if x > bounds.minX, x + size.width > bounds.maxX {
                y += line + spacing
                x = bounds.minX
                line = 0
            }
            view.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
            x += size.width + spacing
            line = max(line, size.height)
        }
    }
}
