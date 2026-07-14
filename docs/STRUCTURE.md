# Code Structure and Design Plan

This document defines the module layout, data structures, design patterns, language features, testing strategy, and implementation order for `meta-ast`. It is the authoritative reference for how code is organized and why.

---

## 1. Module Structure

```
src/
├── lib.rs                    Public API re-exports
├── main.rs                   CLI entrypoint
├── error.rs                  Error + Diagnostic types (thiserror)
├── pipeline.rs               Full graph analysis orchestration
│
├── model/
│   ├── mod.rs                Symbol, SymbolKind, SourceRange, UnresolvedImport, UnresolvedReference, FileExtraction
│   ├── ids.rs                FileId, SymbolId, SnapshotId (newtyped u32 via define_id_type! macro)
│   └── output.rs             InspectOutput, FuncEntry, ClassEntry, ObjectEntry
│
├── language/
│   ├── mod.rs                LangId enum, LanguageSpec struct, DefaultVisibility, DocCommentConfig
│   ├── common.rs             extract_with_spec, extract_imports_and_references_with_spec, associate_docstrings
│   ├── python.rs             Python queries + extraction
│   ├── javascript.rs         JavaScript queries + extraction
│   ├── typescript.rs         TypeScript queries + extraction
│   ├── tsx.rs                TSX queries + extraction (separate grammar from TS)
│   ├── c.rs                  C queries + extraction
│   ├── cpp.rs                C++ queries + extraction
│   ├── rust.rs               Rust queries + extraction
│   ├── go.rs                 Go queries + extraction
│   └── import_resolver.rs    ImportResolver trait, stateful resolvers (Python, Go, JS, TS)
│
├── input/
│   └── mod.rs                File discovery, filtering, language routing
│
├── parser/
│   └── mod.rs                Tree-sitter parser lifecycle, parse function
│
├── extractor/
│   └── mod.rs                Pipeline orchestration: parallel parse + extract per-file (symbols + imports + references)
│
├── graph/
│   ├── mod.rs                DiGraph construction, node/edge types, re-exports
│   ├── node.rs               NodeData enum (File / Symbol / External)
│   ├── edge.rs               EdgeKind enum (Ownership / Import / Reference) with confidence
│   ├── builder.rs            GraphBuilder, from_extractions, import_adjacency, external_index
│   ├── scc.rs                Tarjan SCC + DeployabilityHint
│   └── resolver.rs           FlattenedScopeCache, ResolutionContext, resolve_all_references
│
├── output/
│   ├── mod.rs                OutputFormat enum (Json / Yaml) with serialize dispatch
│   ├── emitter.rs            EmitConfig, emit_inspect(), emit_graph() - CLI output dispatch
│   ├── inspect.rs            Inspect-compatible JSON/YAML emission
│   ├── graph.rs              GraphOutput + SCC serialization (metadata, nodes, edges, sccs)
│   └── dashboard.rs          Interactive HTML dashboard (Cytoscape.js via CDN, --html)
│
└── interface/
    ├── mod.rs                CLI module root
    └── args.rs               Clap derive structs (Inspect, Graph, Deploy + --format, --html)
│
└── deploy/                   [feature: metacall-deploy] See docs/DEPLOY.md
    ├── mod.rs                Entry: run_deploy(), DeployConfig, add_metacall_edge()
    ├── scanner.rs            tree-sitter call-site detection, CallSite, CallSiteVariant, confidence
    ├── pod.rs                Union-Find partition_into_pods(), PodPartition, InterPodEdge
    ├── cut.rs                find_cross_language_cuts(), find_oversized_pod_cut(), CutEdge
    ├── dependency.rs         classify_external(), resolve_dependencies(), per-language resolvers
    ├── metrics.rs            compute_file_metrics(), compute_pod_metrics(), FileMetrics
    ├── manifest.rs           generate_pod_manifest(), PodManifest, ManifestEdge
    ├── mesh.rs               generate_mesh_annotation(), DeploymentUnit, CrossLanguageEdge
    ├── check.rs              check_cut_fairness() - bijection check between cuts and rpc_stub edges
    └── tags.rs               LangId <-> MetaCall runtime tag mapping
```

### Module dependency direction

