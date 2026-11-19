//! `mort summary`, `mort findings`, `mort report` and `mort frames`
//! (ticket 0013, ADR 0007 §4).

use std::collections::HashMap;

use clap::Args;
use serde_json::{Value, json};

use crate::client::{Client, Failure};
use crate::{resolve, show, transport};

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn span(v: &Value) -> String {
    format!("{:.2}–{:.2}s", f(&v["start"]), f(&v["end"]))
}

fn hz(x: f64) -> String {
    if x >= 1000.0 {
        format!("{:.1} kHz", x / 1000.0)
    } else {
        format!("{x:.0} Hz")
    }
}

/// Adds the range, if given, to `params`.
fn with_range(
    client: &mut Client,
    range: Option<&str>,
    mut params: Value,
) -> Result<Value, Failure> {
    if let Some(r) = range {
        let (a, b) = transport::range(client, r)?;
        params["start"] = json!(a);
        params["end"] = json!(b);
    }
    Ok(params)
}

const STALE_NOTE: &str = "* stale: re-measure with `mort measure --stale`";

/// `mort summary` arguments.
#[derive(Args)]
pub struct Summary {
    /// The range, such as `9.1..17.1`; everything measured if left out.
    range: Option<String>,
}

/// Runs `mort summary`.
pub fn summary(client: &mut Client, args: &Summary) -> Result<(Vec<String>, Value), Failure> {
    let p = with_range(client, args.range.as_deref(), json!({}))?;
    let r = client.call("map.summary", p)?;
    let mut lines = Vec::new();
    let mut any_stale = false;
    for sec in r["sections"].as_array().into_iter().flatten() {
        let star = if sec["stale"] == true { " *" } else { "" };
        let m = &sec["master"];
        let head = if m.is_null() {
            "master not measured".to_owned()
        } else {
            format!(
                "{:.1} LUFS, peak {:.1} dBTP, tilt {:+.1} dB/oct, side {:.0} dB, {:.1} onsets/s",
                f(&m["loudness"]),
                f(&m["true_peak"]),
                f(&m["tilt"]),
                f(&m["side"]),
                f(&m["density"])
            )
        };
        lines.push(format!(
            "{} {}{star}  {head}",
            sec["section"].as_str().unwrap_or("?"),
            span(sec)
        ));
        let counts: Vec<String> = sec["findings"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, n)| format!("{k} {n}"))
            .collect();
        lines.push(format!(
            "  findings: {}",
            if counts.is_empty() {
                "none".to_owned()
            } else {
                counts.join(", ")
            }
        ));
        let mut rows = vec![vec![
            "track".to_owned(),
            "level".into(),
            "share".into(),
            "centroid".into(),
            "loudest".into(),
        ]];
        for t in sec["tracks"].as_array().into_iter().flatten() {
            let stale = t["stale"] == true;
            any_stale |= stale;
            rows.push(vec![
                format!(
                    "{}{}",
                    t["name"].as_str().unwrap_or("?"),
                    if stale { " *" } else { "" }
                ),
                format!("{:.1}", f(&t["level"])),
                format!("{:.0}%", f(&t["share"])),
                hz(f(&t["centroid"])),
                hz(f(&t["peak_band"])),
            ]);
        }
        lines.extend(show::table(&rows).into_iter().map(|l| format!("  {l}")));
        any_stale |= sec["stale"] == true;
    }
    lines.push(
        "level: LUFS in the mix (fader and pan applied); share: of the master's energy".into(),
    );
    if any_stale {
        lines.push(STALE_NOTE.into());
    }
    Ok((lines, r))
}

