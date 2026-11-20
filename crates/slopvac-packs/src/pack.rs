//! A loaded pack, and the packs a run knows.

use std::collections::HashSet;
use std::path::PathBuf;

use semver::Version;
use slopvac_config::{PackFile, PresetFailure, Presets, Setting};
use slopvac_context::{GenreSpec, TaggerSpec};
use slopvac_model::PackId;
use slopvac_rules::{CompileOptions, RuleSet};
use slopvac_styles::{Style, StyleError, StylePresets, StyleSource};

use crate::genre_layer::{GenreFile, display_path, settings};
use crate::name::is_local;
use crate::{
    ArtifactFault, Manifest, PackError, PackPath, PackSource, Provenance, Provides, QualifiedName,
    Registry, RuleFault, RuleProblem, Source,
};

/// The id of the built-in pack.
const CORE_ID: &str = "core";

/// One pack with its files read and checked.
#[derive(Debug)]
pub struct LoadedPack {
    manifest: Manifest,
    origin: Source,
    rules: RuleSet,
    genres: Vec<(PackPath, GenreFile)>,
    taggers: Vec<TaggerSpec>,
    styles: Vec<(PackPath, String)>,
    configs: Vec<(PackPath, String)>,
}

fn not_glob(path: &PackPath) -> Result<(), PackError> {
    if path.as_str().contains(['*', '?', '[', '{']) {
        return Err(PackError::Glob {
            path: path.as_str().to_owned(),
        });
    }
    Ok(())
}

/// Refuse a glob in any `provides` list before a file is read.
fn no_globs(provides: &Provides) -> Result<(), PackError> {
    [
        &provides.categories,
        &provides.styles,
        &provides.genres,
        &provides.terms,
        &provides.taggers,
        &provides.configs,
        &provides.prime,
        &provides.blocklists,
    ]
    .into_iter()
    .flatten()
    .try_for_each(not_glob)
}

fn read_all(
    source: &dyn PackSource,
    paths: &[PackPath],
) -> Result<Vec<(PackPath, String)>, PackError> {
    paths
        .iter()
        .map(|p| Ok((p.clone(), source.read(p)?)))
        .collect()
}

/// Reduce a parse failure of `text` to a location and a fixed reason.
fn artifact(path: &PackPath, text: &str, source: &slopvac_context::Error) -> PackError {
    use slopvac_context::Error as E;
    let fault = match source {
        E::Yaml(_) => match slopvac_config::tree::parse_yaml(text) {
            Err(e) => return syntax(path, text, &e),
            Ok(_) => ArtifactFault::Schema,
        },
        E::Pattern { .. } | E::PatternTooLong { .. } => ArtifactFault::Pattern,
        E::Glob { .. } | E::GlobSet(_) | E::SignalGlob { .. } => ArtifactFault::Glob,
        _ => ArtifactFault::Invalid,
    };
    artifact_fault(path, fault)
}

fn artifact_fault(path: &PackPath, fault: ArtifactFault) -> PackError {
    PackError::Artifact {
        path: path.as_str().to_owned(),
        fault,
    }
}

/// A YAML syntax error in `text`, as a line and column.
fn syntax(path: &PackPath, text: &str, error: &slopvac_config::tree::ParseError) -> PackError {
    let at = error.span.as_ref().map_or(0, |s| s.start);
    let (line, column) = slopvac_config::tree::LineIndex::new(text).line_col(at);
    artifact_fault(path, ArtifactFault::Syntax { line, column })
}

fn rule_problems(errors: &slopvac_rules::LoadErrors) -> PackError {
    PackError::Rules {
        problems: errors
            .errors()
            .iter()
            .map(|e| {
                RuleProblem::new(
                    e.file.display().to_string(),
                    e.line,
                    RuleFault::from(&e.kind),
                )
            })
            .collect(),
    }
}

