//! Measuring (ticket 0012, ADR 0006): the `measure` job, the mix map it
//! fills, and `map.status`.
//!
//! `measure` is an edit, because it may add probes: it makes sure every
//! track it measures has one, then queues a job that hashes every track's
//! audio chunk, runs the range (a dry-run render by default, or playback),
//! takes the run's frames and adds them to the map as a capture.
//!
//! The map follows the project file. When the project's path changes, the
//! bridge loads the map saved beside the new file if there is one, and
//! otherwise keeps what it has for tracks the project still holds, saving
//! it there. Captures are written to the sidecar as they are made. An
//! unsaved project keeps its map in memory only (ADR 0006 §6).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use serde::Deserialize;
use serde_json::{Value, json};

use mort_map::analysis::{TrackMeta, View, sections};
use mort_map::thresholds::Overrides;
use mort_map::{
    Capture, MixMap, Mode, Now, Routing, Span, TimedFrame, findings, fnv, query, store,
};
use mort_proto::model::TrackInfo;
use mort_proto::{ErrorKind, ErrorObject, TrackId};
use mort_session::Session;

use std::sync::Arc;

use crate::dispatch::{Ctx, Dispatcher, Job, JobPoll, Outcome};
use crate::feed::{Feed, Kind as FeedKind};
use crate::methods::{NoParams, check_range, ensure_probes, is_probe, parse, probe_slot, read};

/// The map and where it is saved.
#[derive(Debug, Default)]
pub struct Store {
    pub(crate) map: MixMap,
    path: Option<PathBuf>,
    save_error: Option<String>,
    /// The project's threshold overrides (ADR 0012 §7).
    pub(crate) overrides: Overrides,
    /// Why `thresholds.toml` could not be read, if it could not.
    thresholds_error: Option<String>,
    /// Where changes are announced (ADR 0012 §8).
    pub(crate) feed: Arc<Feed>,
    /// Chunk hashes for the current state, built across ticks.
    pub(crate) hashes: crate::audition::HashCache,
    /// The stale spans last announced, and the state they were for.
    stale_seen: Option<(u64, BTreeMap<TrackId, Vec<Span>>)>,
    /// Whether new data has changed what is stale since then, though the
    /// state has not moved.
    stale_dirty: bool,
    /// Findings per section, with the signature they were computed for.
    pub(crate) findings_cache: FindingsCache,
}

/// Findings by section (start and end bits, name), with the signature of
/// what they were computed from.
pub(crate) type FindingsCache =
    std::collections::HashMap<(u64, u64, String), (u64, Vec<findings::Finding>)>;

impl Store {
    fn new(feed: Arc<Feed>) -> Self {
        Self {
            map: MixMap::new(),
            path: None,
            save_error: None,
            overrides: Overrides::default(),
            thresholds_error: None,
            feed,
            hashes: crate::audition::HashCache::default(),
            stale_seen: None,
            stale_dirty: false,
            findings_cache: FindingsCache::new(),
        }
    }

    /// Announces new data from a capture.
    pub(crate) fn note_capture(&mut self, tracks: Vec<TrackId>, span: Span) {
        self.feed.note(FeedKind::Capture, tracks, vec![span]);
        // New data changes what is stale; take that as the baseline
        // quietly, since the capture itself was announced.
        self.stale_dirty = true;
    }

    /// On each tick: once the hashes for the current state are ready,
    /// works out what is stale, and announces what changed since last time
    /// if the state moved.
    pub(crate) fn watch_staleness<S: Session>(&mut self, s: &S) {
        let state = s.state();
        if !self.stale_dirty && self.stale_seen.as_ref().is_some_and(|(st, _)| *st == state) {
            return;
        }
        self.hashes.work(s, state, crate::audition::HASH_BUDGET);
        let Some((hashes, routing)) = self
            .hashes
            .snapshot(state)
            .map(|snap| (snap.hashes.clone(), snap.routing.clone()))
        else {
            return;
        };
        let now = Now {
            hashes,
            routing,
            tempo: s.tempo_map(),
        };
        let now_stale = self.map.stale(&now, None);
        let moved = self.stale_seen.as_ref().is_some_and(|(st, _)| *st != state);
        if moved {
            let before = self
                .stale_seen
                .as_ref()
                .map(|(_, m)| m.clone())
                .unwrap_or_default();
            let tracks: Vec<TrackId> = now_stale
                .keys()
                .chain(before.keys())
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter(|t| now_stale.get(t) != before.get(t))
                .collect();
            if !tracks.is_empty() {
                let spans = tracks
                    .iter()
                    .flat_map(|t| now_stale.get(t).into_iter().chain(before.get(t)).flatten())
                    .copied()
                    .reduce(|a, b| a.union(b))
                    .into_iter()
                    .collect();
                self.feed.note(FeedKind::Stale, tracks, spans);
            }
        }
        self.stale_seen = Some((state, now_stale));
        self.stale_dirty = false;
    }

    fn dir(&self) -> Option<PathBuf> {
        self.path.as_deref().map(store::dir_for)
    }

    /// `<project>.mort/thresholds.toml`.
    fn thresholds_file(&self) -> Option<PathBuf> {
        self.dir()
            .and_then(|d| d.parent().map(|p| p.join("thresholds.toml")))
    }

