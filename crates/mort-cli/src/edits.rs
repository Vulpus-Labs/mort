//! Edit commands (ticket 0006, ADR 0004).
//!
//! Each command reads the track list to resolve names, then sends its edit
//! with the state it read as `expect_state`. If the project changed in
//! between, the bridge refuses with `stale` and nothing happens.

use clap::Subcommand;
use serde_json::{Value, json};

use crate::client::{Client, Failure};
use crate::{resolve, show};

/// `mort track …`
#[derive(Subcommand)]
pub enum TrackCmd {
    /// Add a track.
    Add {
        /// Its name.
        name: String,
        /// Put it at this number in the track list (from 1).
        #[arg(long, value_name = "N", conflicts_with = "folder")]
        at: Option<usize>,
        /// Put it at the end of this folder track.
        #[arg(long = "in", value_name = "TRACK")]
        folder: Option<String>,
    },
    /// Remove a track.
    Rm {
        /// The track.
        track: String,
    },
    /// Change a track's name, volume, pan, mute or solo.
    Set {
        /// The track.
        track: String,
        /// New name.
        #[arg(long)]
        name: Option<String>,
        /// Volume in dB, such as `-6` or `-inf`.
        #[arg(long, allow_hyphen_values = true)]
        volume: Option<String>,
        /// Pan: `C`, `L20`, `R35`, or −1 to 1.
        #[arg(long, allow_hyphen_values = true)]
        pan: Option<String>,
        /// Mute: `on` or `off`.
        #[arg(long)]
        mute: Option<OnOff>,
        /// Solo: `on` or `off`.
        #[arg(long)]
        solo: Option<OnOff>,
    },
}

/// `mort send …`
#[derive(Subcommand)]
pub enum SendCmd {
    /// Add a send.
    Add {
        /// The sending track.
        from: String,
        /// The receiving track.
        to: String,
    },
    /// Change a send, named by its receiving track.
    Set {
        /// The sending track.
        from: String,
        /// The receiving track.
        to: String,
        /// Volume in dB.
        #[arg(long, allow_hyphen_values = true)]
        volume: Option<String>,
        /// Pan: `C`, `L20`, `R35`, or −1 to 1.
        #[arg(long, allow_hyphen_values = true)]
        pan: Option<String>,
        /// Mute: `on` or `off`.
        #[arg(long)]
        mute: Option<OnOff>,
    },
}

/// `mort fx …` actions.
#[derive(Subcommand)]
pub enum FxCmd {
    /// Add a plugin, by the name REAPER's FX browser shows.
    Add {
        /// The track.
        track: String,
        /// The plugin, such as `ReaEQ` or `CLAP: mort probe`.
        plugin: String,
        /// Put it at this position in the chain (from 1).
        #[arg(long, value_name = "N")]
        at: Option<usize>,
    },
    /// Remove an FX instance.
    Rm {
        /// The track.
        track: String,
        /// The FX.
        fx: String,
    },
    /// Move an FX instance within its chain.
    Move {
        /// The track.
        track: String,
        /// The FX.
        fx: String,
        /// Its new position (from 1).
        to: usize,
    },
    /// Bypass an FX instance.
    Bypass {
        /// The track.
        track: String,
        /// The FX.
        fx: String,
    },
    /// Enable a bypassed FX instance.
    Enable {
        /// The track.
        track: String,
        /// The FX.
        fx: String,
    },
    /// Load a preset by name.
    Preset {
        /// The track.
        track: String,
        /// The FX.
        fx: String,
        /// The preset's name.
        preset: String,
    },
}

/// `on` or `off`.
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum OnOff {
    /// On.
    On,
    /// Off.
    Off,
}

impl OnOff {
    fn yes(self) -> bool {
        matches!(self, OnOff::On)
    }
}

fn usage(m: String) -> Failure {
    Failure::Local(m)
}

/// `-6`, `-6dB`, `+3 dB` or `-inf` as linear gain.
pub fn parse_db(text: &str) -> Result<f64, Failure> {
    let t = text
        .trim()
        .trim_end_matches("dB")
        .trim_end_matches("db")
        .trim();
    if t.eq_ignore_ascii_case("-inf") {
        return Ok(0.0);
    }
    t.parse::<f64>()
        .map(|db| 10f64.powf(db / 20.0))
        .map_err(|_| {
            usage(format!(
                "`{text}` is not a level in dB; write `-6`, `+3dB` or `-inf`"
            ))
        })
}

