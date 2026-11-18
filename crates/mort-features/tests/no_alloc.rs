//! `Analyzer::process` must not allocate (ADR 0005 §7).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use mort_features::Analyzer;

/// Counts allocations made on a thread while it is armed.
struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

fn note() {
    if ARMED.with(Cell::get) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: every call forwards to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: the caller's contract is passed through.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: as above.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        // SAFETY: as above.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        // SAFETY: as above.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn process_and_reset_do_not_allocate() {
    // The counter itself works.
    ARMED.with(|a| a.set(true));
    drop(std::hint::black_box(Vec::<u8>::with_capacity(16)));
    ARMED.with(|a| a.set(false));
    assert!(
        ALLOCATIONS.load(Ordering::Relaxed) > 0,
        "the counting allocator saw nothing"
    );

    for (rate, block) in [(48_000.0, 512), (44_100.0, 1_024), (96_000.0, 64)] {
        let mut analyzer = Analyzer::new(rate, block).unwrap();
        // Noise-like input with odd block sizes, including ones larger than
        // the maximum, so every path runs.
        let input: Vec<f32> = (0..block * 3 + 6_007)
            .map(|i| f32::from(u16::try_from((i * 7_919) % 1_000).unwrap()) / 1_000.0 - 0.5)
            .collect();
        let mut frames = 0usize;
        ALLOCATIONS.store(0, Ordering::Relaxed);
        ARMED.with(|a| a.set(true));
        for _ in 0..200 {
            for size in [block, 1, block / 2 + 3, block * 3 + 6_007] {
                let size = size.min(input.len());
                analyzer.process(&input[..size], &input[..size], |_| frames += 1);
            }
        }
        analyzer.reset();
        analyzer.process(&input, &input, |_| frames += 1);
        ARMED.with(|a| a.set(false));
        assert!(frames > 20, "only {frames} frames at {rate} Hz");
        assert_eq!(
            ALLOCATIONS.load(Ordering::Relaxed),
            0,
            "process allocated at {rate} Hz"
        );
    }
}
