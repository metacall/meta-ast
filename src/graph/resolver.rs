//! Reference resolution via FlattenedScopeCache.
//!
//! Pre-computes the visible scope per file by DFS-ing the import graph
//! once, then resolves references with O(1) lookups instead of
//! per-reference BFS.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use rayon::prelude::*;

use crate::error::{Diagnostic, Severity};
use crate::language::LangId;
use crate::model::{FileExtraction, FileId, SymbolId, Visibility};

type ScopeMap = HashMap<String, Vec<(SymbolId, f32)>>;
pub(crate) type SymbolIndexEntry = (SymbolId, String, LangId, Option<Visibility>);
pub(crate) type SymbolIndex = HashMap<FileId, Vec<SymbolIndexEntry>>;

/// Bundles the data needed for scope resolution across files.
pub struct ResolutionContext {
    pub symbol_index: SymbolIndex,
    pub import_adjacency: HashMap<FileId, Vec<FileId>>,
    pub file_languages: HashMap<FileId, LangId>,
    pub file_paths: HashMap<FileId, PathBuf>,
}

impl ResolutionContext {
    /// Build a ResolutionContext from extraction results and graph data.
    pub fn from_extractions(
        extractions: &[FileExtraction],
        path_to_file_id: &HashMap<PathBuf, FileId>,
        import_adjacency: HashMap<FileId, Vec<FileId>>,
    ) -> Self {
        let symbol_index = build_symbol_index(extractions, path_to_file_id);
        let file_languages: HashMap<_, _> = extractions
            .iter()
            .filter_map(|f| Some((path_to_file_id.get(&f.path)?.to_owned(), f.lang)))
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
        }
    }
}

/// Pre-computed visible scope for each file.
///
/// Scope = own symbols + public symbols from imported files transitively.
/// Local symbols take priority over imported (shadowing).
pub struct FlattenedScopeCache {
    scopes: HashMap<FileId, ScopeMap>,
}

impl FlattenedScopeCache {
    /// Build the scope cache from the file->symbols index and import adjacency.
    ///
    /// For each file, DFS over import edges, collecting public symbols from
    /// reachable files. Confidence decays with distance:
    /// - 1.0: own file or direct import, same language
    /// - 0.8: transitive import, same language
    /// - 0.6: cross-language imports
    pub fn build(ctx: &ResolutionContext, diagnostics: &mut Vec<Diagnostic>) -> Self {
        let results: Vec<(FileId, ScopeMap, Vec<Diagnostic>)> = ctx
            .symbol_index
            .par_iter()
            .map(|(&file_id, _)| {
                let (scope, diags) = Self::compute_scope(file_id, ctx);
                (file_id, scope, diags)
            })
            .collect();

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
        let mut visited: HashSet<FileId> = HashSet::new();
        let mut queue: VecDeque<(FileId, usize)> = VecDeque::new();

        queue.push_back((file_id, 0));

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
                        1.0
                    } else if diff_lang {
                        0.6
                    } else {
                        0.8
                    };

                    if let Some(entries) = scope.get_mut(name) {
                        entries.push((*sym_id, confidence));
                    } else {
                        scope.insert(name.clone(), vec![(*sym_id, confidence)]);
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
                            message: format!("circular import: {} -> {}", current.0, root_path),
                            source_range: None,
                        });
                    }
                }
            }
        }

        // Sort each entry: higher confidence first, then by symbol_id (stable)
        for entries in scope.values_mut() {
            entries.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.0.0.cmp(&b.0.0))
            });
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

    /// Returns the number of scopes in the cache.
    pub fn len(&self) -> usize {
        self.scopes.len()
    }

    /// Returns true if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }
}

