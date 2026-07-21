# TODO

## Medium
- [ ] Better path normalization for each language to improve handling of externals and relative imports, and cross-language reference resolution.
- [ ] Language module deduplication - refactor boilerplate code via declarative macros.
- [ ] Refactor `deploy/mod.rs`: `run_deploy` (210 lines) handles scanning, edge injection, pod partitioning, metrics, cuts, dependencies, manifest, mesh, and check. Split into a `DeployOrchestrator` struct for single-responsibility and testability.
- [ ] Eliminate duplicate file parsing in deploy: `run_deploy` calls `pipeline::analyze_graph` (which parses all files) then separately calls `extractor::extract(&files)` (re-parsing all files). Reuse extraction results instead of re-reading/re-parsing 'NEEDS RESEARCH'.
- [ ] Replace `Box<dyn ImportResolver>` with enum dispatch: The `ImportResolver` trait object allocates a heap box per language per pipeline run. Since resolvers are stateless wrappers around fn pointers, an enum dispatch would avoid allocation.
- [ ] Combine tree walks in parser: `count_nodes` and `error_ratio` in `parser/mod.rs` both walk the tree separately. Combine into a single pass.

## Low
- [ ] Make `DEFAULT_MAX_POD_SIZE` configurable: Currently hardcoded to 20 in `cut.rs:44`.
- [ ] Add more languages, starting with C# and Java, then Ruby, PHP, and others based on demand.
- [ ] Micro-optimizations in the graph builder, especially around reference resolution and the resolver.
- [ ] Orchestrator `main.rs` needs micro-optimizations and better error handling around file I/O and parallel processing.
- [ ] Distribution bash scripts for easy install.
- [ ] Improve CLI ergonomics with better flags, help messages, and error reporting.
- [ ] A future improvement could merge queries (symbol + import + reference), but the effort-to-reward ratio is low for now.
- [ ] Add property-based tests (`proptest`) for SCC correctness and graph invariants.
- [ ] Create missing RFC/ADR documents: Code references "RFC 0008" and "ADR 0003" which don't exist in the repo.
- [ ] Implement ignored `--language` CLI flag: Both `InspectArgs` and `GraphArgs` accept `--language` but `main.rs` never uses it to filter `discover_files`.
- [ ] Add `--exclude`/`--include` CLI patterns: Users can't filter which files/directories to analyze.
- [ ] Encapsulate `petgraph::DiGraph` in `CodeGraph`: The `graph` field is `pub`, leaking the underlying implementation.
- [ ] Improve edge normalization performance: `add_edge_normalized` is O(degree) - i will consider `HashMap<(NodeIndex, NodeIndex, EdgeKind), EdgeIndex>` for O(1) lookup.
