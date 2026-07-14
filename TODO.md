# TODO

## Medium
- [ ] Better inline documentation for Phase 1 code.
- [ ] Update inline documentation of Phase 2 and representation PR code.
- [ ] Better path normalization for each language to improve handling of externals and relative imports, and cross-language reference resolution.
- [ ] Language module deduplication - refactor boilerplate code via declarative macros.

## Low
- [ ] Add more languages, starting with C# and Java, then Ruby, PHP, and others based on demand.
- [ ] Micro-optimizations in the graph builder, especially around reference resolution and the resolver.
- [ ] Orchestrator `main.rs` needs micro-optimizations and better error handling around file I/O and parallel processing.
- [ ] Distribution bash scripts for easy install.
- [ ] Improve CLI ergonomics with better flags, help messages, and error reporting.
- [ ] A future improvement could merge queries (symbol + import + reference), but the effort-to-reward ratio is low for now.
