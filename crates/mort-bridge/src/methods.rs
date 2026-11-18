//! The bridge's RPC methods over a [`Session`].
//!
//! Every read returns the project's `state` alongside its result, so a
//! client can send it back as `expect_state` (ADR 0004 §2).

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

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
