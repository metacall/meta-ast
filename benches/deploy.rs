//! Benchmarks for the MetaCall call-site scanner.
//!
//! The scanner visits every call expression in a file and classifies it, so
//! the suite measures a file whose calls are almost all unrelated to MetaCall.

use std::hint::black_box;
use std::path::Path;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use meta_ast::deploy::scanner::scan_file;
use meta_ast::language::{LangId, grammar_for};

fn parse(id: LangId, source: &[u8]) -> tree_sitter::Tree {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar_for(id)).unwrap();
    parser.parse(source, None).unwrap()
}

/// A JavaScript file with `calls` unrelated calls and one aliased MetaCall call.
fn js_source(calls: usize) -> Vec<u8> {
    let mut source = String::with_capacity(calls * 24 + 64);
    source.push_str("const mc = require(\"metacall\");\n");
    for index in 0..calls {
        source.push_str(&format!("helper_{index}(1, 2);\n"));
    }
    source.push_str("mc.metacall(\"target\", 1, 2);\n");
    source.into_bytes()
}

fn bench_scan_javascript(c: &mut Criterion) {
    let mut group = c.benchmark_group("scan_javascript");
    group.sample_size(20);

    for calls in [500usize, 2000, 8000].iter() {
        let source = js_source(*calls);
        let tree = parse(LangId::JavaScript, &source);

        group.bench_with_input(BenchmarkId::from_parameter(calls), calls, |b, _| {
            b.iter(|| {
                let sites =
                    scan_file(LangId::JavaScript, &tree, &source, Path::new("bench.js")).unwrap();
                black_box(sites.len());
            });
        });
    }

    group.finish();
}

criterion_group!(deploy_benches, bench_scan_javascript);
criterion_main!(deploy_benches);
