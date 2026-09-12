//! Output defaults: the dashboard asset, the browser opt-in and the paths a
//! command writes to when the user gave none.

use std::path::{Path, PathBuf};

use meta_ast::graph::{GraphBuilder, SccAnalysis};
use meta_ast::model::SnapshotId;
use meta_ast::output::emitter::{EmitConfig, emit_graph};
use meta_ast::{GraphAnalysis, output};

fn vendored_bundle() -> String {
    // The `.js.txt` name keeps editors and formatters out of the vendored
    // bytes; the file is the artifact published for cytoscape 3.30.4.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/cytoscape-3.30.4.min.js.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn empty_analysis() -> GraphAnalysis {
    let snapshot_id = SnapshotId::new(1);
    assert!(snapshot_id.is_some(), "1 is a valid snapshot id");
    let snapshot_id = snapshot_id.unwrap();
    let graph = GraphBuilder::new(snapshot_id).build();
    let scc = SccAnalysis::analyze(graph.graph());
    GraphAnalysis {
        graph,
        scc,
        snapshot_id,
        extractions: vec![],
    }
}

/// A resource load is a script `src`, a link `href`, a CSS `url()` or an
/// `@import`. A plain anchor is navigation: it fetches nothing until the
/// reader clicks it, so the header keeps its two project links.
#[test]
fn the_dashboard_loads_no_external_resource() {
    let html = render_dashboard();

    for (pattern, what) in [
        ("<script src=", "a script element that fetches a URL"),
        ("<link", "a link element"),
        ("url(http", "a CSS load over http"),
        ("url('http", "a CSS load over http"),
        ("url(\"http", "a CSS load over http"),
        ("@import", "a CSS import"),
    ] {
        assert!(
            !html.contains(pattern),
            "the dashboard must not carry {what}: {pattern}"
        );
    }

    assert!(
        html.contains("<a href=\"https://github.com/metacall\">"),
        "the navigation links stay: they load nothing"
    );
}

fn render_dashboard() -> String {
    let analysis = empty_analysis();
    let rendered = output::dashboard::to_graph_html(
        &analysis.graph,
        &analysis.scc,
        analysis.snapshot_id.to_raw() as u64,
    );
    assert!(
        rendered.is_ok(),
        "the dashboard renders: {:?}",
        rendered.as_ref().err()
    );
    rendered.unwrap()
}

#[test]
fn the_dashboard_embeds_the_vendored_bundle() {
    let html = render_dashboard();
    let bundle = vendored_bundle();

    assert!(
        bundle.contains("kc.version=\"3.30.4\""),
        "the vendored bundle must be the pinned Cytoscape build"
    );
    assert!(
        html.contains(&bundle),
        "the document must carry the vendored bundle verbatim"
    );
    assert!(
        bundle.len() < 512 * 1024,
        "the vendored bundle must stay under the commit size limit"
    );
}

/// The watch module is compiled in only with the `watch` feature.
#[cfg(feature = "watch")]
#[test]
fn the_watch_configuration_does_not_open_a_browser() {
    let config = meta_ast::watch::WatchConfig::new();
    assert!(
        !config.emit.open_browser,
        "opening a browser needs an explicit opt-in"
    );
}

#[test]
fn the_emitter_does_not_invent_an_output_path() {
    let analysis = empty_analysis();
    let marker = PathBuf::from("project.metast");
    let existed = marker.exists();

    let config = EmitConfig {
        output: None,
        format: output::OutputFormat::Json,
        html: true,
        open_browser: false,
    };
    emit_graph(&analysis, &config).unwrap();

    let created = marker.exists() && !existed;
    if created {
        let _ = std::fs::remove_file(&marker);
    }
    assert!(
        !created,
        "a library emit with no path must not write a default file"
    );
}
