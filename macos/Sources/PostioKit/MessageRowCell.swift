import AppKit
import PostioFFI

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
    private let time = NSTextField(labelWithString: "")
    /// The avatar chip: a tinted round square with two letters in it.
    private let avatar = NSView()
    private let avatarLabel = NSTextField(labelWithString: "")
    /// The tint behind a *marked* row.
    ///
    /// Behind everything else and inset, so it reads as a state of the row
    /// rather than as a second highlight competing with `NSTableView`'s own —
    /// which is the cursor, and which a marked row may or may not also be.
    private let marked = NSView()
    /// The vertical stack, kept so density can retune its spacing rather than
    /// rebuild the cell.
    private var stack: NSStackView?
    /// The horizontal pair: the chip and the text column beside it.
    private var content: NSStackView?
    private lazy var avatarWidth = avatar.widthAnchor.constraint(equalToConstant: 30)
    private lazy var avatarHeight = avatar.heightAnchor.constraint(equalToConstant: 30)
    private lazy var padTop = content!.topAnchor.constraint(
        equalTo: topAnchor, constant: PostioTokens.space2)
    private lazy var padBottom = content!.bottomAnchor.constraint(
        lessThanOrEqualTo: bottomAnchor, constant: -PostioTokens.space2)

    /// How much of canvas 1b's row anatomy to draw.
    ///
    /// The setting is `[ui].density` and the numbers are
    /// `postio_ui::row::Metrics`, which GTK lays out with — so Compact means
    /// the same thing on both platforms rather than "a bit tighter, somehow".
    /// The visible part is the snippet: the tightest density drops it, which
    /// is what makes it the tightest.
    public var ui: AppearanceFfi = AppearanceFfi(
        density: .airy,
        theme: .system,
        showHoverActions: true,
        showKeyHints: true,
        senderAvatars: true
    ) {
        didSet { applyDensity() }
    }

    /// Shorthand for the one field most of this cell cares about.
    ///
    /// `ui` rather than `appearance` because `NSView.appearance` is
    /// `NSAppearance` and shadowing it compiles into something else entirely.
    public var density: DensityFfi {
        get { ui.density }
        set { ui.density = newValue }
    }

    /// Whether the snippet line is drawn. Read back rather than inferred, so
    /// a test can assert the row a person sees.
    public var previewIsHiddenForTesting: Bool { preview.isHidden }

    private func applyDensity() {
        let metrics = rowMetrics(density: density)
        preview.isHidden = !metrics.snippet
        stack?.spacing = CGFloat(metrics.subjectGap)
        padTop.constant = CGFloat(metrics.padY)
        padBottom.constant = -CGFloat(metrics.padY)

        // The chip is square and the density decides how big — 30/26/22,
        // the same numbers GTK lays out with.
        avatar.isHidden = !ui.senderAvatars
        avatarWidth.constant = CGFloat(metrics.avatar)
        avatarHeight.constant = CGFloat(metrics.avatar)
        avatar.layer?.cornerRadius = CGFloat(metrics.avatar) / 2
        avatarLabel.font = .systemFont(ofSize: CGFloat(metrics.avatar) * 0.4, weight: .medium)
        content?.spacing = CGFloat(metrics.gap)
    }

    /// Whether the chip is drawn. `sender_avatars` off means the row gets the
    /// space back, not that it draws an empty circle.
    public var avatarIsHiddenForTesting: Bool { avatar.isHidden }

    /// The chip's side, for asserting it follows the density.
    public var avatarSizeForTesting: CGFloat { avatarWidth.constant }

    /// What the row is currently showing, beyond its three text lines.
    public var drawnForTesting: (initials: String, time: String) {
        (avatarLabel.stringValue, time.stringValue)
    }

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

        time.font = .systemFont(ofSize: 11)
        time.textColor = .secondaryLabelColor
        // The sender stretches and the time is pushed to the trailing edge,
        // which is where canvas 1b puts it.
        sender.setContentHuggingPriority(.defaultLow, for: .horizontal)
        time.setContentHuggingPriority(.required, for: .horizontal)
        time.setContentCompressionResistancePriority(.required, for: .horizontal)

        avatar.wantsLayer = true
        avatar.layer?.backgroundColor = PostioTokens.colorAccent.withAlphaComponent(0.22).cgColor
        avatar.translatesAutoresizingMaskIntoConstraints = false
        avatarLabel.alignment = .center
        avatarLabel.textColor = .secondaryLabelColor
        avatarLabel.translatesAutoresizingMaskIntoConstraints = false
        avatar.addSubview(avatarLabel)
        NSLayoutConstraint.activate([
            avatarWidth,
            avatarHeight,
            avatarLabel.centerXAnchor.constraint(equalTo: avatar.centerXAnchor),
            avatarLabel.centerYAnchor.constraint(equalTo: avatar.centerYAnchor),
        ])

        let top = NSStackView(views: [unreadDot, sender, badge, flag, time])
        top.orientation = .horizontal
        top.spacing = PostioTokens.space2
        top.alignment = .centerY

        let stack = NSStackView(views: [top, subject, preview])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = PostioTokens.space1
        stack.translatesAutoresizingMaskIntoConstraints = false
        self.stack = stack

        // The chip sits beside the whole text column, top-aligned, which is
        // the anatomy canvas 1b draws.
        let content = NSStackView(views: [avatar, stack])
        content.orientation = .horizontal
        content.alignment = .top
        content.spacing = PostioTokens.space2
        content.translatesAutoresizingMaskIntoConstraints = false
        addSubview(content)
        self.content = content
        NSLayoutConstraint.activate([
            // Spacing from the design system rather than numbers chosen here.
            content.leadingAnchor.constraint(equalTo: leadingAnchor, constant: PostioTokens.space3),
            content.trailingAnchor.constraint(
                equalTo: trailingAnchor, constant: -PostioTokens.space3),
            padTop,
            padBottom,

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

    /// How tall a row has to be for its three lines to fit.
    ///
    /// Derived rather than chosen. It was a literal `62` in the table, and 62
    /// is not enough: sender, subject and preview at their token sizes, with
    /// the token gaps between them and the token padding around them, come to
    /// about 68 — so **every row after the first had its sender clipped**, in
    /// a way no test could see and a screenshot showed immediately.
    ///
    /// Computed from the same tokens the layout uses, so the two cannot drift
    /// apart again, and `aRowIsTallEnoughForItsContents` measures a real laid
    /// out cell against it rather than trusting this arithmetic.
    public static func preferredHeight(for density: DensityFfi = .airy) -> CGFloat {
        let sender = NSFont(name: PostioTokens.fontBody, size: 13)
            ?? .systemFont(ofSize: 13, weight: .semibold)
        let subject = NSFont(name: PostioTokens.fontBody, size: 13) ?? .systemFont(ofSize: 13)
        let preview = NSFont.systemFont(ofSize: 12)
        let metrics = rowMetrics(density: density)
        // The snippet is a line at airy and snug and no line at all at
        // compact, so it counts for its height and for one of the gaps.
        let lines = ceil(sender.boundingRectForFont.height)
            + ceil(subject.boundingRectForFont.height)
            + (metrics.snippet ? ceil(preview.boundingRectForFont.height) : 0)
        let gaps = CGFloat(metrics.subjectGap) * (metrics.snippet ? 2 : 1)
        return ceil(lines + gaps + CGFloat(metrics.padY) * 2)
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
        // In search results the row shows *why it matched*, with the matched
        // spans emphasised, rather than its own first line. In a folder there
        // is no excerpt and the preview is what there is to show.
        // Two reasons this line can be absent, and they compose: there may be
        // nothing to say, or the density may have decided the row does not
        // carry a third line at all. Setting it from the content alone -- as
        // this did -- silently undid the density on every reuse.
        let drawsSnippet = rowMetrics(density: density).snippet
        if let snippet = presentation.snippet {
            preview.attributedStringValue = NSAttributedString(
                PaletteRow.highlighted(snippet)
            )
            preview.isHidden = !drawsSnippet || snippet.text.isEmpty
        } else {
            preview.stringValue = presentation.preview
            preview.isHidden = !drawsSnippet || presentation.preview.isEmpty
        }

        avatarLabel.stringValue = presentation.initials
        time.stringValue = presentation.time
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
