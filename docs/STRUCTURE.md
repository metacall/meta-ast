# Code Structure and Design Plan

This document defines the module layout, data structures, design patterns, language features, testing strategy, and implementation order for `meta-ast`. It is the authoritative reference for how code is organized and why and it will be refactored at the end of 4 phases to reflect the actual code documentation.

---

## 1. Module Structure

```
src/
├── lib.rs                    Public API re-exports
├── main.rs                   CLI entrypoint
├── error.rs                  Error + Diagnostic types (thiserror)
│
├── model/
│   ├── mod.rs                Symbol, SymbolKind, SourceRange, Visibility, LineColumn
│   ├── ids.rs                FileId, SymbolId, SnapshotId (newtyped u32)
│   └── output.rs             InspectOutput, FuncEntry, ClassEntry, ObjectEntry (serde)
│
├── language/
│   ├── mod.rs                LangId enum, LanguagePack trait, impl_language! macro, dispatch
│   ├── python.rs             Python queries + extraction
│   ├── javascript.rs         JavaScript queries + extraction
│   ├── typescript.rs         TypeScript queries + extraction
│   ├── tsx.rs                TSX queries + extraction (separate grammar from TS)
│   ├── c.rs                  C queries + extraction
│   ├── cpp.rs                C++ queries + extraction
│   ├── rust.rs               Rust queries + extraction
│   └── go.rs                 Go queries + extraction
│
├── input/
│   └── mod.rs                File discovery, filtering, language routing
│
├── parser/
│   └── mod.rs                Tree-sitter parser lifecycle, parse function
│
├── extractor/
│   └── mod.rs                Pipeline orchestration: parallel parse + extract via rayon
│
├── graph/
│   ├── mod.rs                DiGraph construction, node/edge types
│   ├── node.rs               NodeData enum (File / Symbol)
│   ├── edge.rs               EdgeKind enum (Ownership / Import / Reference)
│   ├── builder.rs            GraphBuilder, EdgeNormalizer
│   └── scc.rs                Tarjan SCC + DeployabilityHint
│
├── output/
│   ├── mod.rs                OutputFormat enum (Json / Yaml) with serialize dispatch
│   ├── inspect.rs            Inspect-compatible JSON/YAML emission
│   ├── graph.rs              GraphOutput + SCC serialization (metadata, nodes, edges, sccs)
│   └── dashboard.rs          Interactive HTML dashboard (Cytoscape.js, --html flag)
│
└── interface/
    ├── mod.rs                CLI module root
    └── args.rs               Clap derive structs (Inspect, Graph + --format, --html, --self-contained)
```

### Module dependency direction

```
CLI (interface/)
  → Output (output/)
    → Model (model/)          ← core, no I/O or parsing knowledge
    → Graph (graph/)          → depends on model + petgraph
  → Extractor (extractor/)    → depends on model + language + parser
    → Parser (parser/)        → depends on language (grammar dispatch)
  → Input (input/)            → depends on language (detection)
  → Error (error.rs)          ← cross-cutting
```

Outer layers depend on inner layers. The model layer has zero knowledge of parsing, I/O, or language specifics.

---

## 2. Core Data Structures

### 2.1 ID Types

Sequential `AtomicU32`-generated newtypes. Thread-safe, deterministic within a session, and type-safe against mixing.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct FileId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct SymbolId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct SnapshotId(u32);
```

Each ID type has a corresponding generator (`IdGenerator<T>`) wrapping an `AtomicU32` for thread-safe allocation.

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
    Module,
    Namespace,
    TypeAlias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

### 2.4 Graph Model (Phase 2)

Node and edge kinds per `specs/graph-model.md`:

| Node | Fields |
|------|--------|
| `FileNode` | id, path (project-root-relative), language_id, snapshot_id |
| `SymbolNode` | id, name, kind, file_id, visibility, source_range |
| `DataNode` | id, symbol_id, scope, type_hint (Phase 3) |

| Edge | Direction |
|------|-----------|
| `ImportEdge` | FileNode → FileNode |
| `ReferenceEdge` | SymbolNode → SymbolNode |
| `OwnershipEdge` | FileNode → SymbolNode, SymbolNode → SymbolNode (nesting) |
| `FlowEdge` | DataNode → DataNode (Phase 3) |

Graph invariants (per spec):
1. Every SymbolNode maps to exactly one FileNode.
2. Ownership edges form an acyclic containment structure.
3. SCC applies to dependency/reference subgraph only.
4. Duplicate edges normalized by `(src, dst, edge_kind)`.

### 2.5 Inspect Output

Stable contract per ADR-0005 and `specs/requirements.md` FR-3:

```rust
#[derive(Debug, Serialize)]
pub struct InspectOutput {
    pub funcs: Vec<FuncEntry>,
    pub classes: Vec<ClassEntry>,
    pub objects: Vec<ObjectEntry>,
}
```

Each entry type includes: `name`, `source_range`, optional `signature`, `visibility`, `docstring`, `async` flag.

---

## 3. Language System Design

### 3.1 LanguagePack Trait

Each language is a zero-sized unit struct implementing `LanguagePack`:

```rust
pub trait LanguagePack: Clone + Send + Sync + 'static {
    fn grammar(&self) -> tree_sitter::Language;
    fn id(&self) -> &'static str;
    fn file_extensions(&self) -> &'static [&'static str];
    fn extract_symbols(&self, tree: &Tree, source: &[u8]) -> Vec<Symbol>;
}
```

### 3.2 LangId Enum

The aggregate dispatch enum. `#[non_exhaustive]` for forward compatibility:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
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