    /// Reads the overrides beside the project, or writes those held in
    /// memory there if it has none.
    fn sync_thresholds(&mut self) {
        let Some(file) = self.thresholds_file() else {
            return;
        };
        match std::fs::read_to_string(&file) {
            Ok(text) => match Overrides::from_toml(&text) {
                Ok(o) => {
                    self.overrides = o;
                    self.thresholds_error = None;
                }
                Err(e) => {
                    self.overrides = Overrides::default();
                    self.thresholds_error = Some(format!(
                        "{}: {e}; using the default thresholds",
                        file.display()
                    ));
                }
            },
            Err(_) if self.overrides == Overrides::default() => {}
            Err(_) => self.write_thresholds(),
        }
    }

    fn write_thresholds(&mut self) {
        let Some(file) = self.thresholds_file() else {
            return;
        };
        let r = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, self.overrides.to_toml()));
        self.thresholds_error = r.err().map(|e| format!("{}: {e}", file.display()));
    }

    /// Changes an override (`value`) or removes it (`None`), saving it
    /// beside the project if it has been saved.
    pub(crate) fn set_threshold(&mut self, name: &str, value: Option<f64>) -> Result<(), String> {
        match value {
            Some(v) => self.overrides.set(name, v)?,
            None => self.overrides.reset(name)?,
        }
        self.write_thresholds();
        self.feed.note(FeedKind::Thresholds, vec![], vec![]);
        Ok(())
    }

    /// Follows the project file; see the module docs.
    pub(crate) fn sync<S: Session>(&mut self, s: &S) {
        let path = s.project().path.map(PathBuf::from);
        if path == self.path {
            return;
        }
        self.path = path;
        self.feed.note(FeedKind::Project, vec![], vec![]);
        self.stale_seen = None;
        self.stale_dirty = false;
        self.findings_cache.clear();
        self.sync_thresholds();
        if let Some(dir) = self.dir() {
            match store::load(&dir) {
                Ok(Some(m)) => {
                    self.map = m;
                    self.save_error = None;
                    return;
                }
                Ok(None) => {}
                Err(e) => self.save_error = Some(format!("{}: {e}", dir.display())),
            }
        }
        let ids: HashSet<TrackId> = std::iter::once(s.master())
            .chain(s.tracks().into_iter().map(|t| t.id))
            .collect();
        self.map.retain_tracks(|t| ids.contains(&t));
        if !self.map.is_empty() {
            self.save(true);
        }
    }

    pub(crate) fn save(&mut self, all: bool) {
        let Some(dir) = self.dir() else {
            return;
        };
        self.save_error = store::save(&dir, &mut self.map, all)
            .err()
            .map(|e| format!("{}: {e}", dir.display()));
    }
}

pub(crate) type Shared = Rc<RefCell<Store>>;

/// Every track's audio-chunk hash, the routing and the track list.
type ProjectNow = (BTreeMap<TrackId, u64>, Routing, Vec<TrackInfo>);

/// The project as staleness sees it.
fn project_now<S: Session>(s: &S) -> Result<ProjectNow, ErrorObject> {
    let tracks = s.tracks();
    let master = s.master();
    let mut hashes = BTreeMap::new();
    for id in std::iter::once(master).chain(tracks.iter().map(|t| t.id)) {
        hashes.insert(id, fnv(s.audio_chunk(id)?.as_bytes()));
    }
    Ok((hashes, Routing::of(master, &tracks), tracks))
}

fn invalid(message: impl Into<String>, fix: &str) -> ErrorObject {
    mort_session::invalid(message, None, fix)
}

#[derive(Deserialize)]
struct MeasureParams {
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
    #[serde(default)]
    tracks: Option<Vec<TrackId>>,
    #[serde(default)]
    mode: Mode,
    #[serde(default)]
    stale: bool,
}

/// What to measure: the span and the tracks, buses and master.
fn plan<S: Session>(
    s: &S,
    map: &MixMap,
    p: &MeasureParams,
) -> Result<Option<(Span, Vec<TrackId>)>, ErrorObject> {
    let given = match (p.start, p.end) {
        (Some(a), Some(b)) => {
            check_range(a, b)?;
            Some(Span::new(a, b))
        }
        (None, None) => None,
        _ => {
            return Err(invalid(
                "a range needs both a start and an end",
                "give both, or neither with `stale`",
            ));
        }
    };
    let (hashes, routing, tracks) = project_now(s)?;
    let master = s.master();
    let (span, chosen): (Span, BTreeSet<TrackId>) = if p.stale {
        let now = Now {
            hashes,
            routing: routing.clone(),
            tempo: s.tempo_map(),
        };
        let mut stale = map.stale(&now, given);
        if let Some(only) = &p.tracks {
            stale.retain(|t, _| only.contains(t));
        }
        let Some(span) = stale.values().flatten().copied().reduce(|a, b| a.union(b)) else {
            return Ok(None);
        };
        (span, stale.into_keys().collect())
    } else {
        let span = given.ok_or_else(|| {
            invalid(
                "measure needs a range",
                "give `start` and `end`, or `stale: true` to re-measure what is stale",
            )
        })?;
        let chosen = match &p.tracks {
            Some(ts) => ts.iter().copied().collect(),
            None => tracks.iter().map(|t| t.id).collect(),
        };
        (span, chosen)
    };
    // The buses each track reaches change with it, and the master hears
    // everything.
    let mut targets: BTreeSet<TrackId> = chosen
        .iter()
        .flat_map(|t| routing.downstream(*t))
        .chain(chosen.iter().copied())
        .collect();
    targets.insert(master);
    for t in &targets {
        s.track(*t)?;
    }
    // Project order, master first.
    let order: Vec<TrackId> = std::iter::once(master)
        .chain(tracks.iter().map(|t| t.id))
        .filter(|t| targets.contains(t))
        .collect();
    Ok(Some((span, order)))
}

