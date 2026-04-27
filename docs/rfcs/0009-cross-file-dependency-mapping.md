# RFC 0009: Cross-File Dependency Mapping

## Status

Accepted

## Context

The current `meta-ast` architecture successfully extracts definitions (functions, classes) and builds a foundational `CodeGraph`. However, it does not extract or resolve cross-file dependencies (imports and references) in the main pipeline. For `meta-ast` to serve as a robust backbone for SAST tools, it must accurately map how data and control flow across file boundaries.

This RFC defines the implementation of a graph-driven scope resolution strategy with high-performance optimizations to bridge the gap between extraction and dependency mapping.

## Goals

1. Extract `import` statements and symbol `references` across supported languages.
2. Resolve symbol references to their definitions using graph-aware scoping.
3. Validate cross-file dependency mapping on mixed-language projects.
4. Maintain high precision and performance for large-scale codebases.


### 5. Multi-Pass Resolution Strategy
To ensure all definitions are available before resolution, the pipeline follows four sequential passes:
1. **Pass 1:** Extract symbols, imports, and references (Parallelizable).
2. **Pass 2:** Build File Nodes and resolve/add `ImportEdges`.
3. **Pass 3:** Build **Export Map Cache** (BFS/DFS with circular dependency guards).
4. **Pass 4:** Resolve References using local scope + Export Map.

## Design

### 1. Data Model Extensions

**New Types in `src/model/mod.rs`:**

```rust
pub struct UnresolvedImport {
    pub target_path: String, // Raw path from source
    pub namespace: Option<String>,
    pub alias: Option<String>,
    pub is_star: bool,
    pub range: SourceRange,
}

pub struct UnresolvedReference {
    pub source_symbol: Option<SymbolId>, // None if module-level
    pub name: String,
    pub range: SourceRange,
}
```

### 2. Extraction Pipeline Update

The `ExtractionResult` carries these unresolved items. Note that `source_path` is omitted from individual structs as it is stored at the container level.

```rust
pub struct ExtractionResult {
    pub symbols: Vec<Symbol>,
    pub imports: Vec<UnresolvedImport>,
    pub references: Vec<UnresolvedReference>,
    pub diagnostics: Vec<Diagnostic>,
}
```

## Testing Strategy

1. **Unit Tests:** Verify `GraphBuilder` resolution with shadowing, circular dependencies, and transitive imports.
2. **Integration Tests:** Use `tests/fixtures/mixed/` to validate cross-language resolution precision.
## Key Design Decisions

### 1. Decoupled Extraction Identifiers
`UnresolvedImport` and `UnresolvedReference` use name-based/path-based identification during extraction. This ensures the parser remains stateless and testable in isolation from the global `FileId` state.

### 2. Graph-Driven Scope Resolution with Export Maps
To avoid the $O(R \times (V + E))$ bottleneck of naive BFS traversal, resolution uses a caching strategy:
- **Export Maps:** Once `ImportEdges` are established, the builder pre-computes a "Flattened Export Map" for each file, caching public symbols available via direct and transitive imports.
- **Resolution pass:** Symbol references are resolved against the local file scope first (handling shadowing), then against the cached Export Map ($O(1)$ lookup).

### 3. Star Import Handling
The resolution logic explicitly handles "opaque" or star imports (`import *`). Search paths are flagged as "exhausted" if a star import is encountered, triggering a fallback search against the target file's full public export set.

### 4. Separate Queries Per Language
Each language implements isolated tree-sitter queries for imports and references via `import_query_fn()` and `reference_query_fn()` in `LanguageSpec`.

## References

- RFC 0008 - Graph Module and SCC Analysis
- `docs/specs/graph-model.md`
