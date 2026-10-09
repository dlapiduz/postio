import AppKit
import PostioFFI
import PostioKit

/// The results' query field (specs/010-focus-search T070; design §1 and
/// §3.1; research R11): the chips first, in the query's order, then the
/// plain words. An `NSTextView` whose chips are text attachments, because
/// `NSTokenField` can neither hold free words between tokens nor strike an
/// excluded one through.
///
/// It draws what `FocusQuery` said (`show`) and reports what the keyboard
/// did; it never edits the query itself, and no query text is spelled
/// here:
///
/// - Backspace at the start of the words selects the last chip, and a
///   second one removes it: `onRemove` with its token.
/// - A click, or `/`, asks for the dropdown (`onFocus`): editing happens
///   in the bar's field, on the query text the controller hands it.
/// - Anything typed is handed on (`onType`) for the bar to take, rather
///   than changing words the controller would not hear of.
@MainActor
public final class ChipQueryField: NSTextView {
    /// A chip's token, to remove.
    public var onRemove: ((UInt32) -> Void)?
    /// The field wants the dropdown: a click, or `/`.
    public var onFocus: (() -> Void)?
    /// Words typed into the field, for the bar's field to take.
    public var onType: ((String) -> Void)?

    /// The chips drawn, in order.
    public private(set) var chips: [SearchQueryModel.Chip] = []

    /// Where the words start: after the chips, and the space between.
    private var wordsStart = 0

    /// The words' size (§3.1: 14.5).
    nonisolated static var wordsFont: NSFont { NSFont.systemFont(ofSize: 14.5) }

    public override init(frame frameRect: NSRect, textContainer container: NSTextContainer?) {
        super.init(frame: frameRect, textContainer: container)
        configure()
    }

    public convenience override init(frame: NSRect) {
        let storage = NSTextStorage()
        let layout = NSLayoutManager()
        storage.addLayoutManager(layout)
        let container = NSTextContainer(size: NSSize(width: CGFloat.greatestFiniteMagnitude, height: frame.height))
        container.widthTracksTextView = false
        container.lineFragmentPadding = 0
        layout.addTextContainer(container)
        self.init(frame: frame, textContainer: container)
        // A text view holds its container, which holds its layout manager
        // only weakly, and that its storage: hold the storage here.
        owned = storage
    }

    /// The storage this field built (TextKit 1, for its attachments).
    private var owned: NSTextStorage?

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    private func configure() {
        isRichText = true
        isEditable = true
        isSelectable = true
        allowsUndo = false
        drawsBackground = false
        isHorizontallyResizable = true
        isVerticallyResizable = false
        textContainerInset = NSSize(width: 0, height: 7)
        font = Self.wordsFont
        textColor = .labelColor
        insertionPointColor = .controlAccentColor
        focusRingType = .none
        setAccessibilityRole(.comboBox)
        setAccessibilityLabel(focusSearchWords().queryLabel)
    }

    /// The attachments in the text, in order: what is drawn.
    public var chipAttachments: [ChipAttachment] {
        guard let storage = textStorage else { return [] }
        var found: [ChipAttachment] = []
        storage.enumerateAttribute(.attachment, in: NSRange(location: 0, length: storage.length)) { value, _, _ in
            if let chip = value as? ChipAttachment { found.append(chip) }
        }
        return found
    }

    /// The plain words after the chips.
    public var words: String {
        guard let storage = textStorage else { return "" }
        let text = storage.string as NSString
        let start = min(chips.count, text.length)
        return text.substring(from: start).trimmingCharacters(in: .whitespaces)
    }

