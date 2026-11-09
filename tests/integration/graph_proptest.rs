//! Property tests for edge normalization (graph-model.md §4, invariant 4).
//!
//! Every repeated `(source, target, kind)` triple collapses into one edge. The
//! strongest finite confidence wins, the first flow kind wins, and the edge
//! count never exceeds the number of distinct triples. Self-loops are kept,
//! because SCC analysis (§5) reads them as cyclic units.
//!
//! The expected values are computed from the rule as the spec states it, not
//! from a copy of `merge_repeated`, so a defect in the merge cannot hide in the
//! oracle.

use std::collections::HashMap;
use std::path::PathBuf;

use meta_ast::LangId;
use meta_ast::graph::{
    CodeGraph, DeployabilityHint, EdgeKind, FileNode, GraphBuilder, NodeData, SccAnalysis,
};
use meta_ast::model::{
    FileId, FlowKind, LineColumn, SnapshotId, SourceRange, Symbol, SymbolId, SymbolKind,
};
use petgraph::graph::NodeIndex;
use proptest::prelude::*;

const MAX_NODES: usize = 6;

/// One insertion: node slots, kind, raw confidence, flow kind.
type Op = (usize, usize, EdgeKind, f32, Option<FlowKind>);
type Key = (NodeIndex, NodeIndex, EdgeKind);

fn edge_kind() -> impl Strategy<Value = EdgeKind> {
    prop_oneof![
        Just(EdgeKind::Ownership),
        Just(EdgeKind::Import),
        Just(EdgeKind::Reference),
        Just(EdgeKind::Flow),
    ]
}

fn flow_kind() -> impl Strategy<Value = Option<FlowKind>> {
    prop_oneof![
        2 => Just(None),
        1 => Just(Some(FlowKind::DefUse)),
        1 => Just(Some(FlowKind::Argument)),
        1 => Just(Some(FlowKind::Return)),
        1 => Just(Some(FlowKind::FieldAccess)),
    ]
}

/// Confidence in `[0, 1]`, biased toward the ladder constants so exact ties occur.
fn unit_confidence() -> impl Strategy<Value = f32> {
    prop_oneof![
        3 => 0.0f32..=1.0f32,
        2 => prop::sample::select(vec![1.0f32, 0.9, 0.8, 0.6, 0.5, 0.4, 0.0]),
    ]
}

/// Adds finite out-of-range values (clamped) and NaN / +inf / -inf, which
/// carry no signal (maintainer ruling on #60, 2026-11-08).
fn any_confidence() -> impl Strategy<Value = f32> {
    prop_oneof![
        8 => unit_confidence(),
        1 => -10.0f32..10.0f32,
        1 => prop::sample::select(vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY]),
    ]
}

/// Few node slots, so duplicate triples and self-loops are common.
fn ops(confidence: impl Strategy<Value = f32>) -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(
        (
            0..MAX_NODES,
            0..MAX_NODES,
            edge_kind(),
            confidence,
            flow_kind(),
        ),
        0..64,
    )
}

fn file_graph() -> (CodeGraph, Vec<NodeIndex>) {
    let snapshot = SnapshotId::new(1).unwrap();
    let mut graph = CodeGraph::new(snapshot);
    let nodes = (1..=MAX_NODES as u32)
        .map(|id| {
            graph.add_node(NodeData::File(FileNode {
                id: FileId::new(id).unwrap(),
                path: PathBuf::from(format!("f{id}.rs")),
                language: LangId::Rust,
                snapshot_id: snapshot,
            }))
        })
        .collect();
    (graph, nodes)
}

fn apply(graph: &mut CodeGraph, nodes: &[NodeIndex], ops: &[Op]) {
    for &(s, t, kind, confidence, flow) in ops {
        graph.add_edge_normalized_with_flow(nodes[s], nodes[t], kind, confidence, flow);
    }
}

/// Every edge as `key -> (confidence, flow_kind)`; panics on a duplicate key.
fn edge_map(graph: &CodeGraph) -> HashMap<Key, (f32, Option<FlowKind>)> {
    let g = graph.graph();
    let mut map = HashMap::new();
    for edge in g.edge_indices() {
        let (s, t) = g.edge_endpoints(edge).unwrap();
        let w = g[edge];
        let previous = map.insert((s, t, w.kind), (w.confidence, w.flow_kind));
        assert!(
            previous.is_none(),
            "duplicate edge for {:?}",
            (s, t, w.kind)
        );
    }
    map
}

