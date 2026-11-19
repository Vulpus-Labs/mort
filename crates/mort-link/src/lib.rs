//! The contract between `mort-probe` and `mort-bridge` (ADR 0005).
//!
//! Both are loaded into REAPER's process. The bridge registers
//! [`ATTACH_API`] with REAPER. A probe finds it through
//! `cockos.reaper_extension` and `GetFunc`, calls it with [`LINK_VERSION`]
//! and the size of [`Slot`] it was built with, and gets back a slot that
//! the bridge owns for the rest of the process. The probe writes
//! [`Record`]s into the slot's ring on the audio thread; the bridge reads
//! them on a thread of its own.
//!
//! The ring is single-producer, single-consumer and fixed-size. Pushing
//! never allocates, locks or blocks: when the ring is full the record is
//! dropped and counted.

use std::cell::UnsafeCell;
use std::ffi::CStr;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

pub use mort_features::Frame;

/// The name a probe passes to REAPER's `GetFunc`. The bridge registers it
/// as `API_mort_probe_attach`.
pub const ATTACH_API: &CStr = c"mort_probe_attach";

/// The version of this contract. A probe and bridge with different
/// versions do not attach.
pub const LINK_VERSION: u32 = 1;

/// Records per slot: about 100 seconds of frames at real time, or a second
/// of a render at 100× real time, between the bridge's reads.
pub const RING_RECORDS: usize = 2048;

/// The largest slot id; the probe's `slot` parameter runs from 0 to this.
pub const SLOT_MAX: u32 = 65_535;

/// The attach function: `(version, size_of::<Slot>())` to a slot, or null
/// if the versions or sizes differ.
pub type AttachFn = unsafe extern "C" fn(version: u32, slot_size: usize) -> *const Slot;

/// One frame as it travels from probe to bridge.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Record {
    /// The run the audio belonged to: 0 for none.
    pub run: u64,
    /// The project time of the frame's first sample, in seconds.
    pub start: f64,
    /// The features.
    pub frame: Frame,
}

/// A probe's place in the bridge. See the module docs.
#[repr(C)]
pub struct Slot {
    /// [`LINK_VERSION`] when created.
    pub version: u32,
    /// The slot's id, from 1. The probe shows it as its `slot` parameter.
    pub id: u32,
    /// Whether a probe is attached. The probe clears it when it goes.
    pub attached: AtomicBool,
    /// The probe's sample rate, as `f64` bits; 0 until it activates.
    pub sample_rate: AtomicU64,
    /// Records dropped because the ring was full.
    pub dropped: AtomicU64,
    /// The bridge's current run id, for the probe to stamp records with.
    pub run: &'static AtomicU64,
    /// Records pushed since the slot was created.
    pub pushed: AtomicU64,
    /// Next index to write; only the probe moves it.
    head: AtomicUsize,
    /// Next index to read; only the bridge moves it.
    tail: AtomicUsize,
    /// Set while a probe is between activate and deactivate.
    pub active: AtomicU32,
    /// The ring, on the heap: about 450 KB, too big for a stack.
    buf: Box<[UnsafeCell<MaybeUninit<Record>>]>,
}

// SAFETY: the ring's cells are written only by the single producer at
// `head` and read only by the single consumer at `tail`; the atomics order
// those accesses (see `push` and `pop`).
unsafe impl Sync for Slot {}
// SAFETY: as above; the slot owns plain data.
unsafe impl Send for Slot {}

impl Slot {
    /// A new, empty slot on the heap. Only the bridge creates slots.
    #[must_use]
    pub fn new(id: u32, run: &'static AtomicU64) -> Box<Slot> {
        Box::new(Slot {
            version: LINK_VERSION,
            id,
            attached: AtomicBool::new(true),
            sample_rate: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            run,
            pushed: AtomicU64::new(0),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            active: AtomicU32::new(0),
            buf: (0..RING_RECORDS)
                .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
                .collect(),
        })
    }

    /// Adds a record. Producer side only. Returns `false`, and counts it,
    /// if the ring was full.
    pub fn push(&self, r: &Record) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= RING_RECORDS {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        // SAFETY: the cell at `head` is not visible to the consumer until
        // `head` is published below, and only this producer writes.
        unsafe { (*self.buf[head % RING_RECORDS].get()).write(*r) };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        self.pushed.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Takes the oldest record. Consumer side only.
    pub fn pop(&self) -> Option<Record> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        // SAFETY: `tail != head`, so the producer wrote this cell and
        // published it with the Release store of `head`, read above.
        let r = unsafe { (*self.buf[tail % RING_RECORDS].get()).assume_init_read() };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(r)
    }

    /// The sample rate the probe reported, if it has activated.
    #[must_use]
    pub fn sample_rate(&self) -> Option<f64> {
        let bits = self.sample_rate.load(Ordering::Relaxed);
        (bits != 0).then(|| f64::from_bits(bits))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static RUN: AtomicU64 = AtomicU64::new(7);

    fn record(n: u32) -> Record {
        let mut frame: Frame = unsafe { std::mem::zeroed() };
        frame.index = n;
        Record {
            run: RUN.load(Ordering::Relaxed),
            start: f64::from(n) * 0.05,
            frame,
        }
    }

    #[test]
    fn fifo_and_full() {
        let s = Slot::new(1, &RUN);
        assert!(s.pop().is_none());
        for i in 0..u32::try_from(RING_RECORDS).unwrap() {
            assert!(s.push(&record(i)));
        }
        assert!(!s.push(&record(9999)), "full");
        assert_eq!(s.dropped.load(Ordering::Relaxed), 1);
        for i in 0..3 {
            assert_eq!(s.pop().unwrap().frame.index, i);
        }
        assert!(s.push(&record(5000)), "room again");
        assert_eq!(
            s.pushed.load(Ordering::Relaxed),
            u64::try_from(RING_RECORDS).unwrap() + 1
        );
    }

    #[test]
    fn producer_and_consumer_on_threads() {
        const N: u32 = 200_000;
        let s: &'static Slot = Box::leak(Slot::new(2, &RUN));
        let producer = std::thread::spawn(move || {
            let mut i = 0;
            while i < N {
                if s.push(&record(i)) {
                    i += 1;
                } else {
                    s.dropped.fetch_sub(1, Ordering::Relaxed);
                    std::thread::yield_now();
                }
            }
        });
        let mut next = 0;
        while next < N {
            if let Some(r) = s.pop() {
                assert_eq!(r.frame.index, next, "in order, none lost");
                assert!((r.start - f64::from(next) * 0.05).abs() < 1e-9);
                next += 1;
            }
        }
        producer.join().unwrap();
        assert!(s.pop().is_none());
    }
}