impl LoadedPack {
    /// Read the pack in `source` and check its rules, genres and styles.
    ///
    /// `origin` says who supplies the pack. The core pack takes the binary's version, whatever
    /// its manifest says, and is the only pack whose taggers load: a declarative tagger from
    /// any other pack stays unread until the tag-lookup amplification is closed.
    ///
    /// # Errors
    /// [`PackError`] for a manifest that breaks the contract, a missing file, a rule that does
    /// not compile, a genre that does not parse, a glob entry, the id `core` in another pack,
    /// or a core pack with another id.
    pub fn load(source: &dyn PackSource, origin: Source) -> Result<Self, PackError> {
        let mut manifest = Manifest::parse_allowing_core(&source.manifest_text()?)?;
        let is_core = manifest.id.as_str() == CORE_ID;
        match (origin, is_core) {
            (Source::Core, false) => return Err(PackError::NotCore),
            (Source::Pack | Source::Plugin, true) => return Err(PackError::ReservedId),
            _ => {}
        }
        if origin == Source::Core {
            manifest.version =
                Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| PackError::BinaryVersion)?;
        }
        let provides = &manifest.provides;
        no_globs(provides)?;
        let categories = read_all(source, &provides.categories)?;
        let rules = RuleSet::from_sources(
            &manifest.id,
            categories
                .into_iter()
                .map(|(p, text)| (PathBuf::from(p.as_str()), text)),
            &CompileOptions::default(),
        )
        .map_err(|e| rule_problems(&e))?;
        let mut genres = Vec::new();
        for (path, text) in read_all(source, &provides.genres)? {
            let spec = GenreSpec::from_yaml(&text)
                .and_then(|spec| spec.validate().map(|()| spec))
                .map_err(|e| artifact(&path, &text, &e))?;
            if !is_local(&spec.id) {
                return Err(artifact_fault(&path, ArtifactFault::Invalid));
            }
            let root =
                slopvac_config::tree::parse_yaml(&text).map_err(|e| syntax(&path, &text, &e))?;
            genres.push((path, GenreFile::new(spec, text, root)));
        }
        let mut taggers = Vec::new();
        if origin == Source::Core {
            for (path, text) in read_all(source, &provides.taggers)? {
                let spec = TaggerSpec::from_yaml(&text)
                    .and_then(|spec| spec.clone().compile().map(|_| spec))
                    .map_err(|e| artifact(&path, &text, &e))?;
                taggers.push(spec);
            }
        }
        let styles = read_all(source, &provides.styles)?;
        let configs = read_all(source, &provides.configs)?;
        Ok(Self {
            manifest,
            origin,
            rules,
            genres,
            taggers,
            styles,
            configs,
        })
    }

    /// The pack's manifest.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The pack's rules.
    #[must_use]
    pub fn rules(&self) -> &RuleSet {
        &self.rules
    }

    /// The pack's declarative taggers, in registration order. Empty unless this is the core.
    #[must_use]
    pub fn taggers(&self) -> &[TaggerSpec] {
        &self.taggers
    }

    fn provenance(&self, file: &PackPath) -> Provenance {
        Provenance::new(
            self.manifest.id.clone(),
            self.manifest.version.clone(),
            self.origin,
            file.as_str(),
        )
    }
}

fn stem(path: &PackPath) -> &str {
    let file = path.as_str().rsplit('/').next().unwrap_or_default();
    file.rsplit_once('.').map_or(file, |(stem, _)| stem)
}

/// The packs a run knows, with their genres and styles by qualified name.
#[derive(Debug)]
pub struct Packs {
    loaded: Vec<LoadedPack>,
    genres: Registry<GenreFile>,
    styles: Registry<String>,
}

impl Packs {
    /// Register the genres and styles of `packs`, in order.
    ///
    /// # Errors
    /// [`PackError::Registry`] when two packs supply the same qualified name,
    /// [`PackError::MissingParent`] for a genre that extends one no pack supplies, and
    /// [`PackError::GenreCycle`] for a genre that extends itself through its parents.
    pub fn new(packs: Vec<LoadedPack>) -> Result<Self, PackError> {
        let mut genres = Registry::new();
        let mut styles = Registry::new();
        for pack in &packs {
            for (path, file) in &pack.genres {
                // The load already checked the id, so a refusal here is a defect upstream; it
                // still reports the file and never the id.
                let name = QualifiedName::new(pack.manifest.id.clone(), file.spec.id.clone())
                    .map_err(|_| artifact_fault(path, ArtifactFault::Invalid))?;
                genres.insert(name, file.clone(), pack.provenance(path))?;
            }
            for (path, text) in &pack.styles {
                let name = QualifiedName::new(pack.manifest.id.clone(), stem(path))?;
                styles.insert(name, text.clone(), pack.provenance(path))?;
            }
        }
        check_genre_graph(&genres)?;
        Ok(Self {
            loaded: packs,
            genres,
            styles,
        })
    }

    /// The core pack from the copy inside the binary.
    ///
    /// # Errors
    /// [`PackError`] when the embedded pack does not load.
    pub fn core() -> Result<Self, PackError> {
        Self::new(vec![LoadedPack::load(&crate::core_source(), Source::Core)?])
    }

    /// The loaded packs, in load order.
    #[must_use]
    pub fn packs(&self) -> &[LoadedPack] {
        &self.loaded
    }

    /// Every pack's rule set, in load order.
    pub fn rule_sets(&self) -> impl Iterator<Item = &RuleSet> {
        self.loaded.iter().map(LoadedPack::rules)
    }

