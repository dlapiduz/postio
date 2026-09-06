import PostioFFI
import PostioKit
import SwiftUI

/// One account's folders, under its address.
///
/// A group rather than a flat run, because "On My Mac" holds every account at
/// once and two accounts with a `Projects` folder each are otherwise two rows
/// with the same name and no way to tell them apart.
struct AccountFolders: View {
    let address: String
    let roots: [MailboxFfi]
    let children: (Int64) -> [MailboxFfi]

    /// Open by default: a person who has one account should not have to
    /// click to see their own folders.
    @State private var expanded = true

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            ForEach(roots, id: \.id) { folder in
                FolderRow(folder: folder, children: children(folder.id))
            }
        } label: {
            Text(address)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }
}

/// One folder in the sidebar, and its children.
///
/// Named and iconed from its **role** where it has one. `PRODUCT.md` says the
/// sidebar says "Flagged"; a sidebar built from server paths would say
/// `[Gmail]/All Mail` and read as a bug in Postio rather than a name the
/// server chose.
struct FolderRow: View {
    let folder: MailboxFfi
    let children: [MailboxFfi]

    var body: some View {
        if children.isEmpty {
            row.tag(folder.id)
        } else {
            DisclosureGroup {
                ForEach(children, id: \.id) { child in
                    FolderRow(folder: child, children: []).tag(child.id)
                }
            } label: {
                row.tag(folder.id)
            }
        }
    }

    private var row: some View {
        Label {
            HStack {
                Text(display)
                Spacer()
                // Only when there is something to say. A "0" beside every
                // folder is noise that reads as data.
                if folder.unread > 0 {
                    Text("\(folder.unread)")
                        .font(.callout.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
        } icon: {
            Image(systemName: symbol)
        }
    }

    /// What Postio calls this folder.
    private var display: String {
        switch folder.role {
        case .inbox: "Inbox"
        case .archive: "Archive"
        case .sent: "Sent"
        case .drafts: "Drafts"
        case .trash: "Trash"
        case .junk: "Junk"
        case .flagged: "Flagged"
        case .snoozed: "Snoozed"
        case .regular: folder.name
        }
    }

    private var symbol: String {
        switch folder.role {
        case .inbox: "tray"
        case .archive: "archivebox"
        case .sent: "paperplane"
        case .drafts: "doc"
        case .trash: "trash"
        case .junk: "xmark.bin"
        case .flagged: "flag"
        case .snoozed: "clock"
        case .regular: "folder"
        }
    }
}
