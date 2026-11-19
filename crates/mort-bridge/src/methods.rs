//! The bridge's RPC methods over a [`Session`].
//!
//! Every read returns the project's `state` alongside its result, so a
//! client can send it back as `expect_state` (ADR 0004 §2).

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use mort_proto::model::{EnvelopePoint, EnvelopeTarget};
use mort_proto::{ErrorObject, FxId, Position, Range, TrackId};
use mort_session::Session;

use crate::dispatch::{Ctx, Dispatcher, Outcome};

fn parse<T: DeserializeOwned>(method: &str, params: Value) -> Result<T, ErrorObject> {
    let params = if params.is_null() { json!({}) } else { params };
    serde_json::from_value(params).map_err(|e| ErrorObject::invalid_params(method, &e.to_string()))
}

fn read<S, P: DeserializeOwned>(
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
struct NoParams {}

#[derive(Deserialize)]
struct TrackParams {
    track: TrackId,
}

#[derive(Deserialize)]
struct FxParams {
    fx: FxId,
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
    index: u32,
    #[serde(default)]
    value: Option<f64>,
    #[serde(default)]
    text: Option<String>,
}

/// Registers the edit methods of ticket 0006, and `fx.state`.
pub fn register_edits<S: Session + 'static>(d: &mut Dispatcher<S>) {
    use mort_session::{NewTrack, SendChange, TrackChange};

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
        let value = match (p.value, &p.text) {
            (Some(v), None) => v,
            (None, Some(t)) => value_for_text(s, p.fx, p.index, t)?,
            _ => {
                return Err(mort_session::invalid(
                    "give `value` or `text`, not both or neither",
                    None,
                    "set `value` (0–1) or `text` (as the plugin shows it)",
                ));
            }
        };
        s.set_param(p.fx, p.index, value)?;
        let display = s.format_param(p.fx, p.index, value)?.unwrap_or_default();
        Ok(json!({ "value": value, "display": display }))
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
