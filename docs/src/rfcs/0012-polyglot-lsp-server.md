# RFC 0012: Polyglot LSP Server over the `.metast` Index

## Status

Approved.

## 1. Problem

MetaCall projects mix languages in one workspace. A function defined in Python is called from TypeScript through `metacall()`. No editor sees this: editors run one language server per file type, and each server knows nothing about the other languages. Cross-language navigation, hover, and completion are invisible.

Two past attempts failed:

- `intellisense`: a VSCode extension, never a real language server. Per-request Python subprocesses for goto-definition (with an async race that missed on first use), placeholder types injected destructively into user `.ts` files, hardcoded type maps, and a committed cache with absolute Windows paths "first try". The `vscode-languageclient` dependency was dead weight.
- `vscode-extension`: deploy tooling and snippets only, no code intelligence "which doesn't make sense".

Meanwhile `meta-ast` already computes everything a code-intelligence backend needs: tree-sitter extraction for 9 languages, import resolution, a cross-language dependency graph with a confidence ladder, Tarjan SCC, call-site detection, and incremental re-analysis. But its output is a CLI artifact with no consumer loop except Function Mesh for now.

## 2. Proposal

The etags shape, upgraded:

1. `meta-ast` emits a versioned, incrementally updatable index artifact (`.metast` v2).
2. A new LSP server consumes that index and serves requests for all supported languages from one process.
3. Open-editor buffers feed the same extraction path, so unsaved edits stay consistent.
4. Runtime metadata from `metacall inspect` enriches hover when available; static features never depend on it.

The LSP does not re-parse anything. It serves queries from an immutable index snapshot and coordinates re-indexing when files change.

Home: the server lives in its own repository, `metacall/lsp`, not in this one. `meta-ast` stays a library dependency as discussed, Phase 0 patches below are the only changes required here.

## 3. Architecture

```text
┌──────────────────────────────────────────────────────┐
│  Editor (VSCode / Neovim / emacs)                    │
│    thin client: launch server, forward buffers       │
├─────────────────────────── LSP (stdio) ──────────────┤
│  meta-ast-lsp server                                 │
│  ┌────────────────┐   crossbeam   ┌───────────────┐  │
│  │ sync main loop │<-------------│ reindex worker│  │
│  │ answers from   │   swap Arc   │ runs engine   │  │
│  │ Arc<IndexSnap> │------------->│ incremental   │  │
│  └────────────────┘              └───────────────┘  │
│                          │ embeds                    │
│  ┌───────────────────────▼──────────────────────┐    │
│  │ meta-ast engine (library, not CLI)           │    │
│  │ WatchState + incremental_reanalyze           │    │
│  │ BLAKE3 diff, per-file FileExtraction deltas  │    │
│  └──────────────────────────────────────────────┘    │
├──────────────────────────────────────────────────────┤
│  Disk state                                          │
│    .meta-ast/manifest.jsonl + per-file shards        │
│    (cold start without full re-analysis)             │
└──────────────────────────────────────────────────────┘
```

### 3.1 Engine layer

Embed `meta-ast` as a library inside the server process. Reuse directly:

- `incremental_reanalyze` + `WatchState`: BLAKE3 fingerprint diff, re-extract only changed files, unchanged files keep their cached `Arc<FileExtraction>`.
- `IdGenerator::with_start(max_cached_id + 1)` seam: no ID collisions across ticks.
- `GraphBuilder::from_extractions` + `add_edge_normalized`: dedup and confidence rules stay canonical.

New seams required:

- In-memory ingestion: extract from `(uri, text, version)` for open buffers instead of disk reads. This extends `extract_with_id_gen`, it does not replace disk discovery.
- Resolver invalidation: `PythonResolver`, `TsConfigResolver`, `GoModResolver` cache filesystem state for the process lifetime. An LSP is long-lived, so these caches must invalidate on relevant config-file events. Until then, config changes require a server restart (documented limitation).
- Debounce: reuse the notify-debouncer pattern from `src/watch/watcher.rs`, driven by both OS events and `textDocument/didChange` flushes.

### 3.2 Index artifact (`.metast` v2)

Layout under `.meta-ast/`:

```text
manifest.jsonl     one line per file:
                   { path, content_hash (BLAKE3 hex), size,
                     mtime, shard, schema_version }
shards/<n>.jsonl   one block per file: symbols (full ranges),
                   imports, references, diagnostics, ast_node_count,
                   edges whose source or target belongs to the file
                   (endpoints as stable names)
header.json        { schema_version, tool_version, created_at }
```

Rules:

- Hash bytes, never trust mtime alone. mtime and size are a fast negative check only (git checkout and rsync lie about mtime).
- Stable join key: language-scoped qualified name (SCIP-style descriptor, e.g. `python module . encrypt .`). Numeric `SymbolId`s are per-run and rayon-nondeterministic; they are regenerated in memory at load and never persisted.
- Edge rows carry `(source_name, target_name, kind, confidence, flow_kind)` so a merged load reproduces `add_edge_normalized` semantics exactly.
- Cold start loads `manifest.jsonl` + shards; warm path skips unchanged files entirely.

Optional exchange layer (Phase 3): `meta-ast export scip` writes a standard SCIP index so Sourcegraph-style consumers can read the same data "not needed for now".

### 3.3 Server layer