struct Before {
    tracks: Vec<TrackMeta>,
    hashes: BTreeMap<TrackId, u64>,
    routing: Routing,
    tempo: u64,
    state: u64,
    taken: u64,
}

/// The `measure` job. See the module docs.
struct MeasureJob {
    span: Span,
    targets: Vec<TrackId>,
    mode: Mode,
    store: Shared,
    run: u64,
    before: Option<Before>,
    started: Instant,
}

/// How far before its range a measure starts, so that the analyzer's
/// windows hold the music before the range and not silence (ticket 0017).
/// A render can afford the whole short-term window; playback, which the
/// person sits through, only the low bands' window.
const PRE_ROLL_RENDER: f64 = 3.0;
const PRE_ROLL_PLAY: f64 = 0.7;

impl MeasureJob {
    /// Where the run starts: the range's start less the pre-roll, but not
    /// before the project's start, where the silence before is real. The
    /// pre-roll is whole frames, so that frames still start on the range's
    /// start.
    fn from(&self) -> f64 {
        let pre = match self.mode {
            Mode::Render => PRE_ROLL_RENDER,
            Mode::Play => PRE_ROLL_PLAY,
        }
        .min(self.span.start.max(0.0));
        let frames = (pre / 0.05 + 1e-9).floor();
        self.span.start - frames * 0.05
    }

    /// Whether a frame's short-term window reaches back before the run's
    /// start into audio the analyzer did not hear.
    fn short_term_unknown(&self, start: f64) -> bool {
        let from = self.from();
        from > 0.0 && start + 0.05 < from + 3.0 - 1e-6
    }

    fn finish<S: Session>(&mut self, s: &mut S) -> JobPoll {
        let Some(before) = self.before.take() else {
            return JobPoll::Failed(ErrorObject::internal("measure finished before it started"));
        };
        let mut frames = s.take_frames(self.run);
        let mut measured: BTreeMap<TrackId, Vec<TimedFrame>> = BTreeMap::new();
        let mut names: BTreeMap<TrackId, String> = BTreeMap::new();
        let mut unmeasured = Vec::new();
        for &t in &self.targets {
            let Ok(info) = s.track(t) else {
                unmeasured.push(json!({ "track": t, "why": "the track was removed" }));
                continue;
            };
            names.insert(t, info.name.clone());
            let why = match info.fx.iter().find(|f| is_probe(f)) {
                None => Some("it has no probe"),
                Some(fx) if !fx.enabled => Some("its probe is bypassed"),
                Some(fx) => match probe_slot(s, fx.id) {
                    None => Some(
                        "its probe is not attached to the bridge (FX run in a separate process cannot attach)",
                    ),
                    Some(slot) => match frames.remove(&slot) {
                        Some(recs) if !recs.is_empty() => {
                            measured.insert(
                                t,
                                recs.iter()
                                    .map(|r| {
                                        let mut frame = r.frame;
                                        if self.short_term_unknown(r.start) {
                                            frame.short_term_lufs = f32::NAN;
                                        }
                                        TimedFrame {
                                            start: r.start,
                                            frame,
                                        }
                                    })
                                    .collect(),
                            );
                            None
                        }
                        _ if info.mute => Some(
                            "it is muted, and REAPER does not run a muted track's FX (Preferences > Audio: Do not process muted tracks)",
                        ),
                        _ => Some("its probe sent no frames"),
                    },
                },
            };
            if let Some(why) = why {
                unmeasured.push(json!({ "track": t, "name": info.name, "why": why }));
            }
        }
        if measured.is_empty() {
            return JobPoll::Failed(ErrorObject::new(
                ErrorKind::Reaper,
                "no probe sent any frames",
                None,
                "check the probes with `mort probes`; FX must run in REAPER's own process",
            ));
        }
        let capture = Capture {
            id: 0,
            span: self.span,
            taken: before.taken,
            state: before.state,
            source: self.mode.into(),
            file: None,
            tempo: before.tempo,
            inputs: measured
                .keys()
                .map(|t| (*t, before.routing.upstream(*t).into_iter().collect()))
                .collect(),
            hashes: before.hashes,
            tracks: before.tracks,
        };
        let mut st = self.store.borrow_mut();
        let span = capture.span;
        let added = st.map.add(capture, measured);
        st.note_capture(added.frames.keys().copied().collect(), span);
        st.save(false);
        let list: Vec<Value> = self
            .targets
            .iter()
            .filter_map(|t| {
                let n = added.frames.get(t)?;
                Some(json!({ "track": t, "name": names.get(t), "frames": n }))
            })
            .collect();
        JobPoll::Done(json!({
            "capture": added.id,
            "start": self.span.start,
            "end": self.span.end,
            "mode": self.mode,
            "run": self.run,
            "seconds": self.started.elapsed().as_secs_f64(),
            "measured": list,
            "unmeasured": unmeasured,
            "superseded": added.superseded,
            "forgotten": added.dropped,
            "saved": st.dir(),
            "save_error": st.save_error,
        }))
    }
}

