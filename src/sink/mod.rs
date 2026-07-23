//! Sink adapters for exporting the graph to external systems
//! (files, databases, streaming protocols). Feature-gated behind `dataflow`.

use crate::output::graph::GraphOutput;

/// Trait for pluggable graph export sinks.
pub trait GraphSink {
    /// Emit a `GraphOutput` to the sink target.
    fn emit(&self, export: &GraphOutput) -> anyhow::Result<()>;
}

/// Sink that writes the graph as JSON to a file or stdout.
pub struct JsonSink {
    path: Option<std::path::PathBuf>,
}

impl JsonSink {
    pub fn new(path: Option<std::path::PathBuf>) -> Self {
        Self { path }
    }
}

impl GraphSink for JsonSink {
    fn emit(&self, export: &GraphOutput) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(export)?;
        match &self.path {
            Some(p) => std::fs::write(p, json)?,
            None => println!("{json}"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::GraphBuilder;
    use crate::model::SnapshotId;

    #[test]
    fn json_sink_writes_to_file() {
        let temp = std::env::temp_dir().join("meta_ast_sink_test.json");
        let builder = GraphBuilder::new(SnapshotId(1));
        let graph = builder.build();
        let export = GraphOutput::from_graph(&graph, None, 1);

        let sink = JsonSink::new(Some(temp.clone()));
        sink.emit(&export).unwrap();

        let content = std::fs::read_to_string(&temp).unwrap();
        assert!(content.contains("\"schema_version\""));
        let _ = std::fs::remove_file(&temp);
    }

    #[test]
    fn json_sink_prints_to_stdout() {
        let builder = GraphBuilder::new(SnapshotId(1));
        let graph = builder.build();
        let export = GraphOutput::from_graph(&graph, None, 1);

        let sink = JsonSink::new(None);
        sink.emit(&export).unwrap();
    }
}
