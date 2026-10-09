import AppKit
import PostioKit

/// The inbox toolbar's search field (specs/010-focus-search T054; design
/// §2 "Opening and layout"; McList.dc.html and McSearchType.dc.html).
///
/// Postio's own view in a plain `NSToolbarItem`, because
/// `NSSearchToolbarItem` draws its own field and ignored every width it was
/// given -- `preferredWidthForSearchField`, constraints, a minimum and a
/// maximum -- and stayed about 327 wide (step 8's note), so the dropdown
/// hung from a field a third of its width.
///
/// At rest it is 320 by 28, radius 7, on the control fill with a hairline,
/// the placeholder 13 pt and the palette's keycap on the right. Opened --
/// ⌘K or `/`, or a click -- it grows leftward to `CommandBarGeometry`'s
/// width, its right edge where it was, 34 tall, radius 8, on the text
/// background with a 2 pt accent ring and a 4 pt soft accent halo; the
/// words are 15 pt (14 pt SF Mono while an operator is typed), and the cap
/// is Escape's. The change is not animated: the panel hangs from where the
/// field ends up, and a transition is ≤100 ms or absent (PRODUCT.md §18).
///
/// The words are typed into `field`, the bar's `BarSearchField`, which the
/// engine drives as it drove the toolbar item's: focus, typing, the
/// field-command selectors, the ghost, the mono face. Nothing here decides
/// what the bar does.
@MainActor
public final class ToolbarSearchBox: NSView {
    /// The text the bar edits.
    public let field = BarSearchField()
    /// The field as drawn: what the panel hangs from. It sits `inset` in
    /// from the toolbar item's edges, which leaves the halo room inside
    /// the item; on the right, whatever puts its edge 12 from the
    /// window's, since the room a toolbar keeps after its last item is the
    /// toolbar's (8 in a bare one, 12 in the app's window).
    public let frameView = SearchFieldFrameView()
    private let magnifier = NSImageView()
    private let cap = KeyCapView("")
    private var widthConstraint: NSLayoutConstraint?
    private var heightConstraint: NSLayoutConstraint?
    private var magnifierLeading: NSLayoutConstraint?
    private var trailingInset: NSLayoutConstraint?

    /// The resting size (McList.dc.html: 320 by 28).
    public static let restingHeight: CGFloat = 28
    /// The opened height (§2: 34).
    public static let openHeight: CGFloat = 34
    /// Between the item's edges and the field's: the halo's width, and
    /// what takes the toolbar's 8 to the design's 12.
    public static let inset: CGFloat = SearchFieldChrome.haloWidth

    /// Whether the accent ring and halo are drawn: while the bar is up.
    public var ringed: Bool { frameView.ringed }

    /// The keycaps it shows: the palette's at rest, Escape's while open.
    /// `nil` draws none.
    public var restingCap: String? { didSet { placeCap() } }
    public var openCap: String? { didSet { placeCap() } }

    /// The placeholder, as the controller words it.
    public var placeholder: String = "" { didSet { applyFonts() } }

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

    /// A toolbar item that holds this box.
    public func item(_ identifier: NSToolbarItem.Identifier) -> NSToolbarItem {
        let item = NSToolbarItem(itemIdentifier: identifier)
        item.view = self
        item.label = "Search"
        item.visibilityPriority = .high
        return item
    }

    /// Open it -- grown to the bar's width for a window `windowWidth` wide,
    /// ringed -- or put it back at rest.
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

    /// Put the drawn edge 12 from the window's, whatever room the toolbar
    /// keeps after its last item: lay the toolbar out, read where it put
    /// the item, and take up the difference inside it. The toolbar's room
    /// does not depend on the item's width, so one correction holds.
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

    /// Typing hides the resting keycap, as a field with words has no
    /// room to teach its key.
    public func textChanged() {
        placeCap()
    }

    private func applyFonts() {
        field.textFont = .systemFont(ofSize: ringed ? 15 : 13)
        magnifier.symbolConfiguration = .init(pointSize: ringed ? 14 : 13, weight: .regular)
        magnifier.contentTintColor = ringed ? .secondaryLabelColor : .tertiaryLabelColor
        field.placeholderAttributedString = NSAttributedString(
            string: placeholder,
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

/// The inbox field's drawn frame: the control fill and a hairline at rest
/// (McList.dc.html), the text background, accent ring and halo while open.
@MainActor
public final class SearchFieldFrameView: NSView {
    private let chrome = SearchFieldChrome()
    var onClick: (() -> Void)?

    /// Ringed: the bar is up.
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

    /// A click anywhere on the field is a click into its text.
    public override func mouseDown(with event: NSEvent) {
        onClick?()
    }
}

/// The search field's frame as both toolbars draw it (design §2 and
/// §3.1): a rounded fill with a hairline at rest; ringed -- a 2 pt accent
/// ring inside the edge and a 4 pt soft accent halo outside it -- while the
/// query is being edited, or while nothing matches (screen 13).
@MainActor
public final class SearchFieldChrome {
    /// The halo, outside the field's bounds.
    private let halo = CALayer()

    /// The halo's width and its strength (McSearchType.dc.html's
    /// `0 0 0 4px var(--accsoft)`).
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
