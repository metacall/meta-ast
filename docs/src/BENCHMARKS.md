# Benchmarks

Measured with [criterion](https://github.com/japaric/criterion.rs) on the CI
runner (`ubuntu-latest`) and on a local developer machine. Results below are
the local run from 2026-08-03, machine: x86_64 Linux, release profile,
`--features watch`.

Raw criterion reports are uploaded as CI artifacts from
`.github/workflows/benchmark.yml`. To reproduce locally:

```bash
cargo bench --features watch
```

## Pipeline (extraction)

End-to-end extraction across the per-language fixture suites:

| Benchmark | Time |
|---|---|
| extract/python_fixtures | 137 us |
| extract/javascript_fixtures | 11.5 ms |
| extract/rust_fixtures | 14.1 ms |
| extract/go_fixtures | 126 us |
| extract/c_fixtures | 113 us |
| extract/cpp_fixtures | 173 us |
| extract/typescript_fixtures | 121 us |
| extract/tsx_fixtures | 165 us |
| extract/mixed_fixtures | 459 us |
| extract/all_fixtures | 16.8 ms |

## Graph

Graph construction, Tarjan SCC, edge deduplication, and node lookup at scale:

| Benchmark | Time |
|---|---|
| graph_construction_linear/10 | 3.6 us |
| graph_construction_linear/100 | 40.8 us |
| graph_construction_linear/1000 | 425 us |
| scc_acyclic_chain/1000 | 157 us |
| scc_single_cycle/1000 | 54 us |
| scc_multiple_cycles/100_cycles | 15.2 us |
| scc_dense_graph/200 | 356 us |
| edge_deduplication/10000_duplicates | 486 us |
| node_lookup/10000 | 9.5 us |
| full_pipeline/python_extraction_to_scc | 149 us |
| ownership_graph_only/5000 | 1.03 ms |

## Datagraph and dataflow

| Benchmark | Time |
|---|---|
| datagraph_export/1000 | 325 us |
| datagraph_pipeline/python_to_datagraph | 155 us |
| dataflow_nodes_edges/5000 | 344 us |

## Incremental re-analysis

Requires `--features watch`. Cold `analyze_graph` vs warm
`incremental_reanalyze` after a single-file change (FR-5 target: <100 ms for
files under 5k LOC):

| Benchmark | Time |
|---|---|
| incremental/cold_analyze_graph | 1.66 ms |
| incremental/warm_incremental_reanalyze_single_change | 1.16 ms |
