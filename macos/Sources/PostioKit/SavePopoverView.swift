import Observation
import PostioFFI
import SwiftUI

/// What the Save popover tells the engine (specs/010-focus-search T100).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `SavePopoverModel` without a store.
public protocol SavePopoverEngine: AnyObject {
    /// Save ↩: keep the results' query under `name`, as the switches say.
    func focusSearchSave(_ name: String, pin: Bool, notify: Bool, rolling: Bool)
}

extension PostioSession: SavePopoverEngine {}

/// The Save popover, as the controller's last `FocusSavePopover` drew it
/// (design §3.9, screen 12): the name it offers, the terms as read-only
/// chips and the three switches as they start. What is the Mac's is what
/// the field and the switches hold while the person changes them; Save
/// sends all four down, and the controller takes the popover away.
///
/// No AppKit (#1264): `SavePopover` presents it and `SavePopoverView` draws
/// it.
@MainActor
@Observable
public final class SavePopoverModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// It opened: hang it from the Save search button.
        case open
        /// It closed.
        case close
    }

    /// What it draws; `nil` while it is closed.
    public private(set) var view: SaveViewFfi?
    /// The Name field.
    public var name = ""
    /// Pin to saved searches.
    public var pin = true
    /// Notify when new mail matches.
    public var notify = false
    /// Keep the date rolling.
    public var rolling = false

    /// Called when the popover's own content asked to close: whoever
    /// presents it takes it down.
    @ObservationIgnored public var dismiss: (() -> Void)?

    @ObservationIgnored private let engine: SavePopoverEngine

    public init(engine: SavePopoverEngine) {
        self.engine = engine
    }

    /// Whether it is up.
    public var isOpen: Bool { view != nil }

    /// Apply `event` if it is the Save popover's, and say what it changed;
    /// `nil` for every other event.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        guard case let .focusSavePopover(next) = event else { return nil }
        guard let next else {
            guard view != nil else { return nil }
            view = nil
            return .close
        }
        view = next
        name = next.name
        pin = next.pin
        notify = next.notify
        rolling = next.rolling
        return .open
    }

    /// Save ↩.
    public func save() {
        guard view != nil else { return }
        engine.focusSearchSave(name, pin: pin, notify: notify, rolling: rolling)
    }

    /// Cancel, Esc, or a click away: nothing is written.
    public func cancel() {
        guard view != nil else { return }
        view = nil
        dismiss?()
    }

    /// The toolkit took it down -- a click away. Nothing is written.
    public func closedByToolkit() {
        view = nil
    }
}

/// The Save popover's content (design §3.9, screen 12): 364 wide, "Save as
/// a saved search", the Name field, the terms as small mono chips, three
/// switches with a line under each, and Cancel / Save ↩. Every word is the
/// controller's (`SaveViewFfi`).
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

/// The terms, read-only: small mono chips that wrap.
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

/// Lays its views out in rows, left to right, wrapping at the width it is
/// offered.
private struct WrapLayout: Layout {
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
