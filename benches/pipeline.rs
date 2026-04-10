use std::path::Path;

use criterion::{Criterion, criterion_group, criterion_main};
use meta_ast::extractor::extract;
use meta_ast::input::discover_files;

fn bench_extract_python(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract");
    group.sample_size(10);

    let files = discover_files(Path::new("tests/fixtures/python"), None).unwrap_or_default();
    if !files.is_empty() {
        group.bench_function("python_fixtures", |b| {
            b.iter(|| extract(&files));
        });
    }

    group.finish();
}

fn bench_extract_javascript(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract");

    let files = discover_files(Path::new("tests/fixtures/javascript"), None).unwrap_or_default();
    if !files.is_empty() {
        group.bench_function("javascript_fixtures", |b| {
            b.iter(|| extract(&files));
        });
    }

    group.finish();
}

fn bench_extract_rust(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract");

    let files = discover_files(Path::new("tests/fixtures/rust"), None).unwrap_or_default();
    if !files.is_empty() {
        group.bench_function("rust_fixtures", |b| {
            b.iter(|| extract(&files));
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_extract_python,
    bench_extract_javascript,
    bench_extract_rust,
);
criterion_main!(benches);
