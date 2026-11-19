//! The bridge's RPC methods over a [`Session`].
//!
//! Every read returns the project's `state` alongside its result, so a
//! client can send it back as `expect_state` (ADR 0004 §2).

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use mort_proto::model::{ControlChange, EnvelopePoint, EnvelopeTarget, Note};
use mort_proto::{ErrorObject, FxId, Position, Range, TrackId};
use mort_session::Session;

use crate::dispatch::{Ctx, Dispatcher, Job, JobPoll, Outcome};
use crate::{probes, runs};
use mort_link::Slot;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

pub(crate) fn parse<T: DeserializeOwned>(method: &str, params: Value) -> Result<T, ErrorObject> {
    let params = if params.is_null() { json!({}) } else { params };
    serde_json::from_value(params).map_err(|e| ErrorObject::invalid_params(method, &e.to_string()))
}

pub(crate) fn read<S, P: DeserializeOwned>(
    d: &mut Dispatcher<S>,
    name: &'static str,
    f: impl Fn(&S, P) -> Result<Value, ErrorObject> + 'static,
) where
    S: Session + 'static,
{
    d.register(name, false, move |ctx: &mut Ctx<'_, S>, params| {
        Outcome::Value(parse(name, params).and_then(|p| {
            let mut v = f(ctx.session, p)?;
            v["state"] = json!(ctx.session.state());
            Ok(v)
        }))
    });
}

#[derive(Deserialize)]
pub(crate) struct NoParams {}

#[derive(Deserialize)]
struct TrackParams {
    track: TrackId,
}

#[derive(Deserialize)]
struct FxParams {
    fx: FxId,
}

#[derive(Deserialize)]
struct ActionsFind {
    text: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct ConvertParams {
    #[serde(default)]
    to_seconds: Vec<String>,
    #[serde(default)]
    to_musical: Vec<f64>,
}

fn to_value<T: serde::Serialize>(t: &T) -> Value {
    serde_json::to_value(t).expect("protocol types serialise")
}

/// Registers the read methods of ticket 0005.
pub fn register_reads<S: Session + 'static>(d: &mut Dispatcher<S>) {
    read(d, "project.get", |s: &S, _: NoParams| {
        Ok(json!({ "project": s.project(), "regions": s.regions() }))
    });
    read(d, "tracks.list", |s: &S, _: NoParams| {
        Ok(json!({ "master": s.track(s.master())?, "tracks": s.tracks() }))
    });
    read(d, "track.get", |s: &S, p: TrackParams| {
        Ok(json!({ "track": s.track(p.track)? }))
    });
    read(d, "fx.params", |s: &S, p: FxParams| {
        let (track, fx) = s.fx(p.fx)?;
        Ok(json!({ "track": track, "fx": fx, "params": s.params(p.fx)? }))
    });
    read(d, "items.list", |s: &S, p: TrackParams| {
        Ok(json!({ "items": to_value(&s.items(p.track)?) }))
    });
    read(d, "actions.find", |s: &S, p: ActionsFind| {
        let found: Vec<Value> = s
            .find_actions(&p.text, p.limit.unwrap_or(50))
            .into_iter()
            .map(|(id, name)| json!({ "id": id, "name": name }))
            .collect();
        Ok(json!({ "actions": found }))
    });
    read(d, "time.convert", |s: &S, p: ConvertParams| {
        let map = s.tempo_map();
        let mut prev = None;
        let mut seconds = Vec::with_capacity(p.to_seconds.len());
        for text in &p.to_seconds {
            let bad = |e: mort_proto::PositionError| {
                mort_session::invalid(e.to_string(), Some(text.clone()), "fix the position")
            };
            if text.contains("..") {
                let (a, b) = text
                    .parse::<Range>()
                    .and_then(|r| r.resolve(map, text))
                    .map_err(bad)?;
                seconds.push(json!([a, b]));
                prev = Some(b);
            } else {
                let at = text
                    .parse::<Position>()
                    .and_then(|pos| pos.resolve(map, prev, text))
                    .map_err(bad)?;
                seconds.push(json!(at));
                prev = Some(at);
            }
        }
        let musical: Vec<String> = p
            .to_musical
            .iter()
            .map(|t| Position::format_musical(map, *t))
            .collect();
        Ok(json!({ "seconds": seconds, "musical": musical }))
    });
}

