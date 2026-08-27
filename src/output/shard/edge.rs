//! Shard edge serialization, validation, and restoration.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::graph::{CodeGraph, EdgeKind, NodeData};
use crate::output::shard::error::ShardError;
use crate::output::shard::name::stable_node_name;

/// Serialized cross-node edge in a shard file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShardEdge {
    pub source_name: String,
    pub target_name: String,
    pub kind: ShardEdgeKind,
    pub confidence: f32,
    pub flow_kind: Option<ShardFlowKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShardEdgeKind {
    Ownership,
    Import,
    Reference,
    Flow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShardFlowKind {
    DefUse,
    Argument,
    Return,
    FieldAccess,
}

impl From<EdgeKind> for ShardEdgeKind {
    fn from(kind: EdgeKind) -> Self {
        match kind {
            EdgeKind::Ownership => Self::Ownership,
            EdgeKind::Import => Self::Import,
            EdgeKind::Reference => Self::Reference,
            EdgeKind::Flow => Self::Flow,
        }
    }
}

impl From<ShardEdgeKind> for EdgeKind {
    fn from(kind: ShardEdgeKind) -> Self {
        match kind {
            ShardEdgeKind::Ownership => Self::Ownership,
            ShardEdgeKind::Import => Self::Import,
            ShardEdgeKind::Reference => Self::Reference,
            ShardEdgeKind::Flow => Self::Flow,
        }
    }
}

impl From<crate::model::FlowKind> for ShardFlowKind {
    fn from(kind: crate::model::FlowKind) -> Self {
        match kind {
            crate::model::FlowKind::DefUse => Self::DefUse,
            crate::model::FlowKind::Argument => Self::Argument,
            crate::model::FlowKind::Return => Self::Return,
            crate::model::FlowKind::FieldAccess => Self::FieldAccess,
        }
    }
}

impl From<ShardFlowKind> for crate::model::FlowKind {
    fn from(kind: ShardFlowKind) -> Self {
        match kind {
            ShardFlowKind::DefUse => Self::DefUse,
            ShardFlowKind::Argument => Self::Argument,
            ShardFlowKind::Return => Self::Return,
            ShardFlowKind::FieldAccess => Self::FieldAccess,
        }
    }
}

pub(crate) fn validate_edge(
    edge: &ShardEdge,
    line: usize,
    edge_index: usize,
) -> Result<(), ShardError> {
    let valid_confidence = edge.confidence.is_finite() && (0.0..=1.0).contains(&edge.confidence);
    if !valid_confidence {
        return Err(ShardError::InvalidEdge {
            line,
            edge_index,
            message: "confidence must be finite and in the range 0.0..=1.0".to_string(),
        });
    }
    if edge.kind == ShardEdgeKind::Flow {
        return Err(ShardError::InvalidEdge {
            line,
            edge_index,
            message: "schema version 2 does not persist dataflow nodes".to_string(),
        });
    }
    if edge.flow_kind.is_some() {
        return Err(ShardError::InvalidEdge {
            line,
            edge_index,
            message: "non-flow edges forbid flow_kind".to_string(),
        });
    }
    Ok(())
}

/// Restore persisted edges after `GraphBuilder::from_extractions` regenerates graph nodes.
pub fn restore_shard_edges(graph: &mut CodeGraph, edges: &[ShardEdge]) -> Result<(), ShardError> {
    let endpoint_index = graph
        .graph()
        .node_indices()
        .filter(|index| !matches!(graph.graph()[*index], NodeData::Data(_)))
        .map(|index| stable_node_name(graph, index).map(|name| (name, index)))
        .collect::<Result<HashMap<_, _>, ShardError>>()?;

    for edge in edges {
        validate_edge(edge, 0, 0)?;
        let source = endpoint_index
            .get(&edge.source_name)
            .copied()
            .ok_or_else(|| ShardError::MissingEndpoint {
                name: edge.source_name.clone(),
            })?;
        let target = endpoint_index
            .get(&edge.target_name)
            .copied()
            .ok_or_else(|| ShardError::MissingEndpoint {
                name: edge.target_name.clone(),
            })?;
        graph.add_edge_normalized_with_flow(
            source,
            target,
            edge.kind.into(),
            edge.confidence,
            edge.flow_kind.map(Into::into),
        );
    }
    Ok(())
}
