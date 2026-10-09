import AppKit
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
        setAccessibilityRole(.textField)
        setAccessibilityLabel("Search query")
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