// --- Edits (ticket 0006) ----------------------------------------------------

fn edit<S, P: DeserializeOwned>(
    d: &mut Dispatcher<S>,
    name: &'static str,
    f: impl Fn(&mut S, P) -> Result<Value, ErrorObject> + 'static,
) where
    S: Session + 'static,
{
    d.register(name, true, move |ctx: &mut Ctx<'_, S>, params| {
        Outcome::Value(parse(name, params).and_then(|p| f(ctx.session, p)))
    });
}

#[derive(Deserialize)]
struct RegionAdd {
    name: String,
    start: f64,
    #[serde(default)]
    end: Option<f64>,
}

#[derive(Deserialize)]
struct TrackAdd {
    name: String,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    parent: Option<TrackId>,
}

#[derive(Deserialize)]
struct TrackSet {
    track: TrackId,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    volume: Option<f64>,
    #[serde(default)]
    pan: Option<f64>,
    #[serde(default)]
    mute: Option<bool>,
    #[serde(default)]
    solo: Option<bool>,
}

#[derive(Deserialize)]
struct SendAdd {
    from: TrackId,
    to: TrackId,
}

#[derive(Deserialize)]
struct SendSet {
    from: TrackId,
    send: usize,
    #[serde(default)]
    volume: Option<f64>,
    #[serde(default)]
    pan: Option<f64>,
    #[serde(default)]
    mute: Option<bool>,
}

#[derive(Deserialize)]
struct FxAdd {
    track: TrackId,
    plugin: String,
    #[serde(default)]
    index: Option<usize>,
}

#[derive(Deserialize)]
struct FxMove {
    fx: FxId,
    index: usize,
}

#[derive(Deserialize)]
struct FxEnable {
    fx: FxId,
    enabled: bool,
}

#[derive(Deserialize)]
struct FxPreset {
    fx: FxId,
    preset: String,
}

#[derive(Deserialize)]
struct FxSetState {
    fx: FxId,
    chunk: String,
}

#[derive(Deserialize)]
struct SaveParams {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Deserialize)]
struct ParamSet {
    fx: FxId,
    #[serde(default)]
    index: Option<u32>,
    /// A parameter name, instead of `index`: matched exactly (ignoring
    /// case), or as the one name starting with it, or containing it.
    #[serde(default)]
    param: Option<String>,
    #[serde(default)]
    value: Option<f64>,
    #[serde(default)]
    text: Option<String>,
}