/// `mort findings` arguments.
#[derive(Args)]
pub struct Findings {
    /// The range, such as `9.1..17.1`; everything measured if left out.
    range: Option<String>,
    /// Only findings of these kinds: `masking`, `mud`, `harsh`,
    /// `headroom`, `loudness_jump`, `low_end_width`, `register_clash`.
    #[arg(long, value_delimiter = ',')]
    kind: Vec<String>,
    /// Report every measurement as if it were a finding, to see how close
    /// each is to its threshold.
    #[arg(long)]
    all: bool,
    /// Print the thresholds and the reasons for them instead.
    #[arg(long)]
    thresholds: bool,
    /// Show at most this many, worst first within each section; 0 shows
    /// all. `--json` always gives all.
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

/// Runs `mort findings`.
pub fn findings(client: &mut Client, args: &Findings) -> Result<(Vec<String>, Value), Failure> {
    if args.thresholds {
        let r = client.call("map.thresholds", Value::Null)?;
        let mut rows = vec![vec!["name".to_owned(), "value".into(), "why".into()]];
        for t in r["thresholds"].as_array().into_iter().flatten() {
            rows.push(vec![
                t["name"].as_str().unwrap_or("?").to_owned(),
                format!("{} {}", t["value"], t["unit"].as_str().unwrap_or(""))
                    .trim()
                    .to_owned(),
                t["reason"].as_str().unwrap_or("").to_owned(),
            ]);
        }
        return Ok((show::table(&rows), r));
    }
    let mut p = with_range(client, args.range.as_deref(), json!({ "all": args.all }))?;
    if !args.kind.is_empty() {
        p["kinds"] = json!(args.kind);
    }
    let r = client.call("map.findings", p)?;
    let fs = r["findings"].as_array().cloned().unwrap_or_default();
    let mut lines = Vec::new();
    // The worst, kept in section order.
    let mut order: Vec<usize> = (0..fs.len()).collect();
    order.sort_by(|a, b| f(&fs[*b]["severity"]).total_cmp(&f(&fs[*a]["severity"])));
    let keep = if args.limit == 0 {
        fs.len()
    } else {
        args.limit.min(fs.len())
    };
    let mut shown: Vec<usize> = order[..keep].to_vec();
    shown.sort_unstable();
    let mut by_section: Vec<(String, Vec<&Value>)> = Vec::new();
    for x in shown.iter().map(|i| &fs[*i]) {
        let key = format!("{} {}", x["section"].as_str().unwrap_or("?"), span(x));
        match by_section.last_mut() {
            Some((k, v)) if *k == key => v.push(x),
            _ => by_section.push((key, vec![x])),
        }
    }
    let mut stale = 0;
    for (head, items) in &by_section {
        lines.push(head.clone());
        for x in items {
            let star = if x["stale"] == true {
                stale += 1;
                " *"
            } else {
                ""
            };
            lines.push(format!(
                "  {:.2} {:<14} {}{star}",
                f(&x["severity"]),
                x["kind"].as_str().unwrap_or("?"),
                x["suggest"].as_str().unwrap_or("")
            ));
        }
    }
    lines.push(match fs.len() {
        0 => "no findings".to_owned(),
        1 => "1 finding".to_owned(),
        n if keep < n => format!(
            "{n} findings, the worst {keep} shown; {} more: --limit 0 shows all",
            n - keep
        ),
        n => format!("{n} findings"),
    });
    if stale > 0 {
        lines.push(STALE_NOTE.into());
    }
    Ok((lines, r))
}

/// `mort report` arguments.
#[derive(Args)]
pub struct Report {
    /// The track: number, name, part of a name, id, or `master`.
    track: String,
    /// The range, such as `9.1..17.1`; everything measured if left out.
    range: Option<String>,
}

/// Runs `mort report`.
pub fn report(client: &mut Client, args: &Report) -> Result<(Vec<String>, Value), Failure> {
    let list = client.call("tracks.list", Value::Null)?;
    let t = resolve::track(&list, &args.track)?;
    let names: HashMap<String, String> = list["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(std::iter::once(&list["master"]))
        .map(|t| {
            (
                t["id"].as_str().unwrap_or("").to_owned(),
                t["name"].as_str().unwrap_or("?").to_owned(),
            )
        })
        .collect();
    let p = with_range(client, args.range.as_deref(), json!({ "track": t["id"] }))?;
    let r = client.call("map.track", p)?;
    let mut lines = vec![r["name"].as_str().unwrap_or("?").to_owned()];
    let secs = r["sections"].as_array().cloned().unwrap_or_default();
    if secs.is_empty() {
        lines.push("  no data: measure it with `mort measure <range> <track>`".into());
    }
    let mut any_stale = false;
    for sec in &secs {
        let s = &sec["stats"];
        let stale = !sec["stale"].is_null();
        any_stale |= stale;
        lines.push(format!(
            "{} {}{}  {:.1} LUFS, rms {:.1}, peak {:.1} dBTP, crest {:.1}, centroid {}, tilt {:+.1} dB/oct, {:.1} onsets/s",
            sec["section"].as_str().unwrap_or("?"),
            span(sec),
            if stale { " *" } else { "" },
            f(&s["loudness"]),
            f(&s["rms"]),
            f(&s["true_peak"]),
            f(&s["crest"]),
            hz(f(&s["centroid"])),
            f(&s["tilt"]),
            f(&s["density"]),
        ));
        lines.push(format!(
            "  width: side {:.0} dB, correlation {:.2}, below 150 Hz {:.2}",
            f(&s["side"]),
            f(&s["correlation"]),
            f(&s["correlation_low"])
        ));
        let top: Vec<String> = sec["top_bands"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|b| format!("{} {:.1} dB", hz(f(&b[0])), f(&b[1])))
            .collect();
        lines.push(format!("  loudest bands: {}", top.join(", ")));
        let sh = &sec["share"];
        if !sh.is_null() {
            let bus = sh["bus"]
                .as_str()
                .and_then(|b| names.get(b))
                .map_or("its bus", String::as_str);
            lines.push(format!(
                "  share of {bus}: <150 Hz {:.0}%, 150–500 {:.0}%, 500–2k {:.0}%, 2–6k {:.0}%, >6k {:.0}%",
                f(&sh["low"]),
                f(&sh["low_mid"]),
                f(&sh["mid"]),
                f(&sh["high_mid"]),
                f(&sh["high"])
            ));
        }
    }
    if any_stale {
        lines.push(STALE_NOTE.into());
    }
    Ok((lines, r))
}

/// `mort frames` arguments.
#[derive(Args)]
pub struct Frames {
    /// The track: number, name, part of a name, id, or `master`.
    track: String,
    /// The range, such as `9.1..10.1`.
    range: String,
    /// Features, comma-separated: `momentary_lufs`, `short_term_lufs`,
    /// `sample_peak_dbfs`, `true_peak_dbtp`, `rms_dbfs`, `crest_db`,
    /// `correlation`, `correlation_low`, `centroid_hz`, `flatness`, `onset`,
    /// `band:<Hz>` (a third octave's mid level, dB) and `side:<Hz>` (an
    /// octave's side level, dB).
    #[arg(
        long,
        short,
        value_delimiter = ',',
        default_value = "momentary_lufs,rms_dbfs,centroid_hz"
    )]
    features: Vec<String>,
    /// Seconds per row; frames are 0.05 s.
    #[arg(long, default_value_t = 0.5)]
    step: f64,
}

/// Runs `mort frames`.
pub fn frames(client: &mut Client, args: &Frames) -> Result<(Vec<String>, Value), Failure> {
    let list = client.call("tracks.list", Value::Null)?;
    let t = resolve::track(&list, &args.track)?;
    let p = with_range(
        client,
        Some(&args.range),
        json!({ "track": t["id"], "features": args.features, "step": args.step }),
    )?;
    let r = client.call("map.frames", p)?;
    let mut rows = vec![
        std::iter::once("time".to_owned())
            .chain(args.features.iter().cloned())
            .collect::<Vec<_>>(),
    ];
    for row in r["rows"].as_array().into_iter().flatten() {
        let mut cells = vec![format!("{:.2}", f(&row[0]))];
        cells.extend(
            row[1]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| format!("{:.2}", f(v))),
        );
        rows.push(cells);
    }
    let mut lines = show::table(&rows);
    if rows.len() == 1 {
        lines.push("no frames there: measure it with `mort measure <range> <track>`".into());
    }
    if !r["stale"].is_null() {
        lines.push(STALE_NOTE.into());
    }
    Ok((lines, r))
}
