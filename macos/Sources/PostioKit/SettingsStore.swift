import Foundation
import PostioFFI
import SwiftUI

/// The settings window's model: the file, what is wrong with it, and the way
/// a change gets back to disk.
///
/// **It parses no TOML and writes no TOML** (ADR 0031). Every value it shows
/// came from `postio_config` through the boundary, and every change goes back
/// the same way — `settingsPatchAppearance` rewrites one table with
/// `toml_edit`'s document model and leaves the rest of the file byte for
/// byte. A form here that serialized its own table would be a second writer
/// of a file people edit by hand, with its own idea of key order and comment
/// survival, and the damage would not be visible until someone opened their
/// config and found it rearranged.
///
/// There is no OK, no Cancel and no staging copy, because canvas 3f decided
/// there is no second store: a change lands when you make it, and the footer
/// says what the file says.
@MainActor
@Observable
public final class SettingsStore {
    /// The file this window edits — `postio_config::paths` resolved it, so it
    /// is the one the rest of Postio reads.
    public let path: String

    /// The nav, in canvas order.
    public let sections: [SettingsSectionFfi]

    /// Which section's pane is showing, by `SettingsSectionFfi.key`.
    public var selected: String

    /// The file as it currently stands.
    public private(set) var text: String

    /// The validity line along the foot.
    public private(set) var status: SettingsStatusFfi

    /// Why the last save did not happen, if it did not.
    ///
    /// Distinct from an invalid file: this is "the disk said no", and canvas
    /// 3d's rule is that a state names its reason rather than shrugging.
    public private(set) var failure: String?

    public init(path: String) {
        self.path = path
        self.sections = settingsSections()
        // Appearance, because it is the only pane built here yet. The nav
        // starts at Accounts, and this opens further down it on purpose
        // rather than landing on a section that has nothing to show.
        self.selected = "ui"
        let loaded = settingsLoad(path: path)
        self.text = loaded
        self.status = settingsStatus(text: loaded)
    }

    /// The Appearance pane's values, or `nil` when the file will not parse.
    ///
    /// `nil` disables the pane. Showing defaults instead would draw a form of
    /// plausible settings that are not the user's, and saving it would erase
    /// the file they had opened the window to fix.
    public var appearance: AppearanceFfi? {
        settingsAppearance(text: text)
    }

    /// The sections under one heading, in nav order.
    public func sections(in group: GroupFfi) -> [SettingsSectionFfi] {
        sections.filter { $0.group == group }
    }

    /// The section showing now.
    public var current: SettingsSectionFfi? {
        sections.first { $0.key == selected }
    }

    /// The line along the foot.
    ///
    /// Two different sentences, and which one shows is the whole point. When
    /// the file is good it names the table this pane writes and says the
    /// change is already in effect -- there is no Save to look for. When it
    /// is not, that takes over: canvas 3d's rule is that a state names its
    /// reason, and "[ui] in config.toml" while the file will not parse would
    /// be announcing a write that is not happening.
    public var footer: String {
        guard status.valid else { return status.statusLine }
        if let failure { return failure }
        // What this pane's settings *are*, which is not always a table in
        // this file: accounts live in the encrypted store and privacy state
        // lives beside it, and a footer that named `config.toml` for either
        // would send somebody to edit a file that does not describe them.
        guard let section = current else { return status.statusLine }
        return section.storedIn
    }

    /// Re-read the file, for an edit that arrived from `$EDITOR`.
    public func reload() {
        text = settingsLoad(path: path)
        status = settingsStatus(text: text)
    }

    /// Apply one change to `[ui]` and save.
    ///
    /// Takes a mutation rather than a whole `AppearanceFfi`, and re-reads the
    /// file before applying it. Both halves matter, and a real edit found out
    /// why: `config.toml` is a file people edit by hand, so anything this
    /// window remembers about it is already possibly wrong. Patching a
    /// remembered copy makes this a second writer racing `$EDITOR`, and one
    /// click destroyed a comment, an unknown key and an entire table that had
    /// been added while the window sat open.
    ///
    /// So the file is read, one field is changed, and the result is written:
    /// nothing this window has been holding can overwrite anything it did not
    /// know about. Reading it back afterwards is the same argument once more
    /// -- the footer describes the file as it is, not as this believes it
    /// left it.
    public func apply(_ change: (inout AppearanceFfi) -> Void) {
        reload()
        guard var next = appearance else { return }
        change(&next)
        do {
            let patched = try settingsPatchAppearance(text: text, appearance: next)
            try settingsSave(path: path, text: patched)
            failure = nil
        } catch {
            failure = "\(error)"
        }
        reload()
    }

}
