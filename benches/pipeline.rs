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

fn bench_extract_go(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract");

    let files = discover_files(Path::new("tests/fixtures/go"), None).unwrap_or_default();
    if !files.is_empty() {
        group.bench_function("go_fixtures", |b| {
            b.iter(|| extract(&files));
        });
    }

    group.finish();
}

fn bench_extract_c(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract");

    let files = discover_files(Path::new("tests/fixtures/c"), None).unwrap_or_default();
    if !files.is_empty() {
        group.bench_function("c_fixtures", |b| {
            b.iter(|| extract(&files));
        });
    }

    group.finish();
}

fn bench_extract_cpp(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract");

    let files = discover_files(Path::new("tests/fixtures/cpp"), None).unwrap_or_default();
    if !files.is_empty() {
        group.bench_function("cpp_fixtures", |b| {
            b.iter(|| extract(&files));
        });
    }

    group.finish();
}

fn bench_extract_typescript(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract");

    let files = discover_files(Path::new("tests/fixtures/typescript"), None).unwrap_or_default();
    if !files.is_empty() {
        group.bench_function("typescript_fixtures", |b| {
            b.iter(|| extract(&files));
        });
    }

    group.finish();
}

fn bench_extract_tsx(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract");

    let files = discover_files(Path::new("tests/fixtures/tsx"), None).unwrap_or_default();
    if !files.is_empty() {
        group.bench_function("tsx_fixtures", |b| {
            b.iter(|| extract(&files));
        });
    }

    group.finish();
}

fn bench_extract_mixed(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract");

    let files = discover_files(Path::new("tests/fixtures/mixed"), None).unwrap_or_default();
    if !files.is_empty() {
        group.bench_function("mixed_fixtures", |b| {
            b.iter(|| extract(&files));
        });
    }

    group.finish();
}

fn bench_extract_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract");
    group.sample_size(10);

    let files = discover_files(Path::new("tests/fixtures"), None).unwrap_or_default();
    if !files.is_empty() {
        group.bench_function("all_fixtures", |b| {
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
    bench_extract_go,
    bench_extract_c,
    bench_extract_cpp,
    bench_extract_typescript,
    bench_extract_tsx,
    bench_extract_mixed,
    bench_extract_all,
);
criterion_main!(benches);
