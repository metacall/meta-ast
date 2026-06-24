# CI/CD Architecture

## 1. Objectives

- Enforce correctness and portability.
- Catch regressions early (lint, tests, benchmarks).
- Produce deterministic release artifacts.

## 2. Workflow layout

- `ci.yml` - single workflow with three jobs: test (nextest + doc tests), build (release artifacts), lint (fmt + clippy + cargo-deny).
- `benchmark.yml` - criterion benchmarks and trend tracking.
- `docs.yml` - docs generation and publication via mdbook.
- `release.yml` - tag-driven release and package publication with changelog generation.

## 3. Quality gates

Required for protected branch merge (all within `ci.yml`):

1. Lint job green (fmt + clippy + cargo-deny).
2. Test matrix green (nextest + doc tests across OS/toolchain).
3. Build artifacts generated (release binaries uploaded per OS).

## 4. Matrix strategy

OS targets:

- Linux (ubuntu-latest)
- macOS (macos-latest)
- Windows (windows-latest)
- Windows ARM64 (windows-11-arm)

Rust channels:

- stable (required)
- nightly (advisory compatibility signal)

## 5. Caching and artifacts

- Use cargo target cache for CI acceleration.
- Persist benchmark reports as artifacts.
- Publish release binaries per target triple.

## 6. Branch protection

- Require status checks before merge.
- Require branch up-to-date with target branch.
- Require at least one review approval.
- Require resolved review conversations.

## 7. Security posture

- Dependency vulnerability scanning on schedule and push.
- Keep lockfile current and reviewed.
- Do not suppress warnings as default policy.

## 8. Release policy

- Semantic version tags trigger release workflow.
- Generate changelog summary from merged PRs/issues.
- Publish binary artifacts and crate package (when ready).

## 9. Documentation policy in CI

Any change that touches output contract, graph semantics, or language extraction must update relevant docs under `docs/` in the same PR.

## 10. Toolchain configuration

- `rust-toolchain.toml` pins channel `1.92.0` with `rustfmt` and `clippy` components.
- `deny.toml` enforces license allowlist (MIT, Apache-2.0, BSD, ISC, etc.) and bans wildcard dependencies.
- `clippy.toml` sets MSRV and complexity thresholds.
- `rustfmt.toml` sets edition 2024 formatting rules.

## 11. Pre-commit hooks (lefthook)

Managed via `lefthook.yml`. Install with:

```sh
https://lefthook.dev/install/

lefthook install
```

a dev script will be provided later to automate this process

### pre-commit (parallel, fast)

1. `cargo fmt --all -- --check` - formatting gate.
2. Trailing whitespace check (`*.rs`, `*.toml`, `*.md`, `*.yml`).
3. Merge conflict marker detection.
4. Large file guard (>512KB).

### pre-push (sequential, thorough)

1. `cargo clippy --all-targets --all-features -- -D warnings` - lint gate.
2. `cargo nextest run --all-features` - local test gate.

## 12. Test runner (nextest)

CI and local dev use `cargo-nextest` for faster test execution.

- Config: `.config/nextest.toml`
- CI profile: `fail-fast = false` (full matrix visibility).
- Slow-timeout: 60s per test, terminate after 3 periods.

## 13. Benchmarks (criterion)

- Dev dependency: `criterion` 0.8 with `html_reports` feature.
- Benchmark targets: `benches/pipeline.rs` (extraction throughput per language fixture), `benches/graph.rs` (graph operation throughput).
- Profile: `opt-level = 3`, LTO enabled (see `[profile.bench]` in `Cargo.toml`).

## 14. Release targets

| Target triple | OS | Runner |
|---|---|---|
| `x86_64-unknown-linux-gnu` | Linux (glibc) | `ubuntu-latest` |
| `x86_64-unknown-linux-musl` | Linux (static) | `ubuntu-latest` + musl-tools |
| `aarch64-unknown-linux-gnu` | Linux (ARM64) | `ubuntu-24.04-arm` |
| `x86_64-apple-darwin` | macOS (Intel) | `macos-15` |
| `aarch64-apple-darwin` | macOS (Apple Silicon) | `macos-latest` |
| `x86_64-pc-windows-msvc` | Windows (x64) | `windows-latest` |
| `aarch64-pc-windows-msvc` | Windows (ARM64) | `windows-11-arm` |

## 15. Snapshot policy

- Insta `.snap` files are committed to the repository.
- Pending snapshots (`.snap.new`) are gitignored.
- Update workflow: `cargo insta test` → `cargo insta review` → commit accepted `.snap` files.
