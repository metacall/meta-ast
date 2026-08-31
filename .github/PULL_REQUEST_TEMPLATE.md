<!--
  Title format: Conventional Commits, lowercase type and scope.
  Examples:
    feat(language): implement Rust intra-procedural dataflow extraction
    fix(graph): deduplicate client-call edges by file-to-symbol triple
    docs: update README installation and usage sections
  Branch: type/description, e.g. feat/fm-export-data-augmentation.
-->

## Summary

<!-- What this PR does and why it exists. Link the issue: Closes #NNN. -->

## Type of change

<!-- Mark with an x. -->

- [ ] `feat` - new functionality
- [ ] `fix` - bug fix
- [ ] `refactor` - no behavior change
- [ ] `perf` - performance improvement
- [ ] `docs` - documentation only
- [ ] `test` - tests only
- [ ] `ci` / `chore` - tooling or build

## Affected area

<!-- Which module(s) this touches. -->

- [ ] `model` / ids
- [ ] `graph` / SCC
- [ ] `language` (extraction)
- [ ] `parser`
- [ ] `interface` (CLI)
- [ ] `output`
- [ ] `watch` (feature)
- [ ] `deploy` (feature)
- [ ] `dataflow` (feature)
- [ ] `docs` / `tests`

## Public contract impact

<!-- If this changes CLI, output schema, graph semantics, or manifest format,
     update docs/ and add an ADR or RFC when warranted (see CONTRIBUTING.md). -->

- [ ] No public contract change
- [ ] Public contract changed; `docs/` updated (ADR/RFC where needed)

## Verification

<!-- Run before opening the PR. CI runs the same gates on stable + nightly. -->

- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] `cargo nextest run --all-features` passes
- [ ] `cargo +1.94.0 fmt --check` passes
- [ ] Snapshot changes reviewed with `cargo insta` and `.snap` files committed
- [ ] Any new language has fixtures under `tests/fixtures/<lang>/` and an insta test

## Notes for reviewers

<!-- Anything non-obvious: resolved trade-offs, known limitations, follow-ups. -->
