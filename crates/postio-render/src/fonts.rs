//! Fonts without fontconfig (research R3): Postio's bundled faces first,
//! then every face `fontdb` discovers, with the generic families and the
//! per-script fallbacks set explicitly. fontconfig is C that reads a
//! message's codepoints; `fontdb` and fontique are Rust.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parley::FontContext;
use parley::fontique::{
    Blob, Collection, CollectionOptions, FallbackKey, FamilyId, FontInfoOverride, GenericFamily,
    Script,
};
use skrifa::MetadataProvider as _;

/// A script, a character only it draws, and the families to try first for
/// it, most wanted first. Any other installed face covering the character
/// comes after them.
const SCRIPTS: &[([u8; 4], char, &[&str])] = &[
    (
        *b"Latn",
        'a',
        &["Liberation Sans", "DejaVu Sans", "Noto Sans"],
    ),
    (
        *b"Cyrl",
        'Ж',
        &["Liberation Sans", "DejaVu Sans", "Noto Sans"],
    ),
    (
        *b"Grek",
        'Ω',
        &["Liberation Sans", "DejaVu Sans", "Noto Sans"],
    ),
    (
        *b"Hani",
        '漢',
        &[
            "Noto Sans CJK SC",
            "Noto Sans CJK JP",
            "Source Han Sans",
            "Droid Sans Fallback",
        ],
    ),
    (
        *b"Hira",
        'あ',
        &["Noto Sans CJK JP", "Source Han Sans", "Droid Sans Fallback"],
    ),
    (
        *b"Kana",
        'ア',
        &["Noto Sans CJK JP", "Source Han Sans", "Droid Sans Fallback"],
    ),
    (
        *b"Hang",
        '한',
        &["Noto Sans CJK KR", "Source Han Sans", "Droid Sans Fallback"],
    ),
    (
        *b"Arab",
        'ع',
        &["Noto Sans Arabic", "Noto Naskh Arabic", "DejaVu Sans"],
    ),
    (*b"Hebr", 'א', &["Noto Sans Hebrew", "DejaVu Sans"]),
    (*b"Deva", 'क', &["Noto Sans Devanagari", "Lohit Devanagari"]),
    (*b"Thai", 'ก', &["Noto Sans Thai", "DejaVu Sans"]),
    (
        *b"Zyyy",
        '→',
        &["DejaVu Sans", "Noto Sans Symbols", "Noto Sans"],
    ),
];

/// The emoji a sender writes, drawn in colour where a colour face exists.
const EMOJI: (char, &[&str]) = ('😀', &["Noto Color Emoji", "Twemoji", "Noto Emoji"]);

/// What `serif` resolves to, if installed; otherwise the bundled sans.
const SERIFS: &[&str] = &["Liberation Serif", "DejaVu Serif", "Noto Serif"];

/// The names mail asks for most, which no free system ships under that name,
/// and the metric-compatible face fontconfig would have substituted.
const ALIASES: &[(&str, &str)] = &[
    ("Liberation Sans", "Arial"),
    ("Liberation Sans", "Helvetica"),
    ("Liberation Serif", "Times New Roman"),
    ("Liberation Serif", "Times"),
    ("Liberation Mono", "Courier New"),
    ("Liberation Mono", "Courier"),
];

/// The faces Postio ships (ADR 0023), and which of them stand for the
/// generic families. The bytes are the caller's: they live beside the
/// reader's `@font-face` rules in `postio-ui`.
pub struct Bundled {
    /// Every bundled face's file.
    pub faces: Vec<&'static [u8]>,
    /// The family `sans-serif` and `system-ui` resolve to.
    pub sans: &'static str,
    /// The family `monospace` resolves to.
    pub mono: &'static str,
}

/// Every face a render may draw with. Built once per process, off the UI
/// thread; each render takes a [`FontContext`] from it.
#[derive(Clone)]
pub struct FontSet {
    collection: Collection,
    files: Vec<PathBuf>,
}

impl FontSet {
    /// Register the bundled faces, then discover the installed ones.
    pub fn new(bundled: Bundled) -> FontSet {
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        for bytes in &bundled.faces {
            collection.register_fonts(Blob::new(std::sync::Arc::new(*bytes) as _), None);
        }
        let mut set = FontSet {
            collection,
            files: Vec::new(),
        };
        let installed = set.discover();
        set.set_generic(GenericFamily::SansSerif, bundled.sans);
        set.set_generic(GenericFamily::SystemUi, bundled.sans);
        set.set_generic(GenericFamily::Monospace, bundled.mono);
        let serif = SERIFS
            .iter()
            .copied()
            .find(|family| set.has_family(family))
            .unwrap_or(bundled.sans);
        set.set_generic(GenericFamily::Serif, serif);
        for (script, sample, preferred) in SCRIPTS {
            let families = installed.covering(*sample, preferred);
            set.collection.set_fallbacks(
                FallbackKey::new(Script::from_bytes(*script), None),
                families.into_iter(),
            );
        }
        let emoji = installed.covering(EMOJI.0, EMOJI.1);
        set.collection
            .set_generic_families(GenericFamily::Emoji, emoji.into_iter());
        set
    }

