//! The review bundle: everything an independent reviewer reads, gathered in
//! one directory (`contracts/review.md` § The bundle).
//!
//! The bundle is a manifest, the acceptance, the canvas screens the
//! storyboards name, and symlinks to the run trees, so every frame the
//! manifest cites reads `runs/<app>/<storyboard>/<variant>/NN.outlined.png`.
//! Which runs need a verdict is a question for the comparison; this module
//! takes it as a function, and the default -- no base -- calls every run new.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::apply::App;
use crate::format::{self, Storyboard};
use crate::page::{self, Filmstrip};
use crate::run::variant_key;

/// The most outlined frames one reviewer is asked to read.
pub const MAX_FRAMES_PER_BATCH: usize = 60;

/// The one directory design screens must never come from: it is untracked
/// and its images carry a real first name.
pub const FORBIDDEN_DESIGN_DIR: &str = "postio-focus-design";

/// What the comparison made of one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// Nothing the review looks at changed. Counted, never listed.
    Unchanged,
    /// The base had it and the branch differs.
    Changed,
    /// The base did not have it.
    New,
}

/// A run's class, and which of its steps changed (`None` is all of them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// The class.
    pub class: Class,
    /// The step indices that need a verdict, or all of them.
    pub changed_steps: Option<BTreeSet<usize>>,
}

/// With no base to compare against, every run is new and every step needs a
/// verdict.
pub fn all_new(_: &Filmstrip) -> Classification {
    Classification {
        class: Class::New,
        changed_steps: None,
    }
}

/// `manifest.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// The review key of the tree the runs came from.
    pub tree_key: String,
    /// The base the runs were compared with, when there was one.
    pub base: Option<String>,
    /// What to review, in groups of one app and one surface.
    pub batches: Vec<Batch>,
    /// How many runs were unchanged and so are not listed.
    pub unchanged: usize,
    /// Canvas screens copied into `design/`, by name.
    pub design: Vec<String>,
    /// Screens a storyboard names that no design directory holds.
    pub design_missing: Vec<String>,
}

/// One reviewer's work: one app, one surface, a bounded number of frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch {
    /// The app.
    pub app: App,
    /// The storyboard's directory under the catalogue, such as `list`.
    pub surface: String,
    /// The storyboards in it, by name.
    pub storyboards: Vec<String>,
    /// The canvas screens they name, as `design/<name>.png`.
    pub design: Vec<String>,
    /// The runs, changed or new.
    pub runs: Vec<BatchRun>,
    /// Every step that needs a verdict.
    pub steps: Vec<NeedStep>,
}

/// A changed or new run in a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchRun {
    /// The storyboard.
    pub storyboard: String,
    /// The variant directory name.
    pub variant: String,
    /// Changed or new.
    pub class: Class,
    /// The run's directory in the bundle, `runs/<app>/<storyboard>/<variant>`.
    pub dir: String,
}

/// A step that needs a verdict, with the citation to give it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeedStep {
    /// The storyboard.
    pub storyboard: String,
    /// The step's id, or its index when it has none.
    pub step: String,
    /// The app.
    pub app: App,
    /// The variant directory name.
    pub variant: String,
    /// The outlined frame, relative to the bundle.
    pub frame: String,
}

/// Why a bundle could not be built.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    /// A file or directory could not be read or written.
    #[error("{0}: {1}")]
    Io(PathBuf, io::Error),
    /// A design directory is the forbidden one.
    #[error("{0}: design screens never come from `{FORBIDDEN_DESIGN_DIR}`")]
    ForbiddenDesign(PathBuf),
    /// There are no runs to review.
    #[error("{0}: no runs found")]
    NoRuns(PathBuf),
}

/// What `bundle` is given.
pub struct Inputs<'a> {
    /// The branch's run tree.
    pub runs: &'a Path,
    /// The base's run tree, when there is one.
    pub base: Option<&'a Path>,
    /// The base's commit, for the manifest.
    pub base_sha: Option<String>,
    /// The acceptance text: the issue's, or the spec's scenarios.
    pub acceptance: &'a Path,
    /// The storyboards directory.
    pub catalogue: &'a Path,
    /// Committed reference directories the screens come from.
    pub design_dirs: &'a [PathBuf],
    /// The bundle directory to write.
    pub out: &'a Path,
}

