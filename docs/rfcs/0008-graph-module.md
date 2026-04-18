# RFC 0008:Graph Module and SCC Analysis

## Status

Accepted
 
## Context

Phase 1 of meta-ast established the foundation: file discovery, parsing, symbol extraction, and inspect-compatible JSON output. Phase 2 extends this with dependency graph construction and Strongly Connected Component (SCC) analysis to provide deployability insights for polyglot codebases.

The graph model was specified in `docs/specs/graph-model.md` and the implementation approach using petgraph was validated through research. This RFC defines the concrete module structure, APIs, and implementation strategy.

## Goals

1. Build a directed dependency/reference graph from extracted symbols
2. Implement Tarjan SCC algorithm for cycle detection
3. Provide deployability hints based on SCC analysis
4. Maintain stable inspect output contract (`funcs`, `classes`, `objects`)
5. Support cross-file dependency mapping for mixed-language projects

## Non-Goals

1. Full inter-procedural dataflow analysis (Phase 3)
2. Live graph database integration (Phase 4)
3. Real-time incremental graph updates (Phase 4)
4. Cross-language type inference (out of scope for MVP)

## Design

### 1. Module Structure

New modules under `src/graph/`:

| File | Responsibility |
|------|---------------|
| `mod.rs` | Public exports, `CodeGraph` struct, graph operations |
| `node.rs` | `FileNode`, `SymbolNode`, `NodeData` enum |
| `edge.rs` | `EdgeKind`, `EdgeData` with metadata |
| `builder.rs` | `GraphBuilder` for incremental construction |
| `scc.rs` | `SccAnalysis`, Tarjan SCC, deployability hints |

Output extension: `src/output/graph.rs` for graph serialization.

### 2. Graph Types

Node storage uses petgraph `DiGraph<NodeData, EdgeData>` with stable node indices. NodeData is an enum for heterogeneous node types:

```rust
pub enum NodeData {
    File(FileNode),
    Symbol(SymbolNode),
}

pub struct FileNode {
    pub id: FileId,
    pub path: PathBuf,           // Project-relative as discussed with vicente
    pub language_id: LangId,
    pub snapshot_id: SnapshotId,
}

pub struct SymbolNode {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub file_id: FileId,
    pub visibility: Option<Visibility>,
    pub source_range: SourceRange,
}
```

Edges carry kind and metadata:

```rust
pub enum EdgeKind {
    Import,      // File imports another file
    Reference,   // Symbol references another symbol
    Ownership,   // File owns symbol, or symbol contains nested symbol
}

pub struct EdgeData {
    pub kind: EdgeKind,
    pub strength: EdgeStrength,  // Strong, Weak, or Dynamic
}

pub enum EdgeStrength {
    Strong,      // Direct, resolvable dependency
    Weak,        // Optional or conditional
    Dynamic,     // Runtime-resolved
}
```

### 3. Graph Construction

Two-phase construction from extraction results:

**Phase A - Ownership graph (always acyclic by construction):**
- Add FileNode for each processed file
- Add SymbolNode for each extracted symbol
- Add Ownership edges: FileNode -> SymbolNode

**Phase B - Dependency graph:**
- Add Import edges: FileNode -> FileNode (cross-file imports)
- Add Reference edges: SymbolNode -> SymbolNode (symbol usage)

Import extraction extends the language pack system with import-specific tree-sitter queries per language.

### 4. SCC Analysis

SCC computation follows `specs/graph-model.md` invariants:

1. SCC runs on dependency subgraph only (Import + Reference edges)
2. Ownership edges are explicitly excluded from SCC computation
3. Duplicate edges normalized by (src, dst, edge_kind)

Tarjan's algorithm via `petgraph::algo::tarjan_scc` produces components in reverse topological order.

SccAnalysis result structure:

