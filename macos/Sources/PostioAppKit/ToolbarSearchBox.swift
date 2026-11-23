import AppKit
import PostioKit

/// Postio's own view in a plain `NSToolbarItem`: `NSSearchToolbarItem` ignored every width it was
/// given and stayed about 327 wide, so the dropdown hung from a field a third of its width. Opening
/// grows it leftward to `CommandBarGeometry`'s width, right edge fixed. Not animated: the panel
/// hangs from where the field ends up.
@MainActor
public final class ToolbarSearchBox: NSView {
    public let field = BarSearchField()
    /// The field as drawn, which the panel hangs from. On the right its edge sits 12 from the
    /// window's: a toolbar keeps 8 in a bare window, 12 in the app's.
    public let frameView = SearchFieldFrameView()
    private let magnifier = NSImageView()
    private let cap = KeyCapView("")
    private var widthConstraint: NSLayoutConstraint?
    private var heightConstraint: NSLayoutConstraint?
    private var magnifierLeading: NSLayoutConstraint?
    private var trailingInset: NSLayoutConstraint?

    public static let restingHeight: CGFloat = 28
    public static let openHeight: CGFloat = 34
    public static let inset: CGFloat = SearchFieldChrome.haloWidth

    public var ringed: Bool { frameView.ringed }

    public var restingCap: String? { didSet { placeCap() } }
    public var openCap: String? { didSet { placeCap() } }

    public var placeholder: String = "" { didSet { applyFonts() } }
    public var restingPlaceholder: String = "" { didSet { applyFonts() } }

    public init() {
        super.init(frame: NSRect(
            x: 0, y: 0, width: CommandBarGeometry.restingWidth + 2 * Self.inset,
            height: Self.restingHeight + 2 * Self.inset))
        translatesAutoresizingMaskIntoConstraints = false
        setAccessibilityElement(false)

        field.isBezeled = false
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        if let cell = field.cell as? NSSearchFieldCell {
            cell.usesSingleLineMode = true
            cell.lineBreakMode = .byClipping
            cell.isScrollable = true
        }
        field.frameView = frameView

        magnifier.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)
        cap.setContentCompressionResistancePriority(.required, for: .horizontal)
        cap.setContentHuggingPriority(.required, for: .horizontal)

