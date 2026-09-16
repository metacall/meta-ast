//! Reference resolution via FlattenedScopeCache.
//!
//! Pre-computes the visible scope per file by BFS-ing the import graph
//! once, then resolves references with O(1) lookups instead of
//! per-reference graph traversals.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use rayon::prelude::*;

use crate::error::{Diagnostic, Severity};
use crate::graph::edge::{
    CONFIDENCE_CROSS_LANGUAGE, CONFIDENCE_OWN_OR_DIRECT, CONFIDENCE_TRANSITIVE,
};
use crate::language::LangId;
use crate::model::{FileExtraction, FileId, SourceRange, SymbolId, Visibility};

pub type ScopeMap = HashMap<String, Vec<(SymbolId, f32)>>;
pub(crate) type SymbolIndexEntry = (SymbolId, String, LangId, Option<Visibility>);
pub(crate) type SymbolIndex = HashMap<FileId, Vec<SymbolIndexEntry>>;

/// One visible symbol candidate before shadowing and ranking.
struct Candidate {
    symbol: SymbolId,
    confidence: f32,
    rank: u8,
    path: PathBuf,
    name: String,
}

/// How one resolved import exposes the target file's names.
///
/// Built from the extracted alias, symbol, and star fields at the single
/// place imports resolve. Bindings refine direct imports only; deeper
/// levels stay file-level because re-export analysis is out of scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportBinding {
    /// `from x import *`: every public name is visible.
    Star,
    /// `from x import helper`, `import {helper as h}`: one original name
    /// under one local spelling.
    Named { original: String, local: String },
    /// Anything else (plain and namespace imports, side effects): names
    /// stay file-level, as before. Attribute uses such as `ns.func`
    /// resolve through the bare name today; suppressing them needs
    /// base-aware references, which extraction does not produce yet.
    Unfiltered,
}

/// Classify one extracted import into its binding.
///
/// A star import opens the whole target. A captured symbol names the
/// original; the alias, when present, renames it locally. Python
/// from-imports always capture the name, so a missing alias means the
/// local spelling equals the original. Every other shape keeps the
/// file-level behavior.
pub fn import_binding_for(lang: LangId, import: &crate::model::UnresolvedImport) -> ImportBinding {
    if import.star {
        return ImportBinding::Star;
    }
    match (&import.symbol, &import.alias) {
        (Some(original), Some(alias)) => ImportBinding::Named {
            original: original.clone(),
            local: alias.clone(),
        },
        (Some(original), None) if lang == LangId::Python => ImportBinding::Named {
            original: original.clone(),
            local: original.clone(),
        },
        _ => ImportBinding::Unfiltered,
    }
}

/// Bundles the data needed for scope resolution across files.
pub struct ResolutionContext {
    pub symbol_index: SymbolIndex,
    pub import_adjacency: HashMap<FileId, Vec<FileId>>,
    pub file_languages: HashMap<FileId, LangId>,
    pub file_paths: HashMap<FileId, PathBuf>,
    /// Resolved direct imports with their bindings, grouped by importer.
    /// Absent entries behave as `Unfiltered`.
    pub import_bindings: HashMap<FileId, Vec<(FileId, ImportBinding)>>,
}

impl ResolutionContext {
    /// Build a ResolutionContext from extraction results and graph data.
    pub fn from_extractions<F>(
        extractions: &[F],
        path_to_file_id: &HashMap<PathBuf, FileId>,
        import_adjacency: HashMap<FileId, Vec<FileId>>,
        import_bindings: Vec<(FileId, FileId, ImportBinding)>,
    ) -> Self
    where
        F: std::borrow::Borrow<FileExtraction>,
    {
        let symbol_index = build_symbol_index(extractions, path_to_file_id);
        let file_languages: HashMap<_, _> = extractions
            .iter()
            .filter_map(|f| {
                let f = f.borrow();
                Some((path_to_file_id.get(&f.path)?.to_owned(), f.lang))
            })
            .collect();
        let file_paths: HashMap<_, _> = path_to_file_id
            .iter()
            .map(|(path, &fid)| (fid, path.clone()))
            .collect();

        Self {
            symbol_index,
            import_adjacency,
            file_languages,
            file_paths,
            import_bindings: group_bindings(import_bindings),
        }
    }
}

/// Group resolved bindings by importer, dropping exact duplicates.
fn group_bindings(
    bindings: Vec<(FileId, FileId, ImportBinding)>,
) -> HashMap<FileId, Vec<(FileId, ImportBinding)>> {
    let mut grouped: HashMap<FileId, Vec<(FileId, ImportBinding)>> = HashMap::new();
    for (source, target, binding) in bindings {
        let entry = grouped.entry(source).or_default();
        if !entry.iter().any(|(t, b)| *t == target && *b == binding) {
            entry.push((target, binding));
        }
    }
    grouped
}

/// Pre-computed visible scope for each file.
///
/// Scope = own symbols + public symbols from imported files transitively.
/// Local symbols take priority over imported (shadowing).
#[derive(Debug, Clone, Default)]
pub struct FlattenedScopeCache {
    scopes: HashMap<FileId, ScopeMap>,
}

