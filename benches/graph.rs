//! Benchmarks for the graph module and SCC analysis.
//!
//! These benchmarks measure:
//! - Graph construction performance
//! - SCC analysis on different graph topologies
//! - Node/edge operations at scale

use std::path::PathBuf;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use meta_ast::graph::node::{FileNode, SymbolNode};
use meta_ast::graph::{EdgeData, EdgeKind, GraphBuilder, NodeData, SccAnalysis};
use meta_ast::language::LangId;
use meta_ast::model::{
    LineColumn, SourceRange, Symbol, SymbolId, SymbolKind, Visibility, ids::SnapshotId,
};
use petgraph::graph::DiGraph;
use std::hint::black_box;

/// Create a test symbol with the given ID and name.
fn create_test_symbol(id: u32, name: &str, file_path: &str) -> Symbol {
    Symbol {
        id: SymbolId(id),
        name: name.to_string(),
        kind: SymbolKind::Function,
        language: LangId::Rust,
        file_path: PathBuf::from(file_path),
        source_range: SourceRange {
            byte_start: 0,
            byte_end: 10,
            start: LineColumn { line: 1, column: 0 },
            end: LineColumn {
                line: 1,
                column: 10,
            },
        },
        visibility: Some(Visibility::Public),
        signature: None,
        docstring: None,
        is_async: false,
    }
}

/// Create a test file node.
fn create_test_file_node(id: u32, path: &str) -> FileNode {
    use meta_ast::model::ids::FileId;
    FileNode {
        id: FileId(id),
        path: PathBuf::from(path),
        language: LangId::Rust,
        snapshot_id: SnapshotId(1),
    }
}

/// Benchmark graph construction with varying numbers of nodes.
fn bench_graph_construction_linear(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_construction_linear");

    for size in [10, 50, 100, 500, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut builder = GraphBuilder::new(SnapshotId(1));

                // Add files
                for i in 0..size {
                    let path = format!("src/file_{}.rs", i);
                    builder.add_file(PathBuf::from(&path), LangId::Rust);
                }

                // Add symbols (ownership edges) - match file paths
                for i in 0..size {
                    let path = format!("src/file_{}.rs", i);
                    let symbol = create_test_symbol(i as u32, &format!("func_{}", i), &path);
                    builder.add_symbol(&symbol);
                }

                black_box(builder.build());
            });
        });
    }

    group.finish();
}

/// Benchmark SCC analysis on acyclic graphs (chains).
fn bench_scc_acyclic_chain(c: &mut Criterion) {
    let mut group = c.benchmark_group("scc_acyclic_chain");

    for size in [10, 50, 100, 500, 1000, 5000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            // Build a linear chain graph: A -> B -> C -> ...
            let mut graph = DiGraph::<NodeData, EdgeData>::new();
            let mut nodes = Vec::with_capacity(size);

            // Create nodes
            for i in 0..size {
                let node = if i % 2 == 0 {
                    NodeData::File(create_test_file_node(i as u32, &format!("file_{}.rs", i)))
                } else {
                    NodeData::Symbol(SymbolNode {
                        id: SymbolId(i as u32),
                        name: format!("symbol_{}", i),
                        kind: SymbolKind::Function,
                        file_id: meta_ast::model::ids::FileId(0),
                        visibility: Some(Visibility::Public),
                        source_range: SourceRange {
                            byte_start: 0,
                            byte_end: 10,
                            start: LineColumn { line: 0, column: 0 },
                            end: LineColumn {
                                line: 0,
                                column: 10,
                            },
                        },
                    })
                };
                nodes.push(graph.add_node(node));
            }

            // Create chain edges
            for i in 0..(size - 1) {
                graph.add_edge(nodes[i], nodes[i + 1], EdgeData::new(EdgeKind::Reference));
            }

            b.iter(|| {
                let analysis = SccAnalysis::analyze(&graph);
                black_box(analysis.has_cycles());
            });
        });
    }

    group.finish();
}

