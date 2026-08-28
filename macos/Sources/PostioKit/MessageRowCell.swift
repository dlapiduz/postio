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

    public override init(frame: NSRect) {
        super.init(frame: frame)
        build()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("MessageRowCell is built in code, never from a nib")
    }

    private func build() {
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
        ])
    }

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
        preview.stringValue = presentation.preview
        preview.isHidden = presentation.preview.isEmpty

        unreadDot.isHidden = !presentation.unread
        flag.isHidden = !presentation.flagged
        badge.stringValue = presentation.threadBadge ?? ""
        badge.isHidden = presentation.threadBadge == nil

        // A row still waiting for its page is dimmed rather than blank, so
        // "not here yet" reads differently from "nothing here".
        alphaValue = presentation.isPlaceholder ? 0.45 : 1
    }
}