impl<S: Session> Job<S> for MeasureJob {
    fn start(&mut self, s: &mut S) -> Result<(), ErrorObject> {
        let (hashes, routing, _) = project_now(s)?;
        self.before = Some(Before {
            tracks: track_meta(s)?,
            hashes,
            routing,
            tempo: mort_map::tempo_fingerprint(s.tempo_map(), self.span.end),
            state: s.state(),
            taken: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        });
        self.started = Instant::now();
        self.run = s.begin_run();
        match self.mode {
            Mode::Render => {
                let r = s.render(self.from(), self.span.end);
                s.end_run(self.run);
                r
            }
            Mode::Play => s.play(self.from(), self.span.end),
        }
    }

    fn poll(&mut self, s: &mut S) -> JobPoll {
        if self.mode == Mode::Play {
            let at = s.position();
            if s.is_playing() && at < self.span.end {
                return JobPoll::Running(Some(json!({ "position": at })));
            }
            if s.is_playing() {
                s.stop();
            } else if at < self.span.end {
                // Stopped early: keep what played.
                if at <= self.span.start {
                    s.end_run(self.run);
                    return JobPoll::Failed(invalid(
                        "playback stopped before the range started",
                        "measure again, and leave the transport running",
                    ));
                }
                self.span.end = at;
            }
            s.end_run(self.run);
        }
        self.finish(s)
    }

    fn cancel(&mut self, s: &mut S) {
        s.stop();
        s.end_run(self.run);
        let _ = s.take_frames(self.run);
    }
}

#[derive(Deserialize)]
struct RangeQuery {
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
    #[serde(default)]
    track: Option<TrackId>,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    step: Option<f64>,
    #[serde(default)]
    kinds: Option<Vec<findings::Kind>>,
    #[serde(default)]
    all: bool,
    #[serde(default)]
    a: Option<u64>,
    #[serde(default)]
    b: Option<u64>,
}

/// What analysis needs to know about each track, the master first. A
/// track's gain to its bus is its fader times pan's effect on mid (a
/// balance control), or 0 when muted.
pub(crate) fn track_meta<S: Session>(s: &S) -> Result<Vec<TrackMeta>, ErrorObject> {
    let master = s.track(s.master())?;
    let mut out = vec![TrackMeta {
        id: master.id,
        name: master.name,
        gain: 1.0,
        bus: None,
        notes: vec![],
    }];
    for t in s.tracks() {
        let mut notes = Vec::new();
        for item in s.items(t.id)?.into_iter().filter(|i| i.midi) {
            notes.extend(s.notes(item.id)?);
        }
        let mid = f64::midpoint((1.0 - t.pan).min(1.0), (1.0 + t.pan).min(1.0));
        out.push(TrackMeta {
            id: t.id,
            name: t.name,
            gain: if t.mute { 0.0 } else { t.volume * mid },
            bus: t.main_send.then(|| t.parent.unwrap_or(master.id)),
            notes,
        });
    }
    Ok(out)
}

/// The range a query covers: the one given, or everything the map holds.
fn query_range(map: &MixMap, q: &RangeQuery) -> Result<Span, ErrorObject> {
    match (q.start, q.end) {
        (Some(a), Some(b)) => check_range(a, b).map(|()| Span::new(a, b)),
        (None, None) => map
            .captures()
            .map(|c| c.span)
            .reduce(|a, b| a.union(b))
            .ok_or_else(|| invalid("the map is empty", "measure first: `mort measure <range>`")),
        _ => Err(invalid(
            "a range needs both a start and an end",
            "give both, or neither for everything measured",
        )),
    }
}

/// Runs `f` over a view of the map for the query's range.
fn with_view<S: Session, T>(
    s: &S,
    store: &Shared,
    q: &RangeQuery,
    f: impl FnOnce(&View<'_>, &mut Extra<'_>) -> Result<T, ErrorObject>,
) -> Result<T, ErrorObject> {
    let mut guard = store.borrow_mut();
    guard.sync(s);
    let st = &mut *guard;
    let range = query_range(&st.map, q)?;
    let (hashes, routing, _) = project_now(s)?;
    let now = Now {
        hashes,
        routing,
        tempo: s.tempo_map(),
    };
    let view = View {
        map: &st.map,
        now: &now,
        master: s.master(),
        tracks: track_meta(s)?,
        sections: sections(&s.regions(), range),
    };
    let mut extra = Extra {
        overrides: &st.overrides,
        cache: &mut st.findings_cache,
        state: s.state(),
        version: st.feed.version(),
    };
    f(&view, &mut extra)
}

