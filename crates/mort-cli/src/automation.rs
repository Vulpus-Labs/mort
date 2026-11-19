//! `mort automate` (ticket 0007).
//!
//! ```text
//! mort automate bass volume                       # list the points
//! mort automate bass volume 9.1=-6 13.1=0 --shape slow
//! mort automate bass pan 1.1=L30 +8b=R30
//! mort automate bass eq/gain-band\ 2 9.1=0.4 --clear 9.1..17.1
//! mort automate bass eq/freq 9.1="300 Hz" +4b="2.4 kHz" --text
//! mort automate bass send:verb 17.1=-inf
//! ```
//!
//! Positions chain: `+4b` is four quarter notes after the point before it.
//! Writing replaces the points between the first and last new point,
//! unless `--add` is given. `--clear` with points clears first, and both
//! happen in one undo point.

use clap::{Args, ValueEnum};
use serde_json::{Value, json};

use crate::client::{Client, Failure};
use crate::edits::{parse_db, parse_pan};
use crate::{resolve, show};

/// The shape of the segments that start at the points written.
#[derive(Clone, Copy, ValueEnum)]
pub enum ShapeArg {
    /// Straight line.
    Linear,
    /// Hold, then jump.
    Square,
    /// S-curve.
    Slow,
    /// Fast start, slow end.
    FastStart,
    /// Slow start, fast end.
    FastEnd,
    /// Bézier, bent by --tension.
    Bezier,
}

impl ShapeArg {
    fn wire(self) -> &'static str {
        match self {
            ShapeArg::Linear => "linear",
            ShapeArg::Square => "square",
            ShapeArg::Slow => "slow",
            ShapeArg::FastStart => "fast_start",
            ShapeArg::FastEnd => "fast_end",
            ShapeArg::Bezier => "bezier",
        }
    }
}

/// `mort automate` arguments.
#[derive(Args)]
pub struct Automate {
    /// The track.
    track: String,
    /// What to automate: `volume`, `pan`, `send:<track>`, or `<fx>/<param>`.
    target: String,
    /// Points as POSITION=VALUE: `9.1=-6`, `+4b=R30`, `13.1=0.75`.
    points: Vec<String>,
    /// Segment shape for the points written.
    #[arg(long, value_enum, default_value = "linear")]
    shape: ShapeArg,
    /// Bézier tension, −1 to 1.
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    tension: f64,
    /// Keep existing points between the new ones.
    #[arg(long)]
    add: bool,
    /// Clear this range first, such as `9.1..17.1`.
    #[arg(long, value_name = "RANGE")]
    clear: Option<String>,
    /// Read parameter values as the plugin's display text.
    #[arg(long)]
    text: bool,
}

enum Kind {
    Gain,
    Pan,
    Param { fx: Value, index: u64 },
}

fn name(v: &Value) -> &str {
    v["name"].as_str().unwrap_or("?")
}

/// Resolves the target to its wire form, its kind and a readable name.
fn target(
    client: &mut Client,
    list: &Value,
    track: &Value,
    text: &str,
) -> Result<(Value, Kind, String), Failure> {
    let lower = text.trim().to_lowercase();
    let id = &track["id"];
    if lower == "volume" || lower == "vol" {
        return Ok((
            json!({ "kind": "volume", "track": id }),
            Kind::Gain,
            format!("{} volume", name(track)),
        ));
    }
    if lower == "pan" {
        return Ok((
            json!({ "kind": "pan", "track": id }),
            Kind::Pan,
            format!("{} pan", name(track)),
        ));
    }
    if let Some(dest) = text.trim().strip_prefix("send:") {
        let to = resolve::track(list, dest)?;
        let sends = track["sends"].as_array().cloned().unwrap_or_default();
        let i = sends
            .iter()
            .position(|s| s["dest"] == to["id"])
            .ok_or_else(|| {
                Failure::Local(format!(
                    "{} has no send to {}; add one with `mort send add`",
                    name(track),
                    name(to)
                ))
            })?;
        return Ok((
            json!({ "kind": "send_volume", "track": id, "send": i }),
            Kind::Gain,
            format!("{} send to {}", name(track), name(to)),
        ));
    }
    let Some((fx, param)) = text.split_once('/') else {
        return Err(Failure::Local(format!(
            "`{text}` is not a target; write `volume`, `pan`, `send:<track>` or `<fx>/<param>`"
        )));
    };
    let f = resolve::fx(track, fx)?;
    let ps = client.call("fx.params", json!({ "fx": f["id"] }))?;
    let params = ps["params"].as_array().cloned().unwrap_or_default();
    let p = if let Ok(i) = param.trim().parse::<u64>() {
        params
            .iter()
            .find(|p| p["index"].as_u64() == Some(i))
            .cloned()
            .ok_or_else(|| Failure::Local(format!("{} has no parameter {i}", name(f))))?
    } else {
        let named: Vec<Value> = params
            .iter()
            .map(
                |p| json!({ "name": p["name"], "id": p["index"].to_string(), "index": p["index"] }),
            )
            .collect();
        let hit = resolve::pick(&named, param, "parameter", "name")?;
        params[usize::try_from(hit["index"].as_u64().unwrap_or(0)).unwrap_or(0)].clone()
    };
    let index = p["index"].as_u64().unwrap_or(0);
    Ok((
        json!({ "kind": "param", "fx": f["id"], "index": index }),
        Kind::Param {
            fx: f["id"].clone(),
            index,
        },
        format!("{} {} {}", name(track), name(f), name(&p)),
    ))
}

