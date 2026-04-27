# TODO

## High
- [ ] Handle All it's will work fine '.unwrap()' and '.expect()' in the codebase, and replace them with proper error handling.
- [ ] Add more tests for edge cases and error scenarios, especially for malformed files and supported languages.
- [ ] Implement a more robust error handling strategy across the codebase, especially in the graph builder and orchestrator, to ensure that errors are logged and handled gracefully without crashing the entire process.
## Medium
- [ ] Better inline Documentation for Phase 1 code
- [ ] Update inline Documentation of Phase 2 and representation PR code too
- [ ] Add a README about the project goals and scope and Potential Use Cases with some benchmarking.
- [ ] Better Path Normalization for Each Language to better handle externals and relative imports, and to improve cross-language reference resolution.
- [ ] Language module deduplication and refactor the boilerplate code 'declarative macro usage'.
- [ ] Merge Build and Test CI workflows into a single workflow with multiple jobs to reduce overhead and improve feedback loop.
## Low
- [ ] Add more languages, starting with C#, and then expanding to Ruby, PHP,Java, and others based on demand but mainly we need C# and Java.
- [ ] Micro-optimizations in the graph builder, especially around reference resolution and Resolver.
- [ ] Orchestrator `main.rs` needs Micro-optimizations and better error handling, especially around file I/O and parallel processing.
- [ ] Distribution bash scripts for easy install.
- [ ] Improve CLI ergonomics with better flags, help messages, and error reporting.
- [ ] A future improvement could merge queries (symbol + import + reference) , but the effort-to-reward ratio is low for now.
