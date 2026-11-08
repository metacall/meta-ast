//! Property tests for the graph edge normalization contract.
//!
//! These tests intentionally exercise the public `CodeGraph` API so that the
//! invariants stay covered at the same boundary used by graph writers.

use std::collections::HashMap;

use meta_ast::LangId;
use meta_ast::graph::node::ExternalNode;
use meta_ast::graph::{CodeGraph, DeployabilityHint, EdgeKind, NodeData, SccAnalysis};
use meta_ast::model::{FlowKind, SnapshotId};
use petgraph::graph::NodeIndex;
use proptest::prelude::*;

fn graph_with_nodes() -> (CodeGraph, [NodeIndex; 4]) {
    let mut graph = CodeGraph::new(SnapshotId::new(1).expect("1 is a valid snapshot id"));
    let nodes = [
        graph.add_node(NodeData::External(ExternalNode {
            raw_path: "node-0".to_string(),
            language: LangId::Rust,
            classification: None,
        })),
        graph.add_node(NodeData::External(ExternalNode {
            raw_path: "node-1".to_string(),
            language: LangId::Rust,
            classification: None,
        })),
        graph.add_node(NodeData::External(ExternalNode {
            raw_path: "node-2".to_string(),
            language: LangId::Rust,
            classification: None,
        })),
        graph.add_node(NodeData::External(ExternalNode {
            raw_path: "node-3".to_string(),
            language: LangId::Rust,
            classification: None,
        })),
    ];
    (graph, nodes)
}

fn edge_kind(tag: u8) -> EdgeKind {
    match tag % 4 {
        0 => EdgeKind::Ownership,
        1 => EdgeKind::Import,
        2 => EdgeKind::Reference,
        _ => EdgeKind::Flow,
    }
}

fn flow_kind(tag: u8) -> FlowKind {
    match tag % 4 {
        0 => FlowKind::DefUse,
        1 => FlowKind::Argument,
        2 => FlowKind::Return,
        _ => FlowKind::FieldAccess,
    }
}

fn confidence() -> impl Strategy<Value = f32> {
    prop_oneof![
        -2.0f32..2.0f32,
        Just(f32::NAN),
        Just(f32::INFINITY),
        Just(f32::NEG_INFINITY),
    ]
}

fn normalized(confidence: f32) -> f32 {
    if confidence.is_finite() {
        confidence.clamp(0.0, 1.0)
    } else {
        confidence
    }
}

fn finite_max(first: f32, second: f32) -> f32 {
    match (first.is_finite(), second.is_finite()) {
        (true, true) => first.max(second),
        (true, false) => first,
        (false, true) => second,
        (false, false) => first,
    }
}

fn edge_confidence(graph: &CodeGraph, source: NodeIndex, target: NodeIndex, kind: EdgeKind) -> f32 {
    graph
        .graph()
        .edges_connecting(source, target)
        .find(|edge| edge.weight().kind == kind)
        .expect("every generated triple has one normalized edge")
        .weight()
        .confidence
}

proptest! {
    #[test]
    fn duplicate_edges_keep_the_strongest_finite_confidence(
        insertions in prop::collection::vec(
            (0u8..4, 0u8..4, 0u8..4, confidence()),
            1..64,
        )
    ) {
        let (mut graph, nodes) = graph_with_nodes();
        let mut expected: HashMap<(u8, u8, EdgeKind), f32> = HashMap::new();

        for (source, target, kind_tag, confidence) in insertions {
            let kind = edge_kind(kind_tag);
            let key = (source, target, kind);
            let normalized_confidence = normalized(confidence);
            graph.add_edge_normalized(nodes[source as usize], nodes[target as usize], kind, confidence);
            expected
                .entry(key)
                .and_modify(|existing| *existing = finite_max(*existing, normalized_confidence))
                .or_insert(normalized_confidence);
        }

        prop_assert_eq!(graph.edge_count(), expected.len());
        for ((source, target, kind), expected_confidence) in expected {
            let actual = edge_confidence(&graph, nodes[source as usize], nodes[target as usize], kind);
            if expected_confidence.is_finite() {
                prop_assert_eq!(actual, expected_confidence);
            } else {
                prop_assert!(!actual.is_finite());
            }
        }
    }

    #[test]
    fn first_non_none_flow_kind_is_stable(
        insertions in prop::collection::vec(
            (0u8..4, 0u8..4, confidence(), prop::option::of(0u8..4)),
            1..64,
        )
    ) {
        let (mut graph, nodes) = graph_with_nodes();
        let mut expected: HashMap<(u8, u8), Option<FlowKind>> = HashMap::new();

        for (source, target, confidence, flow_tag) in insertions {
            let flow_kind = flow_tag.map(flow_kind);
            graph.add_edge_normalized_with_flow(
                nodes[source as usize],
                nodes[target as usize],
                EdgeKind::Flow,
                confidence,
                flow_kind,
            );
            expected
                .entry((source, target))
                .and_modify(|first| {
                    if first.is_none() {
                        *first = flow_kind;
                    }
                })
                .or_insert(flow_kind);
        }

        prop_assert_eq!(graph.edge_count(), expected.len());
        for ((source, target), expected_flow_kind) in expected {
            let actual = graph
                .graph()
                .edges_connecting(nodes[source as usize], nodes[target as usize])
                .find(|edge| edge.weight().kind == EdgeKind::Flow)
                .expect("every generated flow triple has one edge")
                .weight()
                .flow_kind;
            prop_assert_eq!(actual, expected_flow_kind);
        }
    }

    #[test]
    fn self_loops_are_kept_and_only_dependency_kinds_are_cyclic(
        node in 0u8..4,
        kind_tag in 0u8..4,
        confidence in confidence(),
    ) {
        let (mut graph, nodes) = graph_with_nodes();
        let kind = edge_kind(kind_tag);
        let index = nodes[node as usize];
        graph.add_edge_normalized(index, index, kind, confidence);

        prop_assert_eq!(graph.edge_count(), 1);
        let analysis = SccAnalysis::analyze(graph.graph());
        let component_index = analysis.node_to_component[&index];
        let component = &analysis.components[component_index];
        let dependency_kind = matches!(kind, EdgeKind::Import | EdgeKind::Reference);
        prop_assert_eq!(component.is_cyclic, dependency_kind);
        prop_assert_eq!(component.hint, if dependency_kind {
            DeployabilityHint::SelfLoop
        } else {
            DeployabilityHint::Independent
        });
    }
}
