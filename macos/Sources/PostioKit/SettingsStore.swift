import Foundation
import PostioFFI
import SwiftUI

/// Parses and writes no TOML (ADR 0031): values come from `postio_config` and changes go back
/// through the `patch_*` edits, which rewrite what they own with `toml_edit` and leave the rest of
/// the file byte for byte. No OK, Cancel or staging copy: a change lands when made.
@MainActor
@Observable
public final class SettingsStore {
    /// `postio_config::paths` resolved it, so it is the one the rest of Postio reads.
    public let path: String

    public let sections: [SettingsSectionFfi]

    public var selected: String

    public private(set) var text: String

    public private(set) var status: SettingsStatusFfi

    /// "The disk said no", distinct from an invalid file; a state names its reason.
    public private(set) var failure: String?

    public init(path: String) {
        self.path = path
        self.sections = settingsSections()
        self.selected = self.sections.first?.key ?? "accounts"
        let loaded = settingsLoad(path: path)
        self.text = loaded
        self.status = settingsStatus(text: loaded)
    }

    /// `nil` until known; the pane says it only while filtering is on.
    public var filteredToday: UInt32?

    /// `nil` disables the pane: showing defaults would draw settings that are not the user's, and
    /// saving would erase the file they opened the window to fix.
    public var filtering: FilteringPageFfi? {
        settingsFiltering(text: text, filteredToday: filteredToday)
    }

    public func applyFiltering(on: Bool) {
        reload()
        write { try settingsPatchFiltering(text: $0, on: on) }
    }

    /// `nil` when the file will not parse.
    public var afterUnsubscribe: AfterUnsubscribeFfi? {
        settingsAfterUnsubscribe(text: text)
    }

    public func applyAfterUnsubscribe(delete: Bool) {
        reload()
        write { try settingsPatchAfterUnsubscribe(text: $0, delete: delete) }
    }

    public func takeBack(_ undo: FilteringUndoFfi) {
        reload()
        write { try settingsTakeBackFilter(text: $0, undo: undo) }
    }

    /// `nil` when the file will not parse; disabled as `filtering` is.
    public var composing: ComposingFfi? {
        settingsComposing(text: text)
    }

    public var syncing: SyncingFfi? {
        settingsSyncing(text: text)
    }

    /// `nil` when the file will not parse; empty when there are none.
    public var filters: [FilterFfi]? {
        settingsFilters(text: text)
    }

    /// `nil` when the file will not parse.
    public var agentSharing: [SharedAgentAccountFfi]? {
        settingsAgentSharing(text: text)
    }

    /// Only `[mcp.accounts."<address>"]` is removed, from a fresh read of the file. Takes effect on
    /// the agent's next call; one already running finishes.
    public func revokeAgent(_ address: String) {
        reload()
        write { try settingsPatchRevokeAgent(text: $0, address: address) }
    }

    public func revokeAllAgents() {
        reload()
        write { try settingsPatchRevokeAllAgents(text: $0) }
    }

    public func sections(in group: GroupFfi) -> [SettingsSectionFfi] {
        sections.filter { $0.group == group }
    }

    public var current: SettingsSectionFfi? {
        sections.first { $0.key == selected }
    }

    /// When the file is good it names the table this pane writes and says the change is in effect;
    /// when not, the reason takes over, since naming a write that is not happening would mislead.
    public var footer: String {
        guard status.valid else { return status.statusLine }
        if let failure { return failure }
        // Accounts and privacy state do not live in this file, so a footer naming `config.toml`
        // would send someone to edit a file that does not describe them.
        guard let section = current else { return status.statusLine }
        return section.storedIn
    }

    public func reload() {
        text = settingsLoad(path: path)
        status = settingsStatus(text: text)
    }

    /// Takes a mutation and re-reads the file first: `config.toml` is hand-edited, so patching a
    /// remembered copy would race `$EDITOR` and destroy what was added while the window sat open.
    public func applyComposing(_ change: (inout ComposingFfi) -> Void) {
        reload()
        guard var next = composing else { return }
        change(&next)
        write { try settingsPatchComposing(text: $0, composing: next) }
    }

    /// The only writer that is not a `patch_*`: this pane is the file (ADR 0031). Saves invalid
    /// TOML too, since it is a text editor over a file someone is part-way through fixing; the
    /// footer says what is wrong.
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

    public func applySyncing(_ change: (inout SyncingFfi) -> Void) {
        reload()
        guard var next = syncing else { return }
        change(&next)
        write { try settingsPatchSyncing(text: $0, syncing: next) }
    }

    public func applyFilter(_ filter: FilterFfi) {
        reload()
        write { try settingsPatchFilter(text: $0, filter: filter) }
    }

    public func removeFilter(_ key: String) {
        reload()
        write { try settingsRemoveFilter(text: $0, key: key) }
    }

    public func addFilter(key: String, query: String) {
        reload()
        write { try settingsAddFilter(text: $0, key: key, query: query) }
    }

    private func write(_ patch: (String) throws -> String) {
        do {
            try settingsSave(path: path, text: patch(text))
            failure = nil
        } catch {
            failure = "\(error)"
        }
        // The footer describes the file as it is, not as this believes it left it.
        reload()
    }

}