/// The parameter `index` names, or the one named `name`.
fn param_index<S: Session>(
    s: &S,
    fx: FxId,
    index: Option<u32>,
    name: Option<&str>,
) -> Result<u32, ErrorObject> {
    let want = match (index, name) {
        (Some(i), None) => return Ok(i),
        (None, Some(n)) => n.to_lowercase(),
        _ => {
            return Err(mort_session::invalid(
                "give `index` or `param`, not both or neither",
                Some(fx.to_string()),
                "name the parameter by its number or its name",
            ));
        }
    };
    let params = s.params(fx)?;
    if let Some(p) = params.iter().find(|p| p.name.to_lowercase() == want) {
        return Ok(p.index);
    }
    // Names that start with it, or failing that, contain it.
    let starts: Vec<_> = params
        .iter()
        .filter(|p| p.name.to_lowercase().starts_with(&want))
        .collect();
    let hits: Vec<_> = if starts.is_empty() {
        params
            .iter()
            .filter(|p| p.name.to_lowercase().contains(&want))
            .collect()
    } else {
        starts
    };
    match hits.as_slice() {
        [p] => Ok(p.index),
        [] => Err(mort_session::not_found(
            "parameter",
            format!("{fx} `{want}`"),
            "list parameters with `mort params <track> <fx>`",
        )),
        many => Err(mort_session::invalid(
            format!(
                "`{want}` matches {} parameters: {}",
                many.len(),
                many.iter()
                    .take(5)
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Some(fx.to_string()),
            "give more of the name, or its number",
        )),
    }
}

/// Registers the edit methods of ticket 0006, and `fx.state`.
pub fn register_edits<S: Session + 'static>(d: &mut Dispatcher<S>) {
    use mort_session::{NewTrack, SendChange, TrackChange};

    edit(d, "region.add", |s: &mut S, p: RegionAdd| {
        let number = s.add_region(&p.name, p.start, p.end)?;
        Ok(json!({ "number": number }))
    });
    edit(d, "track.add", |s: &mut S, p: TrackAdd| {
        let id = s.add_track(NewTrack {
            name: p.name,
            index: p.index,
            parent: p.parent,
        })?;
        Ok(json!({ "track": id }))
    });
    edit(d, "track.rm", |s: &mut S, p: TrackParams| {
        s.remove_track(p.track)?;
        Ok(json!({}))
    });
    edit(d, "track.set", |s: &mut S, p: TrackSet| {
        let change = TrackChange {
            name: p.name,
            volume: p.volume,
            pan: p.pan,
            mute: p.mute,
            solo: p.solo,
        };
        s.set_track(p.track, &change)?;
        Ok(json!({ "track": s.track(p.track)? }))
    });
    edit(d, "send.add", |s: &mut S, p: SendAdd| {
        Ok(json!({ "send": s.add_send(p.from, p.to)? }))
    });
    edit(d, "send.set", |s: &mut S, p: SendSet| {
        s.set_send(
            p.from,
            p.send,
            &SendChange {
                volume: p.volume,
                pan: p.pan,
                mute: p.mute,
            },
        )?;
        Ok(json!({}))
    });
    edit(d, "fx.add", |s: &mut S, p: FxAdd| {
        let fx = s.add_fx(p.track, &p.plugin, p.index)?;
        Ok(json!({ "fx": s.fx(fx)?.1 }))
    });
    edit(d, "fx.rm", |s: &mut S, p: FxParams| {
        s.remove_fx(p.fx)?;
        Ok(json!({}))
    });
    edit(d, "fx.move", |s: &mut S, p: FxMove| {
        s.move_fx(p.fx, p.index)?;
        Ok(json!({ "fx": s.fx(p.fx)?.1 }))
    });
    edit(d, "fx.enable", |s: &mut S, p: FxEnable| {
        s.set_fx_enabled(p.fx, p.enabled)?;
        Ok(json!({}))
    });
    edit(d, "fx.preset", |s: &mut S, p: FxPreset| {
        s.load_preset(p.fx, &p.preset)?;
        Ok(json!({}))
    });
    edit(d, "fx.set_state", |s: &mut S, p: FxSetState| {
        s.set_fx_state(p.fx, &p.chunk)?;
        Ok(json!({}))
    });
    d.register("project.save", false, |ctx: &mut Ctx<'_, S>, params| {
        Outcome::Value(parse::<SaveParams>("project.save", params).and_then(|p| {
            ctx.session.save_project(p.path.as_deref())?;
            Ok(json!({ "state": ctx.session.state() }))
        }))
    });
    read(d, "fx.state", |s: &S, p: FxParams| {
        Ok(json!({ "chunk": s.fx_state(p.fx)? }))
    });
    edit(d, "param.set", |s: &mut S, p: ParamSet| {
        let index = param_index(s, p.fx, p.index, p.param.as_deref())?;
        let value = match (p.value, &p.text) {
            (Some(v), None) => v,
            (None, Some(t)) => value_for_text(s, p.fx, index, t)?,
            _ => {
                return Err(mort_session::invalid(
                    "give `value` or `text`, not both or neither",
                    None,
                    "set `value` (0–1) or `text` (as the plugin shows it)",
                ));
            }
        };
        s.set_param(p.fx, index, value)?;
        let display = s.format_param(p.fx, index, value)?.unwrap_or_default();
        Ok(json!({ "index": index, "value": value, "display": display }))
    });
}

