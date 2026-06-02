use crate::model::Symbol;
use crate::output::OutputFormat;
use crate::pipeline::GraphAnalysis;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct EmitConfig {
    pub output: Option<PathBuf>,
    pub format: OutputFormat,
    pub html: bool,
    pub open_browser: bool,
    pub self_contained: bool,
}

/// Serialize and emit the symbol inspection results.
///
/// If `config.output` is `Some(path)`, writes the serialized contents to that file.
/// Otherwise, prints the contents directly to stdout.
pub fn emit_inspect(symbols: &[Symbol], config: &EmitConfig) -> anyhow::Result<()> {
    let content = crate::output::inspect::serialize_inspect(symbols, &config.format)?;
    match &config.output {
        Some(path) => {
            std::fs::write(path, content)?;
        }
        None => {
            println!("{content}");
        }
    }
    Ok(())
}

/// Serialize and emit the dependency graph analysis results.
///
/// If `config.html` is true, generates an interactive HTML dashboard and writes it
/// to `config.output` (defaulting to "project.metast"). If `config.open_browser` is true,
/// opens the HTML file in the default browser.
///
/// Otherwise, serializes the graph into the requested text format (JSON/YAML) and
/// writes to `config.output` or prints to stdout.
pub fn emit_graph(analysis: &GraphAnalysis, config: &EmitConfig) -> anyhow::Result<()> {
    if config.html {
        let html = crate::output::dashboard::to_graph_html(
            &analysis.graph,
            &analysis.scc,
            analysis.snapshot_id.0 as u64,
            config.self_contained,
        )?;
        let path = config
            .output
            .clone()
            .unwrap_or_else(|| PathBuf::from("project.metast"));
        let path_str = path.to_string_lossy().to_string();
        std::fs::write(&path, html)?;
        if config.open_browser {
            let open_res = webbrowser::open(&path_str);
            if let Err(e) = open_res {
                tracing::warn!(error = %e, "could not open browser");
            }
        }
    } else {
        let content = crate::output::graph::serialize_graph(
            &analysis.graph,
            &analysis.scc,
            analysis.snapshot_id.0 as u64,
            &config.format,
        )?;
        match &config.output {
            Some(path) => {
                std::fs::write(path, content)?;
            }
            None => {
                println!("{content}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{GraphBuilder, SccAnalysis};
    use crate::language::LangId;
    use crate::model::{LineColumn, SnapshotId, SourceRange, Symbol, SymbolId, SymbolKind};
    use crate::output::OutputFormat;

    #[test]
    fn emit_inspect_writes_to_path() {
        let temp_dir = std::env::temp_dir().join("emit_inspect_writes_to_path");
        if temp_dir.exists() {
            let _ = std::fs::remove_dir_all(&temp_dir);
        }
        std::fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("output.json");
        let symbols = vec![Symbol {
            id: SymbolId(1),
            name: "test".into(),
            kind: SymbolKind::Function,
            language: LangId::Python,
            file_path: PathBuf::from("a.py"),
            source_range: SourceRange {
                byte_start: 0,
                byte_end: 10,
                start: LineColumn { line: 1, column: 0 },
                end: LineColumn {
                    line: 1,
                    column: 10,
                },
            },
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        }];
        let config = EmitConfig {
            output: Some(file_path.clone()),
            format: OutputFormat::Json,
            html: false,
            open_browser: false,
            self_contained: false,
        };
        emit_inspect(&symbols, &config).unwrap();
        assert!(file_path.exists());
        let content = std::fs::read_to_string(file_path).unwrap();
        assert!(content.contains("test"));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn emit_inspect_prints_to_stdout_when_no_path() {
        let symbols = vec![Symbol {
            id: SymbolId(1),
            name: "test".into(),
            kind: SymbolKind::Function,
            language: LangId::Python,
            file_path: PathBuf::from("a.py"),
            source_range: SourceRange {
                byte_start: 0,
                byte_end: 10,
                start: LineColumn { line: 1, column: 0 },
                end: LineColumn {
                    line: 1,
                    column: 10,
                },
            },
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        }];
        let config = EmitConfig {
            output: None,
            format: OutputFormat::Json,
            html: false,
            open_browser: false,
            self_contained: false,
        };
        emit_inspect(&symbols, &config).unwrap();
    }

    #[test]
    fn emit_graph_html_writes_file() {
        let temp_dir = std::env::temp_dir().join("emit_graph_html_writes_file");
        if temp_dir.exists() {
            let _ = std::fs::remove_dir_all(&temp_dir);
        }
        std::fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("graph.html");

        let builder = GraphBuilder::new(SnapshotId(1));
        let graph = builder.build();
        let scc = SccAnalysis::analyze(&graph.graph);
        let analysis = GraphAnalysis {
            graph,
            scc,
            snapshot_id: SnapshotId(1),
        };

        let config = EmitConfig {
            output: Some(file_path.clone()),
            format: OutputFormat::Json,
            html: true,
            open_browser: false,
            self_contained: false,
        };
        emit_graph(&analysis, &config).unwrap();
        assert!(file_path.exists());
        let content = std::fs::read_to_string(file_path).unwrap();
        assert!(content.contains("<!DOCTYPE html>"));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn emit_graph_text_json_output() {
        let temp_dir = std::env::temp_dir().join("emit_graph_text_json_output");
        if temp_dir.exists() {
            let _ = std::fs::remove_dir_all(&temp_dir);
        }
        std::fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("graph.json");

        let builder = GraphBuilder::new(SnapshotId(1));
        let graph = builder.build();
        let scc = SccAnalysis::analyze(&graph.graph);
        let analysis = GraphAnalysis {
            graph,
            scc,
            snapshot_id: SnapshotId(1),
        };

        let config = EmitConfig {
            output: Some(file_path.clone()),
            format: OutputFormat::Json,
            html: false,
            open_browser: false,
            self_contained: false,
        };
        emit_graph(&analysis, &config).unwrap();
        assert!(file_path.exists());
        let content = std::fs::read_to_string(file_path).unwrap();
        assert!(content.contains("nodes"));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
