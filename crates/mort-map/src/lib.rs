//! mort's mix map (ADR 0006, ADR 0007).
//!
//! A [`Capture`] is what one run measured: its range, when, the project's
//! state, and a hash of every track's audio chunk at the time. The
//! [`MixMap`] keeps each capture's frames as they were taken, and for each
//! track a list of covers: which capture's frames are current over each
//! stretch of time. A new capture over a range takes over the covers there
//! on the tracks it measured. Captures no cover points at any more are
//! superseded; the latest [`HISTORY`] of them are kept, so that
//! [`MixMap::as_of`] can rebuild the map as it was after any recent
//! capture, for comparing (ADR 0007 §5).
//!
//! Staleness (ADR 0006 §3) compares a segment's capture with the project
//! [`Now`]: the segment is stale if the tempo map changed, or if the track
//! or anything routed into it (then or now) has a different hash.
//!
//! Nothing here depends on REAPER.

pub mod analysis;
pub mod compare;
pub mod findings;
pub mod query;
pub mod store;
pub mod thresholds;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub use mort_features::Frame;
use mort_proto::model::TrackInfo;
use mort_proto::{TempoMap, TrackId};

/// How far a frame's stamp may be from a span's edge and still count as on
/// it; see [`Span::holds_frame`].
pub const STAMP_SLACK: f64 = 1e-6;

/// A stretch of project time, `[start, end)`, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Span {
    /// Start.
    pub start: f64,
    /// End.
    pub end: f64,
}

impl Span {
    /// A span; `end` should be after `start`.
    #[must_use]
    pub fn new(start: f64, end: f64) -> Self {
        Self { start, end }
    }

    /// Whether `t` lies inside.
    #[must_use]
    pub fn contains(&self, t: f64) -> bool {
        t >= self.start && t < self.end
    }

    /// Whether a frame starting at `t` belongs here. Frame stamps carry
    /// rounding from sample positions, so a frame within a microsecond (a
    /// fifth of a sample at 192 kHz) of an edge counts as starting on it.
    #[must_use]
    pub fn holds_frame(&self, t: f64) -> bool {
        t >= self.start - STAMP_SLACK && t < self.end - STAMP_SLACK
    }

    /// The part shared with `other`, if any.
    #[must_use]
    pub fn intersect(&self, other: Span) -> Option<Span> {
        let s = Span::new(self.start.max(other.start), self.end.min(other.end));
        (s.end > s.start).then_some(s)
    }

    /// The smallest span covering both.
    #[must_use]
    pub fn union(&self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }
}

/// How a capture was run (ADR 0006 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// A dry-run render, faster than real time.
    #[default]
    Render,
    /// Playback through the monitors.
    Play,
}

/// One frame and the project time of its first sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedFrame {
    /// Project seconds.
    pub start: f64,
    /// The features.
    pub frame: Frame,
}

/// What one run measured (ADR 0006 §1). Frames live in the map's segments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capture {
    /// Its number, unique within the map.
    pub id: u64,
    /// The range run.
    pub span: Span,
    /// When, in seconds since the Unix epoch.
    pub taken: u64,
    /// The project state count when the run started.
    pub state: u64,
    /// Render or play.
    pub mode: Mode,
    /// [`tempo_fingerprint`] up to the range's end.
    pub tempo: u64,
    /// The audio-chunk hash of every track (and the master) when the run
    /// started.
    pub hashes: BTreeMap<TrackId, u64>,
    /// For each track measured, everything routed into it then.
    pub inputs: BTreeMap<TrackId, Vec<TrackId>>,
    /// Every track's gain, bus and MIDI notes when the run started, so
    /// that the map as of this capture is read with the mix as it was.
    #[serde(default)]
    pub tracks: Vec<analysis::TrackMeta>,
}

/// Frames over a stretch of one track's timeline, from one capture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment<'a> {
    /// The capture they came from.
    pub capture: u64,
    /// The time they cover.
    pub span: Span,
    /// The frames, in time order, each starting inside `span`.
    pub frames: &'a [TimedFrame],
}

/// Which capture's frames are current over a stretch of a track.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cover {
    /// The capture.
    pub capture: u64,
    /// The stretch.
    #[serde(flatten)]
    pub span: Span,
}

/// How many superseded captures the map keeps for comparing.
pub const HISTORY: usize = 8;

/// Where each track's audio goes: its parent folder or the master, and its
/// sends.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Routing {
    feeds: BTreeMap<TrackId, BTreeSet<TrackId>>,
}

impl Routing {
    /// Routing with no connections.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The routing of a project's tracks (not including the master).
    #[must_use]
    pub fn of(master: TrackId, tracks: &[TrackInfo]) -> Self {
        let mut r = Self::new();
        for t in tracks {
            if t.main_send {
                r.connect(t.id, t.parent.unwrap_or(master));
            }
            for s in &t.sends {
                r.connect(t.id, s.dest);
            }
        }
        r
    }