/// The step's label in a citation: its id, or its index.
pub fn step_label(step: &crate::run::StepRun) -> String {
    step.id.clone().unwrap_or_else(|| step.step.to_string())
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> BundleError + '_ {
    move |error| BundleError::Io(path.to_owned(), error)
}

/// Refuses a design directory that is, or resolves to, the forbidden one.
fn refuse_forbidden(dir: &Path) -> Result<(), BundleError> {
    let named = |path: &Path| path.to_string_lossy().contains(FORBIDDEN_DESIGN_DIR);
    let resolved = std::fs::canonicalize(dir).ok();
    if named(dir) || resolved.as_deref().is_some_and(named) {
        return Err(BundleError::ForbiddenDesign(dir.to_owned()));
    }
    Ok(())
}

/// Every storyboard in the catalogue, by name, with its surface: the
/// directory it sits in, relative to the catalogue. Files that are not
/// storyboards (gap lists, the README) are skipped; `lint` is what reports
/// those.
fn catalogue(root: &Path) -> BTreeMap<String, (Storyboard, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "toml") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(root, &mut files);
    files.sort();
    let mut boards = BTreeMap::new();
    for file in files {
        let Ok(board) = format::load(&file) else {
            continue;
        };
        let surface = file
            .parent()
            .and_then(|parent| parent.strip_prefix(root).ok())
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        boards.entry(board.name.clone()).or_insert((board, surface));
    }
    boards
}

/// The canvas screens a storyboard names: its own and each step's.
fn screens(board: &Storyboard) -> BTreeSet<String> {
    board
        .design
        .iter()
        .chain(board.steps.iter().filter_map(|step| step.design.as_ref()))
        .cloned()
        .collect()
}

fn link(target: &Path, at: &Path) -> Result<(), BundleError> {
    if at.symlink_metadata().is_ok() {
        std::fs::remove_file(at).map_err(io_error(at))?;
    }
    let target = std::fs::canonicalize(target).map_err(io_error(target))?;
    std::os::unix::fs::symlink(&target, at).map_err(io_error(at))
}

/// A step that needs a verdict, with the run it is in and that run's class.
type Needed<'a> = (&'a Filmstrip, Class, NeedStep);