// --- Automation (ticket 0007) -----------------------------------------------

#[derive(Deserialize)]
struct EnvelopeRead {
    target: EnvelopeTarget,
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
}

#[derive(Deserialize)]
struct AutomationWrite {
    target: EnvelopeTarget,
    points: Vec<EnvelopePoint>,
    #[serde(default = "yes")]
    replace: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct AutomationClear {
    target: EnvelopeTarget,
    start: f64,
    end: f64,
}

#[derive(Deserialize)]
struct ParamParse {
    fx: FxId,
    index: u32,
    text: String,
}

/// A small distance past a point's time, so a range that ends at a point
/// includes it.
const PAST: f64 = 1e-9;

/// Registers the automation methods of ticket 0007, and `param.parse`.
pub fn register_automation<S: Session + 'static>(d: &mut Dispatcher<S>) {
    read(d, "envelopes.list", |s: &S, p: TrackParams| {
        Ok(json!({ "envelopes": s.envelopes(p.track)? }))
    });
    d.register("envelope.points", false, |ctx: &mut Ctx<'_, S>, params| {
        Outcome::Value(
            parse::<EnvelopeRead>("envelope.points", params).and_then(|p| {
                let s = &mut *ctx.session;
                let state = s.state();
                let Some(env) = s.envelope(p.target, false)? else {
                    return Ok(json!({ "envelope": null, "points": [], "state": state }));
                };
                let pts = s.envelope_points(
                    env.id,
                    p.start.unwrap_or(f64::NEG_INFINITY),
                    p.end.unwrap_or(f64::INFINITY),
                )?;
                let points: Vec<Value> = pts
                    .iter()
                    .map(|pt| {
                        let mut v = to_value(pt);
                        v["display"] =
                            json!(s.format_envelope_value(env.id, pt.value).ok().flatten());
                        v
                    })
                    .collect();
                Ok(json!({ "envelope": env, "points": points, "state": state }))
            }),
        )
    });
    edit(d, "automation.write", |s: &mut S, p: AutomationWrite| {
        let env = s.envelope(p.target, true)?.ok_or_else(|| {
            ErrorObject::internal("the session made no envelope when asked to create one")
        })?;
        if p.replace
            && let (Some(first), Some(last)) = (
                p.points.iter().map(|x| x.time).reduce(f64::min),
                p.points.iter().map(|x| x.time).reduce(f64::max),
            )
        {
            s.delete_envelope_points(env.id, first, last + PAST)?;
        }
        s.insert_envelope_points(env.id, &p.points)?;
        Ok(json!({ "envelope": env, "written": p.points.len() }))
    });
    edit(
        d,
        "automation.clear",
        |s: &mut S, p: AutomationClear| match s.envelope(p.target, false)? {
            None => Ok(json!({ "envelope": null })),
            Some(env) => {
                s.delete_envelope_points(env.id, p.start, p.end)?;
                Ok(json!({ "envelope": env }))
            }
        },
    );
    read(d, "param.parse", |s: &S, p: ParamParse| {
        let value = value_for_text(s, p.fx, p.index, &p.text)?;
        Ok(json!({ "value": value, "display": s.format_param(p.fx, p.index, value)? }))
    });
}

// --- MIDI (ticket 0008) -----------------------------------------------------

