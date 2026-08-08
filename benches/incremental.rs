//! Benchmarks for incremental re-analysis performance.
//!
//! Measures cold `analyze_graph` vs warm `incremental_reanalyze` after a
//! single-file change. Serves as evidence for the Phase 4 exit gate:
//! "Incremental performance target evidence captured" (target <100ms for
//! files under 5k LOC, per FR-5).
//!
//! Run: `cargo bench --bench incremental --features watch`

use std::io::Write;
use std::path::Path;

use criterion::{Criterion, criterion_group, criterion_main};
use meta_ast::model::SnapshotId;
use meta_ast::pipeline::analyze_graph;
use meta_ast::watch::{WatchState, incremental_reanalyze};

fn fixture_root() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("meta_ast_bench_incremental");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn write_file(root: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = root.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path
}

/// Generate a fixture of N Python files with M functions each.
fn generate_fixture(root: &Path, file_count: usize, funcs_per_file: usize) {
    for i in 0..file_count {
        let mut content = String::new();
        for j in 0..funcs_per_file {
            content.push_str(&format!("def func_{}_{}(): pass\n", i, j));
        }
        write_file(root, &format!("mod_{}.py", i), &content);
    }
}

fn bench_cold_vs_warm(c: &mut Criterion) {
    let root = fixture_root();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&root);

    generate_fixture(&root, 50, 20);

    let mut group = c.benchmark_group("incremental");
    group.sample_size(10);

    group.bench_function("cold_analyze_graph", |b| {
        b.iter(|| analyze_graph(&root, SnapshotId::new(1).unwrap(), None).unwrap());
    });

    let mut state = WatchState::new();
    incremental_reanalyze(&root, None, &mut state).unwrap();

    let modified = root.join("mod_42.py");
    let original = std::fs::read_to_string(&modified).unwrap();
    let changed = format!("{original}\ndef extra(): return 1\n");

    let mut toggle = false;
    group.bench_function("warm_incremental_reanalyze_single_change", |b| {
        b.iter_with_setup(
            || {
                toggle = !toggle;
                if toggle {
                    std::fs::write(&modified, &changed).unwrap();
                } else {
                    std::fs::write(&modified, &original).unwrap();
                }
            },
            |_| {
                incremental_reanalyze(&root, None, &mut state).unwrap();
            },
        );
    });

    std::fs::write(&modified, &original).unwrap();
    let _ = incremental_reanalyze(&root, None, &mut state).unwrap();

    group.finish();
}

criterion_group!(benches, bench_cold_vs_warm);
criterion_main!(benches);
