//! Audition captures (ADR 0012 §1–5, ticket 0017).
//!
//! Probes keep sending frames whenever REAPER plays, with run 0 when no
//! `measure` asked for them. On every tick the [`Auditioner`] takes those
//! frames and gathers them into *stretches*: playback with no break in it.
//! A stretch ends when:
//! - the transport stops;
//! - a probe's stamps jump (a seek, a loop wrap, or a gap of more than 1.5
//!   frames);
//! - the project state count, the solo set, the play rate or the set of
//!   record-monitored tracks changes. Frames taken on the tick that sees
//!   such a change could belong to either side of it, so they are dropped.
//!
//! When a stretch ends it is checked (ADR 0012 §2) and added to the map as
//! an `audition` capture:
//! - nothing is kept unless the play rate was 1.0;
//! - record-monitored tracks, and everything they reach, are dropped;
//! - under solo, a track or bus is kept only if nothing routed into it was
//!   silenced by the solo;
//! - the first [`WARM_UP`] seconds are dropped, and short-term loudness is
//!   marked unknown (NaN) until 3 s in.
//!
//! The capture needs the chunk hashes of the state it was played under.
//! Hashing every chunk at once could take longer than a tick, so
//! [`HashCache`] hashes a few tracks per tick after each change, and a
//! finished stretch waits for it. If the state moves on first, the stretch
//! is dropped. Stretches from one playback share one frames file, written
//! when playback stops (ADR 0012 §5).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use mort_map::analysis::TrackMeta;
use mort_map::{Capture, Routing, Source, Span, TimedFrame, fnv};
use mort_proto::TrackId;
use mort_session::Session;

use crate::measure::{Shared, track_meta};
use crate::methods::{is_probe, probe_slot};

/// What a stretch loses at its start: the long FFT window of the low
/// bands (ADR 0007 §1), which still holds audio from before a seek.
pub const WARM_UP: f64 = 0.683;

/// How long short-term loudness's window is.
const SHORT_TERM: f64 = 3.0;

/// One frame.
const FRAME: f64 = 0.05;

/// A stamp further than this from where the last frame said the next would
/// start is a jump: 1.5 frames, less the one frame expected.
const JUMP: f64 = 0.5 * FRAME;

/// How much of a tick hashing may use.
pub(crate) const HASH_BUDGET: Duration = Duration::from_millis(2);

/// What playback was like.
#[derive(Debug, Clone, PartialEq)]
struct Context {
    state: u64,
    solo: BTreeSet<TrackId>,
    rate: f64,
    monitored: BTreeSet<TrackId>,
}

/// Playback with no break in it.
#[derive(Debug)]
struct Stretch {
    ctx: Context,
    frames: BTreeMap<TrackId, Vec<TimedFrame>>,
    /// Where each track's playback began: the warm-up and short-term rules
    /// count from here, across commits of a long stretch.
    origin: BTreeMap<TrackId, f64>,
    /// Each track's last frame stamp, to see jumps across commits.
    last: BTreeMap<TrackId, f64>,
    /// The capture the stretch's first committed part became: later parts
    /// extend it.
    capture: std::rc::Rc<std::cell::Cell<Option<u64>>>,
}

/// A stretch playing longer than this is committed to the map in parts,
/// so that the map fills in while the person listens (ADR 0012 §10).
pub const COMMIT_EVERY: f64 = 2.0;

/// The project as a capture records it, for one state.
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub(crate) hashes: BTreeMap<TrackId, u64>,
    pub(crate) routing: Routing,
    pub(crate) tracks: Vec<TrackMeta>,
}

/// Chunk hashes for the current state, filled in a few tracks at a time.
#[derive(Debug, Default)]
pub struct HashCache {
    state: Option<u64>,
    todo: Vec<TrackId>,
    hashes: BTreeMap<TrackId, u64>,
    /// Snapshots of the last few states, newest last: a stretch that ends
    /// because the state moved still needs the one before.
    ready: Vec<(u64, Snapshot)>,
}

/// How many states' snapshots the cache keeps.
const KEEP: usize = 4;