#[derive(Deserialize)]
struct MidiRead {
    track: TrackId,
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum MidiMode {
    /// Keep what is there.
    #[default]
    Add,
    /// Remove notes starting, and CCs falling, in the range first.
    Replace,
}

#[derive(Deserialize)]
struct MidiWrite {
    track: TrackId,
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
    #[serde(default)]
    notes: Vec<Note>,
    #[serde(default)]
    ccs: Vec<ControlChange>,
    #[serde(default)]
    mode: MidiMode,
}

/// How far, in seconds, an item may fall short of an event and still hold
/// it, allowing for rounding through REAPER's ticks.
const MIDI_SLACK: f64 = 1e-6;

/// Registers the MIDI methods of ticket 0008.
pub fn register_midi<S: Session + 'static>(d: &mut Dispatcher<S>) {
    read(d, "midi.read", |s: &S, p: MidiRead| {
        let (lo, hi) = (
            p.start.unwrap_or(f64::NEG_INFINITY),
            p.end.unwrap_or(f64::INFINITY),
        );
        let mut items = Vec::new();
        for it in s.items(p.track)?.into_iter().filter(|i| i.midi) {
            if it.position + it.length <= lo || it.position >= hi {
                continue;
            }
            let notes: Vec<Note> = s
                .notes(it.id)?
                .into_iter()
                .filter(|n| n.start >= lo && n.start < hi)
                .collect();
            let ccs: Vec<ControlChange> = s
                .control_changes(it.id)?
                .into_iter()
                .filter(|c| c.time >= lo && c.time < hi)
                .collect();
            items.push(json!({ "item": it, "notes": notes, "ccs": ccs }));
        }
        Ok(json!({ "items": items }))
    });
    edit(d, "midi.write", |s: &mut S, p: MidiWrite| {
        let event_lo = p
            .notes
            .iter()
            .map(|n| n.start)
            .chain(p.ccs.iter().map(|c| c.time))
            .reduce(f64::min);
        let event_hi = p
            .notes
            .iter()
            .map(|n| n.end)
            .chain(p.ccs.iter().map(|c| c.time))
            .reduce(f64::max);
        let start = p.start.or(event_lo).ok_or_else(|| {
            mort_session::invalid(
                "nothing to write and no range",
                None,
                "give notes, CCs or a range",
            )
        })?;
        let end = p.end.or(event_hi).unwrap_or(start);
        let (lo, hi) = (
            start.min(event_lo.unwrap_or(start)),
            end.max(event_hi.unwrap_or(end)),
        );
        let existing = s.items(p.track)?.into_iter().find(|i| {
            i.midi && i.position <= lo + MIDI_SLACK && i.position + i.length >= hi - MIDI_SLACK
        });
        let (item, created) = match existing {
            Some(i) => (i.id, false),
            None => (
                s.add_midi_item(p.track, lo, hi.max(lo + MIDI_SLACK * 2.0))?,
                true,
            ),
        };
        if p.mode == MidiMode::Replace && !created {
            s.delete_midi(item, start, end.max(start + MIDI_SLACK))?;
        }
        s.insert_midi(item, &p.notes, &p.ccs)?;
        Ok(json!({ "item": item, "created": created, "notes": p.notes.len(), "ccs": p.ccs.len() }))
    });
}

// --- Transport and renders (ticket 0009) ------------------------------------

#[derive(Deserialize)]
struct RangeParams {
    start: f64,
    end: f64,
}

pub(crate) fn check_range(start: f64, end: f64) -> Result<(), ErrorObject> {
    if end > start {
        Ok(())
    } else {
        Err(mort_session::invalid(
            format!("range {start}..{end} is empty"),
            None,
            "give an end after the start",
        ))
    }
}

/// Plays a range and stops at its end (ADR 0002 §3).
struct PlayJob {
    start: f64,
    end: f64,
}

impl<S: Session> Job<S> for PlayJob {
    fn start(&mut self, s: &mut S) -> Result<(), ErrorObject> {
        // No run: what plays is an audition, and feeds the map (ADR 0012).
        s.play(self.start, self.end)
    }

    fn poll(&mut self, s: &mut S) -> JobPoll {
        let at = s.position();
        let stopped = !s.is_playing();
        if !stopped && at < self.end {
            return JobPoll::Running(Some(json!({ "position": at })));
        }
        if !stopped {
            s.stop();
        }
        JobPoll::Done(json!({
            "start": self.start,
            "end": self.end,
            "stopped_at": at,
            "stopped_early": stopped && at < self.end,
        }))
    }

    fn cancel(&mut self, s: &mut S) {
        s.stop();
    }
}