    /// Draw `chips` then `words`, as `FocusQuery` said.
    public func show(chips: [SearchQueryModel.Chip], words: String) {
        self.chips = chips
        let text = NSMutableAttributedString()
        for chip in chips {
            text.append(NSAttributedString(attachment: ChipAttachment(chip: chip)))
        }
        let wordsAttributes: [NSAttributedString.Key: Any] = [
            .font: Self.wordsFont, .foregroundColor: NSColor.labelColor,
        ]
        wordsStart = chips.count
        if !words.isEmpty {
            if !chips.isEmpty {
                text.append(NSAttributedString(string: " ", attributes: wordsAttributes))
                wordsStart += 1
            }
            text.append(NSAttributedString(string: words, attributes: wordsAttributes))
        }
        textStorage?.setAttributedString(text)
        typingAttributes = wordsAttributes
        setSelectedRange(NSRange(location: text.length, length: 0))
    }

    /// What VoiceOver reads: each chip as its term, "not" before an
    /// excluded one, then the words. (Setting a text view's accessibility
    /// value replaces its text, so it is answered rather than set.)
    public override func accessibilityValue() -> String? {
        (chips.map { ($0.excluded ? "not " : "") + $0.op + $0.value } + [words])
            .filter { !$0.isEmpty }
            .joined(separator: " ")
    }

    // MARK: the keyboard

    /// Take the keyboard and ask for the dropdown: a click, and the path
    /// `/` takes.
    public func focusAndAsk() {
        if window?.firstResponder !== self { window?.makeFirstResponder(self) }
        onFocus?()
    }

    public override func mouseDown(with event: NSEvent) {
        // A click on a chip's × removes it; anywhere else edits the query.
        let point = convert(event.locationInWindow, from: nil)
        if let token = crossed(at: point) {
            onRemove?(token)
            return
        }
        focusAndAsk()
    }

    /// The token of the chip whose × is under `point`, in the view's
    /// coordinates.
    func crossed(at point: NSPoint) -> UInt32? {
        guard let layout = layoutManager, let container = textContainer else { return nil }
        let origin = textContainerOrigin
        for (index, chip) in chips.enumerated() {
            let glyphs = layout.glyphRange(forCharacterRange: NSRange(location: index, length: 1), actualCharacterRange: nil)
            var frame = layout.boundingRect(forGlyphRange: glyphs, in: container)
            frame.origin.x += origin.x
            frame.origin.y += origin.y
            let pillEnd = frame.maxX - ChipAttachment.gap
            let cross = NSRect(
                x: pillEnd - 4 - ChipAttachment.cross - 3, y: frame.minY,
                width: ChipAttachment.cross + 6, height: frame.height)
            if cross.contains(point) { return chip.id }
        }
        return nil
    }

    public override func keyDown(with event: NSEvent) {
        if event.charactersIgnoringModifiers == "/",
           event.modifierFlags.intersection([.command, .control, .option]).isEmpty
        {
            focusAndAsk()
            return
        }
        super.keyDown(with: event)
    }

    public override func insertText(_ string: Any, replacementRange: NSRange) {
        let text = (string as? NSAttributedString)?.string ?? (string as? String) ?? ""
        guard !text.isEmpty else { return }
        onType?(text)
    }

    public override func deleteBackward(_ sender: Any?) {
        let selection = selectedRange()
        if selection.length == 1, selection.location < chips.count {
            // A chip is selected: the second Backspace removes it.
            onRemove?(chips[selection.location].id)
            return
        }
        if selection.length == 0, selection.location <= wordsStart, !chips.isEmpty {
            // At the start of the words: select the chip before.
            let before = min(selection.location, chips.count)
            setSelectedRange(NSRange(location: max(before, 1) - 1, length: 1))
            return
        }
        // Inside the words: the bar edits them, on the query text.
        onType?("")
    }

    public override func insertNewline(_ sender: Any?) {
        focusAndAsk()
    }

    public override func insertTab(_ sender: Any?) {
        window?.selectNextKeyView(sender)
    }

    public override func insertBacktab(_ sender: Any?) {
        window?.selectPreviousKeyView(sender)
    }
}