impl HashCache {
    /// Works on the hashes for `state` for up to `budget`.
    pub(crate) fn work<S: Session>(&mut self, s: &S, state: u64, budget: Duration) {
        if self.ready.iter().any(|(r, _)| *r == state) {
            return;
        }
        if self.state != Some(state) {
            self.state = Some(state);
            self.todo = std::iter::once(s.master())
                .chain(s.tracks().into_iter().map(|t| t.id))
                .collect();
            self.hashes.clear();
        }
        let started = Instant::now();
        while let Some(t) = self.todo.pop() {
            if let Ok(chunk) = s.audio_chunk(t) {
                self.hashes.insert(t, fnv(chunk.as_bytes()));
            }
            if started.elapsed() >= budget {
                break;
            }
        }
        if self.todo.is_empty() {
            let tracks = s.tracks();
            let Ok(meta) = track_meta(s) else {
                return;
            };
            self.ready.push((
                state,
                Snapshot {
                    hashes: std::mem::take(&mut self.hashes),
                    routing: Routing::of(s.master(), &tracks),
                    tracks: meta,
                },
            ));
            if self.ready.len() > KEEP {
                self.ready.remove(0);
            }
        }
    }

    pub(crate) fn snapshot(&self, state: u64) -> Option<&Snapshot> {
        self.ready.iter().find(|(r, _)| *r == state).map(|(_, s)| s)
    }
}

/// Folds auditions into the map. See the module docs.
pub struct Auditioner {
    store: Shared,
    open: Option<Stretch>,
    closed: Vec<Stretch>,
    last: Option<Context>,
    playing: bool,
    /// Probe slots by track, for one state.
    slots: Option<(u64, HashMap<u32, TrackId>)>,
    /// The frames file of the current playback: its first capture.
    file: Option<u64>,
    /// Whether this playback added captures not yet saved.
    unsaved: bool,
}

impl Auditioner {
    /// An auditioner adding to `store`'s map.
    #[must_use]
    pub fn new(store: Shared) -> Self {
        Self {
            store,
            open: None,
            closed: Vec::new(),
            last: None,
            playing: false,
            slots: None,
            file: None,
            unsaved: false,
        }
    }

    fn slot_map<S: Session>(&mut self, s: &S, state: u64) -> HashMap<u32, TrackId> {
        if let Some((st, m)) = &self.slots
            && *st == state
        {
            return m.clone();
        }
        let mut m = HashMap::new();
        for t in std::iter::once(s.master())
            .filter_map(|id| s.track(id).ok())
            .chain(s.tracks())
        {
            for fx in t.fx.iter().filter(|f| is_probe(f) && f.enabled) {
                if let Some(slot) = probe_slot(s, fx.id) {
                    m.insert(slot, t.id);
                }
            }
        }
        self.slots = Some((state, m.clone()));
        m
    }

    /// One tick. See the module docs.
    pub fn tick<S: Session>(&mut self, s: &mut S) {
        let playing = s.is_playing();
        {
            let mut store = self.store.borrow_mut();
            store.feed.transport(s.position(), playing);
            store.watch_staleness(&*s);
        }
        let waiting = self.open.is_some() || !self.closed.is_empty();
        let raw = s.take_frames(0);
        if !playing && !waiting && raw.is_empty() {
            self.save_if_done();
            return;
        }
        let state = s.state();
        let ctx = Context {
            state,
            solo: s
                .tracks()
                .into_iter()
                .filter(|t| t.solo)
                .map(|t| t.id)
                .collect(),
            rate: s.play_rate(),
            monitored: s.monitored().into_iter().collect(),
        };
        let slots = self.slot_map(&*s, state);
        let mut frames: BTreeMap<TrackId, Vec<TimedFrame>> = BTreeMap::new();
        for (slot, recs) in raw {
            if let Some(t) = slots.get(&slot) {
                frames
                    .entry(*t)
                    .or_default()
                    .extend(recs.iter().map(|r| TimedFrame {
                        start: r.start,
                        frame: r.frame,
                    }));
            }
        }
        for fs in frames.values_mut() {
            fs.sort_by(|a, b| a.start.total_cmp(&b.start));
        }
        let changed = self.last.as_ref().is_some_and(|l| *l != ctx);
        if changed {
            // These frames straddle the change: drop them.
            self.close();
        } else {
            self.append(&ctx, frames);
        }
        if !playing {
            self.close();
        } else if self.open_length() >= COMMIT_EVERY {
            self.commit();
        }
        self.last = Some(ctx);
        self.playing = playing;
        // Stretches from a state just left can use its hashes only before
        // the cache moves on to the new state.
        self.finish(&*s);
        if playing || !self.closed.is_empty() {
            self.store.borrow_mut().hashes.work(&*s, state, HASH_BUDGET);
        }
        self.finish(&*s);
        self.save_if_done();
    }

