//! Findings (ADR 0007 §2): problems found in the map, each with its
//! evidence and the usual remedy. Thresholds are in [`crate::thresholds`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use mort_features::{Frame, THIRD_OCTAVES};
use mort_proto::TrackId;

use crate::Span;
use crate::analysis::{Section, TILT_BANDS, TrackMeta, View, centre, db, tilt_fit};
use crate::thresholds::{CLASH_BELOW, Limits, used_by};

/// A kind of finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Two inputs of a bus competing in the same bands.
    Masking,
    /// Too much 200–500 Hz on a bus.
    Mud,
    /// Too much 2–5 kHz on a bus.
    Harsh,
    /// The master too close to full scale, or a bus over it.
    Headroom,
    /// Loudness moving sharply at a section boundary.
    LoudnessJump,
    /// Wide low end.
    LowEndWidth,
    /// Two MIDI parts a minor second apart low down.
    RegisterClash,
}

impl Kind {
    /// Every kind.
    pub const ALL: [Kind; 7] = [
        Kind::Masking,
        Kind::Mud,
        Kind::Harsh,
        Kind::Headroom,
        Kind::LoudnessJump,
        Kind::LowEndWidth,
        Kind::RegisterClash,
    ];

    /// Its name, as in JSON.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Kind::Masking => "masking",
            Kind::Mud => "mud",
            Kind::Harsh => "harsh",
            Kind::Headroom => "headroom",
            Kind::LoudnessJump => "loudness_jump",
            Kind::LowEndWidth => "low_end_width",
            Kind::RegisterClash => "register_clash",
        }
    }
}

/// A problem found in the map.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// What kind.
    pub kind: Kind,
    /// The section it is in.
    pub section: String,
    /// Where.
    #[serde(flatten)]
    pub span: Span,
    /// The tracks involved, most to blame first.
    pub tracks: Vec<TrackId>,
    /// The third-octave bands involved, by centre in Hz.
    pub bands: Vec<f64>,
    /// How bad, from 0 to 1.
    pub severity: f64,
    /// The numbers behind it.
    pub evidence: Value,
    /// The usual remedy, in one sentence. Never applied by mort.
    pub suggest: String,
    /// Whether any of the data behind it is stale.
    pub stale: bool,
    /// The thresholds it used that the project overrides (ADR 0012 §7).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub overridden: Vec<String>,
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn band_text(bands: &[f64]) -> String {
    match (bands.first(), bands.last()) {
        (Some(a), Some(b)) if (a - b).abs() < 1e-9 => format!("{a:.0} Hz"),
        (Some(a), Some(b)) => format!("{a:.0}–{b:.0} Hz"),
        _ => String::new(),
    }
}

/// Frames keyed by their place on a 50 ms grid from the section start.
fn grid<'f>(frames: &[(f64, &'f Frame)], start: f64) -> BTreeMap<i64, &'f Frame> {
    #[expect(clippy::cast_possible_truncation, reason = "grid slots are small")]
    frames
        .iter()
        .map(|(t, f)| (((t - start) * 20.0).round() as i64, *f))
        .collect()
}

/// The lowest excess `mud` and `harsh` severities are measured from.
const MUD_FLOOR: f64 = 0.0;

/// Bands masking looks at: 40 Hz to 12.5 kHz.
const MASK_BANDS: std::ops::RangeInclusive<usize> = 3..=28;