/// Benchmark SCC analysis on cyclic graphs (single large cycle).
fn bench_scc_single_cycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("scc_single_cycle");

    for size in [10, 50, 100, 500, 1000, 5000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            // Build a single cycle: A -> B -> C -> ... -> A
            let mut graph = DiGraph::<NodeData, EdgeData>::new();
            let mut nodes = Vec::with_capacity(size);

            // Create symbol nodes
            for i in 0..size {
                let node = NodeData::Symbol(SymbolNode {
                    id: SymbolId(i as u32),
                    name: format!("symbol_{}", i),
                    kind: SymbolKind::Function,
                    file_id: meta_ast::model::ids::FileId(0),
                    visibility: Some(Visibility::Public),
                    source_range: SourceRange {
                        byte_start: 0,
                        byte_end: 10,
                        start: LineColumn { line: 0, column: 0 },
                        end: LineColumn {
                            line: 0,
                            column: 10,
                        },
                    },
                });
                nodes.push(graph.add_node(node));
            }

            // Create cycle edges
            for i in 0..size {
                let next = (i + 1) % size;
                graph.add_edge(nodes[i], nodes[next], EdgeData::new(EdgeKind::Reference));
            }

            b.iter(|| {
                let analysis = SccAnalysis::analyze(&graph);
                black_box(analysis.has_cycles());
            });
        });
    }

    group.finish();
}

/// Benchmark SCC analysis on multiple small cycles.
fn bench_scc_multiple_cycles(c: &mut Criterion) {
    let mut group = c.benchmark_group("scc_multiple_cycles");

    for cycle_count in [10, 50, 100, 500].iter() {
        let cycle_size = 3; // Small cycles of 3 nodes each
        let total_nodes = cycle_count * cycle_size;

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_cycles", cycle_count)),
            &total_nodes,
            |b, &_total| {
                let mut graph = DiGraph::<NodeData, EdgeData>::new();

                // Create multiple independent cycles
                for c in 0..*cycle_count {
                    let mut cycle_nodes = Vec::with_capacity(cycle_size);

                    // Create nodes for this cycle
                    for i in 0..cycle_size {
                        let global_id = (c * cycle_size + i) as u32;
                        let node = NodeData::Symbol(SymbolNode {
                            id: SymbolId(global_id),
                            name: format!("cycle{}_node{}", c, i),
                            kind: SymbolKind::Function,
                            file_id: meta_ast::model::ids::FileId(c as u32),
                            visibility: Some(Visibility::Public),
                            source_range: SourceRange {
                                byte_start: 0,
                                byte_end: 10,
                                start: LineColumn { line: 0, column: 0 },
                                end: LineColumn {
                                    line: 0,
                                    column: 10,
                                },
                            },
                        });
                        cycle_nodes.push(graph.add_node(node));
                    }

                    // Create cycle edges
                    for i in 0..cycle_size {
                        let next = (i + 1) % cycle_size;
                        graph.add_edge(
                            cycle_nodes[i],
                            cycle_nodes[next],
                            EdgeData::new(EdgeKind::Reference),
                        );
                    }
                }

                b.iter(|| {
                    let analysis = SccAnalysis::analyze(&graph);
                    black_box(analysis.cyclic_components().count());
                });
            },
        );
    }

    group.finish();
}

/// Benchmark SCC analysis on dense graphs (many edges).
fn bench_scc_dense_graph(c: &mut Criterion) {
    let mut group = c.benchmark_group("scc_dense_graph");

    for size in [10, 50, 100, 200].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut graph = DiGraph::<NodeData, EdgeData>::new();
            let mut nodes = Vec::with_capacity(size);

            // Create nodes
            for i in 0..size {
                let node = NodeData::Symbol(SymbolNode {
                    id: SymbolId(i as u32),
                    name: format!("node_{}", i),
                    kind: SymbolKind::Function,
                    file_id: meta_ast::model::ids::FileId(0),
                    visibility: Some(Visibility::Public),
                    source_range: SourceRange {
                        byte_start: 0,
                        byte_end: 10,
                        start: LineColumn { line: 0, column: 0 },
                        end: LineColumn {
                            line: 0,
                            column: 10,
                        },
                    },
                });
                nodes.push(graph.add_node(node));
            }

            // Create dense connections (each node connects to ~50% of others)
            for i in 0..size {
                for j in 0..size {
                    if i != j && (i + j) % 2 == 0 {
                        graph.add_edge(nodes[i], nodes[j], EdgeData::new(EdgeKind::Reference));
                    }
                }
            }

            b.iter(|| {
                let analysis = SccAnalysis::analyze(&graph);
                black_box(analysis.components.len());
            });
        });
    }

    group.finish();
}