    /// Adds frames to the open stretch, splitting it where stamps jump.
    fn append(&mut self, ctx: &Context, mut frames: BTreeMap<TrackId, Vec<TimedFrame>>) {
        while !frames.values().all(Vec::is_empty) {
            let open = self.open.get_or_insert_with(|| Stretch {
                ctx: ctx.clone(),
                frames: BTreeMap::new(),
                origin: BTreeMap::new(),
                last: BTreeMap::new(),
                capture: std::rc::Rc::default(),
            });
            // Where each track's frames first jump from where it was.
            let mut cut: BTreeMap<TrackId, usize> = BTreeMap::new();
            for (t, fs) in &frames {
                let mut expect = open.last.get(t).map(|l| l + FRAME);
                for (i, f) in fs.iter().enumerate() {
                    if expect.is_some_and(|e| (f.start - e).abs() > JUMP) {
                        cut.insert(*t, i);
                        break;
                    }
                    expect = Some(f.start + FRAME);
                }
            }
            let mut rest: BTreeMap<TrackId, Vec<TimedFrame>> = BTreeMap::new();
            for (t, mut fs) in std::mem::take(&mut frames) {
                // A track whose frames did not jump in this batch keeps
                // them all in the stretch that is ending.
                if let Some(i) = cut.get(&t) {
                    rest.insert(t, fs.split_off(*i));
                }
                if let (Some(first), Some(last)) = (fs.first(), fs.last()) {
                    open.origin.entry(t).or_insert(first.start);
                    open.last.insert(t, last.start);
                }
                open.frames.entry(t).or_default().extend(fs);
            }
            if cut.is_empty() {
                return;
            }
            self.close();
            frames = rest;
        }
    }

    /// Hands the open stretch's frames so far to the map, keeping the
    /// stretch open: the next part needs no warm-up.
    fn commit(&mut self) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let part = Stretch {
            ctx: open.ctx.clone(),
            frames: std::mem::take(&mut open.frames),
            origin: open.origin.clone(),
            last: open.last.clone(),
            capture: std::rc::Rc::clone(&open.capture),
        };
        self.closed.push(part);
    }

    /// How long the open stretch's uncommitted frames run.
    fn open_length(&self) -> f64 {
        self.open.as_ref().map_or(0.0, |o| {
            o.frames
                .values()
                .filter_map(|v| Some(v.last()?.start - v.first()?.start))
                .fold(0.0, f64::max)
        })
    }

    fn close(&mut self) {
        if let Some(st) = self.open.take() {
            self.closed.push(st);
        }
    }

    /// Adds every closed stretch whose hashes are ready; drops those whose
    /// state has passed.
    fn finish<S: Session>(&mut self, s: &S) {
        let state = s.state();
        let mut keep = Vec::new();
        for st in std::mem::take(&mut self.closed) {
            let snap = self.store.borrow().hashes.snapshot(st.ctx.state).cloned();
            if let Some(snap) = snap {
                self.add(s, &st, &snap);
            } else if st.ctx.state == state {
                keep.push(st);
            }
        }
        self.closed = keep;
    }

    /// Checks a stretch (ADR 0012 §2) and adds what survives to the map.
    fn add<S: Session>(&mut self, s: &S, st: &Stretch, snap: &Snapshot) {
        let Some(capture_frames) = valid(st, snap) else {
            return;
        };
        let start = capture_frames
            .values()
            .filter_map(|v| v.first())
            .map(|f| f.start)
            .fold(f64::INFINITY, f64::min);
        let end = capture_frames
            .values()
            .filter_map(|v| v.last())
            .map(|f| f.start + FRAME)
            .fold(f64::NEG_INFINITY, f64::max);
        if end <= start {
            return;
        }
        let span = Span::new(start, end);
        if let Some(id) = st.capture.get() {
            let tracks: Vec<TrackId> = capture_frames.keys().copied().collect();
            let mut store = self.store.borrow_mut();
            let from = store.map.capture(id).map_or(start, |c| c.span.end);
            let tempo = mort_map::tempo_fingerprint(s.tempo_map(), end);
            if store.map.extend(id, capture_frames, end, tempo) {
                store.note_capture(tracks, Span::new(from, end));
                self.unsaved = true;
            }
            return;
        }
        let capture = Capture {
            id: 0,
            span,
            taken: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            state: st.ctx.state,
            source: Source::Audition,
            file: self.file,
            tempo: mort_map::tempo_fingerprint(s.tempo_map(), span.end),
            hashes: snap.hashes.clone(),
            inputs: capture_frames
                .keys()
                .map(|t| (*t, snap.routing.upstream(*t).into_iter().collect()))
                .collect(),
            tracks: snap.tracks.clone(),
        };
        let mut store = self.store.borrow_mut();
        store.sync(s);
        let tracks: Vec<TrackId> = capture_frames.keys().copied().collect();
        let added = store.map.add(capture, capture_frames);
        st.capture.set(Some(added.id));
        store.note_capture(tracks, span);
        self.file.get_or_insert(added.id);
        self.unsaved = true;
    }

    /// Saves the playback's captures once it has stopped and every stretch
    /// is in.
    fn save_if_done(&mut self) {
        if self.playing || self.open.is_some() || !self.closed.is_empty() {
            return;
        }
        if self.unsaved {
            self.store.borrow_mut().save(false);
            self.unsaved = false;
        }
        self.file = None;
    }
}