/// Renders a range faster than real time (ADR 0006 §4).
struct RenderJob {
    start: f64,
    end: f64,
    run: u64,
    seconds: f64,
}

impl<S: Session> Job<S> for RenderJob {
    fn start(&mut self, s: &mut S) -> Result<(), ErrorObject> {
        self.run = s.begin_run();
        let t = std::time::Instant::now();
        // REAPER renders inside this call, holding the main thread.
        let r = s.render(self.start, self.end);
        self.seconds = t.elapsed().as_secs_f64();
        s.end_run(self.run);
        r
    }

    fn poll(&mut self, _: &mut S) -> JobPoll {
        JobPoll::Done(json!({
            "run": self.run,
            "start": self.start,
            "end": self.end,
            "seconds": self.seconds,
            "speed": (self.end - self.start) / self.seconds.max(1e-9),
        }))
    }

    fn cancel(&mut self, _: &mut S) {}
}

/// Registers `play`, `render` and `stop` (ticket 0009).
pub fn register_transport<S: Session + 'static>(d: &mut Dispatcher<S>) {
    d.register("play", false, |_, params| {
        match parse::<RangeParams>("play", params)
            .and_then(|p| check_range(p.start, p.end).map(|()| p))
        {
            Err(e) => Outcome::Value(Err(e)),
            Ok(p) => Outcome::Job(Box::new(PlayJob {
                start: p.start,
                end: p.end,
            })),
        }
    });
    d.register("render", false, |_, params| {
        match parse::<RangeParams>("render", params)
            .and_then(|p| check_range(p.start, p.end).map(|()| p))
        {
            Err(e) => Outcome::Value(Err(e)),
            Ok(p) => Outcome::Job(Box::new(RenderJob {
                start: p.start,
                end: p.end,
                run: 0,
                seconds: 0.0,
            })),
        }
    });
    d.register("stop", false, |ctx: &mut Ctx<'_, S>, _| {
        ctx.session.stop();
        Outcome::Value(Ok(json!({ "position": ctx.session.position() })))
    });
}

// --- Probes (ticket 0011) -------------------------------------------------

/// The probe as REAPER names it.
pub const PROBE_PLUGIN: &str = "CLAP: mort probe (Vulpus Labs)";

pub(crate) fn is_probe(fx: &mort_proto::model::FxInfo) -> bool {
    fx.plugin.contains("mort probe")
}

#[derive(Deserialize)]
struct ProbesEnsure {
    #[serde(default)]
    tracks: Option<Vec<TrackId>>,
    #[serde(default = "yes")]
    master: bool,
}

#[derive(Deserialize)]
struct ProbesPeek {
    slot: u32,
    run: u64,
    #[serde(default)]
    limit: Option<usize>,
}

/// The slot a probe shows in its `slot` parameter, if it has one.
pub(crate) fn probe_slot<S: Session>(s: &S, fx: mort_proto::FxId) -> Option<u32> {
    let p = s.params(fx).ok()?.into_iter().find(|p| p.name == "slot")?;
    let v: f64 = p.display.trim().parse().ok()?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "slot ids are small and positive"
    )]
    let id = v.round() as u32;
    (id > 0).then_some(id)
}

/// What [`ensure_probes`] did.
pub struct Ensured {
    /// Probes added.
    pub added: usize,
    /// Probes moved to the end of their chain.
    pub moved: usize,
    /// Extra probes removed.
    pub removed: usize,
    /// The probe on each track.
    pub probes: Vec<(TrackId, FxId)>,
}

