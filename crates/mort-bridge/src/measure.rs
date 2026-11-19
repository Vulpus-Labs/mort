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

use crate::dispatch::{Ctx, Dispatcher, Job, JobPoll, Outcome};
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
    /// Grows with every change to the map or the thresholds (ADR 0012 §8).
    pub(crate) version: u64,
}

impl Store {
    fn new() -> Self {
        Self {
            map: MixMap::new(),
            ..Self::default()
        }
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
        self.version += 1;
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
        self.version += 1;
        Ok(())
    }

    /// Follows the project file; see the module docs.
    pub(crate) fn sync<S: Session>(&mut self, s: &S) {
        let path = s.project().path.map(PathBuf::from);
        if path == self.path {
            return;
        }
        self.path = path;
        self.version += 1;
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
        let added = st.map.add(capture, measured);
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
    f: impl FnOnce(&View<'_>, &Overrides) -> Result<T, ErrorObject>,
) -> Result<T, ErrorObject> {
    let mut st = store.borrow_mut();
    st.sync(s);
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
    f(&view, &st.overrides)
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
        "version": store.version,
    })
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

/// Registers the queries and findings of ticket 0013 over `store`.
fn register_queries<S: Session + 'static>(d: &mut Dispatcher<S>, store: &Shared) {
    let st = Rc::clone(store);
    read(d, "map.findings", move |s: &S, q: RangeQuery| {
        with_view(s, &st, &q, |v, o| {
            let lim = if q.all {
                mort_map::thresholds::Limits::ALL
            } else {
                o.limits()
            };
            let mut fs = findings::findings(v, &lim);
            if !q.all {
                fs.retain(|f| f.severity >= o.min_severity());
            }
            if let Some(kinds) = &q.kinds {
                fs.retain(|f| kinds.contains(&f.kind));
            }
            Ok(json!({ "sections": v.sections, "findings": fs }))
        })
    });
    let st = Rc::clone(store);
    read(d, "map.summary", move |s: &S, q: RangeQuery| {
        with_view(s, &st, &q, |v, o| {
            let mut fs = findings::findings(v, &o.limits());
            fs.retain(|f| f.severity >= o.min_severity());
            Ok(json!({ "sections": query::summary(v, &fs), "findings": fs.len() }))
        })
    });
    let st = Rc::clone(store);
    read(d, "map.track", move |s: &S, q: RangeQuery| {
        let t = the_track(s, &q)?;
        with_view(s, &st, &q, |v, _| {
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
        with_view(s, &st, &q, |v, _| {
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

/// Registers `measure` and `map.status` (ticket 0012), and the queries
/// `map.summary`, `map.track`, `map.frames`, `map.findings` and
/// (ticket 0013), `map.compare` (ticket 0014), and `thresholds.get` and
/// `thresholds.set` (ticket 0019).
pub fn register_measure<S: Session + 'static>(d: &mut Dispatcher<S>) {
    let shared: Shared = Rc::new(RefCell::new(Store::new()));
    register_queries(d, &shared);
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
