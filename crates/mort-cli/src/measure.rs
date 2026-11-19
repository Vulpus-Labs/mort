//! `mort measure` and `mort map` (ticket 0012, ADR 0006).

use std::collections::HashMap;
use std::fmt::Write as _;

use clap::Args;
use serde_json::{Value, json};

use crate::client::{Client, Failure};
use crate::{resolve, show, transport};

/// `mort measure` arguments.
#[derive(Args)]
pub struct Measure {
    /// The range, such as `9.1..17.1`, then the tracks to measure (all of
    /// them if none are named). Buses the tracks reach and the master are
    /// always measured with them. With --stale the range is optional.
    args: Vec<String>,
    /// Play the range through the monitors instead of rendering it.
    #[arg(long)]
    play: bool,
    /// Re-measure only what is stale, with the buses it reaches.
    #[arg(long)]
    stale: bool,
    /// Print the job id and return at once.
    #[arg(long)]
    no_wait: bool,
}

fn secs(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn span(start: f64, end: f64) -> String {
    format!("{start:.2}–{end:.2}s")
}

/// Runs `mort measure`.
pub fn run(client: &mut Client, cmd: &Measure) -> Result<(Vec<String>, Value), Failure> {
    let (range, tracks) = match cmd.args.split_first() {
        Some((first, rest)) if first.contains("..") => (Some(first.as_str()), rest),
        _ => (None, cmd.args.as_slice()),
    };
    let mut params =
        json!({ "mode": if cmd.play { "play" } else { "render" }, "stale": cmd.stale });
    let label = match range {
        Some(text) => {
            let (a, b) = transport::range(client, text)?;
            params["start"] = json!(a);
            params["end"] = json!(b);
            text.to_owned()
        }
        None if cmd.stale => "what is stale".to_owned(),
        None => {
            return Err(Failure::Local(
                "give a range to measure, such as `mort measure 1.1..9.1`, or use --stale".into(),
            ));
        }
    };
    let list = client.call("tracks.list", Value::Null)?;
    if !tracks.is_empty() {
        let ids: Vec<Value> = tracks
            .iter()
            .map(|t| resolve::track(&list, t).map(|t| t["id"].clone()))
            .collect::<Result<_, _>>()?;
        params["tracks"] = json!(ids);
    }
    params["expect_state"] = list["state"].clone();
    params["summary"] = json!("add probes to measure");
    let reply = client.call("measure", params)?;
    if reply["nothing_stale"] == true {
        return Ok((vec!["nothing is stale".into()], reply));
    }
    let job = reply["job"].clone();
    if cmd.no_wait {
        return Ok((vec![format!("measure {label}: job {job}")], reply));
    }
    let s = transport::wait(client, &job, "measure")?;
    let res = &s["result"];
    let mut lines = vec![format!(
        "measured {} by {} in {:.1} s: capture {}",
        span(secs(&res["start"]), secs(&res["end"])),
        res["mode"].as_str().unwrap_or("?"),
        secs(&res["seconds"]),
        res["capture"]
    )];
    let mut rows = vec![vec!["track".to_owned(), "frames".into()]];
    for t in res["measured"].as_array().into_iter().flatten() {
        rows.push(vec![
            t["name"].as_str().unwrap_or("?").to_owned(),
            t["frames"].to_string(),
        ]);
    }
    lines.extend(show::table(&rows).into_iter().map(|l| format!("  {l}")));
    for t in res["unmeasured"].as_array().into_iter().flatten() {
        lines.push(format!(
            "not measured: {}: {}",
            t["name"].as_str().unwrap_or("?"),
            t["why"].as_str().unwrap_or("?")
        ));
    }
    if let Some(d) = res["dropped"].as_array().filter(|d| !d.is_empty()) {
        let ids: Vec<String> = d.iter().map(Value::to_string).collect();
        lines.push(format!("replaced captures {}", ids.join(", ")));
    }
    lines.push(saved_line(res));
    let after = client.call("bridge.status", Value::Null)?;
    Ok((lines, json!({ "job": s, "state": after["state"] })))
}

fn saved_line(r: &Value) -> String {
    if let Some(e) = r["save_error"].as_str() {
        return format!("not saved: {e}");
    }
    match r["saved"].as_str() {
        Some(d) => format!("saved in {d}"),
        None => "in memory only: save the project to keep the map".into(),
    }
}

/// Joins touching spans of the same kind into runs.
fn runs<'a>(spans: impl Iterator<Item = (f64, f64, &'a str)>) -> Vec<(f64, f64, &'a str)> {
    let mut out: Vec<(f64, f64, &str)> = Vec::new();
    for (a, b, k) in spans {
        match out.last_mut() {
            Some(last) if (last.1 - a).abs() < 1e-6 && last.2 == k => last.1 = b,
            _ => out.push((a, b, k)),
        }
    }
    out
}