/// Makes sure each of `targets` has exactly one probe, last in its chain
/// (ADR 0005 §1).
///
/// # Errors
///
/// A track that does not exist, or REAPER failing to add the probe.
pub fn ensure_probes<S: Session>(s: &mut S, targets: &[TrackId]) -> Result<Ensured, ErrorObject> {
    let mut e = Ensured {
        added: 0,
        moved: 0,
        removed: 0,
        probes: Vec::new(),
    };
    for &t in targets {
        let info = s.track(t)?;
        let probes: Vec<_> = info.fx.iter().filter(|f| is_probe(f)).collect();
        let keep = match probes.first() {
            None => {
                e.added += 1;
                s.add_fx(t, PROBE_PLUGIN, None)?
            }
            Some(first) => first.id,
        };
        for extra in probes.iter().skip(1) {
            s.remove_fx(extra.id)?;
            e.removed += 1;
        }
        let chain = s.track(t)?.fx;
        let last = chain.len() - 1;
        if chain.iter().position(|f| f.id == keep) != Some(last) {
            s.move_fx(keep, last)?;
            e.moved += 1;
        }
        e.probes.push((t, keep));
    }
    Ok(e)
}

/// Registers `probes.ensure`, `probes.list` and `probes.peek` (ticket 0011).
pub fn register_probes<S: Session + 'static>(d: &mut Dispatcher<S>) {
    edit(d, "probes.ensure", |s: &mut S, p: ProbesEnsure| {
        let mut targets: Vec<TrackId> = match p.tracks {
            Some(t) => t,
            None => s.tracks().into_iter().map(|t| t.id).collect(),
        };
        if p.master && !targets.contains(&s.master()) {
            targets.push(s.master());
        }
        let e = ensure_probes(s, &targets)?;
        let out: Vec<Value> = e
            .probes
            .iter()
            .map(|(t, fx)| json!({ "track": t, "fx": fx }))
            .collect();
        Ok(json!({ "probes": out, "added": e.added, "moved": e.moved, "removed": e.removed }))
    });
    read(d, "probes.list", |s: &S, _: NoParams| {
        let counts = probes::counts();
        let mut list = Vec::new();
        let mut unmeasured = Vec::new();
        let all = std::iter::once(s.track(s.master())?).chain(s.tracks());
        for t in all {
            let found: Vec<_> = t.fx.iter().filter(|f| is_probe(f)).collect();
            if found.is_empty() {
                unmeasured.push(json!({ "track": t.id, "name": t.name }));
            }
            for f in found {
                let slot = probe_slot(s, f.id);
                let link = slot.and_then(probes::slot);
                let buffered: HashMap<String, usize> = slot
                    .and_then(|id| counts.get(&id))
                    .map(|m| m.iter().map(|(run, n)| (run.to_string(), *n)).collect())
                    .unwrap_or_default();
                list.push(json!({
                    "track": t.id,
                    "name": t.name,
                    "fx": f.id,
                    "last": f.index + 1 == t.fx.len(),
                    "enabled": f.enabled,
                    "slot": slot,
                    "attached": link.is_some_and(|l| l.attached.load(Ordering::Relaxed)),
                    "active": link.is_some_and(|l| l.active.load(Ordering::Relaxed) > 0),
                    "sample_rate": link.and_then(Slot::sample_rate),
                    "pushed": link.map(|l| l.pushed.load(Ordering::Relaxed)),
                    "dropped": link.map(|l| l.dropped.load(Ordering::Relaxed)),
                    "buffered": buffered,
                }));
            }
        }
        Ok(json!({ "probes": list, "unmeasured": unmeasured, "run": runs::current() }))
    });
    read(d, "probes.peek", |_: &S, p: ProbesPeek| {
        probes::drain();
        let recs = probes::peek(p.slot, p.run, p.limit.unwrap_or(1000));
        let frames: Vec<Value> = recs
            .iter()
            .map(|r| {
                json!({
                    "start": r.start,
                    "index": r.frame.index,
                    "momentary_lufs": r.frame.momentary_lufs,
                    "rms_dbfs": r.frame.rms_dbfs,
                    "true_peak_dbtp": r.frame.true_peak_dbtp,
                    "centroid_hz": r.frame.centroid_hz,
                })
            })
            .collect();
        Ok(json!({ "frames": frames }))
    });
}