/// What a query needs besides the view.
pub(crate) struct Extra<'a> {
    overrides: &'a Overrides,
    cache: &'a mut FindingsCache,
    state: u64,
    version: u64,
}

/// What a section's findings depend on: the state (staleness, gains,
/// MIDI), the thresholds, and which captures cover it where.
fn section_sig(map: &MixMap, span: Span, state: u64, thresholds: u64) -> u64 {
    let mut bytes = Vec::new();
    bytes.extend(state.to_le_bytes());
    bytes.extend(thresholds.to_le_bytes());
    for t in map.tracks() {
        // Only the part inside the section: a cover split elsewhere leaves
        // the section's data as it was.
        for c in map.covers(t) {
            let Some(part) = c.span.intersect(span) else {
                continue;
            };
            bytes.extend(t.0.0.to_le_bytes());
            bytes.extend(c.capture.to_le_bytes());
            bytes.extend(part.start.to_bits().to_le_bytes());
            bytes.extend(part.end.to_bits().to_le_bytes());
        }
    }
    fnv(&bytes)
}

/// The view's findings over the project's thresholds, shown at the
/// project's minimum severity, recomputing only sections whose inputs
/// changed (ADR 0012 §8). Also returns the sections recomputed.
fn cached_findings(v: &View<'_>, x: &mut Extra<'_>) -> (Vec<findings::Finding>, Vec<String>) {
    let lim = x.overrides.limits();
    let thresholds = fnv(x.overrides.to_toml().as_bytes());
    let mut recomputed = Vec::new();
    let per = v
        .sections
        .iter()
        .map(|sec| {
            let sig = section_sig(v.map, sec.span, x.state, thresholds);
            let key = (
                sec.span.start.to_bits(),
                sec.span.end.to_bits(),
                sec.name.clone(),
            );
            match x.cache.get(&key) {
                Some((g, fs)) if *g == sig => fs.clone(),
                _ => {
                    let fs = findings::section_findings(v, &lim, sec);
                    x.cache.insert(key, (sig, fs.clone()));
                    recomputed.push(sec.name.clone());
                    fs
                }
            }
        })
        .collect();
    let mut all = findings::assemble(per, findings::jump_findings(v, &lim));
    let min = x.overrides.min_severity();
    all.retain(|f| f.severity >= min);
    (all, recomputed)
}

#[derive(Deserialize)]
struct ThresholdSet {
    name: String,
    #[serde(default)]
    value: Option<f64>,
    #[serde(default)]
    reset: bool,
}

/// Every threshold with its default, the project's value and the reason,
/// and where the overrides live.
fn thresholds_json(store: &Store) -> Value {
    let lim = store.overrides.limits();
    let rows: Vec<Value> = mort_map::thresholds::TABLE
        .iter()
        .map(|t| {
            let value = lim.get(t.name).unwrap_or(t.value);
            json!({
                "name": t.name,
                "default": t.value,
                "value": value,
                "overridden": store.overrides.values.contains_key(t.name),
                "overridable": t.overridable(),
                "unit": t.unit,
                "min": t.min,
                "max": t.max,
                "reason": t.reason,
            })
        })
        .collect();
    json!({
        "thresholds": rows,
        "min_severity": store.overrides.min_severity(),
        "file": store.thresholds_file(),
        "error": store.thresholds_error,
        "version": store.feed.version(),
    })
}