impl FlattenedScopeCache {
    /// Build the scope cache from the file->symbols index and import adjacency.
    ///
    /// For each file, BFS over import edges, collecting public symbols from
    /// reachable files. Direct imports expose names through their binding;
    /// deeper levels stay file-level. Confidence decays with distance:
    /// - 1.0: own file or direct import, same language
    /// - 0.8: transitive import, same language
    /// - 0.6: cross-language imports
    pub fn build(ctx: &ResolutionContext, diagnostics: &mut Vec<Diagnostic>) -> Self {
        let mut results: Vec<(FileId, ScopeMap, Vec<Diagnostic>)> = ctx
            .symbol_index
            .par_iter()
            .map(|(&file_id, _)| {
                let (scope, diags) = Self::compute_scope(file_id, ctx);
                (file_id, scope, diags)
            })
            .collect();

        // The parallel pass returns in hash order; diagnostics must follow the
        // file path so two runs report the same sequence.
        results.sort_by(|a, b| ctx.file_paths.get(&a.0).cmp(&ctx.file_paths.get(&b.0)));

        let mut scopes = HashMap::with_capacity(results.len());
        for (file_id, scope, diags) in results {
            scopes.insert(file_id, scope);
            diagnostics.extend(diags);
        }

        Self { scopes }
    }

    fn compute_scope(file_id: FileId, ctx: &ResolutionContext) -> (ScopeMap, Vec<Diagnostic>) {
        let mut diagnostics = Vec::new();
        let source_lang = ctx.file_languages.get(&file_id).copied();
        let mut scope: ScopeMap = HashMap::new();
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut visited: HashSet<FileId> = HashSet::new();
        let mut queue: VecDeque<(FileId, usize)> = VecDeque::new();

        queue.push_back((file_id, 0));

        // This file's direct import bindings, grouped by target. An unknown
        // direct edge behaves as unfiltered.
        let mut direct: HashMap<FileId, Vec<ImportBinding>> = HashMap::new();
        if let Some(edges) = ctx.import_bindings.get(&file_id) {
            for (target, binding) in edges {
                let entry: &mut Vec<ImportBinding> = direct.entry(*target).or_default();
                if !entry.contains(binding) {
                    entry.push(binding.clone());
                }
            }
        }

        while let Some((current, distance)) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }

            if let Some(symbols) = ctx.symbol_index.get(&current) {
                let default_vis = ctx
                    .file_languages
                    .get(&current)
                    .map(|lang| lang.spec().default_visibility)
                    .unwrap_or(crate::language::DefaultVisibility::PublicByDefault);

                for (sym_id, name, sym_lang, visibility) in symbols {
                    let is_public = match visibility {
                        Some(Visibility::Public) => true,
                        Some(Visibility::Private) => current == file_id,
                        None => {
                            matches!(
                                default_vis,
                                crate::language::DefaultVisibility::PublicByDefault
                            ) || current == file_id
                        }
                    };

                    if !is_public {
                        continue;
                    }

                    let same_lang = source_lang.is_some() && source_lang == Some(*sym_lang);
                    let diff_lang = source_lang.is_some() && source_lang != Some(*sym_lang);

                    let confidence = if distance == 0 || (distance == 1 && same_lang) {
                        CONFIDENCE_OWN_OR_DIRECT
                    } else if diff_lang {
                        CONFIDENCE_CROSS_LANGUAGE
                    } else {
                        CONFIDENCE_TRANSITIVE
                    };

                    // Rank classes follow the shadowing rule: the own file wins,
                    // then a direct same-language import, then a transitive one,
                    // and a cross-language import comes last.
                    let rank = if distance == 0 {
                        0u8
                    } else if distance == 1 && same_lang {
                        1
                    } else if same_lang {
                        2
                    } else {
                        3
                    };

                    for exposed in exposed_names(distance, name, direct.get(&current)) {
                        candidates.push(Candidate {
                            symbol: *sym_id,
                            name: exposed,
                            confidence,
                            rank,
                            path: ctx.file_paths.get(&current).cloned().unwrap_or_default(),
                        });
                    }
                }
            }