    /// Every genre in load order, then file order. Order breaks detection ties.
    pub fn genres(&self) -> impl Iterator<Item = &GenreSpec> {
        self.loaded
            .iter()
            .flat_map(|p| p.genres.iter().map(|(_, g)| &g.spec))
    }

    /// Every declarative tagger, in load order.
    pub fn taggers(&self) -> impl Iterator<Item = &TaggerSpec> {
        self.loaded.iter().flat_map(|p| p.taggers.iter())
    }

    fn style_presets(&self) -> StylePresets<PackStyles<'_>> {
        StylePresets(PackStyles(&self.styles))
    }

    /// The genre and its ancestors, lowest first, each once.
    fn chain(&self, name: &str, seen: &mut HashSet<String>, out: &mut Vec<Vec<Setting>>) {
        let Ok(entry) = self.genres.resolve(name) else {
            return;
        };
        let p = &entry.provenance;
        if !seen.insert(format!("{}:{}", p.pack, entry.value.spec.id)) {
            return;
        }
        for parent in &entry.value.spec.extends {
            self.chain(&parent_name(&p.pack, parent), seen, out);
        }
        out.push(settings(&p.pack, &p.file, &entry.value));
    }
}

/// A parent written `<pack>:<name>` names a pack; a bare one means the child's pack.
fn parent_name(pack: &PackId, parent: &str) -> String {
    if parent.contains(':') {
        parent.to_owned()
    } else {
        format!("{pack}:{parent}")
    }
}

/// Refuse a genre whose parents are missing or lead back to it.
fn check_genre_graph(genres: &Registry<GenreFile>) -> Result<(), PackError> {
    /// Qualified names of the genres on the path from the root of this walk, and the done ones.
    fn visit(
        genres: &Registry<GenreFile>,
        name: &str,
        walking: &mut Vec<String>,
        done: &mut HashSet<String>,
    ) -> Result<(), PackError> {
        if done.contains(name) {
            return Ok(());
        }
        let Ok(entry) = genres.resolve(name) else {
            // The caller reports the parent as missing; a root is always registered.
            return Ok(());
        };
        let (line, column) = entry.value.locate(&["extends"]);
        if walking.iter().any(|w| w == name) {
            return Err(PackError::GenreCycle {
                path: entry.provenance.file.clone(),
                line,
                column,
            });
        }
        walking.push(name.to_owned());
        for parent in &entry.value.spec.extends {
            let qualified = parent_name(&entry.provenance.pack, parent);
            if genres.resolve(&qualified).is_err() {
                return Err(PackError::MissingParent {
                    path: entry.provenance.file.clone(),
                    line,
                    column,
                });
            }
            visit(genres, &qualified, walking, done)?;
        }
        walking.pop();
        done.insert(name.to_owned());
        Ok(())
    }
    let mut done = HashSet::new();
    for (name, _) in genres.iter() {
        visit(genres, &name.to_string(), &mut Vec::new(), &mut done)?;
    }
    Ok(())
}

/// The styles of loaded packs, as a [`StyleSource`].
struct PackStyles<'a>(&'a Registry<String>);

impl StyleSource for PackStyles<'_> {
    fn load(&self, name: &str) -> Result<Style, StyleError> {
        let entry = self
            .0
            .resolve(name)
            .map_err(|_| StyleError::NotFound(name.to_owned()))?;
        let bare = name.rsplit_once(':').map_or(name, |(_, n)| n);
        Style::parse(bare, &entry.value)
    }
}

impl Presets for Packs {
    fn style(&self, name: &str) -> Option<Vec<Setting>> {
        self.style_presets().style(name)
    }

    fn style_layers(&self, listed: &[String]) -> Vec<Vec<Setting>> {
        self.style_presets().style_layers(listed)
    }

    fn check_styles(&self, listed: &[String]) -> Result<(), PresetFailure> {
        self.style_presets().check_styles(listed)
    }

    fn genre_chain(&self, name: &str) -> Vec<Vec<Setting>> {
        let mut out = Vec::new();
        self.chain(name, &mut HashSet::new(), &mut out);
        out
    }

    fn pack_config(&self, name: &str) -> Option<PackFile> {
        let (id, local) = name.split_once(':')?;
        let id = PackId::new(id).ok()?;
        let pack = self.loaded.iter().find(|p| p.manifest.id == id)?;
        let (path, text) = pack.configs.iter().find(|(p, _)| stem(p) == local)?;
        Some(PackFile {
            display: display_path(&id, path.as_str()),
            text: text.clone(),
        })
    }
}