fn ago(taken: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let s = now.saturating_sub(taken);
    match s {
        0..60 => format!("{s}s ago"),
        60..3600 => format!("{}m ago", s / 60),
        3600..86_400 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

/// Runs `mort map`.
pub fn map(client: &mut Client) -> Result<(Vec<String>, Value), Failure> {
    let r = client.call("map.status", Value::Null)?;
    let names: HashMap<String, String> = r["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(r["unmeasured"].as_array().into_iter().flatten())
        .map(|t| {
            (
                t["track"].as_str().unwrap_or("").to_owned(),
                t["name"].as_str().unwrap_or("?").to_owned(),
            )
        })
        .collect();
    let why = |s: &Value| -> String {
        let name = s["track"]
            .as_str()
            .and_then(|t| names.get(t))
            .map_or("a removed track", String::as_str);
        match s["why"].as_str() {
            Some("tempo") => "tempo changed".into(),
            Some("routing") => format!("{name} routed in or out"),
            _ => format!("{name} changed"),
        }
    };
    let captures = r["captures"].as_array().cloned().unwrap_or_default();
    if captures.is_empty() {
        return Ok((
            vec!["the map is empty: measure with `mort measure <range>`".into()],
            r,
        ));
    }
    let mut lines = vec![format!("{} captures", captures.len())];
    let mut rows = vec![vec![
        "#".to_owned(),
        "range".into(),
        "mode".into(),
        "taken".into(),
        "tracks".into(),
    ]];
    for c in &captures {
        rows.push(vec![
            c["id"].to_string(),
            span(secs(&c["start"]), secs(&c["end"])),
            c["mode"].as_str().unwrap_or("?").to_owned(),
            ago(c["taken"].as_u64().unwrap_or(0)),
            c["tracks"].to_string(),
        ]);
    }
    lines.extend(show::table(&rows).into_iter().map(|l| format!("  {l}")));
    let (rows, stale_count) = coverage_rows(&r, &why);
    lines.extend(show::table(&rows).into_iter().map(|l| format!("  {l}")));
    let missing: Vec<&str> = r["unmeasured"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str())
        .collect();
    if !missing.is_empty() {
        lines.push(format!("no data: {}", missing.join(", ")));
    }
    if stale_count > 0 {
        lines.push(format!(
            "{stale_count} tracks stale: re-measure with `mort measure --stale`"
        ));
    }
    lines.push(saved_line(&r));
    Ok((lines, r))
}

/// The track table of `mort map`, and how many tracks are stale.
fn coverage_rows(r: &Value, why: &dyn Fn(&Value) -> String) -> (Vec<Vec<String>>, usize) {
    let mut rows = vec![vec!["track".to_owned(), "covered".into(), "stale".into()]];
    let mut stale_count = 0;
    for t in r["tracks"].as_array().into_iter().flatten() {
        let spans = t["spans"].as_array().cloned().unwrap_or_default();
        let reasons: Vec<String> = spans
            .iter()
            .map(|s| {
                if s["stale"].is_null() {
                    String::new()
                } else {
                    why(&s["stale"])
                }
            })
            .collect();
        let covered = runs(
            spans
                .iter()
                .map(|s| (secs(&s["start"]), secs(&s["end"]), "")),
        );
        let stale = runs(
            spans
                .iter()
                .zip(&reasons)
                .filter(|(_, w)| !w.is_empty())
                .map(|(s, w)| (secs(&s["start"]), secs(&s["end"]), w.as_str())),
        );
        if !stale.is_empty() {
            stale_count += 1;
        }
        let mut stale_text = String::new();
        for (i, (a, b, w)) in stale.iter().enumerate() {
            if i > 0 {
                stale_text.push_str(", ");
            }
            let _ = write!(stale_text, "{} ({w})", span(*a, *b));
        }
        rows.push(vec![
            t["name"].as_str().unwrap_or("?").to_owned(),
            covered
                .iter()
                .map(|(a, b, _)| span(*a, *b))
                .collect::<Vec<_>>()
                .join(", "),
            if stale_text.is_empty() {
                "-".into()
            } else {
                stale_text
            },
        ]);
    }
    (rows, stale_count)
}