fn masking(v: &View<'_>, lim: &Limits, sec: &Section, out: &mut Vec<Finding>) {
    for bus in v.buses() {
        let bus_frames = v.frames(bus.id, sec.span);
        if bus_frames.is_empty() {
            continue;
        }
        let bus_grid = grid(&bus_frames, sec.span.start);
        let inputs: Vec<(&TrackMeta, BTreeMap<i64, &Frame>)> = v
            .inputs(bus.id)
            .into_iter()
            .filter(|t| t.gain > 0.0)
            .map(|t| (t, grid(&v.frames(t.id, sec.span), sec.span.start)))
            .filter(|(_, g)| !g.is_empty())
            .collect();
        #[expect(clippy::cast_precision_loss, reason = "frame counts are small")]
        let total = bus_grid.len() as f64;
        for (i, (a, ga)) in inputs.iter().enumerate() {
            for (c, gc) in inputs.iter().skip(i + 1) {
                let mut hits = [0usize; THIRD_OCTAVES];
                let mut sums = [(0.0, 0.0); THIRD_OCTAVES];
                for (slot, fb) in &bus_grid {
                    let (Some(fa), Some(fc)) = (ga.get(slot), gc.get(slot)) else {
                        continue;
                    };
                    for k in MASK_BANDS {
                        let eb = db(f64::from(fb.mid_bands[k]));
                        let ea = db(f64::from(fa.mid_bands[k]) * a.gain * a.gain);
                        let ec = db(f64::from(fc.mid_bands[k]) * c.gain * c.gain);
                        if eb > lim.mask_floor
                            && (ea - ec).abs() <= lim.mask_spread
                            && ea >= eb - lim.mask_bus
                            && ec >= eb - lim.mask_bus
                        {
                            hits[k] += 1;
                            sums[k].0 += ea;
                            sums[k].1 += ec;
                        }
                    }
                }
                #[expect(clippy::cast_precision_loss, reason = "frame counts are small")]
                let flagged: Vec<(usize, f64)> = MASK_BANDS
                    .map(|k| (k, hits[k] as f64 / total))
                    .filter(|(_, f)| *f > lim.mask_fraction)
                    .collect();
                #[expect(clippy::cast_precision_loss, reason = "a few bands")]
                let few = (flagged.len() as f64) < lim.mask_bands;
                if flagged.is_empty() || few {
                    continue;
                }
                let top = flagged.iter().map(|x| x.1).fold(0.0, f64::max);
                #[expect(clippy::cast_precision_loss, reason = "a few bands")]
                let breadth = (flagged.len() as f64 / 3.0).min(1.0);
                let bands: Vec<f64> = flagged.iter().map(|(k, _)| centre(*k)).collect();
                #[expect(clippy::cast_precision_loss, reason = "frame counts are small")]
                let detail: Vec<Value> = flagged
                    .iter()
                    .map(|(k, f)| {
                        let n = hits[*k] as f64;
                        json!({
                            "hz": centre(*k),
                            "fraction": round2(*f),
                            "a_db": round1(sums[*k].0 / n),
                            "b_db": round1(sums[*k].1 / n),
                        })
                    })
                    .collect();
                out.push(Finding {
                    kind: Kind::Masking,
                    section: sec.name.clone(),
                    span: sec.span,
                    tracks: vec![a.id, c.id],
                    severity: (top * breadth).clamp(0.0, 1.0),
                    evidence: json!({ "bus": bus.id, "bands": detail }),
                    suggest: format!(
                        "{} and {} compete at {} in {}: cut one there, or move one in register or pan",
                        a.name,
                        c.name,
                        band_text(&bands),
                        bus.name
                    ),
                    bands,
                    overridden: vec![],
            stale: v.stale(a.id, sec.span).is_some()
                        || v.stale(c.id, sec.span).is_some()
                        || v.stale(bus.id, sec.span).is_some(),
                });
            }
        }
    }
}

