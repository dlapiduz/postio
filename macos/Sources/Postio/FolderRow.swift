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
            ForEach(roots, id: \.rowId) { folder in
                FolderRow(folder: folder, children: children)
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
    /// The children of any folder, asked for as the tree is walked.
    ///
    /// A **closure**, not a list. It was a list, and each recursive call
    /// passed `[]` — so a folder's children were drawn and their children
    /// were not. `Projects/2026/Q1` had no row anywhere in the sidebar and
    /// could not be opened by any means, because there is no folder finder
    /// either. GTK recurses the whole tree and caps only the *indent*.
    let children: (Int64) -> [MailboxFfi]

    var body: some View {
        let mine = children(folder.id)
        if mine.isEmpty {
            // `rowId`, not `id`. The selection is a `SidebarRowId` because
            // three sidebar rows are queries with no id of their own, and a
            // tag of a different type is a row `List` can never select.
            row.tag(folder.rowId)
        } else {
            DisclosureGroup {
                ForEach(mine, id: \.rowId) { child in
                    FolderRow(folder: child, children: children)
                }
            } label: {
                // A `\Noselect` container keeps its row so the hierarchy it
                // organizes can be reached — and cannot itself be opened,
                // because there is no mailbox behind it. Clicking it would
                // otherwise highlight as though mail had been shown, the way
                // the account heading did before `.selectionDisabled()`.
                row.tag(folder.rowId)
                    .selectionDisabled(!folder.selectable)
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
        case .outbox: "Outbox"
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
        // On its way out, which is what the row is for. Not `paperplane`:
        // that is Sent, and the difference between "gone" and "going" is the
        // whole reason this row exists.
        case .outbox: "tray.and.arrow.up"
        case .regular: "folder"
        }
    }
}