/// The frames of a stretch that fairly measure the mix (ADR 0012 §2), or
/// `None` if nothing does.
fn valid(st: &Stretch, snap: &Snapshot) -> Option<BTreeMap<TrackId, Vec<TimedFrame>>> {
    if (st.ctx.rate - 1.0).abs() > 1e-6 {
        return None;
    }
    let r = &snap.routing;
    let mut dropped: BTreeSet<TrackId> = BTreeSet::new();
    for m in &st.ctx.monitored {
        dropped.insert(*m);
        dropped.extend(r.downstream(*m));
    }
    if !st.ctx.solo.is_empty() {
        let soloed = &st.ctx.solo;
        let silenced: BTreeSet<TrackId> = snap
            .tracks
            .iter()
            .map(|t| t.id)
            .filter(|t| {
                !soloed.contains(t)
                    && r.downstream(*t).is_disjoint(soloed)
                    && r.upstream(*t).is_disjoint(soloed)
            })
            .collect();
        for t in st.frames.keys() {
            if !r.upstream(*t).is_disjoint(&silenced) {
                dropped.insert(*t);
            }
        }
    }
    let mut out = BTreeMap::new();
    for (t, fs) in &st.frames {
        if dropped.contains(t) {
            continue;
        }
        let Some(first) = fs.first() else { continue };
        let t0 = st.origin.get(t).copied().unwrap_or(first.start);
        let kept: Vec<TimedFrame> = fs
            .iter()
            .filter(|f| f.start >= t0 + WARM_UP - 1e-6)
            .map(|f| {
                let mut f = *f;
                if f.start + FRAME < t0 + SHORT_TERM - 1e-6 {
                    f.frame.short_term_lufs = f32::NAN;
                }
                f
            })
            .collect();
        if !kept.is_empty() {
            out.insert(*t, kept);
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mort_session::{FakeSession, NewTrack};

    #[test]
    fn hashes_are_spread_over_ticks_and_follow_the_state() {
        let mut s = FakeSession::new();
        for i in 0..5 {
            s.add_track(NewTrack {
                name: format!("t{i}"),
                ..NewTrack::default()
            })
            .unwrap();
        }
        let mut c = HashCache::default();
        let state = s.state();
        // With no budget, one track a tick: master and five tracks.
        for _ in 0..5 {
            c.work(&s, state, Duration::ZERO);
            assert!(c.snapshot(state).is_none());
        }
        c.work(&s, state, Duration::ZERO);
        let snap = c.snapshot(state).expect("ready after six ticks");
        assert_eq!(snap.hashes.len(), 6);
        assert_eq!(snap.tracks.len(), 6);

        // A change starts again; the old snapshot is not for the new state.
        s.add_track(NewTrack::default()).unwrap();
        let next = s.state();
        c.work(&s, next, Duration::ZERO);
        assert!(c.snapshot(next).is_none());
        assert!(
            c.snapshot(state).is_some(),
            "kept until the new one is ready"
        );
    }
}