**TypeScript and TSX are separate variants** because `tree-sitter-typescript` provides two distinct grammars (`.ts` vs `.tsx` have different parsing rules for JSX).

### 3.3 Registration Macro

A declarative macro minimizes boilerplate when adding a new language:

```rust
macro_rules! impl_language {
    ($lang:ident, $grammar:expr, $extract_fn:expr, $extensions:expr) => {
        #[derive(Clone, Copy, Debug)]
        pub struct $lang;

        impl LanguagePack for $lang {
            fn grammar(&self) -> tree_sitter::Language { $grammar.into() }
            fn id(&self) -> &'static str { stringify!($lang) }
            fn file_extensions(&self) -> &'static [&'static str] { $extensions }
            fn extract_symbols(&self, tree: &Tree, source: &[u8]) -> Vec<Symbol> {
                $extract_fn(tree, source)
            }
        }
    };
}
```

### 3.4 Adding a New Language

The process is:

1. Add the tree-sitter grammar crate to `Cargo.toml`.
2. Create `src/language/<name>.rs` with query constants and an extraction function.
3. Invoke `impl_language!` to implement `LanguagePack`.
4. Add a variant to `LangId` enum.
5. Add a match arm in the dispatch function.
6. Add fixture files and tests.

so no trait objects, no runtime plugins, no `inventory` crate. Compile-time completeness checking via exhaustive match.

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

All language-specific behavior dispatches through `match` on `LangId`. No vtables, no `dyn` - full monomorphization and inline optimization. thanks to  ast-grep for inspiration.

### 4.2 Pipeline Pattern

The analysis pipeline is a linear flow with well-defined phase boundaries:

```
Source Discovery → Parallel Parse + Extract → Graph Assembly → Output
   (sequential)        (rayon par_iter)         (sequential)     (sequential)
```

Parse and extract are combined per-file to avoid materializing all tree-sitter trees simultaneously (memory savings for large codebases).

### 4.3 Newtype Pattern

`FileId`, `SymbolId`, `SnapshotId` are newtyped `u32` values. The compiler prevents mixing them, and `#[serde(transparent)]` keeps serialization clean.

### 4.4 Recoverable Error Accumulation

Parse errors do not abort extraction. The pipeline accumulates `Vec<Diagnostic>` alongside results. Tree-sitter `ERROR` and `MISSING` nodes are skipped during extraction. Diagnostics are a separate concern from the symbol model.

### 4.5 Immutable IR

`Symbol` structs are constructed during extraction and never mutated. Downstream consumers (graph assembly, output serialization) read them immutably.

### 4.6 Adapter Pattern

Language-specific extractors are adapters that transform tree-sitter CST into the normalized `Symbol` model. The model is independent of any specific language grammar (per cargo-semver-checks architecture).

---

## 5. Parallelism Strategy

### 5.1 rayon Integration

`rayon = "1.10"` is used for file-level parallelism in the parse + extract phase.

- `Parser` and `Tree` are `Send + Sync` (safe for parallel use).
- A fresh `Parser` is created per rayon task (creation is ~malloc + zero-init; `set_language` is a pointer assignment to a static grammar table).
- No parser pooling needed - per-task creation is simpler and equally fast.

### 5.2 Pipeline Phases

| Phase | Concurrency | Rationale |
|-------|------------|-----------|
| File discovery | Sequential | Single walk, fast I/O |
| Parse + Extract | rayon `par_iter` | CPU-bound, per-file independent, largest time slice |
| Graph assembly | Sequential | petgraph mutation + cross-file resolution requires single-threaded access |
| Output serialization | Sequential | Single JSON document emission |

### 5.3 Caching

`moka::sync::Cache` with `get_with()` provides atomic single-computation guarantee per key. Used to deduplicate parsing across incremental runs. Key: `(PathBuf, content_hash)`.

### 5.4 When rayon Is Not Worth It

