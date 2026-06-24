# TODO

## High
- [ ] Handle all `.unwrap()` and `.expect()` calls in the codebase and replace them with proper error handling.
- [ ] Add more tests for edge cases and error scenarios, especially for malformed files and all supported languages.
- [ ] Implement a more robust error handling strategy across the codebase, especially in the graph builder and orchestrator, to ensure errors are logged and handled gracefully without crashing the entire process.

## Medium
- [ ] Better inline documentation for Phase 1 code.
- [ ] Update inline documentation of Phase 2 and representation PR code.
- [ ] Add a README about the project goals, scope, and potential use cases with benchmarking.
- [ ] Better path normalization for each language to improve handling of externals and relative imports, and cross-language reference resolution.
- [ ] Language module deduplication - refactor boilerplate code via declarative macros.
- [ ] Merge Build and Test CI workflows into a single workflow with multiple jobs to reduce overhead and improve feedback loop.
- [ ] Add `metacall-deploy` feature to `Cargo.toml` gating the deploy subcommand and all manifest generation logic.
- [ ] Implement Cross-Language Call Site scanner: detect `metacall_load_from_file`, `metacall_load_from_memory`, `metacall_load_from_package`, `metacall_load_from_configuration` call sites across all language packs.
- [ ] Deploy Manifest generator: emit one `metacall.{tag}.json` per detected language group and a root `metacall.json` composing them.
- [ ] Mesh Annotation emitter: emit `metacall.mesh.json` from SCC Deployment Unit analysis, classifying independent Function Mesh candidates vs. co-deployment-required groups.
- [ ] `meta-ast deploy --check`: diff generated manifests against existing `metacall.json` and report divergences as structured diagnostics.
- [ ] Low-confidence annotation for unresolvable Cross-Language Call Site arguments (dynamic tag or path values).

## Low
- [ ] Add more languages, starting with C# and Java, then Ruby, PHP, and others based on demand.
- [ ] Micro-optimizations in the graph builder, especially around reference resolution and the resolver.
- [ ] Orchestrator `main.rs` needs micro-optimizations and better error handling around file I/O and parallel processing.
- [ ] Distribution bash scripts for easy install.
- [ ] Improve CLI ergonomics with better flags, help messages, and error reporting.
- [ ] A future improvement could merge queries (symbol + import + reference), but the effort-to-reward ratio is low for now.
