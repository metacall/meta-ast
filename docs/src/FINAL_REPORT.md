# Final Report: meta-ast, GSoC 2026

Author: [Khaled Alam](https://github.com/k5602)
Organization: [MetaCall](https://github.com/metacall)
Project: `meta-ast` - polyglot static analysis engine
Status: Complete, v0.5.0

## Summary

`meta-ast` is a standalone static analysis engine written in Rust. It parses
nine languages with tree-sitter (Python, JavaScript, TypeScript, TSX, C, C++,
Rust, Go, Ruby), extracts a normalized symbol IR, builds a cross-language
dependency graph, detects import cycles with Tarjan SCC, and - behind the
`metacall-deploy` feature - generates MetaCall pod and mesh deployment
manifests. It never executes user code.

The project shipped in seven phases. Everything below is delivered, tested,
documented, and released.

## What shipped

### Phase 1-2: Core extraction, graph, SCC

- Uniform parser lifecycle over nine languages with thread-local parser pools.
- Symbol extraction (functions, classes, methods, structs, enums, interfaces,
  objects) with visibility and docstring metadata.
- Cross-file import and reference resolution with confidence-weighted edges
  (1.0 own-file, 0.8 transitive same-language, 0.6 cross-language).
- Tarjan SCC with deployment-unit classification and `EdgeFiltered` views that
  keep Ownership and Flow edges out of cycle detection.

### Phase 3: Datagraph and sink

- Optional def-use dataflow model (`DataNode`, `FlowEdge`) with a portable
  schema-versioned export and a pluggable sink trait.

### Phase 4: CLI polish, output, watch

- `--format json|yaml` for both `inspect` and `graph`.
- Interactive Cytoscape.js HTML dashboard (`--html`).
- Watch mode with debounced incremental re-analysis: unchanged files reuse
  cached `Arc<FileExtraction>` values (zero-allocation invariant), changed
  files are re-extracted through an ID seam that guarantees collision-free
  `SymbolId` assignment.

### Phase 5: MetaCall deploy manifests

- Cross-language call-site detection across all ports
  (`metacall_load_from_*`, CommonJS `require`, Rust `use` calls, `metacall()`
  client calls per RFC 0011).
- Same-language pod partitioning via Union-Find, SCC-derived mesh annotation,
  per-language external dependency resolution from lockfiles, and a `--check`
  fairness mode that verifies every cut edge has an RPC stub (ADR 0003).

### Phase 6: Language expansion

- Ruby support (the third new language after the initial eight): full symbol
  pack, `require`/`require_relative` imports, references, and deploy tag and
  call-site coverage.
- C# and Java were scoped out as `NOT_PLANNED` after evaluation; Ruby shipped.

### Phase 7: Validation and delivery

- CI/CD hardening: four-OS matrix (Linux, macOS, Windows, Windows ARM) x
  stable/nightly, nextest, doc tests, clippy `-D warnings`, cargo-deny,
  benchmark workflow, docs workflow publishing this book to GitHub Pages.
- Release engineering: tag-driven multi-target release workflow (7 targets x
  core + deploy binaries), crates.io publication.
- This book, the demo page, the benchmark snapshot, and the release
  announcement.

## Numbers

- 9 languages, 12 language packs (TSX and snapshots included).
- 327 library tests + 106 integration tests, all green on the CI matrix.
- 4 OS targets x 2 toolchains in the test matrix.
- 14 release artifacts per tag (7 targets x 2 feature profiles).
- Incremental warm re-analysis: 1.16 ms for a single-file change on the
  benchmark fixture (target was <100 ms).
- 2 crates.io releases (0.4.0, 0.5.0), 5 GitHub releases (v0.1.0-v0.5.0).

## Releases

- v0.1.0 through v0.5.0 on GitHub Releases with binaries for Linux (glibc +
  musl), macOS (x86_64 + aarch64), and Windows (x86_64 + aarch64).
- `meta-ast` on crates.io.

## Future work (post-GSoC)

- Deeper intra-procedural dataflow (Ruby/JS/TS/Go/C/C++ packs are stubs).
- C ABI/header generation (RFC proposed, not shipped).
- More languages (C#, Java, PHP) on demand.
- RFC 0029: polyglot SAST with ML integration (proposed, POST V1).

## Links

- Repository: https://github.com/metacall/meta-ast
- crates.io: https://crates.io/crates/meta-ast
- Documentation: https://metacall.github.io/meta-ast/
