//! Allocation budget for one extraction pass.
//!
//! Extraction owns one `String` per symbol, import and reference, so the
//! per-node cost is visible here as a large constant. The counter wraps the
//! system allocator in this test binary only; the buffer path is single
//! threaded, so the count is reproducible.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingAllocator;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Upper bound for one extraction of the generated buffer below.
///
/// The bound is a regression guard: it sits above the measured cost with room
/// for allocator noise, and below the cost of a change that stops owning the
/// per-node text, which is the point of the guard.
const EXTRACTION_BUDGET: usize = 3_000;

/// A Python buffer with 200 functions, 200 calls and 200 imports.
fn source_buffer() -> String {
    let mut source = String::with_capacity(64 * 1024);
    for index in 0..200 {
        source.push_str(&format!("import module_{index}\n"));
        source.push_str(&format!("def function_{index}(argument):\n"));
        source.push_str(&format!("    return helper_{index}(argument)\n"));
    }
    for index in 0..200 {
        source.push_str(&format!("def helper_{index}(argument):\n"));
        source.push_str("    return argument\n");
    }
    source
}

fn allocations_of<T>(body: impl FnOnce() -> T) -> (T, usize) {
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    let value = body();
    let after = ALLOCATIONS.load(Ordering::Relaxed);
    (value, after - before)
}

#[test]
fn one_extraction_stays_inside_its_allocation_budget() {
    let buffer = source_buffer();
    let generators = meta_ast::ExtractionIdGenerators::new();
    let options = meta_ast::ExtractOptions::default();

    let make_source = || meta_ast::InMemorySource {
        uri: "file:///tmp/allocation_budget.py",
        text: buffer.as_str(),
        language: meta_ast::LangId::Python,
        version: 1,
    };

    let (warmed, _) =
        allocations_of(|| meta_ast::extract_text_with_id_gen(make_source(), &options, &generators));
    assert!(
        warmed.is_ok(),
        "the buffer extracts: {:?}",
        warmed.as_ref().err()
    );
    let warmed = warmed.unwrap();
    assert!(
        warmed.file.symbols.len() >= 400,
        "the buffer holds at least 400 symbols, found {}",
        warmed.file.symbols.len()
    );

    let (extracted, allocations) =
        allocations_of(|| meta_ast::extract_text_with_id_gen(make_source(), &options, &generators));
    assert!(
        extracted.is_ok(),
        "the buffer extracts again: {:?}",
        extracted.as_ref().err()
    );
    let extracted = extracted.unwrap();

    assert_eq!(
        extracted.file.symbols.len(),
        warmed.file.symbols.len(),
        "two passes over the same buffer agree"
    );
    assert!(
        allocations <= EXTRACTION_BUDGET,
        "one extraction allocates {allocations} times, over the budget of {EXTRACTION_BUDGET}"
    );
}
