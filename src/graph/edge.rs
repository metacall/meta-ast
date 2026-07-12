//! Edge types for the dependency graph.
//!
//! Defines the semantic relationships between nodes in the code graph.
//! Edges are directed and carry metadata about the relationship type
//! and confidence level.
//!
//! ## Design: uni-directional edges
//!
//! All edges are directed. There is no bidirectional or monitor/link
//! pattern - an edge from A to B means A depends on B, not that B will
//! be notified of A's failure. Shared-fate semantics (failure propagation)
//! are a separate, optional annotation that the deploy layer may add
//! during cut-edge RPC conversion. This follows the principle from
//! *A Unified Semantics for Future Erlang* §2.2/§6.2: bidirectional links
//! are replaced by uni-directional links plus monitors, and supervision
//! trees can be built from uni-directional links alone.

/// Semantic kind of a directed edge in the code graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[non_exhaustive]
pub enum EdgeKind {
    /// Ownership edge: File owns/contains a symbol.
    Ownership,

    /// Import edge: File imports/depends on another file.
    Import,

    /// Reference edge: Symbol references/uses another symbol.
    Reference,
}

impl EdgeKind {
    /// Returns true if this edge kind participates in SCC computation.
    pub fn participates_in_scc(self) -> bool {
        matches!(self, EdgeKind::Import | EdgeKind::Reference)
    }

    /// Returns true if this edge represents a cross-file dependency.
    pub fn is_cross_file(self) -> bool {
        matches!(self, EdgeKind::Import)
    }

    /// Returns the human-readable name of this edge kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Ownership => "ownership",
            EdgeKind::Import => "import",
            EdgeKind::Reference => "reference",
        }
    }
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Data stored for each edge in the graph.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct EdgeData {
    /// Semantic kind of the relationship.
    pub kind: EdgeKind,

    /// Confidence level for the edge resolution.
    /// Used for cross-language and best-effort resolution.
    pub confidence: f32,
}

impl EdgeData {
    /// Creates a new edge with full confidence (1.0).
    pub fn new(kind: EdgeKind) -> Self {
        Self {
            kind,
            confidence: 1.0,
        }
    }

    /// Creates a new edge with specified confidence.
    pub fn with_confidence(kind: EdgeKind, confidence: f32) -> Self {
        Self {
            kind,
            confidence: confidence.clamp(0.0, 1.0),
        }
    }
    pub fn participates_in_scc(&self) -> bool {
        self.kind.participates_in_scc()
    }
}

impl Default for EdgeData {
    fn default() -> Self {
        Self::new(EdgeKind::Reference)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_kind_scc_participation() {
        assert!(!EdgeKind::Ownership.participates_in_scc());
        assert!(EdgeKind::Import.participates_in_scc());
        assert!(EdgeKind::Reference.participates_in_scc());
    }

    #[test]
    fn edge_kind_as_str() {
        assert_eq!(EdgeKind::Ownership.as_str(), "ownership");
        assert_eq!(EdgeKind::Import.as_str(), "import");
        assert_eq!(EdgeKind::Reference.as_str(), "reference");
    }

    #[test]
    fn edge_kind_display() {
        assert_eq!(format!("{}", EdgeKind::Import), "import");
    }

    #[test]
    fn edge_data_new_defaults_to_full_confidence() {
        let edge = EdgeData::new(EdgeKind::Import);
        assert_eq!(edge.kind, EdgeKind::Import);
        assert_eq!(edge.confidence, 1.0);
        assert!(edge.participates_in_scc());
    }

    #[test]
    fn edge_data_with_confidence_clamps() {
        let low = EdgeData::with_confidence(EdgeKind::Reference, -0.5);
        assert_eq!(low.confidence, 0.0);

        let high = EdgeData::with_confidence(EdgeKind::Reference, 1.5);
        assert_eq!(high.confidence, 1.0);

        let mid = EdgeData::with_confidence(EdgeKind::Reference, 0.75);
        assert_eq!(mid.confidence, 0.75);
    }

    #[test]
    fn edge_data_default() {
        let edge: EdgeData = Default::default();
        assert_eq!(edge.confidence, 1.0);
        assert!(edge.participates_in_scc());
    }
}