/// `map.view` (ADR 0012 §8): the whole map as a grid of tracks against
/// time, for the view. Rows are the master and then the tracks in project
/// order, with their folder depth. A cell holds how much of it is covered
/// (`c`), the source (`s`) and time (`t`) of its newest data, why that
/// data is stale (`x`), and the kinds (`k`) and worst severity (`f`) of
/// findings on it. Cells with nothing in them are `null`.
#[expect(
    clippy::many_single_char_names,
    clippy::too_many_lines,
    reason = "one grid"
)]
fn map_view<S: Session>(
    s: &S,
    v: &View<'_>,
    x: &mut Extra<'_>,
    step: Option<f64>,
) -> Result<Value, ErrorObject> {
    let (Some(first), Some(last)) = (v.sections.first(), v.sections.last()) else {
        return Err(invalid("the range is empty", "give a range"));
    };
    let range = first.span.union(last.span);
    let step = step.unwrap_or(0.5).max(0.05);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a few thousand columns"
    )]
    let n = (((range.end - range.start) / step).ceil() as usize).clamp(1, 20_000);
    let (fs, recomputed) = cached_findings(v, x);
    let tracks = s.tracks();
    let master = s.master();
    let parents: std::collections::HashMap<TrackId, Option<TrackId>> =
        tracks.iter().map(|t| (t.id, t.parent)).collect();
    let depth = |t: TrackId| {
        let mut d = 0;
        let mut at = parents.get(&t).copied().flatten();
        while let Some(p) = at {
            d += 1;
            at = parents.get(&p).copied().flatten();
        }
        d
    };
    let mut rows = Vec::new();
    // Staleness is per track and capture, not per cell.
    let mut stale_of: std::collections::HashMap<(TrackId, u64), Option<mort_map::Stale>> =
        std::collections::HashMap::new();
    let all = std::iter::once(s.track(master)?).chain(tracks.iter().cloned());
    for t in all {
        let unmeasured = if !t.fx.iter().any(is_probe) {
            Some("no probe")
        } else if t.mute {
            Some("muted")
        } else {
            None
        };
        let segs = v.map.segments(t.id);
        #[expect(clippy::cast_precision_loss, reason = "column indices are small")]
        let cells: Vec<Value> = (0..n)
            .map(|k| {
                let cell = Span::new(
                    range.start + k as f64 * step,
                    range.start + (k + 1) as f64 * step,
                );
                let mut covered = 0.0;
                let mut newest: Option<&mort_map::Capture> = None;
                for seg in &segs {
                    if let Some(o) = seg.span.intersect(cell) {
                        covered += o.end - o.start;
                        let c = v.map.capture(seg.capture);
                        if c.is_some_and(|c| {
                            newest.is_none_or(|n| (c.taken, c.id) > (n.taken, n.id))
                        }) {
                            newest = c;
                        }
                    }
                }
                let Some(c) = newest else {
                    return Value::Null;
                };
                let mut out = json!({
                    "c": (100.0 * covered / step).round() / 100.0,
                    "s": c.source,
                    "t": c.taken,
                });
                let why = *stale_of
                    .entry((t.id, c.id))
                    .or_insert_with(|| v.map.staleness(t.id, c.id, v.now));
                if let Some(why) = why {
                    out["x"] = json!(why);
                }
                let here: Vec<&findings::Finding> = fs
                    .iter()
                    .filter(|f| f.tracks.contains(&t.id) && f.span.intersect(cell).is_some())
                    .collect();
                if let Some(worst) = here.iter().map(|f| f.severity).reduce(f64::max) {
                    let kinds: BTreeSet<&str> = here.iter().map(|f| f.kind.name()).collect();
                    out["f"] = json!(worst);
                    out["k"] = json!(kinds);
                }
                out
            })
            .collect();
        rows.push(json!({
            "track": t.id,
            "name": t.name,
            "depth": if t.id == master { 0 } else { depth(t.id) },
            "master": t.id == master,
            "unmeasured": unmeasured,
            "cells": cells,
        }));
    }
    // Bar starts across the range, for the ruler.
    let tm = s.tempo_map();
    let mut bars = Vec::new();
    let first = tm.bar_at_qn(tm.seconds_to_qn(range.start));
    for bar in first..first.saturating_add(10_000) {
        let t = tm.qn_to_seconds(tm.bar_start_qn(bar));
        if t >= range.end {
            break;
        }
        if t >= range.start - 1e-9 {
            bars.push(json!([bar, t]));
        }
    }
    // The master lane: each column's highest short-term loudness and true
    // peak, skipping unknown values.
    let frames = v.frames(master, range);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "column indices"
    )]
    let lane: Vec<Value> = {
        let mut cols: Vec<(f64, f64)> = vec![(f64::NEG_INFINITY, f64::NEG_INFINITY); n];
        for (t, f) in &frames {
            let k = (((t - range.start) / step).floor() as usize).min(n - 1);
            let st = f64::from(f.short_term_lufs);
            if st.is_finite() {
                cols[k].0 = cols[k].0.max(st);
            }
            cols[k].1 = cols[k].1.max(f64::from(f.true_peak_dbtp));
        }
        cols.into_iter()
            .map(|(st, tp)| {
                if tp.is_finite() {
                    json!([
                        if st.is_finite() {
                            json!((st * 10.0).round() / 10.0)
                        } else {
                            Value::Null
                        },
                        (tp * 10.0).round() / 10.0
                    ])
                } else {
                    Value::Null
                }
            })
            .collect()
    };
    Ok(json!({
        "version": x.version,
        "start": range.start,
        "end": range.end,
        "step": step,
        "columns": n,
        "bars": bars,
        "lane": lane,
        "headroom": x.overrides.limits().headroom,
        "sections": v.sections,
        "rows": rows,
        "findings": fs,
        "now": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        "recomputed": recomputed,
    }))
}

/// The two captures to compare: those given, or the latest capture over
/// the range and the one before it that also covers it.
fn pick_pair(map: &MixMap, q: &RangeQuery) -> Result<(u64, u64), ErrorObject> {
    let range = match (q.start, q.end) {
        (Some(a), Some(b)) => Some(Span::new(a, b)),
        _ => None,
    };
    let over = |id: u64| {
        map.capture(id)
            .is_some_and(|c| range.is_none_or(|r| c.span.intersect(r).is_some()))
    };
    let ids: Vec<u64> = map
        .all_captures()
        .map(|c| c.id)
        .filter(|c| over(*c))
        .collect();
    let b = match q.b {
        Some(b) if map.capture(b).is_some() => b,
        Some(b) => {
            return Err(invalid(
                format!("no capture {b}"),
                "list captures with `mort map`; the last 8 superseded ones are kept",
            ));
        }
        None => *ids.last().ok_or_else(|| {
            invalid(
                "no capture covers the range",
                "measure it: `mort measure <range>`",
            )
        })?,
    };
    let a = match q.a {
        Some(a) if map.capture(a).is_some() => a,
        Some(a) => {
            return Err(invalid(
                format!("no capture {a}"),
                "list captures with `mort map`; the last 8 superseded ones are kept",
            ));
        }
        None => *ids.iter().rev().find(|c| **c < b).ok_or_else(|| {
            invalid(
                "only one capture covers the range",
                "measure again after an edit, then compare",
            )
        })?,
    };
    Ok((a, b))
}