    /// Adds a connection from `from` into `to`.
    pub fn connect(&mut self, from: TrackId, to: TrackId) {
        self.feeds.entry(from).or_default().insert(to);
    }

    fn walk(start: TrackId, next: impl Fn(TrackId) -> Vec<TrackId>) -> BTreeSet<TrackId> {
        let mut seen = BTreeSet::new();
        let mut todo = next(start);
        while let Some(t) = todo.pop() {
            if t != start && seen.insert(t) {
                todo.extend(next(t));
            }
        }
        seen
    }

    /// Every track whose audio reaches `t`, directly or not.
    #[must_use]
    pub fn upstream(&self, t: TrackId) -> BTreeSet<TrackId> {
        Self::walk(t, |x| {
            self.feeds
                .iter()
                .filter(|(_, to)| to.contains(&x))
                .map(|(from, _)| *from)
                .collect()
        })
    }

    /// Every track `t`'s audio reaches: the buses and the master.
    #[must_use]
    pub fn downstream(&self, t: TrackId) -> BTreeSet<TrackId> {
        Self::walk(t, |x| {
            self.feeds
                .get(&x)
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default()
        })
    }
}

/// The project as it is now, for staleness.
pub struct Now<'a> {
    /// The audio-chunk hash of every track and the master.
    pub hashes: BTreeMap<TrackId, u64>,
    /// Current routing.
    pub routing: Routing,
    /// Current tempo map.
    pub tempo: &'a dyn TempoMap,
}

/// Why map data is stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum Stale {
    /// The tempo map changed.
    Tempo,
    /// This track, or one routed into it, changed.
    Changed {
        /// The track that changed.
        track: TrackId,
    },
    /// A track that was or is routed into it was added or removed.
    Routing {
        /// The track added or removed.
        track: TrackId,
    },
}

/// FNV-1a, 64 bits: stable across runs and builds, unlike `std`'s hasher.
#[must_use]
pub fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A hash of the tempo map's timing up to `end` seconds: where each
/// sixteenth note falls, to the microsecond.
#[must_use]
pub fn tempo_fingerprint(map: &dyn TempoMap, end: f64) -> u64 {
    let mut bytes = Vec::new();
    let mut q = 0.0;
    while bytes.len() < 8 * 1_000_000 {
        let t = map.qn_to_seconds(q);
        #[expect(clippy::cast_possible_truncation, reason = "microseconds fit")]
        bytes.extend(((t * 1e6).round() as i64).to_le_bytes());
        if t > end {
            break;
        }
        q += 0.25;
    }
    fnv(&bytes)
}

/// What [`MixMap::add`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct Added {
    /// The new capture's id.
    pub id: u64,
    /// Frames kept per track, after clipping to the range.
    pub frames: BTreeMap<TrackId, usize>,
    /// Captures now wholly superseded: no track's current data comes
    /// from them. They are kept for comparing, up to [`HISTORY`].
    pub superseded: Vec<u64>,
    /// Superseded captures forgotten to keep within [`HISTORY`].
    pub dropped: Vec<u64>,
}

/// A stretch of a track's map data and whether it is still true.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Covered {
    /// The stretch.
    #[serde(flatten)]
    pub span: Span,
    /// Its capture.
    pub capture: u64,
    /// Frames in it.
    pub frames: usize,
    /// Why it is stale, if it is.
    pub stale: Option<Stale>,
}

/// The mix map. See the crate docs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MixMap {
    captures: BTreeMap<u64, Capture>,
    frames: BTreeMap<u64, BTreeMap<TrackId, Vec<TimedFrame>>>,
    covers: BTreeMap<TrackId, Vec<Cover>>,
    next: u64,
    added: BTreeSet<u64>,
    changed: bool,
}

/// The frames of `fs` that start inside `span`, as a slice: `fs` is in
/// time order.
fn slice(fs: &[TimedFrame], span: Span) -> &[TimedFrame] {
    let a = fs.partition_point(|f| f.start < span.start - STAMP_SLACK);
    let b = fs.partition_point(|f| f.start < span.end - STAMP_SLACK);
    &fs[a..b.max(a)]
}