/// Resolve all references across extracted files.
///
/// Returns a list of (source_symbol_id, target_symbol_id, confidence) triples
/// representing ReferenceEdges to add. Confidence is threaded from the
/// FlattenedScopeCache (1.0 local/direct, 0.8 transitive, 0.6 cross-language).
/// Warnings for unresolved references are appended to `diagnostics`.
pub fn resolve_all_references(
    extractions: &[FileExtraction],
    path_to_file_id: &HashMap<PathBuf, FileId>,
    scope_cache: &FlattenedScopeCache,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<(SymbolId, SymbolId, f32)> {
    #[allow(clippy::type_complexity)]
    let results: Vec<(Vec<(SymbolId, SymbolId, f32)>, Vec<Diagnostic>)> = extractions
        .par_iter()
        .map(|file_ext| {
            let mut local_edges = Vec::new();
            let mut local_diags = Vec::new();

            let file_id = match path_to_file_id.get(&file_ext.path) {
                Some(&id) => id,
                None => return (local_edges, local_diags),
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
                            // Don't add self-references (symbol to itself)
                            if source.id != target_id {
                                local_edges.push((source.id, target_id, confidence));
                            }
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
            (local_edges, local_diags)
        })
        .collect();

    let mut edges = Vec::new();
    for (local_edges, local_diags) in results {
        edges.extend(local_edges);
        diagnostics.extend(local_diags);
    }

    // Deduplicate: max-merge confidence for same (src, dst) pairs
    let mut seen: HashMap<(SymbolId, SymbolId), f32> = HashMap::with_capacity(edges.len());
    for (src, dst, conf) in edges {
        seen.entry((src, dst))
            .and_modify(|e| *e = e.max(conf))
            .or_insert(conf);
    }
    let mut deduped: Vec<_> = seen
        .into_iter()
        .map(|((src, dst), conf)| (src, dst, conf))
        .collect();
    deduped.sort_by_key(|(a, b, _)| (a.0, b.0));
    deduped
}

/// Build a symbol index from extracted files and a path-to-FileId mapping.
///
/// Returns: SymbolIndex
pub fn build_symbol_index(
    extractions: &[FileExtraction],
    path_to_file_id: &HashMap<PathBuf, FileId>,
) -> SymbolIndex {
    let mut index: SymbolIndex = HashMap::new();

    for file_ext in extractions {
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
        assert!(cache.resolve(FileId(0), "foo").is_none());
    }

    #[test]
    fn scope_cache_resolve_own_file() {
        let mut symbol_index: SymbolIndex = HashMap::new();
        symbol_index.insert(
            FileId(0),
            vec![(SymbolId(10), "main".into(), LangId::Python, None)],
        );

        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::new(),
            file_languages: HashMap::from([(FileId(0), LangId::Python)]),
            file_paths: HashMap::new(),
        };

        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        let result = cache.resolve(FileId(0), "main");
        assert!(result.is_some());
        let matches = result.unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, SymbolId(10));
        assert_eq!(matches[0].1, 1.0);
    }

    #[test]
    fn scope_cache_resolve_imported_symbol() {
        let mut symbol_index = HashMap::new();
        symbol_index.insert(FileId(0), vec![]);
        symbol_index.insert(
            FileId(1),
            vec![(
                SymbolId(20),
                "helper".into(),
                LangId::Python,
                Some(Visibility::Public),
            )],
        );

        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::from([(FileId(0), vec![FileId(1)])]),
            file_languages: HashMap::from([
                (FileId(0), LangId::Python),
                (FileId(1), LangId::Python),
            ]),
            file_paths: HashMap::new(),
        };

        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        let result = cache.resolve(FileId(0), "helper");
        assert!(result.is_some());
        let matches = result.unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, SymbolId(20));
        assert_eq!(matches[0].1, 1.0);
    }

    #[test]
    fn scope_cache_missing_symbol() {
        let mut symbol_index = HashMap::new();
        symbol_index.insert(
            FileId(0),
            vec![(SymbolId(10), "foo".into(), LangId::Python, None)],
        );

        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::new(),
            file_languages: HashMap::from([(FileId(0), LangId::Python)]),
            file_paths: HashMap::new(),
        };

        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        assert!(cache.resolve(FileId(0), "bar").is_none());
    }

    #[test]
    fn scope_cache_cycle_safe() {
        let mut symbol_index = HashMap::new();
        symbol_index.insert(
            FileId(0),
            vec![(
                SymbolId(10),
                "a".into(),
                LangId::Python,
                Some(Visibility::Public),
            )],
        );
        symbol_index.insert(
            FileId(1),
            vec![(
                SymbolId(20),
                "b".into(),
                LangId::Python,
                Some(Visibility::Public),
            )],
        );

        // Cycle: 0 -> 1 -> 0
        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::from([
                (FileId(0), vec![FileId(1)]),
                (FileId(1), vec![FileId(0)]),
            ]),
            file_languages: HashMap::from([
                (FileId(0), LangId::Python),
                (FileId(1), LangId::Python),
            ]),
            file_paths: HashMap::new(),
        };

        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        // Should not infinite loop
        assert!(cache.resolve(FileId(0), "b").is_some());
        assert!(cache.resolve(FileId(1), "a").is_some());
    }

    #[test]
    fn scope_cache_cross_language_confidence() {
        let mut symbol_index = HashMap::new();
        symbol_index.insert(FileId(0), vec![]);
        symbol_index.insert(
            FileId(1),
            vec![(
                SymbolId(20),
                "util".into(),
                LangId::Rust,
                Some(Visibility::Public),
            )],
        );

        let ctx = ResolutionContext {
            symbol_index,
            import_adjacency: HashMap::from([(FileId(0), vec![FileId(1)])]),
            file_languages: HashMap::from([(FileId(0), LangId::Python), (FileId(1), LangId::Rust)]),
            file_paths: HashMap::new(),
        };

        let cache = FlattenedScopeCache::build(&ctx, &mut Vec::new());
        let result = cache.resolve(FileId(0), "util");
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].1, 0.6);
    }

    #[test]
    fn resolve_references_creates_edges() {
        use crate::model::{LineColumn, SourceRange, Symbol, SymbolKind, UnresolvedReference};

        let sym_a = Symbol {
            id: SymbolId(1),
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
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        };

        let file = FileExtraction {
            path: PathBuf::from("/proj/a.py"),
            lang: LangId::Python,
            symbols: vec![sym_a],
            imports: vec![],
            references: vec![UnresolvedReference {
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
            }],
            diagnostics: vec![],
            ast_node_count: 0,
            #[cfg(feature = "metacall-deploy")]
            call_sites: vec![],
            #[cfg(feature = "dataflow")]
            data_nodes: vec![],
            #[cfg(feature = "dataflow")]
            flow_edges: vec![],
        };

        let mut path_to_file_id = HashMap::new();
        path_to_file_id.insert(PathBuf::from("/proj/a.py"), FileId(0));

        let mut scopes: HashMap<FileId, ScopeMap> = HashMap::new();
        let mut scope = HashMap::new();
        scope.insert("helper".into(), vec![(SymbolId(99), 1.0)]);
        scopes.insert(FileId(0), scope);

        let cache = FlattenedScopeCache { scopes };

        let edges = resolve_all_references(&[file], &path_to_file_id, &cache, &mut Vec::new());
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, SymbolId(1));
        assert_eq!(edges[0].1, SymbolId(99));
        assert_eq!(edges[0].2, 1.0);
    }

    #[test]
    fn resolve_references_selects_innermost_enclosing_symbol_regardless_of_vector_order() {
        use crate::model::{LineColumn, SourceRange, Symbol, SymbolKind, UnresolvedReference};

        // Inner method (span 40: 10..50)
        let inner_method = Symbol {
            id: SymbolId(1),
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
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        };

        // Outer class (span 100: 0..100) placed AFTER inner_method in vector
        let outer_class = Symbol {
            id: SymbolId(2),
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
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        };

        let file = FileExtraction {
            path: PathBuf::from("/proj/a.py"),
            lang: LangId::Python,
            symbols: vec![inner_method, outer_class], // Order: inner first, outer second
            imports: vec![],
            references: vec![UnresolvedReference {
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
            }],
            diagnostics: vec![],
            ast_node_count: 0,
            #[cfg(feature = "metacall-deploy")]
            call_sites: vec![],
            #[cfg(feature = "dataflow")]
            data_nodes: vec![],
            #[cfg(feature = "dataflow")]
            flow_edges: vec![],
        };

        let mut path_to_file_id = HashMap::new();
        path_to_file_id.insert(PathBuf::from("/proj/a.py"), FileId(0));

        let mut scopes: HashMap<FileId, ScopeMap> = HashMap::new();
        let mut scope = HashMap::new();
        scope.insert("helper".into(), vec![(SymbolId(99), 1.0)]);
        scopes.insert(FileId(0), scope);

        let cache = FlattenedScopeCache { scopes };

        let edges = resolve_all_references(&[file], &path_to_file_id, &cache, &mut Vec::new());
        assert_eq!(edges.len(), 1);
        // Must resolve from SymbolId(1) (inner_method), NOT SymbolId(2) (outer_class)
        assert_eq!(
            edges[0].0,
            SymbolId(1),
            "Reference should attach to innermost symbol SymbolId(1), but attached to SymbolId({})",
            edges[0].0.0
        );
        assert_eq!(edges[0].1, SymbolId(99));
    }
}