/// `mud` (200–500 Hz) or `harsh` (2–5 kHz) on every bus.
fn excess(v: &View<'_>, lim: &Limits, sec: &Section, kind: Kind, out: &mut Vec<Finding>) {
    let (test, limit): (Vec<usize>, f64) = match kind {
        Kind::Mud => ((10..=14).collect(), lim.mud_excess),
        _ => ((20..=24).collect(), lim.harsh_excess),
    };
    for bus in v.buses() {
        let Some(st) = v.stats(bus.id, sec.span) else {
            continue;
        };
        #[expect(clippy::cast_precision_loss, reason = "five bands")]
        let level = db(test.iter().map(|k| st.bands[*k]).sum::<f64>() / test.len() as f64);
        if level < lim.tilt_band_floor {
            continue;
        }
        let (slope, icept) = tilt_fit(&st.bands, TILT_BANDS.0, TILT_BANDS.1, &test);
        #[expect(clippy::cast_precision_loss, reason = "five bands")]
        let over = test
            .iter()
            .map(|k| db(st.bands[*k]) - (icept + slope * centre(*k).log2()))
            .sum::<f64>()
            / test.len() as f64;
        if over <= limit {
            continue;
        }
        let mut shares: Vec<(&TrackMeta, f64)> = v
            .inputs(bus.id)
            .into_iter()
            .filter_map(|t| {
                let s = v.stats(t.id, sec.span)?;
                Some((
                    t,
                    test.iter().map(|k| s.bands[*k]).sum::<f64>() * t.gain * t.gain,
                ))
            })
            .collect();
        let sum: f64 = shares.iter().map(|x| x.1).sum();
        shares.sort_by(|a, b| b.1.total_cmp(&a.1));
        let blamed: Vec<&(&TrackMeta, f64)> = shares
            .iter()
            .filter(|x| sum > 0.0 && x.1 / sum >= lim.blame_share)
            .take(4)
            .collect();
        let bands: Vec<f64> = test.iter().map(|k| centre(*k)).collect();
        let names: Vec<String> = blamed
            .iter()
            .map(|(t, e)| format!("{} {:.0}%", t.name, 100.0 * e / sum))
            .collect();
        let (what, fix) = match kind {
            Kind::Mud => ("200–500 Hz", "cut 200–500 Hz on"),
            _ => ("2–5 kHz", "soften 2–5 kHz on"),
        };
        let who = if names.is_empty() {
            bus.name.clone()
        } else {
            blamed
                .iter()
                .map(|(t, _)| t.name.clone())
                .collect::<Vec<_>>()
                .join(" or ")
        };
        out.push(Finding {
            kind,
            section: sec.name.clone(),
            span: sec.span,
            tracks: std::iter::once(bus.id)
                .chain(blamed.iter().map(|(t, _)| t.id))
                .collect(),
            bands,
            severity: ((over - limit.max(MUD_FLOOR)) / 6.0).clamp(0.05, 1.0),
            evidence: json!({
                "bus": bus.id,
                "excess_db": round1(over),
                "tilt_db_per_octave": round1(slope),
                "shares": names,
            }),
            suggest: format!(
                "{} has {:.1} dB more {what} than its tilt predicts: {fix} {who}",
                bus.name, over
            ),
            overridden: vec![],
            stale: v.stale(bus.id, sec.span).is_some(),
        });
    }
}

fn headroom(v: &View<'_>, lim: &Limits, sec: &Section, out: &mut Vec<Finding>) {
    for bus in v.buses() {
        let frames = v.frames(bus.id, sec.span);
        let is_master = bus.id == v.master;
        let (limit, value): (f64, fn(&Frame) -> f32) = if is_master {
            (lim.headroom, |f| f.true_peak_dbtp)
        } else {
            (lim.bus_clip, |f| f.sample_peak_dbfs)
        };
        let over: Vec<&(f64, &Frame)> = frames
            .iter()
            .filter(|(_, f)| f64::from(value(f)) > limit)
            .collect();
        let Some((at, worst)) = over
            .iter()
            .map(|(t, f)| (*t, f64::from(value(f))))
            .max_by(|a, b| a.1.total_cmp(&b.1))
        else {
            continue;
        };
        let suggest = if is_master {
            format!(
                "the master peaks at {worst:.1} dBTP at {at:.2}s: lower the mix into the master, or limit it to {} dBTP",
                crate::thresholds::HEADROOM_DBTP
            )
        } else {
            format!(
                "{} peaks at {worst:.1} dBFS inside: lower its inputs or its own input gain",
                bus.name
            )
        };
        out.push(Finding {
            kind: Kind::Headroom,
            section: sec.name.clone(),
            span: sec.span,
            tracks: vec![bus.id],
            bands: vec![],
            severity: ((worst - limit.max(-1.0)) / 3.0).clamp(0.05, 1.0),
            evidence: json!({
                "peak": round1(worst),
                "at": at,
                "frames_over": over.len(),
                "measure": if is_master { "true_peak_dbtp" } else { "sample_peak_dbfs" },
            }),
            suggest,
            overridden: vec![],
            stale: v.stale(bus.id, sec.span).is_some(),
        });
    }
}