/// `map.compare`: the map as of two captures, compared over the range
/// (the second capture's, by default). Each side is read with the gains
/// and MIDI its capture recorded.
fn compare_query<S: Session>(s: &S, st: &Shared, q: &RangeQuery) -> Result<Value, ErrorObject> {
    let mut store = st.borrow_mut();
    store.sync(s);
    let (first, second) = pick_pair(&store.map, q)?;
    let range = match (q.start, q.end) {
        (Some(x), Some(y)) => check_range(x, y).map(|()| Span::new(x, y))?,
        _ => store.map.capture(second).map(|c| c.span).expect("picked"),
    };
    let (hashes, routing, _) = project_now(s)?;
    let now = Now {
        hashes,
        routing,
        tempo: s.tempo_map(),
    };
    let live = track_meta(s)?;
    let secs = sections(&s.regions(), range);
    let view_of = |map: &'_ MixMap, id: u64| -> Vec<TrackMeta> {
        map.capture(id)
            .map(|c| c.tracks.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| live.clone())
    };
    let (ma, mb) = (
        store.map.as_of(first, range),
        store.map.as_of(second, range),
    );
    let va = View {
        map: &ma,
        now: &now,
        master: s.master(),
        tracks: view_of(&store.map, first),
        sections: secs.clone(),
    };
    let vb = View {
        map: &mb,
        now: &now,
        master: s.master(),
        tracks: view_of(&store.map, second),
        sections: secs,
    };
    let mut c = mort_map::compare::compare(&va, &vb, &store.overrides.limits());
    let min = store.overrides.min_severity();
    c.appeared.retain(|f| f.severity >= min);
    c.gone.retain(|f| f.severity >= min);
    c.kept.retain(|k| k.finding.severity >= min || k.was >= min);
    let mut v = serde_json::to_value(&c).expect("comparisons serialise");
    v["a"] = json!(first);
    v["b"] = json!(second);
    v["start"] = json!(range.start);
    v["end"] = json!(range.end);
    Ok(v)
}

fn the_track<S: Session>(s: &S, q: &RangeQuery) -> Result<TrackId, ErrorObject> {
    let t = q
        .track
        .ok_or_else(|| invalid("name a track", "give `track`"))?;
    s.track(t)?;
    Ok(t)
}

/// Registers `thresholds.get` and `thresholds.set` (ticket 0019).
fn register_thresholds<S: Session + 'static>(d: &mut Dispatcher<S>, store: &Shared) {
    let st = Rc::clone(store);
    read(d, "thresholds.get", move |s: &S, _: NoParams| {
        let mut store = st.borrow_mut();
        store.sync(s);
        Ok(thresholds_json(&store))
    });
    let st = Rc::clone(store);
    d.register(
        "thresholds.set",
        false,
        move |ctx: &mut Ctx<'_, S>, params| {
            let r = parse::<ThresholdSet>("thresholds.set", params).and_then(|p| {
                let mut store = st.borrow_mut();
                store.sync(&*ctx.session);
                let value = if p.reset {
                    None
                } else {
                    Some(p.value.ok_or_else(|| {
                        invalid(
                            format!("give a `value` for `{}`, or `reset: true`", p.name),
                            "see the thresholds with `mort thresholds`",
                        )
                    })?)
                };
                store.set_threshold(&p.name, value).map_err(|e| {
                    mort_session::invalid(
                        e,
                        Some(p.name.clone()),
                        "see the thresholds with `mort thresholds`",
                    )
                })?;
                let mut v = thresholds_json(&store);
                v["state"] = json!(ctx.session.state());
                Ok(v)
            });
            Outcome::Value(r)
        },
    );
}

