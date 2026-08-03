# Contributing to meta-ast

Thanks for wanting to contribute. This file explains how the project works and what happens to your change once you open a pull request. It is written for humans.

`meta-ast` is a static analysis engine written in Rust. It parses 9 languages with tree-sitter, extracts symbols, builds cross-language dependency graphs, and detects import cycles. It never executes the code it analyzes. The README covers the tool itself; `docs/` covers the architecture.

## Table of contents

- [Code of conduct](#code-of-conduct)
- [Getting started](#getting-started)
- [Development loop](#development-loop)
- [Testing](#testing)
- [Style and linting](#style-and-linting)
- [Commit messages](#commit-messages)
- [Pull requests](#pull-requests)
- [Reporting bugs and ideas](#reporting-bugs-and-ideas)
- [Project structure](#project-structure)
- [Adding a new language](#adding-a-new-language)
- [Documentation](#documentation)
- [Releases](#releases)
- [License](#license)

## Code of conduct

`meta-ast` is part of the MetaCall organization. Be respectful and constructive in issues, reviews, and discussions. There is no code of conduct file in this repository yet, so the GitHub community guidelines are the baseline. Flag problems to the maintainers directly.

## Getting started

### Prerequisites

- Rust 1.94.0. The version is pinned in `rust-toolchain.toml`, so rustup installs it automatically. Newer stable versions usually work. CI also tests nightly, but nightly failures are advisory.
- `cargo-nextest` if you want to run tests the same way CI does.
- `cargo-insta` for the snapshot workflow.
- `cargo-deny` if you want to run license and dependency checks locally.

### First build

```bash
git clone https://github.com/metacall/meta-ast.git
cd meta-ast
cargo build --release
```

The binary lands in `target/release/meta-ast`. Try it on the fixture corpus:

```bash
./target/release/meta-ast inspect tests/fixtures/python
./target/release/meta-ast graph tests/fixtures/mixed --html
```

### Where to start

`TODO.md` lists open engineering tasks. `docs/ROADMAP.md` shows the phase plan. The language modules under `src/language/` are a good first change: each is one self-contained `LanguageSpec` const.

## Development loop

Build with the features you need:

```bash
cargo build --features watch --features metacall-deploy --features dataflow
```

Run the CLI with `cargo run`. Logs come from `tracing`; set `RUST_LOG` to get more detail:

```bash
RUST_LOG=debug cargo run -- graph tests/fixtures/mixed
```

Watch mode re-analyzes on file changes with a debounce:

```bash
cargo run --features watch -- graph tests/fixtures/mixed --watch --watch-debounce 100 --html
```

The `deploy` subcommand needs the `metacall-deploy` feature:

```bash
cargo run --features metacall-deploy -- deploy tests/fixtures/mixed --check
```

## Testing

Run the full suite:

```bash
cargo test --all-features
```

CI uses nextest, so matching it locally is a good idea:

```bash
cargo nextest run --all-features
```

Plain `cargo test` skips feature-gated tests. The `watch`, `metacall-deploy`, and `dataflow` modules only exist under their features, so `--all-features` matters.

Run a single test by name:

```bash
cargo test --all-features rust_insta_snapshot
```

### Snapshot tests

Language extraction output is verified with insta snapshots. When extraction output changes deliberately:

```bash
cargo insta test --all-features
cargo insta review
```

Accept the changes and commit the `.snap` files. `.snap.new` files are gitignored; never commit them.

### Benchmarks

Three criterion suites live in `benches/`:

```bash
cargo bench                                  # all suites
cargo bench --bench pipeline
cargo bench --bench graph
cargo bench --bench incremental --features watch
```

Benchmarks read from `tests/fixtures/`, so results scale with that corpus. For regression tracking, use `cargo bench -- --save-baseline <name>` and compare baselines. Numbers are only comparable on a quiet machine.

## Style and linting

Formatting is rustfmt with the repo config: edition 2024, 100 columns, 4-space indent, Unix newlines.

```bash
cargo +1.94.0 fmt --check
```

Clippy is a hard gate in CI. No warnings are allowed:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

`clippy.toml` sets complexity thresholds (cognitive-complexity 30, too-many-args 8) and bans placeholder identifiers such as `foo`, `baz`, and `quux`.

Lefthook enforces the basics on commit: formatting, trailing whitespace, merge conflict markers, and a 512KB file size cap. On push it runs clippy and the nextest suite. Install the hooks with `lefthook install`.

Code style notes:

- snake_case for functions and variables, PascalCase for types.
- Module-level `//!` docs state what a module does and its invariants. Public items get `///` docs.
- Errors: recoverable per-file problems are `Diagnostic` values; structural failures use the typed `Error` enum in `src/error.rs`. `anyhow` is for CLI boundaries only.
- Feature gates live at module level in `src/lib.rs`. The `dataflow` feature also gates fields and parameters.

## Commit messages

The repository uses Conventional Commits. Type and scope are lowercase:

```
feat(language): implement Rust and Python intra-procedural dataflow extraction
refactor(model): optimize ID memory layout with NonZeroU32 niche optimization
docs: update README installation and usage sections
```

Types seen in history: `feat`, `fix`, `refactor`, `docs`, `chore`, `test`, `ci`, `perf`. Use a scope when the change touches one module: `model`, `graph`, `language`, `deploy`, `watch`, `output`, `cli`, `deps`. No tooling enforces this format; it is the style the project uses, so match it.

## Pull requests

Work on a branch off `main`, then open a PR against `main`. Branch names in this project use a `type/description` prefix, for example `feat/fm-export-data-augmentation` or `perf/opt_id`.

Describe the change briefly: what it does, why it exists, and how you tested it. If it changes a public contract, point at the doc you updated (see the documentation section below).

Run through this checklist before opening the PR:

- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] `cargo nextest run --all-features` passes
- [ ] `cargo +1.94.0 fmt --check` passes
- [ ] Snapshot changes are reviewed and `.snap` files are committed
- [ ] Public contract changes update `docs/`, with an ADR or RFC if warranted

What happens after you open it:

- CI runs the full matrix: tests on 4 operating systems on stable and nightly (nightly failures are advisory), a release build, and lint. Everything except nightly must be green.
- Branch protection requires an up-to-date branch, one approving review, and resolved conversations.
- A maintainer merges the PR with a `Merge pull request #N` commit.

## Reporting bugs and ideas

Open an issue on GitHub. Include the command you ran, the output, and what you expected. For extraction or graph bugs, a small fixture that reproduces the problem helps a lot, because every language test is fixture-driven. Feature ideas that change the pipeline or output contract go through the RFC process in `docs/rfcs/`; everything else is a normal issue.

## Project structure

The pipeline is short and linear:

`CLI (src/interface)` -> file discovery and language detection (`src/input`) -> parallel extraction (`src/extractor`, `src/parser`, `src/language`) -> graph and SCC (`src/graph`) -> output (`src/output`).

| Path | Responsibility |
|------|----------------|
| `src/interface` | clap CLI: `inspect`, `graph`, `deploy` |
| `src/input` | extension-based language detection, file discovery |
| `src/model` | IR: `Symbol`, `FileExtraction`, newtype IDs, dataflow types |
| `src/extractor` | rayon-parallel per-file extraction |
| `src/parser` | thread-local tree-sitter parser pool |
| `src/language` | one `LanguageSpec` const per language, import resolvers, snapshots |
| `src/graph` | `CodeGraph`, `GraphBuilder`, scope resolver, Tarjan SCC |
| `src/output` | JSON/YAML serialization, HTML dashboard |
| `src/watch` | feature `watch`: debounced incremental re-analysis |
| `src/deploy` | feature `metacall-deploy`: pod and mesh manifests |
| `src/sink` | feature `dataflow`: datagraph sinks |

`docs/STRUCTURE.md` goes deeper into every module.

## Adding a new language

Adding a language touches a handful of places. The design is static dispatch: `LangId` is an enum, and each variant maps to a `const LanguageSpec` in an exhaustive match. No trait objects.

1. Add a `LangId` variant in `src/language/mod.rs`. The enum stays dense, because the parser pool indexes by `lang as usize` into a fixed array sized by `LangId::COUNT`.
2. Add the tree-sitter grammar crate to `Cargo.toml` under dependencies.
3. Create `src/language/<lang>.rs` with a `const LanguageSpec`: extensions, grammar fn, symbol query, import/reference query, import path resolver, class-like parents, visibility rules, default visibility, doc comment config. Extensions must not collide with existing specs; a test enforces uniqueness.
4. Wire the variant into `spec_for` and any other exhaustive matches.
5. Add fixtures under `tests/fixtures/<lang>/`. Look at an existing language's fixtures for the shape: simple functions, classes or structs, deep nesting, a file with a syntax error.
6. Add an `*_insta_snapshot` unit test in the new module and generate the snapshot.
7. Run the full verification: tests, clippy, fmt. Update `docs/STRUCTURE.md` and `docs/specs/` per the documentation policy.

## Documentation

The docs live in `docs/`. `docs/README.md` is the index and defines the update policy: any PR that changes a public contract updates the matching document.

| Document | Covers |
|----------|--------|
| `docs/ARCHITECTURE.md` | pipeline stages and component boundaries |
| `docs/STRUCTURE.md` | module layout and data structures |
| `docs/CI_CD.md` | workflows, quality gates, branch protection |
| `docs/DEPLOY.md` | deploy module artifacts and CLI |
| `docs/ROADMAP.md` | phase plan and exit gates |
| `docs/adr/` | numbered architecture decision records |
| `docs/rfcs/` | numbered design RFCs |
| `docs/specs/` | requirements and traceability |

Architectural decisions go in `docs/adr/` as a numbered ADR with context, decision, alternatives, and consequences. Design proposals go through `docs/rfcs/` with a Status header.

## Releases

Releases are driven by tags. Pushing a `v*` tag runs `release.yml`, which builds the binary for 7 target triples (Linux gnu and musl, aarch64 Linux, macOS x64 and arm64, Windows x64 and arm64) in both core and `metacall-deploy` variants, then creates a GitHub release with a changelog generated from `git log` since the previous tag. Versioning is semver. Maintainers cut releases; if you think one is due, say so in the issues.

## License

Apache-2.0. See `LICENSE`.