            if let Some(neighbors) = ctx.import_adjacency.get(&current) {
                for &neighbor in neighbors {
                    if !visited.contains(&neighbor) {
                        queue.push_back((neighbor, distance + 1));
                    } else if neighbor == file_id {
                        let path = ctx
                            .file_paths
                            .get(&current)
                            .cloned()
                            .unwrap_or_else(|| PathBuf::from("<unknown>"));
                        let root_path = ctx
                            .file_paths
                            .get(&file_id)
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "<unknown>".to_string());
                        diagnostics.push(Diagnostic {
                            path,
                            severity: Severity::Warning,
                            message: format!(
                                "circular import: {} -> {}",
                                current.to_raw(),
                                root_path
                            ),
                            source_range: None,
                        });
                    }
                }
            }
        }

        // Nearer definitions shadow farther ones. The remaining candidates keep
        // a total order: rank, confidence, path, then identifier. The raw
        // identifier comes last, because it is unique only inside a run.
        let mut grouped: HashMap<String, Vec<Candidate>> = HashMap::new();
        for candidate in candidates {
            grouped
                .entry(candidate.name.clone())
                .or_default()
                .push(candidate);
        }
        for (name, mut group) in grouped {
            let best = group.iter().map(|candidate| candidate.rank).min();
            if let Some(best) = best {
                group.retain(|candidate| candidate.rank == best);
            }
            group.sort_by(|a, b| {
                a.rank
                    .cmp(&b.rank)
                    .then(b.confidence.total_cmp(&a.confidence))
                    .then(a.path.cmp(&b.path))
                    .then(a.symbol.to_raw().cmp(&b.symbol.to_raw()))
            });
            scope.insert(
                name,
                group
                    .into_iter()
                    .map(|candidate| (candidate.symbol, candidate.confidence))
                    .collect(),
            );
        }

        (scope, diagnostics)
    }

    /// Look up a name in a file's flattened scope.
    ///
    /// Returns matching symbols with confidence scores, or None if not found.
    pub fn resolve(&self, file_id: FileId, name: &str) -> Option<&[(SymbolId, f32)]> {
        self.scopes
            .get(&file_id)
            .and_then(|s| s.get(name).map(|v| v.as_slice()))
    }

    /// Full scope for a file: name to candidates with confidence.
    pub fn scope(&self, file_id: FileId) -> Option<&ScopeMap> {
        self.scopes.get(&file_id)
    }

    /// Iterate over every file scope.
    pub fn iter_scopes(&self) -> impl Iterator<Item = (FileId, &ScopeMap)> {
        self.scopes.iter().map(|(&file_id, scope)| (file_id, scope))
    }

    /// Returns the number of scopes in the cache.
    pub fn len(&self) -> usize {
        self.scopes.len()
    }

    /// Returns true if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }
}

/// One resolved use of a name, with the site that produced it.
///
/// The graph edge alone names the source and target symbols; this record keeps
/// the reference range too, so a consumer can point at the exact use instead of
/// re-deriving the mapping from names.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedReference {
    /// Path of the referencing file, as extracted.
    pub file_path: PathBuf,
    /// Range of the unresolved reference itself.
    pub range: SourceRange,
    /// Symbol that contains the reference.
    pub source: SymbolId,
    /// Symbol the name resolves to.
    pub target: SymbolId,
    /// Confidence threaded from the scope cache.
    pub confidence: f32,
}

