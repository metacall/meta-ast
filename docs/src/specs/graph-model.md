# Graph Model Specification

## 1. Purpose

Define the normalized graph model used for dependency analysis and SCC computation.

## 2. Node categories

### FileNode

- `id`: stable identifier
- `path`: normalized path relative to the analyzed project root
- `language_id`: configured language/runtime identifier
- `snapshot_id`: snapshot identifier for the analysis version this file belongs to

### SymbolNode

- `id`: stable identifier
- `name`: symbol name
- `kind`: source symbol category such as function | class | object | interface | trait | struct | enum | method
- `file_id`: references the owning `FileNode.id`
- `visibility`: optional, when applicable: public | private
- `source_range`: byte/line range

### DataNode

- `id`: stable identifier for a value-bearing node
- `symbol_id`: optional symbol reference when the data node is derived from a named symbol
- `scope`: local, parameter, closure, member, or temporary scope classification
- `type_hint`: optional inferred or declared type information

DataNode represents a value or variable instance used for def-use and flow analysis rather than a declaration boundary.

## 3. Edge categories

### ImportEdge

`FileNode -> FileNode` representing import/include/use relationships.

### ReferenceEdge

`SymbolNode -> SymbolNode` representing symbol usage/call/reference candidates.

### OwnershipEdge

`FileNode -> SymbolNode` and optional `SymbolNode -> SymbolNode` for nesting.

### FlowEdge

`DataNode -> DataNode` for def-use transitions.

## 4. Graph invariants

1. Every SymbolNode must map to exactly one FileNode.
2. Ownership edges must form an acyclic containment structure.
3. SCC computation applies to dependency/reference subgraph, not ownership edges. Self-loop detection and independence classification follow the same subgraph rule.
4. Duplicate edges should be normalized by `(src, dst, edge_kind)` key. Client-call edges (FileNode to SymbolNode) can only merge with other client-call edges; scope-resolved references (SymbolNode to SymbolNode) never collide with them. Strongest evidence wins within a triple.

## 5. SCC semantics

Tarjan SCC runs on directed dependency/reference graph.

- SCC size = 1 with no self-loop => acyclic unit.
- SCC size > 1 or self-loop => cyclic unit.

Deployability hint policy:

- Acyclic SCCs are preferred deployment candidates.
- Cyclic SCCs require grouped deployment or refactor guidance.

## 6. Serialization contract

Graph serialization for external consumers shall preserve:

- stable IDs
- edge kinds
- source ranges for symbol nodes

Sink adapters (e.g., Dgraph) that must preserve semantic equivalence.

The serialized `language` field on file and external nodes uses the canonical
lowercase language names, in enum declaration order:

`python`, `javascript`, `typescript`, `tsx`, `c`, `cpp`, `rust`, `go`, `ruby`

These values also parse back through the CLI `--language` flag. The same names
apply to the strum `Display`/`AsRefStr` and serde representations of `LangId`.

## 7. Known limitations

- Cross-language resolution is initially best-effort string/scope matching.
- Full semantic type equivalence across languages is deferred.
- DataNode/FlowEdge extraction is implemented for Rust (let bindings,
  parameters, def-use chains) behind the `dataflow` feature flag;
  other languages return empty vectors. Python/JS/TS/Go/C/C++ are
  stubbed with TODO markers. Full coverage is tracked in Phase 6.
