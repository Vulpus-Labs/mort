//! Run ids (ADR 0005 §3).
//!
//! Each play or render gets a run id before it starts. Probes stamp their
//! frames with the id current when the audio passed through them, so the
//! bridge can tell one run's frames from another's. The id is a single
//! atomic: probes read it on the audio thread without locking.

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);
static CURRENT: AtomicU64 = AtomicU64::new(0);

/// The atomic probes read the current run from.
#[must_use]
pub fn atomic() -> &'static AtomicU64 {
    &CURRENT
}

/// Starts a new run and returns its id.
pub fn begin() -> u64 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    CURRENT.store(id, Ordering::Release);
    id
}

/// Ends the run `id`, if it is still current. Frames after this carry 0.
pub fn end(id: u64) {
    let _ = CURRENT.compare_exchange(id, 0, Ordering::AcqRel, Ordering::Relaxed);
}

/// The run in progress, or 0 when none is.
#[must_use]
pub fn current() -> u64 {
    CURRENT.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_are_unique_and_end_only_themselves() {
        let a = begin();
        let b = begin();
        assert!(b > a);
        assert_eq!(current(), b);
        end(a);
        assert_eq!(current(), b, "ending an old run leaves the new one");
        end(b);
        assert_eq!(current(), 0);
    }
}