/// The rule as graph-model.md states it, per distinct triple.
fn expected(nodes: &[NodeIndex], ops: &[Op]) -> HashMap<Key, (f32, Option<FlowKind>)> {
    let mut inputs: HashMap<Key, (Vec<f32>, Option<FlowKind>)> = HashMap::new();
    for &(s, t, kind, confidence, flow) in ops {
        let entry = inputs.entry((nodes[s], nodes[t], kind)).or_default();
        entry.0.push(confidence);
        entry.1 = entry.1.or(flow);
    }
    inputs
        .into_iter()
        .map(|(key, (values, flow))| {
            let strongest = values
                .iter()
                .copied()
                .filter(|c| c.is_finite())
                .map(|c| c.clamp(0.0, 1.0))
                .reduce(f32::max)
                // No finite input: the edge stays non-finite for the output layer.
                .unwrap_or(f32::NAN);
            (key, (strongest, flow))
        })
        .collect()
}

/// Equal, or both non-finite (an all-non-finite edge keeps its first input,
/// so which non-finite value survives depends on order).
fn same_confidence(a: f32, b: f32) -> bool {
    (!a.is_finite() && !b.is_finite()) || a == b
}

proptest! {
    /// Invariant 1: confidence of a triple equals max(c_1, ..., c_k) exactly.
    #[test]
    fn duplicate_triples_keep_the_max_confidence(ops in ops(unit_confidence())) {
        let (mut graph, nodes) = file_graph();
        apply(&mut graph, &nodes, &ops);
        let actual = edge_map(&graph);
        for (key, (confidence, _)) in expected(&nodes, &ops) {
            prop_assert_eq!(actual[&key].0, confidence, "triple {:?}", key);
        }
    }

    /// Invariant 2: the first non-None flow kind survives later duplicates.
    #[test]
    fn first_flow_kind_is_stable(ops in ops(unit_confidence())) {
        let (mut graph, nodes) = file_graph();
        apply(&mut graph, &nodes, &ops);
        let actual = edge_map(&graph);
        for (key, (_, flow)) in expected(&nodes, &ops) {
            prop_assert_eq!(actual[&key].1, flow, "triple {:?}", key);
        }
    }

    /// Invariant 3: one edge per distinct triple, and never more.
    #[test]
    fn edge_count_equals_distinct_triples(ops in ops(any_confidence())) {
        let (mut graph, nodes) = file_graph();
        apply(&mut graph, &nodes, &ops);
        let distinct = expected(&nodes, &ops).len();
        prop_assert!(graph.edge_count() <= ops.len());
        prop_assert_eq!(graph.edge_count(), distinct);
        prop_assert_eq!(edge_map(&graph).len(), distinct);
    }

    /// Replaying the same ops is a no-op.
    #[test]
    fn replay_is_idempotent(ops in ops(any_confidence())) {
        let (mut graph, nodes) = file_graph();
        apply(&mut graph, &nodes, &ops);
        let once = edge_map(&graph);
        apply(&mut graph, &nodes, &ops);
        let twice = edge_map(&graph);
        prop_assert_eq!(once.len(), twice.len());
        for (key, (c, f)) in &once {
            let (c2, f2) = twice[key];
            prop_assert!(same_confidence(*c, c2));
            prop_assert_eq!(*f, f2);
        }
    }

    /// Clamping and non-finite input: the result is the max of the finite clamped inputs,
    /// stays in [0, 1], and does not depend on arrival order.
    #[test]
    fn confidence_is_clamped_and_order_independent(
        (ops, shuffled) in ops(any_confidence())
            .prop_flat_map(|ops| (Just(ops.clone()), Just(ops).prop_shuffle()))
    ) {
        let (mut forward, nodes) = file_graph();
        apply(&mut forward, &nodes, &ops);
        let (mut reordered, _) = file_graph();
        apply(&mut reordered, &nodes, &shuffled);

        let forward = edge_map(&forward);
        let reordered = edge_map(&reordered);
        for (key, (confidence, _)) in expected(&nodes, &ops) {
            let a = forward[&key].0;
            prop_assert!(same_confidence(a, confidence), "{:?}: {} vs {}", key, a, confidence);
            prop_assert!(same_confidence(a, reordered[&key].0), "order changed {:?}", key);
            prop_assert!(!a.is_finite() || (0.0..=1.0).contains(&a));
        }
    }

    /// A merge never lowers an edge's confidence.
    #[test]
    fn confidence_never_decreases(ops in ops(any_confidence())) {
        let (mut graph, nodes) = file_graph();
        let mut seen: HashMap<Key, f32> = HashMap::new();
        for op in &ops {
            apply(&mut graph, &nodes, std::slice::from_ref(op));
            let key = (nodes[op.0], nodes[op.1], op.2);
            let now = edge_map(&graph)[&key].0;
            if let Some(&before) = seen.get(&key) {
                prop_assert!(!before.is_finite() || now >= before, "{:?}: {} -> {}", key, before, now);
            }
            seen.insert(key, now);
        }
    }

    /// Invariant 4: self-loops are kept (one per kind, deduplicated), and SCC
    /// analysis marks a node `SelfLoop` exactly when it has an Import or
    /// Reference self-loop. Ownership and Flow self-loops stay acyclic.
    #[test]
    fn self_loops_are_deduplicated_and_classified(
        loops in prop::collection::vec(
            (0..MAX_NODES, edge_kind(), unit_confidence(), flow_kind()), 0..32)
    ) {
        let (mut graph, nodes) = file_graph();
        let ops: Vec<Op> = loops.iter().map(|&(n, k, c, f)| (n, n, k, c, f)).collect();
        apply(&mut graph, &nodes, &ops);

        let edges = edge_map(&graph);
        prop_assert_eq!(edges.len(), expected(&nodes, &ops).len());
        prop_assert!(edges.keys().all(|(s, t, _)| s == t));

        let analysis = SccAnalysis::analyze(graph.graph());
        for &node in &nodes {
            let cyclic = edges.keys().any(|&(s, _, k)| {
                s == node && matches!(k, EdgeKind::Import | EdgeKind::Reference)
            });
            let component = &analysis.components[analysis.component_of(node).unwrap()];
            prop_assert_eq!(component.is_cyclic, cyclic);
            let hint = if cyclic { DeployabilityHint::SelfLoop } else { DeployabilityHint::Independent };
            prop_assert_eq!(component.hint, hint);
        }
    }

    /// The builder and the post-build path share one merge rule.
    #[test]
    fn builder_and_code_graph_agree(
        refs in prop::collection::vec((0..MAX_NODES, 0..MAX_NODES, any_confidence()), 0..64)
    ) {
        let snapshot = SnapshotId::new(1).unwrap();
        let mut builder = GraphBuilder::new(snapshot);
        builder.add_file(PathBuf::from("a.rs"), LangId::Rust);
        let ids: Vec<SymbolId> = (1..=MAX_NODES as u32).map(|i| SymbolId::new(i).unwrap()).collect();
        for (i, &id) in ids.iter().enumerate() {
            builder.add_symbol(&symbol(id, &format!("s{i}"))).unwrap();
        }
        for &(s, t, c) in &refs {
            builder.add_reference(ids[s], ids[t], c);
        }
        let built = builder.build();

        let mut injected = built.clone();
        let idx: Vec<NodeIndex> = ids.iter().map(|&id| injected.symbol_node_index(id).unwrap()).collect();
        // Strip nothing: replaying the refs through the CodeGraph path must not change anything.
        for &(s, t, c) in &refs {
            injected.add_edge_normalized(idx[s], idx[t], EdgeKind::Reference, c);
        }
        let a = edge_map(&built);
        let b = edge_map(&injected);
        prop_assert_eq!(a.len(), b.len());

        let ops: Vec<Op> = refs.iter().map(|&(s, t, c)| (s, t, EdgeKind::Reference, c, None)).collect();
        for (key, (confidence, _)) in expected(&idx, &ops) {
            prop_assert!(same_confidence(a[&key].0, confidence), "builder {:?}", key);
            prop_assert!(same_confidence(b[&key].0, confidence), "code graph {:?}", key);
        }
    }
}

fn symbol(id: SymbolId, name: &str) -> Symbol {
    let range = SourceRange {
        byte_start: 0,
        byte_end: 1,
        start: LineColumn { line: 1, column: 0 },
        end: LineColumn { line: 1, column: 1 },
    };
    Symbol {
        id,
        name: name.into(),
        kind: SymbolKind::Function,
        language: LangId::Rust,
        file_path: PathBuf::from("a.rs"),
        source_range: range,
        name_range: None,
        visibility: None,
        signature: None,
        docstring: None,
        is_async: false,
    }
}