/// Resolve every reference and keep its use site.
///
/// Records follow extraction order and, within one file, reference order. A
/// reference with no scope match emits one Warning and no record; the warnings
/// are appended to `diagnostics` exactly as `resolve_all_references` reports
/// them. Confidence is threaded from the `FlattenedScopeCache` (1.0
/// local/direct, 0.8 transitive, 0.6 cross-language).
///
/// A self-recursive reference, such as a function that calls itself, keeps
/// `source == target`. The record is a real use site; the graph folds it into a
/// self-loop `Reference` edge, which is what classifies a self-recursive unit
/// as a `SelfLoop` deployability hint.
pub fn resolve_references_detailed<F>(
    extractions: &[F],
    path_to_file_id: &HashMap<PathBuf, FileId>,
    scope_cache: &FlattenedScopeCache,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<ResolvedReference>
where
    F: std::borrow::Borrow<FileExtraction> + Sync,
{
    #[allow(clippy::type_complexity)]
    let results: Vec<(Vec<ResolvedReference>, Vec<Diagnostic>)> = extractions
        .par_iter()
        .map(|file_ext| {
            let file_ext = file_ext.borrow();
            let mut local_refs = Vec::new();
            let mut local_diags = Vec::new();

            let file_id = match path_to_file_id.get(&file_ext.path) {
                Some(&id) => id,
                None => return (local_refs, local_diags),
            };

            let file_path = &file_ext.path;
            for ref_ in &file_ext.references {
                if let Some(matches) = scope_cache.resolve(file_id, &ref_.name) {
                    // Find the innermost source symbol that contains this reference range
                    // Pick the symbol with the smallest byte span length
                    let source_sym = file_ext
                        .symbols
                        .iter()
                        .filter(|s| {
                            s.source_range.byte_start <= ref_.range.byte_start
                                && s.source_range.byte_end >= ref_.range.byte_end
                        })
                        .min_by_key(|s| s.source_range.byte_end - s.source_range.byte_start);

                    if let Some(source) = source_sym {
                        for &(target_id, confidence) in matches {
                            local_refs.push(ResolvedReference {
                                file_path: file_path.clone(),
                                range: ref_.range.clone(),
                                source: source.id,
                                target: target_id,
                                confidence,
                            });
                        }
                    }
                } else {
                    local_diags.push(Diagnostic {
                        path: file_path.clone(),
                        severity: Severity::Warning,
                        message: format!("unresolved reference: '{}'", ref_.name),
                        source_range: Some(ref_.range.clone()),
                    });
                }
            }
            (local_refs, local_diags)
        })
        .collect();

    let mut resolved = Vec::new();
    for (mut local_refs, local_diags) in results {
        resolved.append(&mut local_refs);
        diagnostics.extend(local_diags);
    }
    resolved
}

/// Fold resolved use sites into one edge per `(source, target)` pair.
///
/// Duplicates max-merge their confidence and the result is ordered by symbol
/// id, so the edge list does not depend on discovery or resolution order. A
/// self-recursive use site folds into a self edge, which is intended: the SCC
/// pass reports such a unit as a `SelfLoop` hint.
pub fn reference_edges(resolved: &[ResolvedReference]) -> Vec<(SymbolId, SymbolId, f32)> {
    let mut seen: HashMap<(SymbolId, SymbolId), f32> = HashMap::with_capacity(resolved.len());
    for reference in resolved {
        seen.entry((reference.source, reference.target))
            .and_modify(|confidence| *confidence = confidence.max(reference.confidence))
            .or_insert(reference.confidence);
    }
    let mut edges: Vec<_> = seen
        .into_iter()
        .map(|((source, target), confidence)| (source, target, confidence))
        .collect();
    edges.sort_by_key(|(source, target, _)| (source.to_raw(), target.to_raw()));
    edges
}

/// Resolve all references across extracted files.
///
/// Returns a list of (source_symbol_id, target_symbol_id, confidence) triples
/// representing ReferenceEdges to add. Confidence is threaded from the
/// FlattenedScopeCache (1.0 local/direct, 0.8 transitive, 0.6 cross-language).
/// Warnings for unresolved references are appended to `diagnostics`.
pub fn resolve_all_references<F>(
    extractions: &[F],
    path_to_file_id: &HashMap<PathBuf, FileId>,
    scope_cache: &FlattenedScopeCache,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<(SymbolId, SymbolId, f32)>
where
    F: std::borrow::Borrow<FileExtraction> + Sync,
{
    let resolved =
        resolve_references_detailed(extractions, path_to_file_id, scope_cache, diagnostics);
    reference_edges(&resolved)
}

/// Build a symbol index from extracted files and a path-to-FileId mapping.
///
/// Returns: SymbolIndex
pub fn build_symbol_index<F>(
    extractions: &[F],
    path_to_file_id: &HashMap<PathBuf, FileId>,
) -> SymbolIndex
where
    F: std::borrow::Borrow<FileExtraction>,
{
    let mut index: SymbolIndex = HashMap::new();

    for file_ext in extractions {
        let file_ext = file_ext.borrow();
        if let Some(&file_id) = path_to_file_id.get(&file_ext.path) {
            let entries: Vec<_> = file_ext
                .symbols
                .iter()
                .map(|s| (s.id, s.name.clone(), s.language, s.visibility))
                .collect();
            index.entry(file_id).or_default().extend(entries);
        }
    }

    index
}

/// Names under which one target symbol enters the importer's scope.
///
/// A direct pair whose records are all precise (named or star) exposes
/// exactly its declared names. Any unfiltered record keeps file scope and
/// each named record additionally exposes its local spelling, so a rename
/// resolves without narrowing the rest. Deeper levels always keep file
/// scope: bindings describe direct imports.
fn exposed_names(
    distance: usize,
    name: &str,
    bindings: Option<&Vec<ImportBinding>>,
) -> Vec<String> {
    let direct = bindings.is_some() && distance == 1;
    let list = bindings.filter(|_| direct);
    let precise = list.is_some_and(|list| {
        !list.is_empty()
            && list
                .iter()
                .all(|b| matches!(b, ImportBinding::Star | ImportBinding::Named { .. }))
    });
    let open =
        !precise || list.is_some_and(|list| list.iter().any(|b| matches!(b, ImportBinding::Star)));

    let mut out = Vec::new();
    if open {
        out.push(name.to_owned());
    }
    if let Some(list) = list {
        out.extend(list.iter().filter_map(|binding| match binding {
            ImportBinding::Named { original, local } if original == name && local != name => {
                Some(local.clone())
            }
            _ => None,
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn empty_cache() {
        let cache = FlattenedScopeCache {
            scopes: HashMap::new(),
        };
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert!(cache.resolve(FileId::new(1).unwrap(), "foo").is_none());
    }

    #[test]
    fn detailed_resolution_keeps_each_use_site_and_the_triples_are_unchanged() {
        use crate::model::{LineColumn, Symbol, SymbolKind, UnresolvedReference};

        fn range(start: usize, end: usize) -> SourceRange {
            SourceRange {
                byte_start: start,
                byte_end: end,
                start: LineColumn {
                    line: 0,
                    column: start,
                },
                end: LineColumn {
                    line: 0,
                    column: end,
                },
            }
        }

        fn symbol(id: u32, name: &str, start: usize, end: usize, path: &std::path::Path) -> Symbol {
            Symbol {
                id: SymbolId::new(id).unwrap(),
                name: name.to_string(),
                kind: SymbolKind::Function,
                language: LangId::Python,
                file_path: path.to_path_buf(),
                source_range: range(start, end),
                name_range: None,
                visibility: None,
                signature: None,
                docstring: None,
                is_async: false,
            }
        }

        let path = PathBuf::from("a.py");
        let file_id = FileId::new(1).unwrap();
        let mut file = FileExtraction::empty(path.clone(), LangId::Python);
        file.symbols = vec![
            symbol(1, "caller", 0, 100, &path),
            symbol(2, "helper", 200, 210, &path),
        ];
        file.references = vec![
            UnresolvedReference {
                name: "helper".into(),
                range: range(10, 16),
            },
            UnresolvedReference {
                name: "helper".into(),
                range: range(30, 36),
            },
        ];
        let mut symbol_index: SymbolIndex = HashMap::new();
        symbol_index.insert(
            file_id,
            vec![
                (
                    SymbolId::new(1).unwrap(),
                    "caller".into(),
                    LangId::Python,
                    None,
                ),
                (
                    SymbolId::new(2).unwrap(),
                    "helper".into(),
                    LangId::Python,
                    None,
                ),
            ],
        );
        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::new(),
            file_languages: HashMap::from([(file_id, LangId::Python)]),
            file_paths: HashMap::from([(file_id, path.clone())]),
            import_bindings: HashMap::new(),
        };
        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        let extractions = vec![file];
        let paths = HashMap::from([(path, file_id)]);

        let mut diagnostics = Vec::new();
        let resolved = resolve_references_detailed(&extractions, &paths, &cache, &mut diagnostics);

        assert!(diagnostics.is_empty());
        assert_eq!(resolved.len(), 2, "one record per use site");
        assert_eq!(resolved[0].range.byte_start, 10);
        assert_eq!(resolved[1].range.byte_start, 30);
        assert!(
            resolved
                .iter()
                .all(|record| record.source == SymbolId::new(1).unwrap())
        );
        assert!(
            resolved.iter().all(
                |record| record.target == SymbolId::new(2).unwrap() && record.confidence == 1.0
            )
        );

        let mut wrapper_diagnostics = Vec::new();
        let triples =
            resolve_all_references(&extractions, &paths, &cache, &mut wrapper_diagnostics);
        assert!(wrapper_diagnostics.is_empty());
        assert_eq!(
            triples,
            vec![(SymbolId::new(1).unwrap(), SymbolId::new(2).unwrap(), 1.0)],
            "the wrapper max-merges the two use sites into one edge"
        );
    }

    #[test]
    fn scope_cache_resolve_own_file() {
        let mut symbol_index: SymbolIndex = HashMap::new();
        symbol_index.insert(
            FileId::new(1).unwrap(),
            vec![(
                SymbolId::new(10).unwrap(),
                "main".into(),
                LangId::Python,
                None,
            )],
        );

        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::new(),
            file_languages: HashMap::from([(FileId::new(1).unwrap(), LangId::Python)]),
            file_paths: HashMap::new(),
            import_bindings: HashMap::new(),
        };

        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        let result = cache.resolve(FileId::new(1).unwrap(), "main");
        assert!(result.is_some());
        let matches = result.unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, SymbolId::new(10).unwrap());
        assert_eq!(matches[0].1, 1.0);
    }

    #[test]
    fn scope_cache_resolve_imported_symbol() {
        let mut symbol_index = HashMap::new();
        symbol_index.insert(FileId::new(1).unwrap(), vec![]);
        symbol_index.insert(
            FileId::new(2).unwrap(),
            vec![(
                SymbolId::new(20).unwrap(),
                "helper".into(),
                LangId::Python,
                Some(Visibility::Public),
            )],
        );

        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::from([(
                FileId::new(1).unwrap(),
                vec![FileId::new(2).unwrap()],
            )]),
            file_languages: HashMap::from([
                (FileId::new(1).unwrap(), LangId::Python),
                (FileId::new(2).unwrap(), LangId::Python),
            ]),
            file_paths: HashMap::new(),
            import_bindings: HashMap::new(),
        };

        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        let result = cache.resolve(FileId::new(1).unwrap(), "helper");
        assert!(result.is_some());
        let matches = result.unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, SymbolId::new(20).unwrap());
        assert_eq!(matches[0].1, 1.0);
    }

    #[test]
    fn scope_cache_missing_symbol() {
        let mut symbol_index = HashMap::new();
        symbol_index.insert(
            FileId::new(1).unwrap(),
            vec![(
                SymbolId::new(10).unwrap(),
                "foo".into(),
                LangId::Python,
                None,
            )],
        );

        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::new(),
            file_languages: HashMap::from([(FileId::new(1).unwrap(), LangId::Python)]),
            file_paths: HashMap::new(),
            import_bindings: HashMap::new(),
        };

        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        assert!(cache.resolve(FileId::new(1).unwrap(), "bar").is_none());
    }

    #[test]
    fn scope_cache_cycle_safe() {
        let mut symbol_index = HashMap::new();
        symbol_index.insert(
            FileId::new(1).unwrap(),
            vec![(
                SymbolId::new(10).unwrap(),
                "a".into(),
                LangId::Python,
                Some(Visibility::Public),
            )],
        );
        symbol_index.insert(
            FileId::new(2).unwrap(),
            vec![(
                SymbolId::new(20).unwrap(),
                "b".into(),
                LangId::Python,
                Some(Visibility::Public),
            )],
        );

        // Cycle: 0 -> 1 -> 0
        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::from([
                (FileId::new(1).unwrap(), vec![FileId::new(2).unwrap()]),
                (FileId::new(2).unwrap(), vec![FileId::new(1).unwrap()]),
            ]),
            file_languages: HashMap::from([
                (FileId::new(1).unwrap(), LangId::Python),
                (FileId::new(2).unwrap(), LangId::Python),
            ]),
            file_paths: HashMap::new(),
            import_bindings: HashMap::new(),
        };

        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        // Should not infinite loop
        assert!(cache.resolve(FileId::new(1).unwrap(), "b").is_some());
        assert!(cache.resolve(FileId::new(2).unwrap(), "a").is_some());
    }

    #[test]
    fn scope_cache_cross_language_confidence() {
        let mut symbol_index = HashMap::new();
        symbol_index.insert(FileId::new(1).unwrap(), vec![]);
        symbol_index.insert(
            FileId::new(2).unwrap(),
            vec![(
                SymbolId::new(20).unwrap(),
                "util".into(),
                LangId::Rust,
                Some(Visibility::Public),
            )],
        );

        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::from([(
                FileId::new(1).unwrap(),
                vec![FileId::new(2).unwrap()],
            )]),
            file_languages: HashMap::from([
                (FileId::new(1).unwrap(), LangId::Python),
                (FileId::new(2).unwrap(), LangId::Rust),
            ]),
            file_paths: HashMap::new(),
            import_bindings: HashMap::new(),
        };

        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        let result = cache.resolve(FileId::new(1).unwrap(), "util");
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].1, 0.6);
    }

    #[test]
    fn resolve_references_creates_edges() {
        use crate::model::{LineColumn, SourceRange, Symbol, SymbolKind, UnresolvedReference};

        let sym_a = Symbol {
            id: SymbolId::new(1).unwrap(),
            name: "caller".into(),
            kind: SymbolKind::Function,
            language: LangId::Python,
            file_path: PathBuf::from("/proj/a.py"),
            source_range: SourceRange {
                byte_start: 0,
                byte_end: 50,
                start: LineColumn { line: 0, column: 0 },
                end: LineColumn { line: 2, column: 0 },
            },
            name_range: None,
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        };

        let mut file = FileExtraction::empty(PathBuf::from("/proj/a.py"), LangId::Python);
        file.symbols = vec![sym_a];
        file.references = vec![UnresolvedReference {
            name: "helper".into(),
            range: SourceRange {
                byte_start: 20,
                byte_end: 26,
                start: LineColumn { line: 1, column: 4 },
                end: LineColumn {
                    line: 1,
                    column: 10,
                },
            },
        }];

        let mut path_to_file_id = HashMap::new();
        path_to_file_id.insert(PathBuf::from("/proj/a.py"), FileId::new(1).unwrap());

        let mut scopes: HashMap<FileId, ScopeMap> = HashMap::new();
        let mut scope = HashMap::new();
        scope.insert("helper".into(), vec![(SymbolId::new(99).unwrap(), 1.0)]);
        scopes.insert(FileId::new(1).unwrap(), scope);

        let cache = FlattenedScopeCache { scopes };

        let edges = resolve_all_references(&[file], &path_to_file_id, &cache, &mut Vec::new());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, SymbolId::new(1).unwrap());
        assert_eq!(edges[0].1, SymbolId::new(99).unwrap());
        assert_eq!(edges[0].2, 1.0);
    }

    #[test]
    fn resolve_references_selects_innermost_enclosing_symbol_regardless_of_vector_order() {
        use crate::model::{LineColumn, SourceRange, Symbol, SymbolKind, UnresolvedReference};

        // Inner method (span 40: 10..50)
        let inner_method = Symbol {
            id: SymbolId::new(1).unwrap(),
            name: "inner_method".into(),
            kind: SymbolKind::Method,
            language: LangId::Python,
            file_path: PathBuf::from("/proj/a.py"),
            source_range: SourceRange {
                byte_start: 10,
                byte_end: 50,
                start: LineColumn { line: 1, column: 0 },
                end: LineColumn { line: 3, column: 0 },
            },
            name_range: None,
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        };

        // Outer class (span 100: 0..100) placed AFTER inner_method in vector
        let outer_class = Symbol {
            id: SymbolId::new(2).unwrap(),
            name: "OuterClass".into(),
            kind: SymbolKind::Class,
            language: LangId::Python,
            file_path: PathBuf::from("/proj/a.py"),
            source_range: SourceRange {
                byte_start: 0,
                byte_end: 100,
                start: LineColumn { line: 0, column: 0 },
                end: LineColumn { line: 5, column: 0 },
            },
            name_range: None,
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        };

        let mut file = FileExtraction::empty(PathBuf::from("/proj/a.py"), LangId::Python);
        file.symbols = vec![inner_method, outer_class]; // Order: inner first, outer second
        file.references = vec![UnresolvedReference {
            name: "helper".into(),
            range: SourceRange {
                byte_start: 20,
                byte_end: 26,
                start: LineColumn { line: 2, column: 4 },
                end: LineColumn {
                    line: 2,
                    column: 10,
                },
            },
        }];

        let mut path_to_file_id = HashMap::new();
        path_to_file_id.insert(PathBuf::from("/proj/a.py"), FileId::new(1).unwrap());

        let mut scopes: HashMap<FileId, ScopeMap> = HashMap::new();
        let mut scope = HashMap::new();
        scope.insert("helper".into(), vec![(SymbolId::new(99).unwrap(), 1.0)]);
        scopes.insert(FileId::new(1).unwrap(), scope);

        let cache = FlattenedScopeCache { scopes };

        let edges = resolve_all_references(&[file], &path_to_file_id, &cache, &mut Vec::new());
        assert_eq!(edges.len(), 1);
        // Must resolve from SymbolId(1) (inner_method), NOT SymbolId(2) (outer_class)
        assert_eq!(
            edges[0].0,
            SymbolId::new(1).unwrap(),
            "Reference should attach to innermost symbol SymbolId(1), but attached to SymbolId({})",
            edges[0].0.to_raw()
        );
        assert_eq!(edges[0].1, SymbolId::new(99).unwrap());
    }

    #[test]
    fn local_symbol_shadows_the_imported_symbol() {
        let mut symbol_index: SymbolIndex = HashMap::new();
        symbol_index.insert(
            FileId::new(1).unwrap(),
            vec![(
                SymbolId::new(10).unwrap(),
                "helper".into(),
                LangId::Python,
                Some(Visibility::Public),
            )],
        );
        symbol_index.insert(
            FileId::new(2).unwrap(),
            vec![(
                SymbolId::new(20).unwrap(),
                "helper".into(),
                LangId::Python,
                Some(Visibility::Public),
            )],
        );

        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::from([(
                FileId::new(1).unwrap(),
                vec![FileId::new(2).unwrap()],
            )]),
            file_languages: HashMap::from([
                (FileId::new(1).unwrap(), LangId::Python),
                (FileId::new(2).unwrap(), LangId::Python),
            ]),
            file_paths: HashMap::from([
                (FileId::new(1).unwrap(), PathBuf::from("/proj/main.py")),
                (FileId::new(2).unwrap(), PathBuf::from("/proj/lib.py")),
            ]),
            import_bindings: HashMap::new(),
        };

        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        let matches = cache.resolve(FileId::new(1).unwrap(), "helper").unwrap();
        assert_eq!(
            matches.len(),
            1,
            "the local definition must shadow the imported one"
        );
        assert_eq!(matches[0].0, SymbolId::new(10).unwrap());
        assert_eq!(matches[0].1, 1.0);
    }

    #[test]
    fn imported_candidates_are_ranked_by_path_not_by_id() {
        let mut symbol_index: SymbolIndex = HashMap::new();
        symbol_index.insert(FileId::new(1).unwrap(), vec![]);
        // File 2 sorts before file 3 by path but holds the higher symbol id.
        symbol_index.insert(
            FileId::new(2).unwrap(),
            vec![(
                SymbolId::new(30).unwrap(),
                "util".into(),
                LangId::Python,
                Some(Visibility::Public),
            )],
        );
        symbol_index.insert(
            FileId::new(3).unwrap(),
            vec![(
                SymbolId::new(20).unwrap(),
                "util".into(),
                LangId::Python,
                Some(Visibility::Public),
            )],
        );

        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::from([(
                FileId::new(1).unwrap(),
                vec![FileId::new(2).unwrap(), FileId::new(3).unwrap()],
            )]),
            file_languages: HashMap::from([
                (FileId::new(1).unwrap(), LangId::Python),
                (FileId::new(2).unwrap(), LangId::Python),
                (FileId::new(3).unwrap(), LangId::Python),
            ]),
            file_paths: HashMap::from([
                (FileId::new(1).unwrap(), PathBuf::from("/proj/app.py")),
                (FileId::new(2).unwrap(), PathBuf::from("/proj/a_util.py")),
                (FileId::new(3).unwrap(), PathBuf::from("/proj/z_util.py")),
            ]),
            import_bindings: HashMap::new(),
        };

        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        let matches = cache.resolve(FileId::new(1).unwrap(), "util").unwrap();
        assert_eq!(
            matches.len(),
            2,
            "an ambiguous import keeps both candidates"
        );
        assert_eq!(
            matches[0].0,
            SymbolId::new(30).unwrap(),
            "candidate order must follow the file path, not the raw symbol id"
        );
    }

    fn binding_ctx(
        bindings: Vec<(FileId, FileId, ImportBinding)>,
    ) -> (ResolutionContext, FileId, SymbolId) {
        let app = FileId::new(1).unwrap();
        let lib = FileId::new(2).unwrap();
        let foo = SymbolId::new(20).unwrap();
        let mut symbol_index: SymbolIndex = HashMap::new();
        symbol_index.insert(app, vec![]);
        symbol_index.insert(
            lib,
            vec![
                (
                    foo,
                    "foo".into(),
                    LangId::JavaScript,
                    Some(Visibility::Public),
                ),
                (
                    SymbolId::new(21).unwrap(),
                    "other".into(),
                    LangId::JavaScript,
                    Some(Visibility::Public),
                ),
            ],
        );
        let mut grouped: HashMap<FileId, Vec<(FileId, ImportBinding)>> = HashMap::new();
        for (source, target, binding) in bindings {
            grouped.entry(source).or_default().push((target, binding));
        }
        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::from([(app, vec![lib])]),
            file_languages: HashMap::from([(app, LangId::JavaScript), (lib, LangId::JavaScript)]),
            file_paths: HashMap::from([
                (app, PathBuf::from("/proj/app.js")),
                (lib, PathBuf::from("/proj/utils.js")),
            ]),
            import_bindings: grouped,
        };
        (ctx, app, foo)
    }

    #[test]
    fn named_binding_exposes_the_local_spelling_only() {
        let (ctx, app, foo) = binding_ctx(vec![(
            FileId::new(1).unwrap(),
            FileId::new(2).unwrap(),
            ImportBinding::Named {
                original: "foo".into(),
                local: "bar".into(),
            },
        )]);
        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());

        let bar = cache.resolve(app, "bar").unwrap();
        assert_eq!(bar.len(), 1);
        assert_eq!(bar[0].0, foo);
        assert_eq!(bar[0].1, 1.0);
        assert!(
            cache.resolve(app, "foo").is_none(),
            "the original spelling is not in scope under a rename"
        );
        assert!(
            cache.resolve(app, "other").is_none(),
            "names outside the import stay out of scope"
        );
    }

    #[test]
    fn star_binding_exposes_everything() {
        let (ctx, app, foo) = binding_ctx(vec![(
            FileId::new(1).unwrap(),
            FileId::new(2).unwrap(),
            ImportBinding::Star,
        )]);
        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());

        assert_eq!(cache.resolve(app, "foo").unwrap()[0].0, foo);
        assert!(
            cache.resolve(app, "other").is_some(),
            "a star import opens the whole target"
        );
    }

    #[test]
    fn unfiltered_binding_keeps_file_scope() {
        let (ctx, app, foo) = binding_ctx(vec![(
            FileId::new(1).unwrap(),
            FileId::new(2).unwrap(),
            ImportBinding::Unfiltered,
        )]);
        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());

        assert_eq!(cache.resolve(app, "foo").unwrap()[0].0, foo);
        assert!(
            cache.resolve(app, "other").is_some(),
            "plain imports keep the file-level behavior"
        );
    }

    #[test]
    fn mixed_pair_adds_the_local_spelling_without_narrowing() {
        let (ctx, app, foo) = binding_ctx(vec![
            (
                FileId::new(1).unwrap(),
                FileId::new(2).unwrap(),
                ImportBinding::Unfiltered,
            ),
            (
                FileId::new(1).unwrap(),
                FileId::new(2).unwrap(),
                ImportBinding::Named {
                    original: "foo".into(),
                    local: "bar".into(),
                },
            ),
        ]);
        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());

        let bar = cache.resolve(app, "bar").unwrap();
        assert_eq!(bar.len(), 1);
        assert_eq!(bar[0].0, foo);
        assert!(
            cache.resolve(app, "foo").is_some(),
            "the unfiltered record keeps the own spelling"
        );
        assert!(
            cache.resolve(app, "other").is_some(),
            "the unfiltered record keeps the rest of the file"
        );
    }

    #[test]
    fn import_binding_for_classifies_extraction_shapes() {
        use crate::model::UnresolvedImport;

        fn import(symbol: Option<&str>, alias: Option<&str>, star: bool) -> UnresolvedImport {
            UnresolvedImport {
                import_specifier: "target".into(),
                alias: alias.map(str::to_owned),
                symbol: symbol.map(str::to_owned),
                star,
                range: crate::model::SourceRange {
                    byte_start: 0,
                    byte_end: 6,
                    start: crate::model::LineColumn { line: 0, column: 0 },
                    end: crate::model::LineColumn { line: 0, column: 6 },
                },
            }
        }

        assert_eq!(
            import_binding_for(LangId::Python, &import(None, None, true)),
            ImportBinding::Star
        );
        assert_eq!(
            import_binding_for(LangId::JavaScript, &import(Some("foo"), Some("bar"), false)),
            ImportBinding::Named {
                original: "foo".into(),
                local: "bar".into()
            }
        );
        assert_eq!(
            import_binding_for(LangId::Python, &import(Some("helper"), None, false)),
            ImportBinding::Named {
                original: "helper".into(),
                local: "helper".into()
            }
        );
        assert_eq!(
            import_binding_for(LangId::JavaScript, &import(Some("ns"), None, false)),
            ImportBinding::Unfiltered,
            "a bare captured name without alias keeps file scope"
        );
        assert_eq!(
            import_binding_for(LangId::Python, &import(None, None, false)),
            ImportBinding::Unfiltered
        );
        assert_eq!(
            import_binding_for(LangId::Go, &import(None, Some("alias"), false)),
            ImportBinding::Unfiltered
        );
    }
}