```
CLI (interface/)
  → Pipeline (pipeline.rs)  → orchestrates the full graph analysis
    → Extractor (extractor/) → depends on model + language + parser
      → Parser (parser/)     → depends on language (grammar dispatch)
    → Graph (graph/)         → depends on model + petgraph
      → Resolver (graph/resolver.rs) → cross-file reference resolution
    → Input (input/)         → depends on language (detection)
  → Output (output/)         → depends on model + graph
  → Deploy (deploy/)         → depends on pipeline + graph + input [feature: metacall-deploy]
  → Error (error.rs)         ← cross-cutting
```

Outer layers depend on inner layers. The model layer has zero knowledge of parsing, I/O, or language specifics.

---

## 2. Core Data Structures

### 2.1 ID Types

Sequential `AtomicU32`-generated newtypes via `define_id_type!` macro. Thread-safe, deterministic within a session, and type-safe against mixing.

```rust
define_id_type!(FileId);
define_id_type!(SymbolId);
define_id_type!(SnapshotId);
```

Each ID type has a corresponding `IdGenerator<T>` wrapping an `AtomicU32` for thread-safe allocation.

### 2.2 Source Location

```rust
pub struct LineColumn {
    pub line: usize,    // 0-indexed
    pub column: usize,  // 0-indexed, byte offset within line
}

pub struct SourceRange {
    pub byte_start: usize,
    pub byte_end: usize,
    pub start: LineColumn,
    pub end: LineColumn,
}
```

### 2.3 Symbol Model

Immutable IR - constructed once during extraction, never mutated.

```rust
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Interface,
    Trait,
    Enum,
    Object,
    Constant,
    Static,
    Module,
    Namespace,
    TypeAlias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum Visibility {
    Public,
    Private,
}

#[derive(Debug, Clone, Serialize)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub language: LangId,
    pub file_path: PathBuf,
    pub source_range: SourceRange,
    pub visibility: Option<Visibility>,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub is_async: bool,
}
```

### 2.4 Graph Model

Node and edge types:

| Node | Fields |
|------|--------|
| `FileNode` | id, path (project-root-relative), language, snapshot_id |
| `SymbolNode` | id, name, kind, file_id, visibility, source_range |
| `ExternalNode` | raw_path, language |

| Edge | Direction |
|------|-----------|
| `Ownership` | FileNode -> SymbolNode, SymbolNode -> SymbolNode (nesting) |
| `Import` | FileNode -> FileNode |
| `Reference` | SymbolNode -> SymbolNode |

Graph invariants:

1. Every SymbolNode maps to exactly one FileNode.
2. Ownership edges form an acyclic containment structure.
3. SCC applies to dependency/reference subgraph only (Ownership excluded).
4. Duplicate edges normalized by `(src, dst, edge_kind)`.
5. External dependencies get `NodeData::External` placeholder nodes.

### 2.5 Inspect Output

Stable contract:

```rust
pub struct InspectOutput {
    pub funcs: Vec<FuncEntry>,
    pub classes: Vec<ClassEntry>,
    pub objects: Vec<ObjectEntry>,
}
```

Each entry type includes: `name`, `source_range`, optional `signature`, `visibility`, `docstring`. `FuncEntry` additionally includes an `async` flag.

---

## 3. Language System Design

### 3.1 LanguageSpec Struct

Each language is a static `LanguageSpec` constant with function pointers (not a trait):

```rust
pub struct LanguageSpec {
    pub extensions: &'static [&'static str],
    pub grammar_fn: fn() -> tree_sitter::Language,
    pub query_fn: fn() -> &'static Query,
    pub import_path_resolver: fn(&str, &Path, &Path) -> Option<PathBuf>,
    pub import_ref_query_fn: fn() -> &'static Query,
    pub class_like_parents: &'static [&'static str],
    pub ancestor_visibility_rules: &'static [(&'static str, Visibility)],
    pub visibility_from_name: Option<fn(&str) -> Option<Visibility>>,
    pub import_statement_kinds: &'static [&'static str],
    pub default_visibility: DefaultVisibility,
    pub doc_comment_config: Option<DocCommentConfig>,
}
```

### 3.2 LangId Enum