/// The first number in a display string, with a `k` after it meaning
/// thousands. `-inf` reads as negative infinity.
fn number_in(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.to_lowercase().starts_with("-inf") {
        return Some(f64::NEG_INFINITY);
    }
    let start = t.find(|c: char| c.is_ascii_digit() || c == '-' || c == '+' || c == '.')?;
    let rest = &t[start..];
    let end = rest
        .char_indices()
        .skip(1)
        .find(|(_, c)| !(c.is_ascii_digit() || *c == '.'))
        .map_or(rest.len(), |(i, _)| i);
    let n: f64 = rest[..end].parse().ok()?;
    let after = rest[end..].trim_start();
    Some(if after.starts_with(['k', 'K']) {
        n * 1000.0
    } else {
        n
    })
}

/// Finds the normalised value at which a parameter displays `text`.
///
/// The display is sampled across the range. Text that matches a sample
/// exactly (ignoring case), as for switches and modes, gives the middle of
/// the run of values showing it. Otherwise the first number in `text` is
/// looked for between neighbouring samples and narrowed down by bisection.
fn value_for_text<S: Session>(s: &S, fx: FxId, index: u32, text: &str) -> Result<f64, ErrorObject> {
    const SAMPLES: u32 = 200;
    let shown = |v: f64| -> Result<String, ErrorObject> {
        Ok(s.format_param(fx, index, v)?.unwrap_or_default())
    };
    let vs: Vec<f64> = (0..=SAMPLES)
        .map(|i| f64::from(i) / f64::from(SAMPLES))
        .collect();
    let ds: Vec<String> = vs.iter().map(|v| shown(*v)).collect::<Result<_, _>>()?;
    let want = text.trim().to_lowercase();
    let run: Vec<f64> = vs
        .iter()
        .zip(&ds)
        .filter(|(_, d)| d.trim().to_lowercase() == want)
        .map(|(v, _)| *v)
        .collect();
    if let (Some(a), Some(b)) = (run.first(), run.last()) {
        return Ok(f64::midpoint(*a, *b));
    }
    let outside = || {
        mort_session::invalid(
            format!(
                "`{text}` is not something this parameter shows; it runs from `{}` to `{}`",
                ds[0],
                ds[ds.len() - 1]
            ),
            Some(format!("{fx} #{index}")),
            "give a value in that range, or a normalised `value`",
        )
    };
    let target = number_in(text).ok_or_else(outside)?;
    let ns: Vec<Option<f64>> = ds.iter().map(|d| number_in(d)).collect();
    for i in 0..ns.len() - 1 {
        let (Some(a), Some(b)) = (ns[i], ns[i + 1]) else {
            continue;
        };
        if (a <= target && target <= b) || (b <= target && target <= a) {
            let rising = b >= a;
            // `below(m)`: m lies before the target in the sweep's direction.
            // The two edges bound the values that display as the target; the
            // middle of them is the value least likely to show otherwise.
            let edge = |strict: bool| -> Result<f64, ErrorObject> {
                let (mut lo, mut hi) = (vs[i], vs[i + 1]);
                for _ in 0..40 {
                    let mid = f64::midpoint(lo, hi);
                    let Some(m) = number_in(&shown(mid)?) else {
                        break;
                    };
                    let before = if rising {
                        m < target || (!strict && m <= target)
                    } else {
                        m > target || (!strict && m >= target)
                    };
                    if before {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                Ok(f64::midpoint(lo, hi))
            };
            return Ok(f64::midpoint(edge(true)?, edge(false)?));
        }
    }
    Err(outside())
}

#[cfg(test)]
mod tests {
    use super::number_in;

    #[test]
    fn numbers_in_display_text() {
        assert_eq!(number_in("632 Hz"), Some(632.0));
        assert_eq!(number_in("-3.00 dB"), Some(-3.0));
        assert_eq!(number_in("2.4 kHz"), Some(2400.0));
        assert_eq!(number_in("2.4k"), Some(2400.0));
        assert_eq!(number_in("-inf"), Some(f64::NEG_INFINITY));
        assert_eq!(number_in("Q 0.70"), Some(0.7));
        assert_eq!(number_in("Band"), None);
    }
}