        frameView.translatesAutoresizingMaskIntoConstraints = false
        addSubview(frameView)
        for view in [magnifier, field, cap] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            frameView.addSubview(view)
        }
        let width = frameView.widthAnchor.constraint(equalToConstant: CommandBarGeometry.restingWidth)
        let height = frameView.heightAnchor.constraint(equalToConstant: Self.restingHeight)
        let leading = magnifier.leadingAnchor.constraint(equalTo: frameView.leadingAnchor, constant: 8)
        let trailing = frameView.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -Self.inset)
        NSLayoutConstraint.activate([
            width, height, leading,
            frameView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Self.inset),
            trailing,
            frameView.topAnchor.constraint(equalTo: topAnchor, constant: Self.inset),
            frameView.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -Self.inset),
            magnifier.centerYAnchor.constraint(equalTo: frameView.centerYAnchor),
            field.leadingAnchor.constraint(equalTo: magnifier.trailingAnchor, constant: 6),
            field.centerYAnchor.constraint(equalTo: frameView.centerYAnchor),
            field.trailingAnchor.constraint(equalTo: cap.leadingAnchor, constant: -8),
            cap.trailingAnchor.constraint(equalTo: frameView.trailingAnchor, constant: -7),
            cap.centerYAnchor.constraint(equalTo: frameView.centerYAnchor),
        ])
        widthConstraint = width
        heightConstraint = height
        magnifierLeading = leading
        trailingInset = trailing
        frameView.onClick = { [weak self] in
            guard let self else { return }
            self.window?.makeFirstResponder(self.field)
        }
        applyFonts()
        placeCap()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    public func item(_ identifier: NSToolbarItem.Identifier) -> NSToolbarItem {
        let item = NSToolbarItem(itemIdentifier: identifier)
        item.view = self
        item.label = "Search"
        item.visibilityPriority = .high
        return item
    }

    public func open(_ open: Bool, windowWidth: CGFloat) {
        field.listOpen = open
        frameView.ringed = open
        frameView.radius = open ? 8 : 7
        widthConstraint?.constant = open
            ? CommandBarGeometry.fieldWidth(window: windowWidth)
            : CommandBarGeometry.restingWidth
        heightConstraint?.constant = open ? Self.openHeight : Self.restingHeight
        magnifierLeading?.constant = open ? 10 : 8
        applyFonts()
        placeCap()
        needsLayout = true
        keepRightEdge()
    }

    /// Lays the toolbar out, reads where it put the item, and takes up the difference inside it;
    /// the toolbar's room does not depend on the item's width, so one correction holds.
    public func keepRightEdge() {
        guard let window, let trailingInset else { return }
        let frame = window.contentView?.superview
        frame?.layoutSubtreeIfNeeded()
        let itemGap = window.frame.width - convert(bounds, to: nil).maxX
        let wanted = max(CommandBarGeometry.edge - itemGap, 0)
        guard abs(trailingInset.constant + wanted) > 0.25 else { return }
        trailingInset.constant = -wanted
        frame?.layoutSubtreeIfNeeded()
    }

    public func textChanged() {
        placeCap()
    }

    private func applyFonts() {
        field.textFont = .systemFont(ofSize: ringed ? 15 : 13)
        magnifier.symbolConfiguration = .init(pointSize: ringed ? 14 : 13, weight: .regular)
        magnifier.contentTintColor = ringed ? .secondaryLabelColor : .tertiaryLabelColor
        field.placeholderAttributedString = NSAttributedString(
            string: ringed ? placeholder : restingPlaceholder,
            attributes: [
                .font: NSFont.systemFont(ofSize: ringed ? 14 : 13),
                .foregroundColor: NSColor.tertiaryLabelColor,
            ])
    }

    private func placeCap() {
        let text = ringed ? openCap : (field.stringValue.isEmpty ? restingCap : nil)
        cap.text = text ?? ""
        cap.isHidden = text?.isEmpty ?? true
    }
}

@MainActor
public final class SearchFieldFrameView: NSView {
    private let chrome = SearchFieldChrome()
    var onClick: (() -> Void)?

    public var ringed = false { didSet { needsDisplay = true } }
    var radius: CGFloat = 7 { didSet { needsDisplay = true } }

    init() {
        super.init(frame: .zero)
        wantsLayer = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    public override var wantsUpdateLayer: Bool { true }

    public override func updateLayer() {
        chrome.apply(
            to: layer, appearance: effectiveAppearance, ringed: ringed, radius: radius,
            fill: ringed ? .textBackgroundColor : .quaternarySystemFill, hairline: ringed ? 0 : 0.5)
    }

    public override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }

    public override func layout() {
        super.layout()
        chrome.layout(in: bounds)
    }

    public override func mouseDown(with event: NSEvent) {
        onClick?()
    }
}

@MainActor
public final class SearchFieldChrome {
    private let halo = CALayer()

    public static let haloWidth: CGFloat = 4
    static let haloAlpha: CGFloat = 0.25
    static let ringWidth: CGFloat = 2

    func apply(
        to layer: CALayer?, appearance: NSAppearance, ringed: Bool, radius: CGFloat, fill: NSColor,
        hairline: CGFloat
    ) {
        guard let layer else { return }
        if halo.superlayer !== layer {
            layer.masksToBounds = false
            layer.addSublayer(halo)
        }
        appearance.performAsCurrentDrawingAppearance {
            layer.cornerRadius = radius
            layer.backgroundColor = fill.cgColor
            layer.borderWidth = ringed ? Self.ringWidth : hairline
            layer.borderColor = (ringed ? NSColor.controlAccentColor : NSColor.separatorColor).cgColor
            halo.isHidden = !ringed
            halo.cornerRadius = radius + Self.haloWidth
            halo.borderWidth = Self.haloWidth
            halo.borderColor = NSColor.controlAccentColor.withAlphaComponent(Self.haloAlpha).cgColor
        }
    }

    func layout(in bounds: CGRect) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        halo.frame = bounds.insetBy(dx: -Self.haloWidth, dy: -Self.haloWidth)
        CATransaction.commit()
    }
}