impl MixMap {
    /// An empty map.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next: 1,
            ..Self::default()
        }
    }

    /// Whether the map holds nothing current.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.covers.is_empty()
    }

    /// The captures that hold current frames, oldest first.
    pub fn captures(&self) -> impl Iterator<Item = &Capture> {
        let used = self.used();
        self.captures.values().filter(move |c| used.contains(&c.id))
    }

    /// Every capture kept, current or superseded, oldest first.
    pub fn all_captures(&self) -> impl Iterator<Item = &Capture> {
        self.captures.values()
    }

    /// A capture by id, current or superseded.
    #[must_use]
    pub fn capture(&self, id: u64) -> Option<&Capture> {
        self.captures.get(&id)
    }

    /// The tracks a capture measured.
    pub fn measured(&self, id: u64) -> impl Iterator<Item = TrackId> + '_ {
        self.frames
            .get(&id)
            .into_iter()
            .flat_map(|m| m.keys().copied())
    }

    /// A capture's frames for one track, as taken.
    #[must_use]
    pub fn capture_frames(&self, id: u64, t: TrackId) -> &[TimedFrame] {
        self.frames
            .get(&id)
            .and_then(|m| m.get(&t))
            .map_or(&[], Vec::as_slice)
    }

    /// The tracks with current data.
    pub fn tracks(&self) -> impl Iterator<Item = TrackId> + '_ {
        self.covers.keys().copied()
    }

    /// A track's covers, in time order.
    #[must_use]
    pub fn covers(&self, t: TrackId) -> &[Cover] {
        self.covers.get(&t).map_or(&[], Vec::as_slice)
    }

    /// A track's current segments, in time order.
    #[must_use]
    pub fn segments(&self, t: TrackId) -> Vec<Segment<'_>> {
        self.covers(t)
            .iter()
            .map(|c| Segment {
                capture: c.capture,
                span: c.span,
                frames: slice(self.capture_frames(c.capture, t), c.span),
            })
            .collect()
    }

    fn used(&self) -> BTreeSet<u64> {
        self.covers.values().flatten().map(|c| c.capture).collect()
    }

    /// Lays `span` of capture `id` over track `t`'s covers.
    fn cover(&mut self, t: TrackId, id: u64, span: Span) {
        let covers = self.covers.entry(t).or_default();
        let mut next = Vec::with_capacity(covers.len() + 2);
        for c in covers.drain(..) {
            if c.span.intersect(span).is_none() {
                next.push(c);
                continue;
            }
            for part in [
                Span::new(c.span.start, c.span.end.min(span.start)),
                Span::new(c.span.start.max(span.end), c.span.end),
            ] {
                if part.end > part.start {
                    next.push(Cover {
                        capture: c.capture,
                        span: part,
                    });
                }
            }
        }
        next.push(Cover { capture: id, span });
        next.sort_by(|a, b| a.span.start.total_cmp(&b.span.start));
        *covers = next;
    }

    /// Adds a capture with its frames by track. The capture's `id` is
    /// replaced with a fresh one. Frames that start outside the capture's
    /// range are dropped (ADR 0006 §5). The capture becomes current over
    /// its range on every track it has frames for, even none.
    pub fn add(
        &mut self,
        mut capture: Capture,
        frames: BTreeMap<TrackId, Vec<TimedFrame>>,
    ) -> Added {
        let before = self.used();
        let id = self.next.max(1);
        self.next = id + 1;
        capture.id = id;
        let span = capture.span;
        let mut kept = BTreeMap::new();
        let mut own = BTreeMap::new();
        for (t, mut fs) in frames {
            fs.retain(|f| span.holds_frame(f.start));
            fs.sort_by(|a, b| a.start.total_cmp(&b.start));
            kept.insert(t, fs.len());
            self.cover(t, id, span);
            own.insert(t, fs);
        }
        self.frames.insert(id, own);
        self.captures.insert(id, capture);
        self.added.insert(id);
        self.changed = true;
        let after = self.used();
        let superseded = before.difference(&after).copied().collect();
        Added {
            id,
            frames: kept,
            superseded,
            dropped: self.trim(),
        }
    }

    /// Forgets superseded captures beyond the latest [`HISTORY`]. Returns
    /// those forgotten.
    fn trim(&mut self) -> Vec<u64> {
        let used = self.used();
        let old: Vec<u64> = self
            .captures
            .keys()
            .filter(|c| !used.contains(c))
            .copied()
            .collect();
        let n = old.len().saturating_sub(HISTORY);
        let dropped: Vec<u64> = old[..n].to_vec();
        for c in &dropped {
            self.captures.remove(c);
            self.frames.remove(c);
            self.added.remove(c);
            self.changed = true;
        }
        dropped
    }

    /// Forgets the data of tracks `keep` refuses, current and past, and
    /// any capture left with nothing. Returns the captures forgotten.
    pub fn retain_tracks(&mut self, keep: impl Fn(TrackId) -> bool) -> Vec<u64> {
        self.covers.retain(|t, _| keep(*t));
        for m in self.frames.values_mut() {
            m.retain(|t, _| keep(*t));
        }
        let empty: Vec<u64> = self
            .frames
            .iter()
            .filter(|(_, m)| m.is_empty())
            .map(|(c, _)| *c)
            .collect();
        for c in &empty {
            self.captures.remove(c);
            self.frames.remove(c);
            self.added.remove(c);
        }
        self.changed = true;
        empty
    }

    /// The map as it was just after capture `id`, within `range`: every
    /// capture kept up to `id` laid over the tracks it measured, in order.
    /// Captures already forgotten leave gaps.
    #[must_use]
    pub fn as_of(&self, id: u64, range: Span) -> MixMap {
        let mut m = MixMap::new();
        for (cid, c) in self.captures.range(..=id) {
            let Some(span) = c.span.intersect(range) else {
                continue;
            };
            let mut own = BTreeMap::new();
            for (t, fs) in &self.frames[cid] {
                m.cover(*t, *cid, span);
                own.insert(*t, slice(fs, span).to_vec());
            }
            m.frames.insert(*cid, own);
            m.captures.insert(*cid, c.clone());
        }
        m.next = id + 1;
        m
    }

    /// Why track `t`'s data from capture `capture` is stale, or `None`
    /// if it is still true.
    #[must_use]
    pub fn staleness(&self, t: TrackId, capture: u64, now: &Now<'_>) -> Option<Stale> {
        let Some(c) = self.captures.get(&capture) else {
            return Some(Stale::Changed { track: t });
        };
        if tempo_fingerprint(now.tempo, c.span.end) != c.tempo {
            return Some(Stale::Tempo);
        }
        let then: BTreeSet<TrackId> = c.inputs.get(&t).into_iter().flatten().copied().collect();
        let current = now.routing.upstream(t);
        if let Some(u) = then.symmetric_difference(&current).next() {
            return Some(Stale::Routing { track: *u });
        }
        std::iter::once(t)
            .chain(current)
            .find(|u| now.hashes.get(u) != c.hashes.get(u))
            .map(|u| Stale::Changed { track: u })
    }

    /// Each track's covered stretches and whether they are stale.
    #[must_use]
    pub fn coverage(&self, now: &Now<'_>) -> BTreeMap<TrackId, Vec<Covered>> {
        self.covers
            .keys()
            .map(|t| {
                let spans = self
                    .segments(*t)
                    .iter()
                    .map(|s| Covered {
                        span: s.span,
                        capture: s.capture,
                        frames: s.frames.len(),
                        stale: self.staleness(*t, s.capture, now),
                    })
                    .collect();
                (*t, spans)
            })
            .collect()
    }

    /// The stale stretches, inside `within` if given, by track.
    #[must_use]
    pub fn stale(&self, now: &Now<'_>, within: Option<Span>) -> BTreeMap<TrackId, Vec<Span>> {
        let mut out: BTreeMap<TrackId, Vec<Span>> = BTreeMap::new();
        for (t, covered) in self.coverage(now) {
            for c in covered.into_iter().filter(|c| c.stale.is_some()) {
                let span = match within {
                    Some(w) => c.span.intersect(w),
                    None => Some(c.span),
                };
                if let Some(s) = span {
                    out.entry(t).or_default().push(s);
                }
            }
        }
        out
    }

    /// Captures added since the last [`MixMap::mark_saved`], whose frames
    /// the sidecar has not yet written.
    #[must_use]
    pub fn unsaved(&self) -> &BTreeSet<u64> {
        &self.added
    }

    /// Whether anything changed since the last [`MixMap::mark_saved`].
    #[must_use]
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// Marks everything as saved.
    pub fn mark_saved(&mut self) {
        self.added.clear();
        self.changed = false;
    }

    /// The next capture id; for the sidecar.
    #[must_use]
    pub fn next_id(&self) -> u64 {
        self.next
    }

    /// Every track's covers; for the sidecar.
    #[must_use]
    pub fn all_covers(&self) -> &BTreeMap<TrackId, Vec<Cover>> {
        &self.covers
    }

    /// A map from saved parts. Covers whose capture is missing are
    /// dropped.
    #[must_use]
    pub fn from_parts(
        next: u64,
        captures: Vec<Capture>,
        frames: BTreeMap<u64, BTreeMap<TrackId, Vec<TimedFrame>>>,
        covers: BTreeMap<TrackId, Vec<Cover>>,
    ) -> Self {
        let mut m = Self {
            captures: captures
                .into_iter()
                .filter(|c| frames.contains_key(&c.id))
                .map(|c| (c.id, c))
                .collect(),
            frames,
            covers,
            next: next.max(1),
            added: BTreeSet::new(),
            changed: false,
        };
        m.frames.retain(|c, _| m.captures.contains_key(c));
        for cs in m.covers.values_mut() {
            cs.retain(|c| m.captures.contains_key(&c.capture));
            cs.sort_by(|a, b| a.span.start.total_cmp(&b.span.start));
        }
        m.covers.retain(|_, c| !c.is_empty());
        m
    }
}
