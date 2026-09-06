import Foundation
import PostioFFI
import SwiftUI

/// The settings window's model: the file, what is wrong with it, and the way
/// a change gets back to disk.
///
/// **It parses no TOML and writes no TOML** (ADR 0029). Every value it shows
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
        let sections = settingsSections()
        self.sections = sections
        // The pane ADR 0029 Q3 ships first, and the first nav row either way.
        self.selected = sections.first?.key ?? "ui"
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

    /// Re-read the file, for an edit that arrived from `$EDITOR`.
    public func reload() {
        text = settingsLoad(path: path)
        status = settingsStatus(text: text)
    }

    /// Write `appearance` into `[ui]` and save.
    ///
    /// Reads the file back afterwards rather than trusting what it just
    /// wrote: the footer is the only thing telling the user their change took
    /// effect — canvas 3f put it where OK and Cancel would be — so it has to
    /// describe the file as it is, not as this believes it left it.
    public func apply(_ appearance: AppearanceFfi) {
        do {
            let patched = try settingsPatchAppearance(text: text, appearance: appearance)
            try settingsSave(path: path, text: patched)
            failure = nil
        } catch {
            failure = "\(error)"
        }
        reload()
    }
}
