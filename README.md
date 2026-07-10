<div align="center">
  <img src="docs/assets/metast.png" alt="meta-ast Logo" width="400">
  <p align="center"><strong>Standalone static analysis and dependency graph generator for polyglot source trees</strong></p>
</div>

---

`meta-ast` is a fast, standalone static analysis engine that parses multi-language projects, builds symbol-level dependency graphs, detects cyclic imports, and generates MetaCall deployment manifests. Written in Rust, powered by tree-sitter, with no runtime execution of user code.

Built as part of **Google Summer of Code 2026** for the **MetaCall** organization by **[Khaled Alam](https://github.com/k5602)**.

Supports **8 languages**: Python, JavaScript, TypeScript, TSX, C, C++, Rust, Go.

---

## Quick start

```bash
# Requires Rust toolchain - https://rustup.rs
git clone https://github.com/metacall/meta-ast.git
cd meta-ast
cargo build --release
```

The binary is at `./target/release/meta-ast`.

---

## Subcommands

### `inspect`

Extracts all function, class, and object declarations from a codebase.

```bash
meta-ast inspect <path> [-f json|yaml] [-o output.json]
```

### `graph`

Builds the cross-file dependency graph, resolves imports, and runs Tarjan SCC to identify cyclic clusters and independent deployment units.

```bash
meta-ast graph <path> [-f json|yaml] [-o graph.json]
meta-ast graph <path> --html                    # interactive Cytoscape.js dashboard
meta-ast graph <path> --html --self-contained   # offline, no CDN (embed-cytoscape feature)
```

### `deploy` *(requires `--features metacall-deploy`)*

Scans for cross-language `metacall_load_from_*` call sites and generates the full MetaCall manifest suite.

```bash
cargo build --release --features metacall-deploy

meta-ast deploy <path> --out ./out      # generate manifests
meta-ast deploy <path> --check          # CI validation against existing metacall.json
```

Generates three artifacts:

| File | Description |
|---|---|
| `metacall.json` | Root manifest composing all language groups |
| `metacall.<tag>.json` | Per-language manifest (e.g. `metacall.py.json`) |
| `metacall.mesh.json` | SCC-derived Function Mesh topology annotation |

See [docs/DEPLOY.md](docs/DEPLOY.md) for full scanner details, confidence scoring, mesh annotation schema, and extension guide.

---

## Documentation

| Document | Description |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | High-level pipeline and component boundaries |
| [docs/STRUCTURE.md](docs/STRUCTURE.md) | Module layout, data structures, design patterns |
| [docs/DEPLOY.md](docs/DEPLOY.md) | Deploy module: scanner, manifests, mesh annotation |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Phase-by-phase delivery plan |
| [docs/adr/](docs/adr/) | Architecture Decision Records |
| [docs/rfcs/](docs/rfcs/) | Design RFCs |
| [docs/specs/](docs/specs/) | Requirements and traceability |

---

## Roadmap

- **Phase 4 (In Progress)**: Watch mode, incremental analysis, C ABI.
- **Phase 5 (Implemented, feature-gated)**: `metacall-deploy` - call site scanning, manifests, Function Mesh annotation.
- **Phase 6 (Planned)**: Language expansion (C#, Java).

Full details in [docs/ROADMAP.md](docs/ROADMAP.md).

---

## License

Apache License, Version 2.0. See `LICENSE` for details.