fn value(client: &mut Client, kind: &Kind, text: &str, as_text: bool) -> Result<f64, Failure> {
    match kind {
        Kind::Gain => parse_db(text),
        Kind::Pan => parse_pan(text),
        Kind::Param { fx, index } => {
            if as_text {
                let r = client.call(
                    "param.parse",
                    json!({ "fx": fx, "index": index, "text": text }),
                )?;
                Ok(r["value"].as_f64().unwrap_or(0.0))
            } else {
                text.trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|v| (0.0..=1.0).contains(v))
                    .ok_or_else(|| {
                        Failure::Local(format!(
                            "`{text}` is not a normalised value from 0 to 1; use --text for the plugin's units"
                        ))
                    })
            }
        }
    }
}

/// Runs `mort automate`.
pub fn run(client: &mut Client, a: &Automate) -> Result<(Vec<String>, Value), Failure> {
    let list = client.call("tracks.list", Value::Null)?;
    let state = list["state"].clone();
    let track = resolve::track(&list, &a.track)?.clone();
    let (target, kind, what) = target(client, &list, &track, &a.target)?;

    if a.points.is_empty() && a.clear.is_none() {
        return list_points(client, &target, &what);
    }

    let mut positions = Vec::new();
    let mut values = Vec::new();
    for p in &a.points {
        let (pos, v) = p.split_once('=').ok_or_else(|| {
            Failure::Local(format!(
                "`{p}` is not a point; write POSITION=VALUE, such as `9.1=-6`"
            ))
        })?;
        positions.push(pos.trim().to_owned());
        values.push(value(client, &kind, v, a.text)?);
    }
    let mut to_seconds = positions.clone();
    if let Some(r) = &a.clear {
        to_seconds.push(r.clone());
    }
    let times = client.call("time.convert", json!({ "to_seconds": to_seconds }))?;
    let seconds = times["seconds"].as_array().cloned().unwrap_or_default();

    let mut calls = Vec::new();
    let mut said = Vec::new();
    if let Some(r) = &a.clear {
        let range = &seconds[seconds.len() - 1];
        calls.push(json!({ "method": "automation.clear",
            "params": { "target": target, "start": range[0], "end": range[1] } }));
        said.push(format!("cleared {r}"));
    }
    if !a.points.is_empty() {
        let points: Vec<Value> = seconds[..a.points.len()]
            .iter()
            .zip(&values)
            .map(|(t, v)| json!({ "time": t, "value": v, "shape": a.shape.wire(), "tension": a.tension }))
            .collect();
        calls.push(json!({ "method": "automation.write",
            "params": { "target": target, "points": points, "replace": !a.add } }));
        said.push(format!(
            "wrote {} point{}",
            a.points.len(),
            if a.points.len() == 1 { "" } else { "s" }
        ));
    }
    let summary = format!("automate {what}: {}", said.join(", "));
    let r = client.call(
        "batch",
        json!({ "calls": calls, "summary": summary, "expect_state": state }),
    )?;
    let after = client.call("bridge.status", Value::Null)?;
    let mut lines = vec![format!("{what}: {}", said.join(", "))];
    let (shown, _) = list_points(client, &target, &what)?;
    lines.extend(shown);
    Ok((
        lines,
        json!({ "results": r["results"], "state": after["state"] }),
    ))
}

fn list_points(
    client: &mut Client,
    target: &Value,
    what: &str,
) -> Result<(Vec<String>, Value), Failure> {
    let r = client.call("envelope.points", json!({ "target": target }))?;
    let points = r["points"].as_array().cloned().unwrap_or_default();
    if r["envelope"].is_null() {
        return Ok((vec![format!("{what}: no envelope")], r));
    }
    let times: Vec<f64> = points.iter().filter_map(|p| p["time"].as_f64()).collect();
    let musical = client.call("time.convert", json!({ "to_musical": times }))?;
    let mut rows = Vec::new();
    for (p, m) in points
        .iter()
        .zip(musical["musical"].as_array().into_iter().flatten())
    {
        rows.push(vec![
            m.as_str().unwrap_or("?").to_owned(),
            format!("{:.3}s", p["time"].as_f64().unwrap_or(0.0)),
            p["display"]
                .as_str()
                .map_or_else(|| p["value"].to_string(), str::to_owned),
            p["shape"].as_str().unwrap_or("").to_owned(),
        ]);
    }
    let mut out = vec![format!(
        "{what}: {} point{}",
        points.len(),
        if points.len() == 1 { "" } else { "s" }
    )];
    out.extend(show::table(&rows).into_iter().map(|l| format!("  {l}")));
    Ok((out, r))
}