    /// Register every installed face `fontdb` finds, each file mapped once,
    /// plus the aliases; return what each family covers.
    fn discover(&mut self) -> Installed {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        let ids: Vec<fontdb::ID> = database.faces().map(|face| face.id).collect();
        let mut installed = Installed::default();
        let mut mapped: HashMap<PathBuf, Blob<u8>> = HashMap::new();
        let mut blobs_of: HashMap<FamilyId, Vec<Blob<u8>>> = HashMap::new();
        for id in ids {
            let Some((fontdb::Source::File(path), _)) = database.face_source(id) else {
                continue;
            };
            if mapped.contains_key(&path) {
                continue;
            }
            let Some(data) = map_installed(&mut database, id) else {
                continue;
            };
            let blob = Blob::new(data);
            for (family, fonts) in self.collection.register_fonts(blob.clone(), None) {
                blobs_of.entry(family).or_default().push(blob.clone());
                for font in fonts {
                    if let Ok(face) = skrifa::FontRef::from_index(blob.as_ref(), font.index()) {
                        installed.add(family, &face);
                    }
                }
            }
            self.files.push(path.clone());
            mapped.insert(path, blob);
        }
        for (real, alias) in ALIASES {
            let Some(family) = self.collection.family_by_name(real) else {
                continue;
            };
            for blob in blobs_of.get(&family.id()).into_iter().flatten() {
                self.collection.register_fonts(
                    blob.clone(),
                    Some(FontInfoOverride {
                        family_name: Some(alias),
                        ..Default::default()
                    }),
                );
            }
        }
        for family in installed.names.keys().copied().collect::<Vec<_>>() {
            if let Some(name) = self.collection.family_name(family) {
                installed.names.insert(family, name.to_owned());
            }
        }
        installed
    }

    /// A context for one render: the same faces, its own caches.
    pub fn context(&self) -> FontContext {
        FontContext {
            collection: self.collection.clone(),
            source_cache: Default::default(),
        }
    }

    /// Whether `family` is registered.
    pub fn has_family(&self, family: &str) -> bool {
        self.collection.clone().family_by_name(family).is_some()
    }

    /// The family a CSS generic (`serif`, `sans-serif`, `monospace`, …)
    /// resolves to first.
    pub fn generic(&self, generic: &str) -> Option<String> {
        let generic = GenericFamily::parse(generic)?;
        let mut collection = self.collection.clone();
        let first = collection.generic_families(generic).next()?;
        collection.family_name(first).map(str::to_owned)
    }

    /// The installed font files registered, each once.
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    fn set_generic(&mut self, generic: GenericFamily, family: &str) {
        if let Some(family) = self.collection.family_by_name(family) {
            self.collection
                .set_generic_families(generic, std::iter::once(family.id()));
        }
    }
}

/// What each installed family covers, of the sample characters.
#[derive(Default)]
struct Installed {
    /// In registration order, so the result is stable run to run.
    order: Vec<FamilyId>,
    covers: HashMap<FamilyId, Vec<char>>,
    names: HashMap<FamilyId, String>,
}

impl Installed {
    fn add(&mut self, family: FamilyId, face: &skrifa::FontRef<'_>) {
        let map = face.charmap();
        let samples = SCRIPTS
            .iter()
            .map(|(_, sample, _)| *sample)
            .chain(std::iter::once(EMOJI.0));
        let covered: Vec<char> = samples.filter(|c| map.map(*c).is_some()).collect();
        if !self.covers.contains_key(&family) {
            self.order.push(family);
            self.names.insert(family, String::new());
        }
        let entry = self.covers.entry(family).or_default();
        for c in covered {
            if !entry.contains(&c) {
                entry.push(c);
            }
        }
    }

    /// Families covering `sample`: the preferred ones in their order, then
    /// every other, in registration order.
    fn covering(&self, sample: char, preferred: &[&str]) -> Vec<FamilyId> {
        let covers = |family: &FamilyId| self.covers[family].contains(&sample);
        let mut out: Vec<FamilyId> = preferred
            .iter()
            .filter_map(|name| {
                self.order
                    .iter()
                    .find(|family| self.names[family] == *name)
                    .copied()
            })
            .filter(covers)
            .collect();
        let rest: Vec<FamilyId> = self
            .order
            .iter()
            .filter(|family| covers(family) && !out.contains(family))
            .copied()
            .collect();
        out.extend(rest);
        out
    }
}

/// An installed font file, memory-mapped: only the pages a render touches
/// become resident (research R3; reading every file cost 198 MiB).
#[allow(unsafe_code)]
fn map_installed(
    database: &mut fontdb::Database,
    id: fontdb::ID,
) -> Option<Arc<dyn AsRef<[u8]> + Send + Sync>> {
    // SAFETY: the file is a font `fontdb` found in the system's font
    // directories, mapped read-only. Another process truncating it while
    // mapped could fault the reader; that is the hazard every font stack
    // that maps fonts accepts, and the alternative is 198 MiB resident.
    unsafe { database.make_shared_face_data(id) }.map(|(data, _)| data)
}
