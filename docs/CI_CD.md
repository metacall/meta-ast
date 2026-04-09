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