```rust
pub struct SccAnalysis {
    pub components: Vec<Scc>,
    pub node_to_component: HashMap<NodeIndex, usize>,
}

pub struct Scc {
    pub index: usize,
    pub nodes: Vec<NodeIndex>,
    pub is_cyclic: bool,
    pub deployability_hint: DeployabilityHint,
}

pub enum DeployabilityHint {
    Independent,        // Size=1, no self-loop
    AcyclicDependency,  // Size=1, depends on other components
    CyclicCluster,      // Size>1 or self-loop present
}
```

### 5. CLI Integration

New subcommand `graph` alongside existing `inspect`:

```rust
pub enum Cli {
    Inspect(InspectArgs),
    Graph(GraphArgs),  // New
}
```

`GraphArgs` accepts same path/language filters as `InspectArgs` plus output format options.

Output format (JSON):

```json
{
  "meta": {
    "snapshot_id": 1,
    "file_count": 10,
    "symbol_count": 150,
    "edge_count": 200
  },
  "nodes": [
    {"id": "F0", "kind": "file", "path": "src/main.py", "language": "python"},
    {"id": "S42", "kind": "symbol", "name": "main", "kind": "function", "file_id": "F0"}
  ],
  "edges": [
    {"source": "F0", "target": "F1", "kind": "import"},
    {"source": "S42", "target": "S43", "kind": "reference"}
  ],
  "sccs": [
    {
      "index": 0,
      "nodes": ["S10", "S11"],
      "is_cyclic": true,
      "deployability": "cyclic_cluster",
      "size": 2
    }
  ],
  "deployability_report": {
    "independent_units": 120,
    "cyclic_clusters": 5,
    "total_components": 125
  }
}
```

### 6. Error Handling

Graph construction errors are recoverable and emit diagnostics:

- Missing import target: warning, edge not added
- Duplicate edge: deduplicated silently
- Cycle in ownership edges: error (violates invariant)

SCC computation is infallible for valid graphs.

### 7. Testing Strategy

1. Unit tests for GraphBuilder with known input/output
2. Unit tests for SCC computation on synthetic cyclic/acyclic graphs
3. Fixture tests for cross-file import detection per language
4. Integration tests for mixed-language dependency chains
5. Snapshot tests for graph JSON output format

### 8. Performance Considerations

- Graph construction: sequential (linear in symbols + edges)
- Symbol extraction remains parallel (rayon)
- SCC computation: O(V + E) via Tarjan
- Memory: adjacency list via petgraph (O(V + E))
- No incremental updates in Phase 2 (full graph rebuild)

## Migration Path

This RFC introduces new modules without breaking existing Phase 1 functionality:

- `inspect` subcommand unchanged
- Symbol extraction interface unchanged
- New graph functionality additive only

Existing tests continue to pass. New tests validate graph-specific behavior.

## Alternatives Considered

### Alternative 1: Relational-first representation
Instead of adjacency list, use relational tables (Vec of nodes, Vec of edges with indices).

Rejected: petgraph provides battle-tested algorithms and the graph is inherently graph-structured. Relational adds indirection without benefit for SCC computation.
 
### Alternative 2: Custom SCC implementation
Implement Tarjan from scratch instead of using petgraph.

Rejected: petgraph's implementation is optimized and widely tested. No performance justification for custom implementation.

### Alternative 3: Separate ownership and dependency graphs
Maintain two distinct graph structures.

Rejected: Single graph with edge kind filtering is simpler and memory-efficient. SCC explicitly filters by edge kind.


## Open Questions

1. Should we include intra-file reference edges (symbol calls within same file) in SCC? yes, for completeness.

2. How to handle unresolved imports (external dependencies)? Skip with warning,? Decision: Skip with warning for Phase2.

3. Should deployability hints include suggested entry points for cyclic clusters? Deferred to Phase 3 when call graph is richer.

## References

- `docs/specs/graph-model.md` - Graph semantics and invariants
- `docs/specs/requirements.md` - FR-4, FR-5 for graph and SCC requirements
- ADR 0004 - Graph representation decision record
- petgraph documentation: https://docs.rs/petgraph