/// One chip in the field (design §1): SF Mono 12.5, 26 tall, radius 6,
/// the quaternary fill, the operator tertiary, the value in label colour,
/// and × -- struck through when excluded, the accent ring when focused.
///
/// Drawn by a handler at draw time, so the semantic colours follow the
/// appearance the field is drawn in.
public final class ChipAttachment: NSTextAttachment {
    public let chip: SearchQueryModel.Chip

    static var font: NSFont { NSFont.monospacedSystemFont(ofSize: 12.5, weight: .regular) }
    static var valueFont: NSFont { NSFont.monospacedSystemFont(ofSize: 12.5, weight: .medium) }
    static let height: CGFloat = 26
    /// The space after a chip, before the next one or the words.
    static let gap: CGFloat = 6
    static let cross: CGFloat = 12

    /// The operator and the value as drawn, struck through when excluded.
    public var label: NSAttributedString {
        let strike: [NSAttributedString.Key: Any] = chip.excluded
            ? [.strikethroughStyle: NSUnderlineStyle.single.rawValue] : [:]
        let text = NSMutableAttributedString(
            string: chip.op,
            attributes: [.font: Self.font, .foregroundColor: NSColor.tertiaryLabelColor].merging(strike) { $1 })
        // Screen 10's "from: Ada Moreno": 4 points between the operator and
        // its value.
        if text.length > 0 {
            text.addAttribute(.kern, value: 4, range: NSRange(location: text.length - 1, length: 1))
        }
        text.append(NSAttributedString(
            string: chip.value,
            attributes: [.font: Self.valueFont, .foregroundColor: NSColor.labelColor].merging(strike) { $1 }))
        return text
    }

    public init(chip: SearchQueryModel.Chip) {
        self.chip = chip
        super.init(data: nil, ofType: nil)
        let label = self.label
        let textWidth = ceil(label.size().width)
        // 8 before the words, 4 between them and ×, 4 after it.
        let pill = NSSize(width: 8 + textWidth + 4 + Self.cross + 4, height: Self.height)
        let size = NSSize(width: pill.width + Self.gap, height: pill.height)
        let focused = chip.focused
        image = NSImage(size: size, flipped: false) { _ in
            let shape = NSBezierPath(
                roundedRect: NSRect(origin: .zero, size: pill).insetBy(dx: 0.5, dy: 0.5), xRadius: 6, yRadius: 6)
            NSColor.quaternaryLabelColor.setFill()
            shape.fill()
            if focused {
                NSColor.controlAccentColor.setStroke()
                shape.lineWidth = 2
                shape.stroke()
            }
            let textHeight = label.size().height
            label.draw(at: NSPoint(x: 8, y: (pill.height - textHeight) / 2))
            // The ×, tertiary.
            let cross = NSBezierPath()
            let box = NSRect(x: 8 + textWidth + 4, y: (pill.height - Self.cross) / 2, width: Self.cross, height: Self.cross)
                .insetBy(dx: 3, dy: 3)
            cross.move(to: NSPoint(x: box.minX, y: box.minY))
            cross.line(to: NSPoint(x: box.maxX, y: box.maxY))
            cross.move(to: NSPoint(x: box.minX, y: box.maxY))
            cross.line(to: NSPoint(x: box.maxX, y: box.minY))
            cross.lineWidth = 1.3
            cross.lineCapStyle = .round
            NSColor.tertiaryLabelColor.setStroke()
            cross.stroke()
            return true
        }
        // Centred on the words' line rather than sat on its baseline.
        bounds = NSRect(x: 0, y: (ChipQueryField.wordsFont.capHeight - Self.height) / 2, width: size.width, height: size.height)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }
}

