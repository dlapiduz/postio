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

    /// The Composing pane's values, or `nil` when the file will not parse.
    ///
    /// Disabled for the same reason as `appearance`, and it is the same file.
    public var composing: ComposingFfi? {
        settingsComposing(text: text)
    }

    /// The Sync & storage pane's values, or `nil` when the file will not
    /// parse. Disabled for the same reason as `appearance`.
    public var syncing: SyncingFfi? {
        settingsSyncing(text: text)
    }

    /// Every filter, in the order the sidebar shows them. `nil` when the
    /// file will not parse; empty when there simply are none.
    public var filters: [FilterFfi]? {
        settingsFilters(text: text)
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
        write { try settingsPatchAppearance(text: $0, appearance: next) }
    }

    /// Apply one change to `[compose]` and save.
    ///
    /// The same bargain `apply` makes, over the other table this window
    /// writes: read the file, change one field, write the result. See
    /// `apply` for why nothing this window remembers may be written back.
    public func applyComposing(_ change: (inout ComposingFfi) -> Void) {
        reload()
        guard var next = composing else { return }
        change(&next)
        write { try settingsPatchComposing(text: $0, composing: next) }
    }

    /// Write the file as text — the Config file pane's edit (#1156).
    ///
    /// The only writer here that is not a `patch_*`, and legitimately so:
    /// this pane *is* the file, so there is nothing to preserve around the
    /// edit. Everything else goes through the boundary's patchers, which is
    /// what stops a form reordering keys and dropping comments (ADR 0031).
    ///
    /// Deliberately saves invalid TOML too. It is a text editor over a file
    /// somebody is part-way through fixing, and refusing to write until it
    /// parses would make the pane useless for the one job it has. The footer
    /// says what is wrong; `[logging]` and the file watcher decide what a
    /// running Postio does about it.
    public func write(text edited: String) {
        text = edited
        status = settingsStatus(text: edited)
        do {
            try settingsSave(path: path, text: edited)
            failure = nil
        } catch {
            failure = "\(error)"
        }
    }

    /// Apply one change to `[sync]` and save. See `apply`.
    public func applySyncing(_ change: (inout SyncingFfi) -> Void) {
        reload()
        guard var next = syncing else { return }
        change(&next)
        write { try settingsPatchSyncing(text: $0, syncing: next) }
    }

    /// Change one filter and save. See `apply`.
    public func applyFilter(_ filter: FilterFfi) {
        reload()
        write { try settingsPatchFilter(text: $0, filter: filter) }
    }

    /// Remove a filter and save.
    public func removeFilter(_ key: String) {
        reload()
        write { try settingsRemoveFilter(text: $0, key: key) }
    }

    /// Add a filter running `query`, and say what went wrong if it could not
    /// be added — a name already taken, or an empty one.
    public func addFilter(key: String, query: String) {
        reload()
        write { try settingsAddFilter(text: $0, key: key, query: query) }
    }

    /// Save whatever `patch` makes of the file, and describe what happened.
    private func write(_ patch: (String) throws -> String) {
        do {
            try settingsSave(path: path, text: patch(text))
            failure = nil
        } catch {
            failure = "\(error)"
        }
        // Reading it back is the same argument once more: the footer
        // describes the file as it is, not as this believes it left it.
        reload()
    }

}
