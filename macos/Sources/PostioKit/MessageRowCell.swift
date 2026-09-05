import AppKit

/// One row of the message list.
///
/// Stock views in a stack rather than a custom `draw(_:)`. The GTK row draws
/// itself because it needed one snapshot per row at scroll speed; AppKit's
/// cell reuse gets to the same place without hand-drawing, and hand-drawing
/// would mean re-deriving the layout the design system already describes.
public final class MessageRowCell: NSTableCellView {
    private let unreadDot = NSView()
    private let sender = NSTextField(labelWithString: "")
    private let subject = NSTextField(labelWithString: "")
    private let preview = NSTextField(labelWithString: "")
    private let badge = NSTextField(labelWithString: "")
    private let flag = NSImageView()
    /// The tint behind a *marked* row.
    ///
    /// Behind everything else and inset, so it reads as a state of the row
    /// rather than as a second highlight competing with `NSTableView`'s own —
    /// which is the cursor, and which a marked row may or may not also be.
    private let marked = NSView()

    public override init(frame: NSRect) {
        super.init(frame: frame)
        build()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("MessageRowCell is built in code, never from a nib")
    }

    private func build() {
        marked.wantsLayer = true
        marked.layer?.cornerRadius = PostioTokens.radiusSm
        marked.layer?.backgroundColor = PostioTokens.colorAccent.withAlphaComponent(0.18).cgColor
        marked.translatesAutoresizingMaskIntoConstraints = false
        marked.isHidden = true
        addSubview(marked)

        unreadDot.wantsLayer = true
        unreadDot.layer?.cornerRadius = 4
        // Postio's accent, from the design system, not the system's — the
        // canvas is the visual truth and a row that followed the user's
        // accent colour would stop matching it.
        unreadDot.layer?.backgroundColor = PostioTokens.colorAccent.cgColor
        unreadDot.translatesAutoresizingMaskIntoConstraints = false
        unreadDot.widthAnchor.constraint(equalToConstant: 8).isActive = true
        unreadDot.heightAnchor.constraint(equalToConstant: 8).isActive = true

        sender.font = NSFont(name: PostioTokens.fontBody, size: 13)
            ?? .systemFont(ofSize: 13, weight: .semibold)
        sender.lineBreakMode = .byTruncatingTail
        subject.font = NSFont(name: PostioTokens.fontBody, size: 13) ?? .systemFont(ofSize: 13)
        subject.lineBreakMode = .byTruncatingTail
        preview.font = .systemFont(ofSize: 12)
        preview.textColor = .secondaryLabelColor
        preview.lineBreakMode = .byTruncatingTail

        badge.font = .systemFont(ofSize: 11, weight: .medium)
        badge.textColor = .secondaryLabelColor

        flag.image = NSImage(systemSymbolName: "flag.fill", accessibilityDescription: "Flagged")
        flag.contentTintColor = .systemOrange

        let top = NSStackView(views: [unreadDot, sender, badge, flag])
        top.orientation = .horizontal
        top.spacing = PostioTokens.space2
        top.alignment = .centerY

        let stack = NSStackView(views: [top, subject, preview])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = PostioTokens.space1
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            // Spacing from the design system rather than numbers chosen here.
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: PostioTokens.space3),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -PostioTokens.space3),
            stack.topAnchor.constraint(equalTo: topAnchor, constant: PostioTokens.space2),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -PostioTokens.space2),

            marked.leadingAnchor.constraint(equalTo: leadingAnchor, constant: PostioTokens.space1),
            marked.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -PostioTokens.space1),
            marked.topAnchor.constraint(equalTo: topAnchor),
            marked.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    /// Whether this cell is currently drawn as marked.
    ///
    /// For the same reason `renderedForTesting` exists: the mark is a state
    /// the cell has to *clear* on reuse, and a cell that only ever set it
    /// would show the previous row's mark on this row's message — which
    /// misreports what an action is about to hit.
    public var isMarkedForTesting: Bool { !marked.isHidden }

    /// What this cell is currently showing.
    ///
    /// Exists so reuse can be asserted: the recycled-cell bug leaves the
    /// previous row's text in a field the new row did not set, and the only
    /// way to see that is to read the fields back.
    public var renderedForTesting: (sender: String, subject: String, preview: String) {
        (sender.stringValue, subject.stringValue, preview.stringValue)
    }

    /// Draw `presentation`.
    ///
    /// Every field is set on every call, including to its empty value. A cell
    /// that only set the fields it had would show the previous row's subject
    /// under this row's sender after reuse — the classic recycled-cell bug,
    /// and one that looks like a data problem rather than a drawing one.
    public func show(_ presentation: RowPresentation) {
        sender.stringValue = presentation.sender
        subject.stringValue = presentation.subject
        // In search results the row shows *why it matched*, with the matched
        // spans emphasised, rather than its own first line. In a folder there
        // is no excerpt and the preview is what there is to show.
        if let snippet = presentation.snippet {
            preview.attributedStringValue = NSAttributedString(
                PaletteRow.highlighted(snippet)
            )
            preview.isHidden = snippet.text.isEmpty
        } else {
            preview.stringValue = presentation.preview
            preview.isHidden = presentation.preview.isEmpty
        }

        unreadDot.isHidden = !presentation.unread
        flag.isHidden = !presentation.flagged
        badge.stringValue = presentation.threadBadge ?? ""
        badge.isHidden = presentation.threadBadge == nil

        // A row still waiting for its page is dimmed rather than blank, so
        // "not here yet" reads differently from "nothing here".
        alphaValue = presentation.isPlaceholder ? 0.45 : 1

        // Marked rows are tinted; the cursor is `NSTableView`'s own highlight.
        // Two different things drawn two different ways, because they are two
        // different things (`PRODUCT.md` §9) -- a row can be either, both or
        // neither, and a user who cannot tell which cannot tell what an
        // action is about to hit.
        marked.isHidden = !presentation.selected

        // One utterance, not four. `Announcements.row` decides what it says;
        // this is only where it is hung. The children are hidden from the
        // accessibility tree so VoiceOver reads the row rather than walking
        // the six labels it is drawn from.
        setAccessibilityElement(true)
        setAccessibilityRole(.row)
        setAccessibilityLabel(Announcements.row(presentation))
        for child in [unreadDot, sender, subject, preview, badge, flag] as [NSView] {
            child.setAccessibilityElement(false)
        }
    }
}
