//! The bridge's side of the probe link (ADR 0005 §4–7).
//!
//! - [`attach`] is the function REAPER hands probes as
//!   `mort_probe_attach`. It gives each probe a [`Slot`] that lives for the
//!   rest of the process.
//! - A drain thread empties every slot's ring every few milliseconds into
//!   per-slot buffers, so rings never fill while a render holds the main
//!   thread. It only reads memory; it never calls REAPER.
//! - [`take`] hands the buffered records to whoever assembles captures.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use mort_link::{LINK_VERSION, Record, Slot};

use crate::runs;

/// How often the drain thread empties the rings.
pub const DRAIN_EVERY: Duration = Duration::from_millis(5);

static SLOTS: Mutex<Vec<&'static Slot>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU32 = AtomicU32::new(1);
static BUFFERS: OnceLock<Mutex<HashMap<u32, Vec<Record>>>> = OnceLock::new();
static DRAINING: AtomicBool = AtomicBool::new(false);

fn buffers() -> &'static Mutex<HashMap<u32, Vec<Record>>> {
    BUFFERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Gives a probe a slot: the function registered as `API_mort_probe_attach`.
/// Returns null if the probe was built against another version of the link.
///
/// # Safety
///
/// Callable from any thread; it takes plain values.
pub unsafe extern "C" fn attach(version: u32, slot_size: usize) -> *const Slot {
    if version != LINK_VERSION || slot_size != std::mem::size_of::<Slot>() {
        return std::ptr::null();
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let slot: &'static Slot = Box::leak(Slot::new(id, runs::atomic()));
    if let Ok(mut s) = SLOTS.lock() {
        s.push(slot);
    }
    std::ptr::from_ref(slot)
}

/// Every slot handed out so far.
#[must_use]
pub fn slots() -> Vec<&'static Slot> {
    SLOTS.lock().map(|s| s.clone()).unwrap_or_default()
}

/// The slot with `id`, if one was handed out.
#[must_use]
pub fn slot(id: u32) -> Option<&'static Slot> {
    slots().into_iter().find(|s| s.id == id)
}

/// Moves every record waiting in the rings into the buffers.
pub fn drain() {
    let all = slots();
    let mut moved: Vec<(u32, Vec<Record>)> = Vec::new();
    for s in all {
        let recs: Vec<Record> = std::iter::from_fn(|| s.pop()).collect();
        if !recs.is_empty() {
            moved.push((s.id, recs));
        }
    }
    if moved.is_empty() {
        return;
    }
    if let Ok(mut b) = buffers().lock() {
        for (id, recs) in moved {
            b.entry(id).or_default().extend(recs);
        }
    }
}

/// Starts the drain thread, once.
pub fn start_draining() {
    if DRAINING.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("mort-drain".into())
        .spawn(|| {
            loop {
                drain();
                std::thread::sleep(DRAIN_EVERY);
            }
        });
}

/// Takes the buffered records of `run` from every slot, leaving the rest.
#[must_use]
pub fn take(run: u64) -> HashMap<u32, Vec<Record>> {
    drain();
    let mut out = HashMap::new();
    if let Ok(mut b) = buffers().lock() {
        for (id, recs) in b.iter_mut() {
            let (mine, rest): (Vec<Record>, Vec<Record>) =
                recs.drain(..).partition(|r| r.run == run);
            *recs = rest;
            if !mine.is_empty() {
                out.insert(*id, mine);
            }
        }
    }
    out
}

/// Copies of up to `limit` buffered records of `run` from slot `id`, in
/// order, leaving them buffered.
#[must_use]
pub fn peek(id: u32, run: u64, limit: usize) -> Vec<Record> {
    buffers()
        .lock()
        .map(|b| {
            b.get(&id)
                .map(|recs| {
                    recs.iter()
                        .filter(|r| r.run == run)
                        .take(limit)
                        .copied()
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

/// How many records are buffered for each slot, by run.
#[must_use]
pub fn counts() -> HashMap<u32, HashMap<u64, usize>> {
    drain();
    let mut out: HashMap<u32, HashMap<u64, usize>> = HashMap::new();
    if let Ok(b) = buffers().lock() {
        for (id, recs) in b.iter() {
            for r in recs {
                *out.entry(*id).or_default().entry(r.run).or_default() += 1;
            }
        }
    }
    out
}

/// Forgets buffered records of runs other than those in `keep`.
pub fn discard_except(keep: &[u64]) {
    if let Ok(mut b) = buffers().lock() {
        for recs in b.values_mut() {
            recs.retain(|r| keep.contains(&r.run));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_checks_version_and_size() {
        // SAFETY: plain values.
        unsafe {
            assert!(attach(LINK_VERSION + 1, std::mem::size_of::<Slot>()).is_null());
            assert!(attach(LINK_VERSION, 7).is_null());
            let a = attach(LINK_VERSION, std::mem::size_of::<Slot>());
            let b = attach(LINK_VERSION, std::mem::size_of::<Slot>());
            assert!(!a.is_null() && !b.is_null());
            assert_ne!((*a).id, (*b).id);
            assert!(slot((*a).id).is_some());
        }
    }
}
