# CI/CD Architecture

## 1. Objectives

- Enforce correctness and portability.
- Catch regressions early (lint, tests, benchmarks).
- Produce deterministic release artifacts.

## 2. Workflow layout

- `lint.yml` — format + clippy + dependency policy checks.
- `test.yml` — unit/integration/doc tests on Linux/macOS/Windows.
- `security.yml` — dependency vulnerability checks.
- `build.yml` — release build matrix and artifact upload.
- `benchmark.yml` — criterion benchmarks and trend tracking.
- `docs.yml` — docs generation and publication 'mdbook'.
- `release.yml` — tag-driven release and package publication.

## 3. Quality gates

Required for protected branch merge:

1. Lint workflow green.
2. Test matrix green.
3. Security checks green.
4. Build artifacts generated.

Recommended optional gate:

- Benchmark regression threshold enforcement.

## 4. Matrix strategy

OS targets:

- Linux
- macOS
- Windows

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

1. `cargo fmt --all -- --check` — formatting gate.
2. Trailing whitespace check (`*.rs`, `*.toml`, `*.md`, `*.yml`).
3. Merge conflict marker detection.
4. Large file guard (>512KB).

### pre-push (sequential, thorough)

1. `cargo clippy --all-targets --all-features -- -D warnings` — lint gate.
2. `cargo nextest run --all-features` — local test gate.

## 12. Test runner (nextest)

CI and local dev use `cargo-nextest` for faster test execution.

- Config: `.config/nextest.toml`
- CI profile: `fail-fast = false` (full matrix visibility).
- Slow-timeout: 60s per test, terminate after 3 periods.

## 13. Benchmarks (criterion)

- Dev dependency: `criterion` 0.5 with `html_reports` feature.
- Benchmark target: `benches/pipeline.rs` — extraction throughput per language fixture.
- Profile: `opt-level = 3`, LTO enabled (see `[profile.bench]` in `Cargo.toml`).

## 14. Release targets

| Target triple | OS | Runner |
|---|---|---|
| `x86_64-unknown-linux-gnu` | Linux (glibc) | `ubuntu-latest` |
| `x86_64-unknown-linux-musl` | Linux (static) | `ubuntu-latest` + musl-tools |
| `x86_64-apple-darwin` | macOS (Intel) | `macos-13` |
| `aarch64-apple-darwin` | macOS (Apple Silicon) | `macos-latest` |
| `x86_64-pc-windows-msvc` | Windows | `windows-latest` |

## 15. Snapshot policy

- Insta `.snap` files are committed to the repository.
- Pending snapshots (`.snap.new`) are gitignored.
- Update workflow: `cargo insta test` → `cargo insta review` → commit accepted `.snap` files.