/// Registers the queries and findings of ticket 0013 over `store`.
fn register_queries<S: Session + 'static>(d: &mut Dispatcher<S>, store: &Shared) {
    let st = Rc::clone(store);
    read(d, "map.findings", move |s: &S, q: RangeQuery| {
        with_view(s, &st, &q, |v, x| {
            let (mut fs, recomputed) = if q.all {
                (
                    findings::findings(v, &mort_map::thresholds::Limits::ALL),
                    vec![],
                )
            } else {
                cached_findings(v, x)
            };
            if let Some(kinds) = &q.kinds {
                fs.retain(|f| kinds.contains(&f.kind));
            }
            Ok(json!({ "sections": v.sections, "findings": fs, "recomputed": recomputed }))
        })
    });
    let st = Rc::clone(store);
    read(d, "map.summary", move |s: &S, q: RangeQuery| {
        with_view(s, &st, &q, |v, x| {
            let (fs, _) = cached_findings(v, x);
            Ok(json!({ "sections": query::summary(v, &fs), "findings": fs.len() }))
        })
    });
    let st = Rc::clone(store);
    read(d, "map.view", move |s: &S, q: RangeQuery| {
        with_view(s, &st, &q, |v, x| map_view(s, v, x, q.step))
    });
    let st = Rc::clone(store);
    read(d, "map.track", move |s: &S, q: RangeQuery| {
        let t = the_track(s, &q)?;
        with_view(s, &st, &q, |v, _: &mut Extra<'_>| {
            let name = v.meta(t).map(|m| m.name.clone());
            Ok(json!({ "track": t, "name": name, "sections": query::track(v, t) }))
        })
    });
    let st = Rc::clone(store);
    read(d, "map.frames", move |s: &S, q: RangeQuery| {
        let t = the_track(s, &q)?;
        if q.features.is_empty() {
            return Err(invalid(
                "name the features to read",
                &format!(
                    "give `features`: any of {}, band:<Hz>, side:<Hz>",
                    query::SCALARS.join(", ")
                ),
            ));
        }
        with_view(s, &st, &q, |v, _: &mut Extra<'_>| {
            let range = v.sections.first().map_or(Span::new(0.0, 0.0), |a| {
                a.span.union(v.sections.last().map_or(a.span, |b| b.span))
            });
            let r = query::frames(v, t, range, &q.features, q.step.unwrap_or(0.05))
                .map_err(|e| invalid(e, "see `mort frames --help`"))?;
            Ok(
                json!({ "track": t, "columns": r.columns, "rows": r.rows, "stale": v.stale(t, range) }),
            )
        })
    });
    let st = Rc::clone(store);
    read(d, "map.compare", move |s: &S, q: RangeQuery| {
        compare_query(s, &st, &q)
    });
}

/// Registers `measure` and `map.status` (ticket 0012), and the queries
/// `map.summary`, `map.track`, `map.frames`, `map.findings` and
/// (ticket 0013), `map.compare` (ticket 0014), and `thresholds.get` and
/// `thresholds.set` (ticket 0019).
pub fn register_measure<S: Session + 'static>(d: &mut Dispatcher<S>) {
    register_measure_with_feed(d, Arc::new(Feed::new()));
}

/// As [`register_measure`], announcing changes on `feed`, which the
/// server answers `map.watch` from (ADR 0012 §8).
pub fn register_measure_with_feed<S: Session + 'static>(d: &mut Dispatcher<S>, feed: Arc<Feed>) {
    let shared: Shared = Rc::new(RefCell::new(Store::new(feed)));
    register_queries(d, &shared);
    register_thresholds(d, &shared);
    let mut auditions = crate::audition::Auditioner::new(Rc::clone(&shared));
    d.on_tick(move |s| auditions.tick(s));
    let st = Rc::clone(&shared);
    d.register("measure", true, move |ctx: &mut Ctx<'_, S>, params| {
        let p: MeasureParams = match parse("measure", params) {
            Ok(p) => p,
            Err(e) => return Outcome::Value(Err(e)),
        };
        let s = &mut *ctx.session;
        st.borrow_mut().sync(s);
        let planned = plan(s, &st.borrow().map, &p);
        let (span, targets) = match planned {
            Err(e) => return Outcome::Value(Err(e)),
            Ok(None) => {
                return Outcome::Value(Ok(json!({ "nothing_stale": true })));
            }
            Ok(Some(x)) => x,
        };
        if let Err(e) = ensure_probes(s, &targets) {
            return Outcome::Value(Err(e));
        }
        Outcome::Job(Box::new(MeasureJob {
            span,
            targets,
            mode: p.mode,
            store: Rc::clone(&st),
            run: 0,
            before: None,
            started: Instant::now(),
        }))
    });
    let st = shared;
    read(d, "map.status", move |s: &S, _: NoParams| {
        let mut store = st.borrow_mut();
        store.sync(s);
        let (hashes, routing, tracks) = project_now(s)?;
        let now = Now {
            hashes,
            routing,
            tempo: s.tempo_map(),
        };
        let mut cov = store.map.coverage(&now);
        let master = s.master();
        let all = std::iter::once((master, "MASTER".to_owned()))
            .chain(tracks.iter().map(|t| (t.id, t.name.clone())));
        let mut rows = Vec::new();
        let mut unmeasured = Vec::new();
        for (id, name) in all {
            match cov.remove(&id) {
                Some(spans) => rows.push(json!({ "track": id, "name": name, "spans": spans })),
                None => unmeasured.push(json!({ "track": id, "name": name })),
            }
        }
        let captures: Vec<Value> = store
            .map
            .captures()
            .map(|c| {
                let tracks = store
                    .map
                    .tracks()
                    .filter(|t| store.map.segments(*t).iter().any(|g| g.capture == c.id))
                    .count();
                json!({
                    "id": c.id,
                    "start": c.span.start,
                    "end": c.span.end,
                    "taken": c.taken,
                    "source": c.source,
                    "state": c.state,
                    "tracks": tracks,
                })
            })
            .collect();
        Ok(json!({
            "saved": store.dir(),
            "save_error": store.save_error,
            "captures": captures,
            "tracks": rows,
            "unmeasured": unmeasured,
        }))
    });
}