/// Benchmark edge deduplication performance.
fn bench_edge_deduplication(c: &mut Criterion) {
    let mut group = c.benchmark_group("edge_deduplication");

    for duplicate_count in [10, 100, 1000, 10000].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_duplicates", duplicate_count)),
            duplicate_count,
            |b, &count| {
                b.iter(|| {
                    let mut builder = GraphBuilder::new(SnapshotId(1));

                    // Add two files
                    let file_a = builder.add_file(PathBuf::from("a.rs"), LangId::Rust);
                    let _file_b = builder.add_file(PathBuf::from("b.rs"), LangId::Rust);

                    // Add the same import edge multiple times
                    for _ in 0..count {
                        builder.add_import(file_a, PathBuf::from("b.rs"));
                    }

                    let graph = builder.build();
                    black_box(graph.edge_count());
                });
            },
        );
    }

    group.finish();
}

/// Benchmark node lookup performance.
fn bench_node_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("node_lookup");

    for size in [100, 1000, 10000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut builder = GraphBuilder::new(SnapshotId(1));
            let mut symbol_ids = Vec::with_capacity(size);

            // Setup: create files and symbols
            let _file_id = builder.add_file(PathBuf::from("main.rs"), LangId::Rust);

            for i in 0..size {
                let symbol = create_test_symbol(i as u32, &format!("func_{}", i), "main.rs");
                builder.add_symbol(&symbol);
                symbol_ids.push(SymbolId(i as u32));
            }

            let graph = builder.build();

            b.iter(|| {
                // Random access pattern
                let mut found = 0;
                for i in (0..size).step_by(7) {
                    if let Some(_node) = graph.symbol_node(SymbolId(i as u32)) {
                        found += 1;
                    }
                }
                black_box(found);
            });
        });
    }

    group.finish();
}

/// Benchmark complete pipeline: extraction to SCC analysis.
fn bench_full_pipeline_small(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_pipeline");
    group.sample_size(50);

    // Benchmark with fixture files if available
    let files =
        meta_ast::input::discover_files(std::path::Path::new("tests/fixtures/python"), None)
            .unwrap_or_default();

    if !files.is_empty() {
        group.bench_function("python_extraction_to_scc", |b| {
            b.iter(|| {
                // Build graph
                let mut builder = GraphBuilder::new(SnapshotId(1));

                // Add files
                for (path, lang) in &files {
                    builder.add_file(path.clone(), *lang);
                }

                // Extract symbols
                let result = meta_ast::extractor::extract(black_box(&files));

                // Add symbols - must use black_box to prevent optimization
                for symbol in black_box(
                    &result
                        .files
                        .iter()
                        .flat_map(|f| &f.symbols)
                        .collect::<Vec<_>>(),
                ) {
                    builder.add_symbol(symbol);
                }

                // Build and analyze
                let graph = builder.build();
                let scc = SccAnalysis::analyze(&graph.graph);

                black_box((graph.node_count(), scc.has_cycles()));
            });
        });
    }

    group.finish();
}

/// Benchmark ownership graph construction only.
fn bench_ownership_graph_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("ownership_graph_only");

    for size in [10, 100, 1000, 5000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let mut builder = GraphBuilder::new(SnapshotId(1));

                // Create files
                for i in 0..10 {
                    let path = format!("src/module_{}.rs", i);
                    builder.add_file(PathBuf::from(&path), LangId::Rust);
                }

                // Create many symbols per file
                for i in 0..size {
                    let file_idx = i % 10;
                    let symbol = create_test_symbol(
                        i as u32,
                        &format!("func_{}", i),
                        &format!("src/module_{}.rs", file_idx),
                    );
                    builder.add_symbol(&symbol);
                }

                let graph = builder.build();
                black_box(graph.edge_count());
            });
        });
    }

    group.finish();
}

criterion_group!(
    graph_benches,
    bench_graph_construction_linear,
    bench_scc_acyclic_chain,
    bench_scc_single_cycle,
    bench_scc_multiple_cycles,
    bench_scc_dense_graph,
    bench_edge_deduplication,
    bench_node_lookup,
    bench_full_pipeline_small,
    bench_ownership_graph_only,
);
criterion_main!(graph_benches);
