//! The probe's audio-thread path: stamping, run ids, restarts, and no
//! allocation (ADR 0005 §3, §7).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use mort_link::Slot;
use mort_probe::Meter;

struct Counting;

thread_local! {
    // Counted per thread, so other tests running alongside do not count.
    static COUNTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards to the system allocator, counting while enabled.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if COUNTING.with(std::cell::Cell::get) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static A: Counting = Counting;

static RUN: AtomicU64 = AtomicU64::new(0);

fn sine(n: usize, start: usize, rate: f64) -> Vec<f32> {
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "test signal"
    )]
    (0..n)
        .map(|i| {
            (0.5 * (2.0 * std::f64::consts::PI * 440.0 * (start + i) as f64 / rate).sin()) as f32
        })
        .collect()
}

fn drain(slot: &Slot) -> Vec<mort_link::Record> {
    std::iter::from_fn(|| slot.pop()).collect()
}

#[test]
fn frames_are_stamped_with_project_time_and_run() {
    let rate = 48_000.0;
    let slot = Slot::new(1, &RUN);
    let mut m = Meter::new(rate, 512).unwrap();
    RUN.store(5, Ordering::Relaxed);
    // Play from 10 s in 512-sample blocks for one second.
    let mut pos = 10.0;
    for b in 0..94 {
        let x = sine(512, b * 512, rate);
        m.block(Some(pos), &x, &x, &slot);
        pos += 512.0 / rate;
    }
    let recs = drain(&slot);
    assert_eq!(recs.len(), 20, "20 frames in a second");
    for (k, r) in recs.iter().enumerate() {
        let want = 10.0 + 0.05 * f64::from(u32::try_from(k).unwrap());
        assert!(
            (r.start - want).abs() < 1e-9,
            "frame {k} starts at {} not {want}",
            r.start
        );
        assert_eq!(r.run, 5);
    }
    // Stopping measures nothing; starting elsewhere starts frames afresh.
    m.block(None, &[0.0; 512], &[0.0; 512], &slot);
    assert!(drain(&slot).is_empty());
    let x = sine(4800, 0, rate);
    m.block(Some(30.0), &x, &x, &slot);
    let recs = drain(&slot);
    assert_eq!(recs.len(), 2);
    assert!(
        (recs[0].start - 30.0).abs() < 1e-9 && recs[0].frame.index == 0,
        "{}",
        recs[0].start
    );
    assert!((recs[1].start - 30.05).abs() < 1e-9);
}

#[test]
fn the_audio_path_does_not_allocate() {
    let rate = 44_100.0;
    let slot = Slot::new(2, &RUN);
    let mut m = Meter::new(rate, 1024).unwrap();
    let blocks: Vec<Vec<f32>> = (0..200).map(|b| sine(1000, b * 1000, rate)).collect();
    COUNTING.with(|c| c.set(true));
    let mut pos = 0.0;
    for x in &blocks {
        m.block(Some(pos), x, x, &slot);
        pos += 1000.0 / rate;
        // The bridge drains on another thread; here, keep the ring from filling.
        while slot.pop().is_some() {}
    }
    m.block(None, &blocks[0], &blocks[0], &slot);
    m.block(Some(99.0), &blocks[0], &blocks[0], &slot);
    COUNTING.with(|c| c.set(false));
    assert_eq!(ALLOCS.load(Ordering::SeqCst), 0);
}