/// `C`, `L20`, `R35` or a number from −1 to 1.
pub fn parse_pan(text: &str) -> Result<f64, Failure> {
    let t = text.trim();
    let bad = || {
        usage(format!(
            "`{text}` is not a pan; write `C`, `L20`, `R35` or −1 to 1"
        ))
    };
    if t.eq_ignore_ascii_case("c") {
        return Ok(0.0);
    }
    let pct = |s: &str| s.parse::<f64>().map(|n| n / 100.0).map_err(|_| bad());
    match t.chars().next().map(|c| c.to_ascii_uppercase()) {
        Some('L') => pct(&t[1..]).map(|p| -p),
        Some('R') => pct(&t[1..]),
        _ => t.parse().map_err(|_| bad()),
    }
}

/// An edit: send `method` with `params`, plus the summary and the state the
/// command read.
fn send_edit(
    client: &mut Client,
    method: &str,
    mut params: Value,
    summary: &str,
    state: &Value,
) -> Result<Value, Failure> {
    params["summary"] = json!(summary);
    params["expect_state"] = state.clone();
    client.call(method, params)
}

fn name(v: &Value) -> &str {
    v["name"].as_str().unwrap_or("?")
}

/// Runs `mort track …`.
pub fn track(client: &mut Client, cmd: &TrackCmd) -> Result<(Vec<String>, Value), Failure> {
    let list = client.call("tracks.list", Value::Null)?;
    let state = &list["state"];
    match cmd {
        TrackCmd::Add {
            name: n,
            at,
            folder,
        } => {
            let mut p = json!({ "name": n });
            let mut where_ = String::new();
            if let Some(at) = at {
                if *at == 0 {
                    return Err(usage("track numbers start at 1".into()));
                }
                p["index"] = json!(at - 1);
                where_ = format!(" at {at}");
            }
            if let Some(f) = folder {
                let f = resolve::track(&list, f)?;
                p["parent"] = f["id"].clone();
                where_ = format!(" in {}", name(f));
            }
            let r = send_edit(
                client,
                "track.add",
                p,
                &format!("add track {n}{where_}"),
                state,
            )?;
            Ok((
                vec![format!(
                    "added {n}{where_}  {}",
                    r["track"].as_str().unwrap_or("")
                )],
                r,
            ))
        }
        TrackCmd::Rm { track } => {
            let t = resolve::track(&list, track)?;
            let r = send_edit(
                client,
                "track.rm",
                json!({ "track": t["id"] }),
                &format!("remove track {}", name(t)),
                state,
            )?;
            Ok((vec![format!("removed {}", name(t))], r))
        }
        TrackCmd::Set {
            track,
            name: new_name,
            volume,
            pan,
            mute,
            solo,
        } => {
            let t = resolve::track(&list, track)?;
            let mut p = json!({ "track": t["id"] });
            let mut what = Vec::new();
            if let Some(n) = new_name {
                p["name"] = json!(n);
                what.push(format!("name {n}"));
            }
            if let Some(v) = volume {
                let g = parse_db(v)?;
                p["volume"] = json!(g);
                what.push(format!("volume {}", show::db(g)));
            }
            if let Some(v) = pan {
                let x = parse_pan(v)?;
                p["pan"] = json!(x);
                what.push(format!("pan {}", show::pan(x)));
            }
            if let Some(m) = mute {
                p["mute"] = json!(m.yes());
                what.push(format!("mute {}", if m.yes() { "on" } else { "off" }));
            }
            if let Some(s) = solo {
                p["solo"] = json!(s.yes());
                what.push(format!("solo {}", if s.yes() { "on" } else { "off" }));
            }
            if what.is_empty() {
                return Err(usage(
                    "say what to change: --name, --volume, --pan, --mute or --solo".into(),
                ));
            }
            let summary = format!("{} {}", name(t), what.join(", "));
            let r = send_edit(client, "track.set", p, &summary, state)?;
            Ok((vec![format!("set {summary}")], r))
        }
    }
}