Framework: `lsp-server` (rust-analyzer's sync framework, actively released from the rust-analyzer monorepo).

Rationale for this specific shape:

- Indexed queries are hash-map lookups. A synchronous main loop answering from `Arc<IndexSnapshot>` needs no async runtime.
- Reindex coordination is plain threads: worker mutates nothing shared, then hands the new snapshot over the channel; main loop swaps the `Arc`.
- This mirrors rust-analyzer's `main_loop.rs` task-passing design without salsa.
- Cancellation: `$/cancelRequest` becomes a token registry checked between handler phases. With sub-millisecond queries, cancellation pressure is minimal.

### 3.4 Client layer

Thin clients hosted under `clients/` in `metacall/lsp`: VSCode and Zed first. Each client (~200 lines) activates on supported languages, spawns the server binary over stdio, forwards `didOpen/didChange/didSave/didClose`, and surfaces status. No intelligence lives in a client.

Editors without an extension story (emacs, vim, helix) work with zero client code: any LSP configuration pointing at the binary is enough. The TypeScript/Rust clients are deliberately simple onboarding tasks for new contributors arriving through Discord.

### 3.5 Distribution and packaging

The end user installs one extension and sees none of this machinery:

1. CI publishes static server binaries per platform to GitHub releases of `metacall/lsp`.
2. The client extension downloads the matching binary on activation and checks releases for updates automatically.
3. If MetaCall itself is missing, the extension guides installation instead of failing silently. Static features never require MetaCall; only runtime enrichment (Phase 3) does.
4. Optional later step: bundle the server into the existing MetaCall installer 'not decided', which already ships deploy and FaaS components. The maintainer confirmed this path stays open. Default stays GitHub releases because it decouples tooling releases from runtime releases.

## 4. Feature Phasing

### Phase 0: meta-ast prep (prerequisite, small diffs)

1. `serialize_symbol_node` emits `source_range` and `file_path` for symbol nodes (currently dropped, which makes the graph export unusable for navigation).
2. Expose `extract_with_id_gen` over in-memory text.
3. Shard writer/reader module behind no new feature flag (it is pure output).

### Phase 1: single-language correctness (not alive in this crate)

- `initialize`, capabilities, workspace root handling (one root per server instance).
- Incremental document sync + buffer overlay.
- `textDocument/documentSymbol` (tree-sitter symbols already carry ranges).
- `textDocument/hover`: signature + docstring markdown.
- `textDocument/definition` within one language via resolved references.
- Publish diagnostics on re-extract.

### Phase 2: the polyglot payoff

- Cross-language `definition` and `references` routed through `CodeGraph` edges, including `metacall()` client-call edges with the RFC 0011 confidence ladder.
- `workspace/symbol`.
- `textDocument/completion`: bucketed symbol list with kind, signature, defining file; cross-language candidates ranked by edge confidence.
- Debounced background reindex on file change with snapshot swap.

### Phase 3: enrichment and ecosystem

- Hover merge with runtime metadata. Two sources, both optional enrichment:
    1. Local: parse `metacall inspect` JSON (parameter and return annotations where the loader captured them, e.g. Python type hints).
    2. Deployed: query a live FaaS deployment through the same `metacall/protocol` API that `metacall/deploy-mcp-server` wraps, so signatures reflect the running runtime.
  Absence of either changes nothing statically.
- Semantic tokens (symbol kinds map cleanly).
- Stub generation (recycled concept from `deprecated/intellisense`, done right): emit `.pyi` / `.d.ts` from the static model so native per-language servers also see cross-language signatures. Never write into user source files; write sibling stub directories configured as extra paths. Stubs are a one-way complement that feeds foreign signatures to native servers, they never carry navigation or diagnostics. The two-way channel is this LSP itself.
- `meta-ast export scip`.

## 5. Non-Goals

- Full type inference (RFC 0006 scope stands).
- Renaming across languages (write path; later decision if demand appears).
- Running or loading user code. Static analysis only; runtime inspection happens out-of-band via `metacall inspect`.
- Replacing native per-language servers. This server coexists; editors keep their TypeScript or Python LSP for deep single-language semantics and gain cross-language features from this one. We do not rebuild nine type systems: pyright and tsserver stay authoritative for deep inference.

## 6. Failure Modes We Will Not Repeat

From the old forensics:

1. No destructive edits of user source. Stubs go to generated directories.
2. No per-request subprocesses. Everything answers from the loaded snapshot.
3. No placeholder or fabricated types. Absent metadata means absent hover detail.
4. No committed caches with machine-specific paths.
5. No VSCode-only logic. Intelligence lives in the server; clients stay thin.

## 7. Deliverables

1. `meta-ast` Phase 0 patches in this repository: serializer ranges, memory-text extraction seam, shard IO. These land first so `metacall/lsp` starts against a capable library.
2. `metacall/lsp` repository: server crate (`meta-ast-lsp`: engine host, sync server loop, index store) plus thin TypeScript clients under `clients/vscode` and `clients/zed`.
3. Release engineering: per-platform binary builds published to GitHub releases; extension-side download and auto-update flow.

Verification for `metacall/lsp` follows the same repo standards:

```bash
cargo build --features watch --features metacall-deploy --features dataflow
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo +1.94.0 fmt --check
```

Plus new integration tests: shard round-trip, snapshot swap under concurrent queries, cross-language definition fixtures under `tests/fixtures/mixed/`.