The aggregate dispatch enum. `#[non_exhaustive]` for forward compatibility:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, strum::Display, strum::AsRefStr)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[repr(usize)]
pub enum LangId {
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    C,
    Cpp,
    Rust,
    Go,
}
```

### 3.3 Stateful Import Resolution Seam

To support complex stateful import resolution (e.g. resolving paths using configuration files like `tsconfig.json` or module boundary scanning like `go.mod`), `meta-ast` implements a hybrid seam combining static `LanguageSpec` specs with a stateful `ImportResolver` trait:

```rust
pub trait ImportResolver: Send + Sync {
    fn resolve(
        &self,
        raw: &str,
        source_dir: &Path,
        project_root: &Path,
    ) -> Option<PathBuf>;
}
```

#### Hybrid Resolution Bridge

1. **`LanguageSpec`** remains static and `const` (containing a stateless `import_path_resolver` fn pointer).
2. **`ImportResolver`** represents a stateful trait interface.
3. Concrete adapters bridge the two:
   - `StatelessResolver`: Zero-cost wrapper delegating to static fn pointers.
   - `PythonResolver`, `GoModResolver`, `JsResolver`, `TsConfigResolver`: Concrete structs implementing `ImportResolver`, prepped to hold caches or parse configs.
4. **`make_resolver(LangId) -> Box<dyn ImportResolver>`**: Factory function constructing the stateful resolver for each language dynamically.

#### Stateful Caching and Memoization Engines

To guarantee maximum throughput and avoid redundant filesystem traversal during large-scale workspace parsing, the stateful resolvers employ optimized, thread-safe caching strategies:

- **`OnceLock` Module Boundary Scanning (`GoModResolver`)**: Scans for the root `go.mod` file and parses the module path at most once per execution using a standard `OnceLock`. Subsequent resolution calls query the in-memory boundary in $O(1)$ time.
- **`RwLock` File Existence Memoization (`PythonResolver`, `JsResolver`, `TsConfigResolver`)**: Memoizes `exists()` and `is_file()` filesystem checks using an `RwLock<HashMap<PathBuf, bool>>`. This minimizes expensive system calls during TypeScript candidate extensions resolution (e.g. trying `.ts`, `.tsx`, `.js`) and Python relative path matching, while remaining safe for concurrency.
- **Stateless Fallback**: When candidate paths do not match or cannot be resolved using stateful logic, all resolvers gracefully fallback to their underlying stateless `LanguageSpec` function pointer, ensuring 100% backward compatibility.

During graph assembly, resolvers are created once per run and cached inside the builder to ensure O(1) config-file reading and caching properties.

### 3.4 Adding a New Language

The process is:

1. Add the tree-sitter grammar crate to `Cargo.toml`.
2. Create `src/language/<name>.rs` with query constants, extraction function, and `LanguageSpec` constant.
3. Add a variant to `LangId` enum.
4. Add a match arm in `spec_for()`.
5. Add fixture files and tests.

No trait objects, no runtime plugins. Compile-time completeness checking via exhaustive match.

### 3.5 Language Detection

`detect_language(path: &Path) -> Option<LangId>` maps file extensions to `LangId` variants. Lives in `input/mod.rs`.

| Extension(s) | LangId |
|---|---|
| `.py`, `.pyi` | `Python` |
| `.js`, `.mjs`, `.cjs` | `JavaScript` |
| `.ts`, `.cts`, `.mts` | `TypeScript` |
| `.tsx` | `Tsx` |
| `.c` | `C` |
| `.cc`, `.cpp`, `.cxx` | `Cpp` |
| `.rs` | `Rust` |
| `.go` | `Go` |

---

## 4. Design Patterns

### 4.1 Enum Static Dispatch (Language System)

All language-specific behavior dispatches through `match` on `LangId`. No vtables, no `dyn` - full monomorphization and inline optimization.

### 4.2 Pipeline Pattern

The analysis pipeline is orchestrated by `pipeline.rs`:

```
Source Discovery -> Parallel Parse + Extract -> Graph Assembly -> Import Resolution -> Reference Resolution -> SCC -> Output
   (sequential)       (rayon par_iter)         (sequential)       (sequential)          (sequential)      (sequential)
```

Parse and extract are combined per-file to avoid materializing all tree-sitter trees simultaneously.

### 4.3 Newtype Pattern

`FileId`, `SymbolId`, `SnapshotId` are newtyped `u32` values via `define_id_type!` macro. The compiler prevents mixing them, and `#[serde(transparent)]` keeps serialization clean.

### 4.4 Recoverable Error Accumulation

Parse errors do not abort extraction. The pipeline accumulates `Vec<Diagnostic>` alongside results. Tree-sitter `ERROR` and `MISSING` nodes are skipped during extraction. Diagnostics are a separate concern from the symbol model.