/// The results toolbar's query box (specs/010-focus-search T070; design
/// §3.1): 34 tall, radius 8, on the text background with a hairline ring
/// -- the magnifier, the chips and words (`ChipQueryField`), and the quiet
/// hint on the right, "/ to edit".
///
/// While the dropdown is up the query is edited as text, in the bar's own
/// search field (`editor`), laid over the chips in the same place: the
/// controller hands the bar the query text and reads it back, so nothing
/// here spells it. It is ringed then, as the inbox's field is when it
/// opens (`ToolbarSearchBox`, the same `SearchFieldChrome`), and while
/// nothing matches (screen 13), when the hint says how to clear filters.
@MainActor
public final class ResultsQueryBox: NSView {
    public let field = ChipQueryField(frame: NSRect(x: 0, y: 0, width: 400, height: 34))
    public let editor = BarSearchField()
    private let hint = NSTextField(labelWithString: "")
    private let magnifier = NSImageView()
    private let chrome = SearchFieldChrome()

    /// The box's height (§3.1).
    public static let height: CGFloat = 34

    public init() {
        super.init(frame: NSRect(x: 0, y: 0, width: 600, height: Self.height))
        wantsLayer = true
        translatesAutoresizingMaskIntoConstraints = false
        magnifier.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)
        magnifier.contentTintColor = .secondaryLabelColor
        magnifier.symbolConfiguration = .init(pointSize: 13, weight: .regular)
        hint.font = .systemFont(ofSize: 12)
        hint.textColor = .tertiaryLabelColor
        hint.setContentCompressionResistancePriority(.required, for: .horizontal)
        editor.isHidden = true
        editor.isBezeled = false
        editor.isBordered = false
        editor.drawsBackground = false
        editor.focusRingType = .none
        if let cell = editor.cell as? NSSearchFieldCell {
            cell.usesSingleLineMode = true
            cell.lineBreakMode = .byClipping
            cell.isScrollable = true
        }
        // §2: the words 15 pt while the query is edited as text.
        editor.textFont = .systemFont(ofSize: 15)
        editor.frameView = self
        for view in [magnifier, field, hint, editor] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        let width = widthAnchor.constraint(equalToConstant: 600)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: Self.height),
            width,
            magnifier.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            magnifier.centerYAnchor.constraint(equalTo: centerYAnchor),
            field.leadingAnchor.constraint(equalTo: magnifier.trailingAnchor, constant: 6),
            field.topAnchor.constraint(equalTo: topAnchor),
            field.bottomAnchor.constraint(equalTo: bottomAnchor),
            field.trailingAnchor.constraint(equalTo: hint.leadingAnchor, constant: -8),
            hint.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            hint.centerYAnchor.constraint(equalTo: centerYAnchor),
            editor.leadingAnchor.constraint(equalTo: magnifier.trailingAnchor, constant: 6),
            editor.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            editor.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        widthConstraint = width
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not from a nib") }

    private var widthConstraint: NSLayoutConstraint?

    /// How wide the box is: what the toolbar leaves it.
    public var boxWidth: CGFloat {
        get { widthConstraint?.constant ?? 0 }
        set { widthConstraint?.constant = newValue }
    }

    /// Draw the query as `FocusQuery` said it, with its hint; `nothingFound`
    /// rings the field (screen 13).
    public func show(chips: [SearchQueryModel.Chip], words: String, hint: String, nothingFound: Bool = false) {
        field.show(chips: chips, words: words)
        self.hint.stringValue = hint
        self.nothingFound = nothingFound
    }

    /// Nothing matches: the field is ringed while the page is up.
    public private(set) var nothingFound = false {
        didSet { needsDisplay = true }
    }

    /// The dropdown is up: the query is edited as text in `editor`.
    public var editing: Bool = false {
        didSet {
            editor.isHidden = !editing
            editor.listOpen = editing
            field.isHidden = editing
            hint.isHidden = editing
            needsDisplay = true
        }
    }

    /// Whether the accent ring and halo are drawn.
    public var ringed: Bool { editing || nothingFound }

    public override var wantsUpdateLayer: Bool { true }

    public override func updateLayer() {
        chrome.apply(
            to: layer, appearance: effectiveAppearance, ringed: ringed, radius: 8,
            fill: .textBackgroundColor, hairline: 1)
    }

    public override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }

    public override func layout() {
        super.layout()
        chrome.layout(in: bounds)
    }
}