/// Runs `mort send …`.
pub fn send(client: &mut Client, cmd: &SendCmd) -> Result<(Vec<String>, Value), Failure> {
    let list = client.call("tracks.list", Value::Null)?;
    let state = &list["state"];
    match cmd {
        SendCmd::Add { from, to } => {
            let (src, dst) = (resolve::track(&list, from)?, resolve::track(&list, to)?);
            let summary = format!("send {} to {}", name(src), name(dst));
            let r = send_edit(
                client,
                "send.add",
                json!({ "from": src["id"], "to": dst["id"] }),
                &summary,
                state,
            )?;
            Ok((vec![format!("added {summary}")], r))
        }
        SendCmd::Set {
            from,
            to,
            volume,
            pan,
            mute,
        } => {
            let (src, dst) = (resolve::track(&list, from)?, resolve::track(&list, to)?);
            let sends = src["sends"].as_array().cloned().unwrap_or_default();
            let i = sends
                .iter()
                .position(|s| s["dest"] == dst["id"])
                .ok_or_else(|| {
                    Failure::Local(format!(
                        "{} has no send to {}; add one with `mort send add`",
                        name(src),
                        name(dst)
                    ))
                })?;
            let mut p = json!({ "from": src["id"], "send": i });
            let mut what = Vec::new();
            if let Some(v) = volume {
                let g = parse_db(v)?;
                p["volume"] = json!(g);
                what.push(format!("volume {}", show::db(g)));
            }
            if let Some(v) = pan {
                let x = parse_pan(v)?;
                p["pan"] = json!(x);
                what.push(format!("pan {}", show::pan(x)));
            }
            if let Some(m) = mute {
                p["mute"] = json!(m.yes());
                what.push(format!("mute {}", if m.yes() { "on" } else { "off" }));
            }
            if what.is_empty() {
                return Err(usage(
                    "say what to change: --volume, --pan or --mute".into(),
                ));
            }
            let summary = format!("send {} to {}: {}", name(src), name(dst), what.join(", "));
            let r = send_edit(client, "send.set", p, &summary, state)?;
            Ok((vec![format!("set {summary}")], r))
        }
    }
}

/// Runs `mort fx <action> …`.
pub fn fx(client: &mut Client, cmd: &FxCmd) -> Result<(Vec<String>, Value), Failure> {
    let list = client.call("tracks.list", Value::Null)?;
    let state = &list["state"];
    let pick = |track: &str, fx: &str| -> Result<(Value, Value), Failure> {
        let t = resolve::track(&list, track)?;
        let f = resolve::fx(t, fx)?;
        Ok((t.clone(), f.clone()))
    };
    match cmd {
        FxCmd::Add { track, plugin, at } => {
            let t = resolve::track(&list, track)?;
            let mut p = json!({ "track": t["id"], "plugin": plugin });
            if let Some(at) = at {
                if *at == 0 {
                    return Err(usage("chain positions start at 1".into()));
                }
                p["index"] = json!(at - 1);
            }
            let r = send_edit(
                client,
                "fx.add",
                p,
                &format!("add {plugin} to {}", name(t)),
                state,
            )?;
            let f = &r["fx"];
            Ok((
                vec![format!(
                    "added {} to {} at {}  {}",
                    name(f),
                    name(t),
                    f["index"].as_u64().unwrap_or(0) + 1,
                    f["id"].as_str().unwrap_or("")
                )],
                r,
            ))
        }
        FxCmd::Rm { track, fx } => {
            let (t, f) = pick(track, fx)?;
            let summary = format!("remove {} from {}", name(&f), name(&t));
            let r = send_edit(client, "fx.rm", json!({ "fx": f["id"] }), &summary, state)?;
            Ok((vec![format!("removed {} from {}", name(&f), name(&t))], r))
        }
        FxCmd::Move { track, fx, to } => {
            let (t, f) = pick(track, fx)?;
            if *to == 0 {
                return Err(usage("chain positions start at 1".into()));
            }
            let summary = format!("move {} on {} to {to}", name(&f), name(&t));
            let r = send_edit(
                client,
                "fx.move",
                json!({ "fx": f["id"], "index": to - 1 }),
                &summary,
                state,
            )?;
            Ok((vec![summary], r))
        }
        FxCmd::Bypass { track, fx } | FxCmd::Enable { track, fx } => {
            let on = matches!(cmd, FxCmd::Enable { .. });
            let (t, f) = pick(track, fx)?;
            let summary = format!(
                "{} {} on {}",
                if on { "enable" } else { "bypass" },
                name(&f),
                name(&t)
            );
            let r = send_edit(
                client,
                "fx.enable",
                json!({ "fx": f["id"], "enabled": on }),
                &summary,
                state,
            )?;
            Ok((vec![summary], r))
        }
        FxCmd::Preset { track, fx, preset } => {
            let (t, f) = pick(track, fx)?;
            let summary = format!("{} on {}: preset {preset}", name(&f), name(&t));
            let r = send_edit(
                client,
                "fx.preset",
                json!({ "fx": f["id"], "preset": preset }),
                &summary,
                state,
            )?;
            Ok((vec![format!("loaded {summary}")], r))
        }
    }
}

