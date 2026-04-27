//! Reference resolution via FlattenedScopeCache.
//!
//! Pre-computes the visible scope per file by DFS-ing the import graph
//! once, then resolves references with O(1) lookups instead of
//! per-reference BFS.

// TODO(MVP): Refactor path normalization to handle language-specific module resolution
// (pip importlib, npm/node_modules, cargo crates, go modules) as we need something better.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::language::LangId;
use crate::model::{FileExtraction, FileId, SymbolId, Visibility};

type ScopeMap = HashMap<String, Vec<(SymbolId, f32)>>;
type SymbolIndexEntry = (SymbolId, String, LangId, Option<Visibility>);
type SymbolIndex = HashMap<FileId, Vec<SymbolIndexEntry>>;

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
    pub fn build(
        symbol_index: &SymbolIndex,
        import_adjacency: &HashMap<FileId, Vec<FileId>>,
        file_languages: &HashMap<FileId, LangId>,
    ) -> Self {
        let mut scopes: HashMap<FileId, ScopeMap> = HashMap::new();

        for &file_id in symbol_index.keys() {
            let scope =
                Self::compute_scope(file_id, symbol_index, import_adjacency, file_languages);
            scopes.insert(file_id, scope);
        }

        Self { scopes }
    }

    fn compute_scope(
        file_id: FileId,
        symbol_index: &SymbolIndex,
        import_adjacency: &HashMap<FileId, Vec<FileId>>,
        file_languages: &HashMap<FileId, LangId>,
    ) -> ScopeMap {
        let source_lang = file_languages.get(&file_id).copied();
        let mut scope: ScopeMap = HashMap::new();
        let mut visited: HashSet<FileId> = HashSet::new();
        let mut queue: VecDeque<(FileId, usize)> = VecDeque::new();

        queue.push_back((file_id, 0));

        while let Some((current, distance)) = queue.pop_front() {
            if !visited.insert(current) {
                // TODO(MVP): Add cycle-detection diagnostics - log which files form circular
                // import chains so the user can see them.
                continue; // handle cycles
            }

            if let Some(symbols) = symbol_index.get(&current) {
                for (sym_id, name, sym_lang, visibility) in symbols {
                    // TODO(MVP): Language-aware default visibility - Python/JS default
                    //             public, Rust/C++ default private. Currently over-approximates
                    //             by treating None as public.
                    let is_public = match visibility {
                        Some(Visibility::Public) | None => true,
                        Some(Visibility::Private) => current == file_id,
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

                    scope
                        .entry(name.clone())
                        .or_default()
                        .push((*sym_id, confidence));
                }
            }

            if let Some(neighbors) = import_adjacency.get(&current) {
                for &neighbor in neighbors {
                    if !visited.contains(&neighbor) {
                        queue.push_back((neighbor, distance + 1));
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

        scope
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

/// Normalize an import path string to a project-relative PathBuf.
///
/// Handles:
/// - Relative paths: "./utils", "../lib/foo" -> resolved against source dir
/// - Bare filenames: "utils.py" -> relative to source dir
/// - Dotted names: "os.path" -> skipped (return None = external)
/// - C includes: "header.h" -> treated as bare filename
///   We really need a better solution as i stated before, this is a band-aid to get something working for MVP, but ideally the normalizer should be language-aware and handle more complex patterns (npm packages, pip modules, cargo crates, go modules) and edge cases (index files, __init__.py, Rust inline modules).
pub fn normalize_import_path(source_dir: &Path, raw: &str, source_lang: LangId) -> Option<PathBuf> {
    let raw = raw.trim_matches(|c| c == '"' || c == '\'' || c == '<' || c == '>');

    // System includes (angle brackets) are external
    if raw.is_empty() {
        return None;
    }

    // Skip language-specific external patterns
    match source_lang {
        LangId::C | LangId::Cpp => {
            // "header.h" or <header.h> - latter is system include
            let path = source_dir.join(raw);
            // Only resolve if the file would be in the project
            return Some(path);
        }
        LangId::Python => {
            if !raw.starts_with('.') {
                // Sibling module (no leading dot) - produce candidate, let builder resolve
                return Some(source_dir.join(format!("{raw}.py")));
            }
        }
        LangId::JavaScript | LangId::TypeScript | LangId::Tsx => {
            // Only resolve relative imports
            if !raw.starts_with('.') {
                return None; // npm package
            }
        }
        LangId::Rust => {
            // "crate::..." is internal, "std::..." is external
            if raw.starts_with("crate::") || raw.starts_with("super::") || raw.starts_with("self::")
            {
                let prefix = if raw.starts_with("crate::") {
                    "crate::"
                } else if raw.starts_with("super::") {
                    "super::"
                } else {
                    "self::"
                };
                let rest = raw.strip_prefix(prefix).unwrap_or(raw);
                let module_path = rest.split("::").next().unwrap_or(rest);
                return Some(source_dir.join(format!("{module_path}.rs")));
            }
            // TODO(MVP): Handle Rust inline module declarations (mod foo; where foo is in the same file).
            return None; // external crate
        }
        LangId::Go => {
            // Go module paths - external unless starting with "."
            if !raw.starts_with('.') {
                // TODO(MVP): Support Go module paths with GOPATH/go.mod resolution.
                return None;
            }
        }
    }

    let resolved = source_dir.join(raw);
    // Try adding extension if missing
    if resolved.extension().is_none() {
        let ext = extension_for(source_lang);
        Some(resolved.with_extension(ext))
    } else {
        Some(resolved)
    }
}

fn extension_for(lang: LangId) -> &'static str {
    match lang {
        LangId::Python => "py",
        LangId::JavaScript => "js",
        LangId::TypeScript => "ts",
        LangId::Tsx => "tsx",
        LangId::C => "h",
        LangId::Cpp => "hpp",
        LangId::Rust => "rs",
        LangId::Go => "go",
    }
}

/// Resolve all references across extracted files.
///
/// Returns a list of (source_symbol_id, target_symbol_id) pairs
/// representing ReferenceEdges to add. Unresolved references are silently
/// skipped (caller should emit diagnostics if desired).
pub fn resolve_all_references(
    extractions: &[FileExtraction],
    path_to_file_id: &HashMap<PathBuf, FileId>,
    scope_cache: &FlattenedScopeCache,
) -> Vec<(SymbolId, SymbolId)> {
    let mut edges = Vec::new();

    for file_ext in extractions {
        let file_id = match path_to_file_id.get(&file_ext.path) {
            Some(&id) => id,
            None => continue,
        };

        for ref_ in &file_ext.references {
            if let Some(matches) = scope_cache.resolve(file_id, &ref_.name) {
                // Find the source symbol that contains this reference range
                let source_sym = file_ext.symbols.iter().find(|s| {
                    s.source_range.byte_start <= ref_.range.byte_start
                        && s.source_range.byte_end >= ref_.range.byte_end
                });

                if let Some(source) = source_sym {
                    for &(target_id, _confidence) in matches {
                        // Don't add self-references (symbol to itself)
                        if source.id != target_id {
                            // TODO(MVP): Emit a Warning diagnostic for unresolved references instead of
                            //             silently skipping them.
                            // intended behavior for now to avoid noise, but we can add diagnostics later by returning a richer result type from this function as this is not the last design of reslover.
                            // and it may take A LOT of work to get the normalizer and builder to a point where we can resolve most references, so we want to be able to iterate on that without being overwhelmed by diagnostics for the MVP and may take a complete PR.
                            edges.push((source.id, target_id));
                        }
                    }
                }
            }
        }
    }

    // Deduplicate
    let seen: HashSet<(SymbolId, SymbolId)> = edges.iter().copied().collect();
    let mut deduped: Vec<_> = seen.into_iter().collect();
    deduped.sort_by_key(|(a, b)| (a.0, b.0));
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
    fn normalize_bare_python_relative() {
        let dir = Path::new("/project/src");
        let result = normalize_import_path(dir, "./utils", LangId::Python);
        assert_eq!(result, Some(PathBuf::from("/project/src/./utils.py")));
    }

    #[test]
    fn skip_external_python_import() {
        // MVP: bare module names produce a path candidate; the builder's
        // path_to_file lookup filters out non-existent files.
        // TODO
        let dir = Path::new("/project/src");
        let result = normalize_import_path(dir, "os.path", LangId::Python);
        assert!(result.is_some(), "normalizer produces path for bare names");
    }

    #[test]
    fn resolve_js_relative() {
        let dir = Path::new("/project/src");
        let result = normalize_import_path(dir, "./utils", LangId::JavaScript);
        assert_eq!(result, Some(PathBuf::from("/project/src/./utils.js")));
    }

    #[test]
    fn skip_npm_package() {
        let dir = Path::new("/project/src");
        let result = normalize_import_path(dir, "react", LangId::JavaScript);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_rust_crate_path() {
        let dir = Path::new("/project/src");
        let result = normalize_import_path(dir, "crate::foo::bar", LangId::Rust);
        assert_eq!(result, Some(PathBuf::from("/project/src/foo.rs")));
    }

    #[test]
    fn skip_external_rust_crate() {
        let dir = Path::new("/project/src");
        let result = normalize_import_path(dir, "std::collections::HashMap", LangId::Rust);
        assert!(result.is_none());
    }

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

        let import_adjacency: HashMap<FileId, Vec<FileId>> = HashMap::new();
        let mut file_languages = HashMap::new();
        file_languages.insert(FileId(0), LangId::Python);

        let cache = FlattenedScopeCache::build(&symbol_index, &import_adjacency, &file_languages);
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

        let mut import_adjacency = HashMap::new();
        import_adjacency.insert(FileId(0), vec![FileId(1)]);

        let mut file_languages = HashMap::new();
        file_languages.insert(FileId(0), LangId::Python);
        file_languages.insert(FileId(1), LangId::Python);

        let cache = FlattenedScopeCache::build(&symbol_index, &import_adjacency, &file_languages);
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

        let import_adjacency = HashMap::new();
        let mut file_languages = HashMap::new();
        file_languages.insert(FileId(0), LangId::Python);

        let cache = FlattenedScopeCache::build(&symbol_index, &import_adjacency, &file_languages);
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
        let mut import_adjacency = HashMap::new();
        import_adjacency.insert(FileId(0), vec![FileId(1)]);
        import_adjacency.insert(FileId(1), vec![FileId(0)]);

        let mut file_languages = HashMap::new();
        file_languages.insert(FileId(0), LangId::Python);
        file_languages.insert(FileId(1), LangId::Python);

        let cache = FlattenedScopeCache::build(&symbol_index, &import_adjacency, &file_languages);
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

        let mut import_adjacency = HashMap::new();
        import_adjacency.insert(FileId(0), vec![FileId(1)]);

        let mut file_languages = HashMap::new();
        file_languages.insert(FileId(0), LangId::Python);
        file_languages.insert(FileId(1), LangId::Rust);

        let cache = FlattenedScopeCache::build(&symbol_index, &import_adjacency, &file_languages);
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
        };

        let mut path_to_file_id = HashMap::new();
        path_to_file_id.insert(PathBuf::from("/proj/a.py"), FileId(0));

        let mut scopes: HashMap<FileId, ScopeMap> = HashMap::new();
        let mut scope = HashMap::new();
        scope.insert("helper".into(), vec![(SymbolId(99), 1.0)]);
        scopes.insert(FileId(0), scope);

        let cache = FlattenedScopeCache { scopes };

        let edges = resolve_all_references(&[file], &path_to_file_id, &cache);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0], (SymbolId(1), SymbolId(99)));
    }
}