### 4.5 Immutable IR

`Symbol` structs are constructed during extraction and never mutated. Downstream consumers (graph assembly, output serialization) read them immutably.

---

## 5. Parallelism Strategy

### 5.1 rayon Integration

`rayon = "1.10"` is used for file-level parallelism in the parse + extract phase.

- A thread-local pool of `Parser` instances (one per language) is maintained within each worker thread via `thread_local!` and `RefCell` caching. This avoids sharing the non-`Sync` `Parser` across threads.
- Emitted `Tree` and symbol models are `Send` and are safely returned from rayon workers to the main thread for graph assembly.
- Caching `Parser` instances avoids redundant grammar re-initialization and allocation overhead on every task.

### 5.2 Pipeline Phases

| Phase | Concurrency | Rationale |
|-------|------------|-----------|
| File discovery | Sequential | Single walk, fast I/O |
| Parse + Extract | rayon `par_iter` | CPU-bound, per-file independent, largest time slice |
| Graph assembly | Sequential | petgraph mutation + cross-file resolution requires single-threaded access |
| Import resolution | Sequential | Uses per-language import path resolvers |
| Reference resolution | Sequential | FlattenedScopeCache + cross-file lookup |
| Output serialization | Sequential | Single JSON/YAML document emission |

---

## 6. Error Handling

### 6.1 Error Type Hierarchy

```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse error in {path}: {message}")]
    Parse { path: PathBuf, message: String },

    #[error("query error ({language}): {message}")]
    Query { language: LangId, message: String },

    #[error("config: {0}")]
    Config(String),

    #[error("graph error: {0}")]
    Graph(String),
}
```

Library uses `Result<T, Error>` with `?` propagation. Application boundary (CLI) uses `anyhow::Result`.

### 6.2 Diagnostics

```rust
pub struct Diagnostic {
    pub path: PathBuf,
    pub severity: Severity,  // Warning, Error
    pub message: String,
    pub source_range: Option<SourceRange>,
}
```

Diagnostics are accumulated in a `Vec<Diagnostic>` separate from the symbol model. Extraction continues on recoverable errors.

### 6.3 Error Recovery Rules

1. Tree-sitter `ERROR` and `MISSING` nodes are skipped during extraction.
2. Partial extraction is allowed and expected for malformed source files.
3. If > 50% of a file's nodes are errors, the file is marked as unparseable but does not abort the pipeline.
4. Fatal errors are reserved for invalid configuration or unrecoverable I/O failures.

### 6.4 Query Compilation Failure Strategy

Tree-sitter queries are hardcoded constants in each language pack. If a query fails to compile, it indicates a programmer bug in the shipped query text, not a runtime input error.

**Strategy**: `compile_query` uses `panic!()` rather than `std::process::abort()` or `Result` propagation.

**Why not `abort()`**: `panic!()` runs destructors, is propagated by rayon, and integrates with Rust's panic infrastructure. `abort()` skips all cleanup.

**Why not `Result`**: Queries are compiled inside `LazyLock<T>::new()` closures which require `FnOnce() -> T` (infallible return).

**Mitigation**: `language::validate_queries()` eagerly initializes all 16 `LazyLock` statics at startup, ensuring any query bug panics immediately rather than after processing files.

---

## 7. Rust Language Features Used

| Feature | Usage |
|---------|-------|
| Edition 2024 | MSRV 1.92.0 |
| `#[non_exhaustive]` | All public enums (`LangId`, `SymbolKind`, `Visibility`, `Severity`) |
| Newtype pattern | `FileId`, `SymbolId`, `SnapshotId` via `define_id_type!` macro |
| `impl From<X> for Error` | Automatic error conversion for `?` propagation |
| `AtomicU32` | Thread-safe ID generation |
| `serde` derive | All serializable types with `#[serde(rename_all = "snake_case")]` |
| `thiserror` derive | Error types with formatted messages |
| `clap` derive | CLI argument structs |
| rayon `par_iter` | File-level parallelism |
| `strum` derives | `LangId` display/serialization |
| `LazyLock` | Language query static initialization |

---

## 8. Dependencies