/// Runs `mort set <track> <fx> <param> <value>`.
pub fn set(
    client: &mut Client,
    track: &str,
    fx: &str,
    param: &str,
    value: &str,
    normalised: bool,
) -> Result<(Vec<String>, Value), Failure> {
    let list = client.call("tracks.list", Value::Null)?;
    let t = resolve::track(&list, track)?;
    let f = resolve::fx(t, fx)?;
    let ps = client.call("fx.params", json!({ "fx": f["id"] }))?;
    let params = ps["params"].as_array().map_or(&[][..], Vec::as_slice);
    let p = if let Ok(i) = param.parse::<u64>() {
        params
            .iter()
            .find(|p| p["index"].as_u64() == Some(i))
            .ok_or_else(|| Failure::Local(format!("{} has no parameter {i}", name(f))))?
    } else {
        // Parameters are numbered from 0, so pick by name only.
        let named: Vec<Value> = params
            .iter()
            .map(
                |p| json!({ "name": p["name"], "id": p["index"].to_string(), "index": p["index"] }),
            )
            .collect();
        let hit = resolve::pick(&named, param, "parameter", "name")?;
        &params[usize::try_from(hit["index"].as_u64().unwrap_or(0)).unwrap_or(0)]
    };
    let mut body = json!({ "fx": f["id"], "index": p["index"] });
    if normalised {
        let v: f64 = value
            .parse()
            .map_err(|_| usage(format!("`{value}` is not a number from 0 to 1")))?;
        body["value"] = json!(v);
    } else {
        body["text"] = json!(value);
    }
    let summary = format!("{} {} {} = {value}", name(t), name(f), name(p));
    let r = send_edit(client, "param.set", body, &summary, &ps["state"])?;
    Ok((
        vec![format!(
            "{} on {}: {} = {} ({:.4})",
            name(f),
            name(t),
            name(p),
            r["display"].as_str().unwrap_or("?"),
            r["value"].as_f64().unwrap_or(0.0)
        )],
        r,
    ))
}

/// Runs `mort undo` or `mort redo`.
pub fn undo(client: &mut Client, redo: bool) -> Result<(Vec<String>, Value), Failure> {
    let r = client.call(if redo { "redo" } else { "undo" }, Value::Null)?;
    let label = r["label"].as_str().unwrap_or("?");
    Ok((
        vec![format!("{} {label}", if redo { "redid" } else { "undid" })],
        r,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        assert!((parse_db("-6").unwrap() - 0.501_187).abs() < 1e-6);
        assert!((parse_db("+3dB").unwrap() - 1.412_538).abs() < 1e-6);
        assert!((parse_db("0 dB").unwrap() - 1.0).abs() < 1e-12);
        assert!(parse_db("-inf").unwrap().abs() < f64::EPSILON);
        assert!(parse_db("loud").is_err());
    }

    #[test]
    fn pans() {
        assert!(parse_pan("C").unwrap().abs() < f64::EPSILON);
        assert!((parse_pan("L20").unwrap() + 0.2).abs() < 1e-12);
        assert!((parse_pan("r35").unwrap() - 0.35).abs() < 1e-12);
        assert!((parse_pan("-0.5").unwrap() + 0.5).abs() < 1e-12);
        assert!(parse_pan("middle").is_err());
    }
}