- **< 50 files**: Thread pool startup + synchronization overhead may exceed savings.
- **Very small files (< 1KB)**: Parse time per file is microseconds; rayon overhead is comparable.
- **Mitigation**: Sort files by size descending (largest first) to improve work stealing balance.

** but the decision to use rayon has been made because this project aims to compete at large scale codebases and the usage of rayon maybe dynamically  gated behind a config later **
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

Diagnostics are accumulated in a `Vec<Diagnostic>` separate from the symbol model. Extraction continues on recoverable errors (ADR-0002).

### 6.3 Error Recovery Rules

1. Tree-sitter `ERROR` and `MISSING` nodes are skipped during extraction.
2. Partial extraction is allowed and expected for malformed source files.
3. If > 50% of a file's nodes are errors, the file is marked as unparseable but does not abort the pipeline.
4. Fatal errors are reserved for invalid configuration or unrecoverable I/O failures.

---

## 7. Rust Language Features Used

| Feature | Usage |
|---------|-------|
| Edition 2024 | MSRV 1.92.0 |
| `#[non_exhaustive]` | All public enums (`LangId`, `SymbolKind`, `Visibility`, `Severity`) |
| Newtype pattern | `FileId(u32)`, `SymbolId(u32)` - prevent mixing at compile time |
| `impl From<X> for Error` | Automatic error conversion for `?` propagation |
| `AtomicU32` | Thread-safe ID generation |
| Declarative macros (`macro_rules!`) | `impl_language!` for zero-boilerplate language registration |
| `serde` derive | All serializable types with `#[serde(rename_all = "snake_case")]` |
| `thiserror` derive | Error types with formatted messages |
| `clap` derive | CLI argument structs |
| rayon `par_iter` | File-level parallelism |
| moka `sync::Cache` | Parse deduplication cache |

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
| `clap` | 4.5 | CLI (derive API, env, color) |
| `notify` | 8.2.0 | Filesystem watch mode |
| `moka` | 0.12 | Multi-level caching (sync + future) |
| `rayon` | 1.10 | Parallel file processing |
| `thiserror` | 2.0 | Library error types |
| `anyhow` | 1.0 | Application error boundary |
| `dunce` | 1.0.5 | Path normalization |
| `dotenvy` | 0.15 | Configuration |
| `tracing` + `tracing-subscriber` | latest | Structured diagnostics (NFR-5) |

### 8.2 Development Dependencies

| Crate | Purpose |
|-------|---------|
| `insta` | Snapshot testing for JSON output contracts |
| `jsonschema` | Output schema validation in tests |
| `criterion` | Benchmark gating (Phase 4) |

---

## 9. Test Structure

```
tests/
├── language_detection.rs              Existing language detection tests
│
├── integration/
│   ├── pipeline_test.rs               End-to-end: discover → parse → extract → output
│   └── inspect_output_test.rs         JSON contract validation via jsonschema
│
└── fixtures/
    ├── python/
    │   ├── simple_functions.py
    │   ├── classes.py
    │   ├── async_decorators.py
    │   ├── imports.py
    │   ├── partial_syntax_error.py
    │   └── expected/
    │       └── *.snap.json            Insta snapshot files
    ├── javascript/
    │   ├── functions.js
    │   ├── arrow_functions.js
    │   ├── classes.js
    │   ├── imports_exports.js
    │   └── expected/
    ├── typescript/
    │   ├── interfaces.ts
    │   ├── enums_ts.ts
    │   ├── type_aliases.ts
    │   └── expected/
    ├── tsx/
    │   ├── components.tsx
    │   └── expected/
    ├── c/
    │   ├── functions.c
    │   ├── structs_enums.c
    │   └── expected/
    ├── cpp/
    │   ├── classes.cpp
    │   ├── namespaces.cpp
    │   ├── templates.cpp
    │   └── expected/
    ├── rust/
    │   ├── functions.rs
    │   ├── structs_enums.rs
    │   ├── traits_impls.rs
    │   └── expected/
    └── go/
        ├── functions.go
        ├── methods.go
        ├── interfaces.go
        └── expected/
```

### Testing Strategy

| Layer | Tool | Purpose |
|-------|------|---------|
| Language detection | Unit tests | Extension-to-LangId mapping |
| Per-language extraction | Fixture files + unit tests | Query correctness, capture mapping |
| JSON output contract | `insta` snapshots | Regression detection |
| Schema validation | `jsonschema` | Required keys, types, structure |
| Error recovery | Fixture with invalid syntax | Partial results, no panics |
| End-to-end pipeline | Integration tests | Full discover → output flow |
| Determinism | Property test | Same input → identical output |
| Performance | `criterion` benchmarks | Phase 4: < 100ms incremental target |
