import PostioFFI
import SwiftUI

/// The blocked-images notice, and the grants behind it (#1274, canvas 26).
///
/// One row that never wraps: an icon, what was held back, `Show`, and a `⋯`
/// that opens an anchored popover — not a menu strip, because the choices are
/// about *this sender* and a menu that appeared at the pointer would not say
/// so.
///
/// Per message, never per pane. A conversation can hold back pictures from
/// three different senders, and one notice above them all could not say
/// whose.
public struct BlockedImagesNotice: View {
    private let notice: ReaderNoticeFfi
    private let session: PostioSession
    private let show: () -> Void
    private let openSettings: () -> Void

    @State private var showingGrants = false
    @State private var showingDetail = false

    public init(
        notice: ReaderNoticeFfi,
        session: PostioSession,
        show: @escaping () -> Void,
        openSettings: @escaping () -> Void
    ) {
        self.notice = notice
        self.session = session
        self.show = show
        self.openSettings = openSettings
    }

    public var body: some View {
        HStack(spacing: PostioTokens.space3) {
            Image(systemName: "eye.slash")
                .foregroundStyle(.secondary)
            Text(notice.summary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: PostioTokens.space2)
            Button("Show", action: show)
                .controlSize(.small)
            Button {
                showingGrants = true
            } label: {
                Image(systemName: "ellipsis")
            }
            .controlSize(.small)
            .accessibilityLabel("What to do about this sender's images")
            .popover(isPresented: $showingGrants, arrowEdge: .bottom) {
                grants
            }
        }
        .padding(.horizontal, PostioTokens.space3)
        .padding(.vertical, PostioTokens.space2)
        .background(.quaternary.opacity(0.4), in: .rect(cornerRadius: PostioTokens.radiusMd))
        .accessibilityElement(children: .contain)
        .popover(isPresented: $showingDetail, arrowEdge: .bottom) {
            detail
        }
    }

    /// The four choices, in the canvas' order: the narrow grant, the wide
    /// one, an explanation, and the settings that hold every grant made.
    private var grants: some View {
        VStack(alignment: .leading, spacing: 0) {
            item("Always allow \(BlockedImagesNotice.shortened(notice.sender))") {
                session.allowSender(notice.sender)
                showingGrants = false
                show()
            }
            item("Always allow this domain") {
                session.allowDomain(notice.domain)
                showingGrants = false
                show()
            }
            Divider()
            item("What was blocked?") {
                showingGrants = false
                showingDetail = true
            }
            item("Privacy Settings…") {
                showingGrants = false
                openSettings()
            }
        }
        .padding(.vertical, PostioTokens.space2)
        .frame(minWidth: 260)
    }

    private var detail: some View {
        VStack(alignment: .leading, spacing: PostioTokens.space3) {
            Text(notice.summary)
                .fontWeight(.semibold)
            Text(
                """
                Remote images are loaded from the sender's server, so \
                fetching one tells them the message was opened, when, and \
                roughly where from. Postio does not fetch any until you say so.
                """
            )
            .foregroundStyle(.secondary)
            Text(notice.sender)
                .font(.system(.callout, design: .monospaced))
                .textSelection(.enabled)
        }
        .padding(PostioTokens.space4)
        .frame(maxWidth: 360, alignment: .leading)
    }

    private func item(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack {
                Text(title)
                    .lineLimit(1)
                Spacer()
            }
            .contentShape(Rectangle())
            .padding(.horizontal, PostioTokens.space4)
            .padding(.vertical, PostioTokens.space2)
        }
        .buttonStyle(.plain)
    }

    /// The address, middle-truncated: `notices_at_…@relay.example.net`.
    ///
    /// The wording is `postio_ui::format::middle_truncate`'s, and the middle
    /// is what goes because both ends identify the sender — the local part
    /// says which service and the domain says whose it is. A tail-truncated
    /// address would hide exactly the half a person judges by.
    public static func shortened(_ address: String, width: Int = 34) -> String {
        middleTruncate(text: address, width: UInt32(width))
    }
}