fn loudness_jumps(v: &View<'_>, lim: &Limits, out: &mut Vec<Finding>) {
    for pair in v.sections.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        let at = prev.span.end;
        if (next.span.start - at).abs() > 1e-6 {
            continue;
        }
        let before = v
            .frames(v.master, prev.span)
            .last()
            .map(|(_, f)| f64::from(f.short_term_lufs));
        // The first frame whose 3 s window lies wholly in the new section,
        // or the section's last frame if it is shorter.
        let later = v.frames(v.master, next.span);
        let after = later
            .iter()
            .find(|(t, _)| *t + 0.05 >= at + 3.0 - 1e-6)
            .or(later.last())
            .map(|(t, f)| (*t, f64::from(f.short_term_lufs)));
        // Short-term loudness is NaN where it is unknown (ADR 0012 §2).
        let (Some(from), Some((t, to))) = (before, after) else {
            continue;
        };
        if !from.is_finite() || !to.is_finite() {
            continue;
        }
        let jump = to - from;
        if jump.abs() <= lim.loudness_jump || from < -69.0 && to < -69.0 {
            continue;
        }
        out.push(Finding {
            kind: Kind::LoudnessJump,
            section: next.name.clone(),
            span: Span::new(at, t + 0.05),
            tracks: vec![v.master],
            bands: vec![],
            severity: ((jump.abs() - lim.loudness_jump.max(0.0)) / 6.0).clamp(0.05, 1.0),
            evidence: json!({
                "from": prev.name,
                "before_lufs": round1(from),
                "after_lufs": round1(to),
                "jump_lu": round1(jump),
            }),
            suggest: format!(
                "short-term loudness moves {jump:+.1} LU into {}: if that is not meant, ride the level across the boundary",
                next.name
            ),
            overridden: vec![],
            stale: v.stale(v.master, prev.span).is_some() || v.stale(v.master, next.span).is_some(),
        });
    }
}

/// Energy below 141 Hz: the third octaves up to 125 Hz.
fn low_energy(f: &Frame) -> f64 {
    f.mid_bands[..=8].iter().map(|b| f64::from(*b)).sum()
}

fn low_end_width(v: &View<'_>, lim: &Limits, sec: &Section, out: &mut Vec<Finding>) {
    for t in &v.tracks {
        let is_bus = t.id != v.master && !v.inputs(t.id).is_empty();
        if is_bus {
            continue;
        }
        let frames = v.frames(t.id, sec.span);
        let low: Vec<f64> = frames
            .iter()
            .filter(|(_, f)| db(low_energy(f)) > lim.low_floor)
            .map(|(_, f)| f64::from(f.correlation_low))
            .collect();
        if low.is_empty() {
            continue;
        }
        #[expect(clippy::cast_precision_loss, reason = "frame counts are small")]
        let n = low.len() as f64;
        #[expect(clippy::cast_precision_loss, reason = "frame counts are small")]
        let fraction = low.iter().filter(|c| **c < lim.low_corr).count() as f64 / n;
        if fraction <= lim.low_fraction {
            continue;
        }
        let mean = low.iter().sum::<f64>() / n;
        out.push(Finding {
            kind: Kind::LowEndWidth,
            section: sec.name.clone(),
            span: sec.span,
            tracks: vec![t.id],
            bands: (0..=8).map(centre).collect(),
            severity: ((lim.low_corr - mean) / (1.0 + lim.low_corr)).clamp(0.05, 1.0),
            evidence: json!({
                "correlation_low": round2(mean),
                "fraction": round2(fraction),
            }),
            suggest: format!(
                "{}'s low end is wide (correlation {mean:.2} below 150 Hz): make it mono below about 150 Hz",
                t.name
            ),
            overridden: vec![],
            stale: v.stale(t.id, sec.span).is_some(),
        });
    }
}