/// Builds the bundle and returns its manifest.
pub fn build(
    inputs: &Inputs<'_>,
    classify: &dyn Fn(&Filmstrip) -> Classification,
) -> Result<Manifest, BundleError> {
    for dir in inputs.design_dirs {
        refuse_forbidden(dir)?;
    }
    let strips = page::collect(inputs.runs, "runs").map_err(io_error(inputs.runs))?;
    if strips.is_empty() {
        return Err(BundleError::NoRuns(inputs.runs.to_owned()));
    }
    let boards = catalogue(inputs.catalogue);

    let mut unchanged = 0;
    // (app, surface) -> the steps that need a verdict, with their run.
    let mut groups: BTreeMap<(App, String), Vec<Needed<'_>>> = BTreeMap::new();
    for strip in &strips {
        let classification = classify(strip);
        if classification.class == Class::Unchanged {
            unchanged += 1;
            continue;
        }
        let name = &strip.run.storyboard.name;
        let surface = boards
            .get(name)
            .map(|(_, surface)| surface.clone())
            .unwrap_or_default();
        for step in &strip.run.steps {
            let wanted = classification
                .changed_steps
                .as_ref()
                .is_none_or(|steps| steps.contains(&step.step));
            let Some(outlined) = step.outlined.as_ref().filter(|_| wanted) else {
                continue;
            };
            groups
                .entry((strip.run.app, surface.clone()))
                .or_default()
                .push((
                    strip,
                    classification.class,
                    NeedStep {
                        storyboard: name.clone(),
                        step: step_label(step),
                        app: strip.run.app,
                        variant: variant_key(&strip.run.variant),
                        frame: format!("{}/{outlined}", strip.dir),
                    },
                ));
        }
    }

    let mut batches = Vec::new();
    for ((app, surface), steps) in groups {
        for chunk in steps.chunks(MAX_FRAMES_PER_BATCH) {
            let mut runs: Vec<BatchRun> = Vec::new();
            for (strip, class, _) in chunk {
                if !runs.iter().any(|run| run.dir == strip.dir) {
                    runs.push(BatchRun {
                        storyboard: strip.run.storyboard.name.clone(),
                        variant: variant_key(&strip.run.variant),
                        class: *class,
                        dir: strip.dir.clone(),
                    });
                }
            }
            let names: BTreeSet<String> = runs.iter().map(|run| run.storyboard.clone()).collect();
            let design: BTreeSet<String> = names
                .iter()
                .filter_map(|name| boards.get(name))
                .flat_map(|(board, _)| screens(board))
                .collect();
            batches.push(Batch {
                app,
                surface: surface.clone(),
                storyboards: names.into_iter().collect(),
                design: design.into_iter().collect(),
                runs,
                steps: chunk.iter().map(|(_, _, step)| step.clone()).collect(),
            });
        }
    }

    // Design screens: only those a listed storyboard names, only from the
    // directories the caller gave.
    let wanted: BTreeSet<&String> = batches.iter().flat_map(|b| &b.design).collect();
    let design_out = inputs.out.join("design");
    std::fs::create_dir_all(inputs.out).map_err(io_error(inputs.out))?;
    if design_out.exists() {
        std::fs::remove_dir_all(&design_out).map_err(io_error(&design_out))?;
    }
    std::fs::create_dir_all(&design_out).map_err(io_error(&design_out))?;
    let (mut design, mut design_missing) = (Vec::new(), Vec::new());
    for name in wanted {
        let file = format!("{name}.png");
        match inputs
            .design_dirs
            .iter()
            .find(|dir| dir.join(&file).is_file())
        {
            Some(dir) => {
                std::fs::copy(dir.join(&file), design_out.join(&file))
                    .map_err(io_error(&dir.join(&file)))?;
                design.push(name.clone());
            }
            None => design_missing.push(name.clone()),
        }
    }

    std::fs::copy(inputs.acceptance, inputs.out.join("acceptance.md"))
        .map_err(io_error(inputs.acceptance))?;
    link(inputs.runs, &inputs.out.join("runs"))?;
    if let Some(base) = inputs.base {
        link(base, &inputs.out.join("base"))?;
    }

    let manifest = Manifest {
        tree_key: strips[0].run.tree_key.clone(),
        base: inputs.base.and(inputs.base_sha.clone()),
        batches,
        unchanged,
        design,
        design_missing,
    };
    let path = inputs.out.join("manifest.json");
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| io_error(&path)(e.into()))?;
    std::fs::write(&path, json + "\n").map_err(io_error(&path))?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{run, storyboard, write};

    struct Tree {
        dir: tempfile::TempDir,
    }

    impl Tree {
        fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }
    }

    /// Runs for classic and terminal across two surfaces, a catalogue naming
    /// canvas screens, and a committed design directory holding them.
    fn tree() -> Tree {
        let tree = Tree {
            dir: tempfile::tempdir().expect("temp"),
        };
        let runs = tree.path("runs");
        write(&runs, &run("archive-walks-down", App::Classic, &[], 3));
        write(
            &runs,
            &run("archive-walks-down", App::Classic, &[("scheme", "dark")], 3),
        );
        write(&runs, &run("archive-walks-down", App::Terminal, &[], 3));
        write(&runs, &run("open-settings", App::Classic, &[], 2));
        let catalogue = tree.path("storyboards");
        for (dir, name, design) in [
            ("list", "archive-walks-down", Some("01-inbox-reading")),
            ("screens", "open-settings", None),
        ] {
            std::fs::create_dir_all(catalogue.join(dir)).expect("dir");
            std::fs::write(
                catalogue.join(dir).join(format!("{name}.toml")),
                storyboard(design),
            )
            .expect("board");
        }
        let design = tree.path("screens");
        std::fs::create_dir_all(&design).expect("design");
        for name in ["01-inbox-reading", "step-screen", "unnamed-extra"] {
            std::fs::write(design.join(format!("{name}.png")), b"png").expect("png");
        }
        std::fs::write(tree.path("acceptance.md"), "the issue's acceptance").expect("acc");
        tree
    }

    fn make(
        tree: &Tree,
        base: Option<&Path>,
        design: &[PathBuf],
        classify: &dyn Fn(&Filmstrip) -> Classification,
    ) -> Result<Manifest, BundleError> {
        let runs = tree.path("runs");
        let acceptance = tree.path("acceptance.md");
        let catalogue = tree.path("storyboards");
        let out = tree.path("bundle");
        build(
            &Inputs {
                runs: &runs,
                base,
                base_sha: base.map(|_| "abc123".to_owned()),
                acceptance: &acceptance,
                catalogue: &catalogue,
                design_dirs: design,
                out: &out,
            },
            classify,
        )
    }

    #[test]
    fn with_no_base_every_run_is_new_and_every_step_needs_a_verdict() {
        let tree = tree();
        let manifest = make(&tree, None, &[tree.path("screens")], &all_new).expect("built");
        assert_eq!(manifest.tree_key, "treekey");
        assert_eq!(manifest.base, None);
        let steps: usize = manifest.batches.iter().map(|b| b.steps.len()).sum();
        assert_eq!(steps, 3 + 3 + 3 + 2, "every step of every run");
        assert!(
            manifest
                .batches
                .iter()
                .flat_map(|b| &b.runs)
                .all(|r| r.class == Class::New)
        );
        assert_eq!(manifest.unchanged, 0);
        let on_disk: Manifest = serde_json::from_str(
            &std::fs::read_to_string(tree.path("bundle/manifest.json")).expect("manifest.json"),
        )
        .expect("parses");
        assert_eq!(on_disk, manifest);
    }

    #[test]
    fn batches_hold_one_app_and_one_surface() {
        let tree = tree();
        let manifest = make(&tree, None, &[tree.path("screens")], &all_new).expect("built");
        let keys: Vec<(App, &str)> = manifest
            .batches
            .iter()
            .map(|b| (b.app, b.surface.as_str()))
            .collect();
        assert_eq!(
            keys,
            [
                (App::Classic, "list"),
                (App::Classic, "screens"),
                (App::Terminal, "list")
            ]
        );
        for batch in &manifest.batches {
            assert!(batch.steps.iter().all(|s| s.app == batch.app));
            assert!(
                batch
                    .runs
                    .iter()
                    .all(|r| batch.storyboards.contains(&r.storyboard)),
                "{batch:?}"
            );
        }
    }

    #[test]
    fn a_step_is_cited_by_storyboard_step_app_variant_and_frame() {
        let tree = tree();
        let manifest = make(&tree, None, &[tree.path("screens")], &all_new).expect("built");
        let dark = manifest
            .batches
            .iter()
            .flat_map(|b| &b.steps)
            .find(|s| s.variant == "scheme=dark" && s.step == "2")
            .expect("the dark variant's step 2");
        assert_eq!(dark.storyboard, "archive-walks-down");
        assert_eq!(dark.app, App::Classic);
        assert_eq!(
            dark.frame,
            "runs/classic/archive-walks-down/scheme=dark/02.outlined.png"
        );
        assert!(
            tree.path("bundle").join(&dark.frame).exists(),
            "the frame path resolves through the runs symlink"
        );
    }

    #[test]
    fn a_batch_holds_at_most_sixty_frames_and_a_big_run_is_split() {
        let tree = tree();
        write(
            &tree.path("runs"),
            &run(
                "archive-walks-down",
                App::Classic,
                &[("width", "wide")],
                130,
            ),
        );
        let manifest = make(&tree, None, &[tree.path("screens")], &all_new).expect("built");
        let list: Vec<&Batch> = manifest
            .batches
            .iter()
            .filter(|b| b.app == App::Classic && b.surface == "list")
            .collect();
        assert!(list.len() >= 3, "{} batches", list.len());
        assert!(list.iter().all(|b| b.steps.len() <= MAX_FRAMES_PER_BATCH));
        let total: usize = list.iter().map(|b| b.steps.len()).sum();
        assert_eq!(total, 3 + 3 + 130, "no step lost in the split");
    }

    #[test]
    fn only_the_classified_steps_are_listed_and_unchanged_runs_are_counted() {
        let tree = tree();
        let classify = |strip: &Filmstrip| match strip.run.storyboard.name.as_str() {
            "open-settings" => Classification {
                class: Class::Unchanged,
                changed_steps: None,
            },
            _ if strip.run.app == App::Terminal => Classification {
                class: Class::Changed,
                changed_steps: Some(BTreeSet::from([2])),
            },
            _ => all_new(strip),
        };
        let manifest = make(&tree, None, &[tree.path("screens")], &classify).expect("built");
        assert_eq!(manifest.unchanged, 1);
        assert!(
            manifest
                .batches
                .iter()
                .flat_map(|b| &b.runs)
                .all(|r| r.storyboard != "open-settings"),
            "unchanged runs are not listed"
        );
        let terminal: Vec<&str> = manifest
            .batches
            .iter()
            .filter(|b| b.app == App::Terminal)
            .flat_map(|b| &b.steps)
            .map(|s| s.step.as_str())
            .collect();
        assert_eq!(terminal, ["2"]);
    }

    #[test]
    fn design_screens_are_copied_only_when_a_storyboard_names_them() {
        let tree = tree();
        let manifest = make(&tree, None, &[tree.path("screens")], &all_new).expect("built");
        let copied = |name: &str| {
            tree.path("bundle/design")
                .join(format!("{name}.png"))
                .exists()
        };
        assert!(copied("01-inbox-reading"), "the storyboard's design");
        assert!(copied("step-screen"), "a step's design");
        assert!(!copied("unnamed-extra"), "nothing else is copied");
        assert_eq!(manifest.design, ["01-inbox-reading", "step-screen"]);
        assert!(manifest.design_missing.is_empty());
    }

    #[test]
    fn a_design_directory_under_postio_focus_design_is_refused() {
        let tree = tree();
        let forbidden = tree.path("Design/postio-focus-design/screens");
        std::fs::create_dir_all(&forbidden).expect("dir");
        std::fs::write(forbidden.join("01-inbox-reading.png"), b"png").expect("png");
        let error =
            make(&tree, None, std::slice::from_ref(&forbidden), &all_new).expect_err("refused");
        assert!(matches!(error, BundleError::ForbiddenDesign(_)), "{error}");
        assert!(
            !tree.path("bundle/design/01-inbox-reading.png").exists(),
            "nothing was copied"
        );
    }

    #[test]
    fn a_symlink_into_the_forbidden_directory_is_refused_too() {
        let tree = tree();
        let forbidden = tree.path("Design/postio-focus-design");
        std::fs::create_dir_all(&forbidden).expect("dir");
        let link = tree.path("innocent");
        std::os::unix::fs::symlink(&forbidden, &link).expect("link");
        let error = make(&tree, None, &[link], &all_new).expect_err("refused");
        assert!(matches!(error, BundleError::ForbiddenDesign(_)), "{error}");
    }

    #[test]
    fn a_screen_no_directory_holds_is_reported_missing_not_invented() {
        let tree = tree();
        let manifest = make(&tree, None, &[], &all_new).expect("built");
        assert!(manifest.design.is_empty());
        assert_eq!(manifest.design_missing, ["01-inbox-reading", "step-screen"]);
    }

    #[test]
    fn the_acceptance_is_copied_and_runs_and_base_are_linked() {
        let tree = tree();
        let base = tree.path("base");
        write(&base, &run("archive-walks-down", App::Classic, &[], 3));
        let manifest = make(&tree, Some(&base), &[tree.path("screens")], &all_new).expect("built");
        assert_eq!(manifest.base.as_deref(), Some("abc123"));
        assert_eq!(
            std::fs::read_to_string(tree.path("bundle/acceptance.md")).expect("acceptance"),
            "the issue's acceptance"
        );
        assert!(
            tree.path("bundle/runs/classic/archive-walks-down/default/run.json")
                .exists()
        );
        assert!(
            tree.path("bundle/base/classic/archive-walks-down/default/run.json")
                .exists()
        );
    }
}