### 8.1 Runtime Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tree-sitter` | 0.26.3 | Core parsing |
| `tree-sitter-python` | 0.25.0 | Python grammar |
| `tree-sitter-javascript` | 0.25.0 | JavaScript grammar |
| `tree-sitter-typescript` | 0.23.2 | TypeScript + TSX grammars |
| `tree-sitter-c` | 0.24.1 | C grammar |
| `tree-sitter-cpp` | 0.23.4 | C++ grammar |
| `tree-sitter-rust` | 0.24.0 | Rust grammar |
| `tree-sitter-go` | 0.25.0 | Go grammar |
| `petgraph` | 0.8.3 | Directed graph + Tarjan SCC |
| `serde` + `serde_json` | 1.0 | JSON serialization |
| `yaml_serde` | 0.10 | YAML serialization |
| `strum` | 0.28 | Enum derive macros (Display, AsRefStr) |
| `webbrowser` | 1.2 | Auto-open HTML dashboard in browser |
| `clap` | 4.6 | CLI (derive API, env, color) |
| `rayon` | 1.10 | Parallel file processing |
| `thiserror` | 2.0 | Library error types |
| `anyhow` | 1.0 | Application error boundary |
| `ignore` | 0.4 | Gitignore-aware file walking |
| `tracing` + `tracing-subscriber` | 0.1 / 0.3 | Structured diagnostics |

### 8.2 Development Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `insta` | 1.47 | Snapshot testing for JSON output contracts |
| `criterion` | 0.8 | Benchmark gating |

### 8.3 Feature Flags

| Feature | Purpose |
|---------|---------|
| `metacall-deploy` | Generate MetaCall deployment manifests and mesh annotations |

---

## 9. Test Structure

```
tests/
├── integration.rs                   Integration test module root
├── integration/
│   ├── pipeline_test.rs             End-to-end: discover -> parse -> extract -> graph -> output
│   ├── dashboard_test.rs            HTML dashboard generation tests
│   ├── output_format_test.rs        JSON/YAML output format tests
│   └── inspect_output_test.rs       Inspect-compatible output validation
└── fixtures/
    ├── python/
    │   ├── simple_functions.py
    │   ├── classes.py
    │   ├── async_decorators.py
    │   ├── deep_nesting.py
    │   ├── partial_syntax_error.py
    │   └── sample.py
    ├── javascript/
    │   ├── functions.js
    │   ├── classes.js
    │   └── large_classes.js
    ├── typescript/
    │   └── interfaces.ts
    ├── tsx/
    │   └── components.tsx
    ├── c/
    │   ├── functions.c
    │   └── structs_enums.c
    ├── cpp/
    │   ├── classes.cpp
    │   └── namespaces.cpp
    ├── rust/
    │   ├── functions.rs
    │   ├── structs_enums.rs
    │   └── large_file.rs
    ├── go/
    │   ├── functions.go
    │   ├── methods.go
    │   └── deep_nesting.go
    ├── mixed/                      Multi-language single-directory fixtures
    │   ├── app.py, index.js, main.rs, test.generated.py
    │   └── auth_microservice{,_level2,_level3}  Deploy edge-case fixtures
    │       (star / cross-language SCC cycle / full-module stress)
    └── multi/                      Multi-file cross-language fixtures
        ├── main.py, lib.py, app.js, util.js
        ├── c_app/, cpp_app/, go_app/, rust_crate/, ts_app/, tsx_app/
        └── edge_*/                 Edge case fixtures (circular, alias, shadowing, etc.)
```

### Snapshot policy

Insta snapshot files live in `src/language/snapshots/` as `.snap` files (not under fixture directories). Each
language module generates snapshots via inline unit tests. Update workflow: `cargo insta test` then
`cargo insta review` then commit accepted `.snap` files.

### Testing Strategy

| Layer | Tool | Purpose |
|-------|------|---------|
| Language detection | Unit tests | Extension-to-LangId mapping |
| Per-language extraction | Fixture files + unit tests | Query correctness, capture mapping |
| JSON output contract | `insta` snapshots in `src/language/snapshots/` | Regression detection |
| Error recovery | Fixture with invalid syntax | Partial results, no panics |
| End-to-end pipeline | Integration tests | Full discover -> output flow |
| Deploy module | Tiered `mixed/auth_microservice*` fixtures + `cut.rs` unit tests | Cross-language SCC cut, intra-language collapse, oversized-pod, load variants, dependency classification |
| Performance | `criterion` benchmarks | Extraction throughput |