fn note_name(p: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", NAMES[usize::from(p % 12)], i32::from(p / 12) - 1)
}

fn register_clash(v: &View<'_>, lim: &Limits, sec: &Section, out: &mut Vec<Finding>) {
    let parts: Vec<&TrackMeta> = v.tracks.iter().filter(|t| !t.notes.is_empty()).collect();
    for (i, a) in parts.iter().enumerate() {
        for c in parts.iter().skip(i + 1) {
            let mut total = 0.0;
            let mut count = 0;
            let mut first: Option<(f64, u8, u8)> = None;
            for n in a.notes.iter().filter(|n| n.pitch < CLASH_BELOW) {
                for m in c.notes.iter().filter(|m| m.pitch < CLASH_BELOW) {
                    if n.pitch.abs_diff(m.pitch) != 1 {
                        continue;
                    }
                    let s = n.start.max(m.start).max(sec.span.start);
                    let e = n.end.min(m.end).min(sec.span.end);
                    if e - s > 0.0 && e - s >= lim.clash_min {
                        total += e - s;
                        count += 1;
                        if first.is_none_or(|f| s < f.0) {
                            first = Some((s, n.pitch, m.pitch));
                        }
                    }
                }
            }
            let Some((at, p, q)) = first else {
                continue;
            };
            out.push(Finding {
                kind: Kind::RegisterClash,
                section: sec.name.clone(),
                span: sec.span,
                tracks: vec![a.id, c.id],
                bands: vec![],
                severity: (total / 2.0).clamp(0.05, 1.0),
                evidence: json!({
                    "seconds": round1(total * 10.0) / 10.0,
                    "clashes": count,
                    "first": { "at": at, "notes": [note_name(p), note_name(q)] },
                }),
                suggest: format!(
                    "{} ({}) and {} ({}) sustain a minor second apart low down, first at {at:.2}s: revoice one, or move it an octave",
                    a.name,
                    note_name(p),
                    c.name,
                    note_name(q)
                ),
                stale: false,
                overridden: vec![],
            });
        }
    }
}

/// Every finding in the view's sections, by section, most severe first.
#[must_use]
pub fn findings(v: &View<'_>, lim: &Limits) -> Vec<Finding> {
    let mut out = Vec::new();
    for sec in &v.sections {
        let mut here = Vec::new();
        masking(v, lim, sec, &mut here);
        excess(v, lim, sec, Kind::Mud, &mut here);
        excess(v, lim, sec, Kind::Harsh, &mut here);
        headroom(v, lim, sec, &mut here);
        low_end_width(v, lim, sec, &mut here);
        register_clash(v, lim, sec, &mut here);
        here.sort_by(|a, b| b.severity.total_cmp(&a.severity));
        out.extend(here);
    }
    let mut jumps = Vec::new();
    loudness_jumps(v, lim, &mut jumps);
    for j in jumps {
        let at = out
            .iter()
            .position(|f| f.span.start >= j.span.start - 1e-6)
            .unwrap_or(out.len());
        out.insert(at, j);
    }
    for f in &mut out {
        f.severity = round2(f.severity);
        f.overridden = used_by(f.kind)
            .iter()
            .filter(|k| lim.get(k) != Limits::DEFAULT.get(k))
            .map(|k| (*k).to_owned())
            .collect();
    }
    out
}
