import PostioFFI
import SwiftUI

/// Per message, never per pane: a conversation can hold back pictures from several senders, and one
/// notice could not say whose.
/// The `⋯` opens an anchored popover because the choices are about this sender.
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
                .buttonStyle(ReaderButtonStyle(.secondary))
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

    /// The middle goes, because both ends identify the sender: a tail-truncated address would hide
    /// the half a person judges by.
    public static func shortened(_ address: String, width: Int = 34) -> String {
        middleTruncate(text: address, width: UInt32(width))
    }
}
