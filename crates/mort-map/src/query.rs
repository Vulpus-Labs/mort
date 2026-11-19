//! Queries at three levels (ADR 0007 §4): sections, one track, and raw
//! frames.

use std::collections::BTreeMap;

use serde::Serialize;

use mort_features::{Frame, OCTAVE_CENTRES_HZ, THIRD_OCTAVES};
use mort_proto::TrackId;

use crate::Span;
use crate::analysis::{Stats, View, centre, db};
use crate::findings::{Finding, Kind};

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// The master over one section.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MasterBrief {
    /// Loudness, LUFS.
    pub loudness: f64,
    /// Highest short-term loudness, LUFS.
    pub short_term_max: f64,
    /// Highest true peak, dBTP.
    pub true_peak: f64,
    /// Spectral tilt, dB per octave.
    pub tilt: f64,
    /// Side energy relative to mid, dB.
    pub side: f64,
    /// Mean correlation below 150 Hz.
    pub correlation_low: f64,
    /// Onsets per second.
    pub density: f64,
}

/// One track over one section, as the summary shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackBrief {
    /// The track.
    pub track: TrackId,
    /// Its name.
    pub name: String,
    /// Its loudness in the mix: its own, plus its gain to the master, LUFS.
    pub level: f64,
    /// Its share of the master's energy, percent.
    pub share: f64,
    /// Its centroid, Hz.
    pub centroid: f64,
    /// Its loudest third octave, Hz.
    pub peak_band: f64,
    /// Whether its data here is stale.
    pub stale: bool,
}

/// One section of `summary`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SectionSummary {
    /// The section.
    pub section: String,
    /// Where.
    #[serde(flatten)]
    pub span: Span,
    /// The master, if measured.
    pub master: Option<MasterBrief>,
    /// Every measured track but the master, loudest in the mix first.
    pub tracks: Vec<TrackBrief>,
    /// Findings here, by kind.
    pub findings: BTreeMap<Kind, usize>,
    /// Whether any data here is stale.
    pub stale: bool,
}

fn total(s: &Stats) -> f64 {
    s.bands.iter().sum()
}

fn peak_band(s: &Stats) -> f64 {
    (0..THIRD_OCTAVES)
        .max_by(|a, b| s.bands[*a].total_cmp(&s.bands[*b]))
        .map_or(0.0, centre)
}

/// `summary(range)`: one entry per section.
#[must_use]
pub fn summary(v: &View<'_>, findings: &[Finding]) -> Vec<SectionSummary> {
    v.sections
        .iter()
        .map(|sec| {
            let m = v.stats(v.master, sec.span);
            let master_energy = m.as_ref().map_or(0.0, total);
            let mut tracks: Vec<TrackBrief> = v
                .tracks
                .iter()
                .filter(|t| t.id != v.master)
                .filter_map(|t| {
                    let s = v.stats(t.id, sec.span)?;
                    let g = v.gain_to_master(t.id);
                    let share = if master_energy > 0.0 {
                        100.0 * total(&s) * g * g / master_energy
                    } else {
                        0.0
                    };
                    Some(TrackBrief {
                        track: t.id,
                        name: t.name.clone(),
                        level: r1(s.loudness + db(g * g)),
                        share: r1(share),
                        centroid: s.centroid.round(),
                        peak_band: peak_band(&s),
                        stale: v.stale(t.id, sec.span).is_some(),
                    })
                })
                .collect();
            tracks.sort_by(|a, b| b.level.total_cmp(&a.level));
            let mut counts = BTreeMap::new();
            for f in findings
                .iter()
                .filter(|f| f.section == sec.name && sec.span.intersect(f.span).is_some())
            {
                *counts.entry(f.kind).or_insert(0) += 1;
            }
            let stale = v.stale(v.master, sec.span).is_some() || tracks.iter().any(|t| t.stale);
            SectionSummary {
                section: sec.name.clone(),
                span: sec.span,
                master: m.map(|s| MasterBrief {
                    loudness: r1(s.loudness),
                    short_term_max: r1(s.short_term_max),
                    true_peak: r1(s.true_peak),
                    tilt: r1(s.tilt),
                    side: r1(s.side),
                    correlation_low: r1(s.correlation_low * 10.0) / 10.0,
                    density: r1(s.density),
                }),
                tracks,
                findings: counts,
                stale,
            }
        })
        .collect()
}

/// A track's share of its bus's energy in five ranges, percent.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Shares {
    /// The bus.
    pub bus: TrackId,
    /// Below 150 Hz.
    pub low: f64,
    /// 150–500 Hz.
    pub low_mid: f64,
    /// 500 Hz – 2 kHz.
    pub mid: f64,
    /// 2–6 kHz.
    pub high_mid: f64,
    /// Above 6 kHz.
    pub high: f64,
}

/// Third-octave index ranges of [`Shares`].
const SHARE_RANGES: [(usize, usize); 5] = [(0, 8), (9, 13), (14, 19), (20, 25), (26, 30)];

/// One section of `track`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackSection {
    /// The section.
    pub section: String,
    /// Where.
    #[serde(flatten)]
    pub span: Span,
    /// Its statistics.
    pub stats: Stats,
    /// Its three loudest third octaves: centre in Hz and level in dB.
    pub top_bands: Vec<(f64, f64)>,
    /// Its share of its bus, if the bus was measured.
    pub share: Option<Shares>,
    /// Why its data here is stale, if it is.
    pub stale: Option<crate::Stale>,
}

/// `track(id, range)`: the track's features per section.
#[must_use]
pub fn track(v: &View<'_>, id: TrackId) -> Vec<TrackSection> {
    let meta = v.meta(id);
    v.sections
        .iter()
        .filter_map(|sec| {
            let s = v.stats(id, sec.span)?;
            let mut order: Vec<usize> = (0..THIRD_OCTAVES).collect();
            order.sort_by(|a, b| s.bands[*b].total_cmp(&s.bands[*a]));
            let top_bands = order
                .iter()
                .take(3)
                .map(|k| (centre(*k), r1(db(s.bands[*k]))))
                .collect();
            let share = meta.and_then(|m| {
                let bus = m.bus?;
                let b = v.stats(bus, sec.span)?;
                let pct = |(lo, hi): (usize, usize)| {
                    let mine: f64 = s.bands[lo..=hi].iter().sum::<f64>() * m.gain * m.gain;
                    let all: f64 = b.bands[lo..=hi].iter().sum();
                    if all > 0.0 {
                        r1(100.0 * mine / all)
                    } else {
                        0.0
                    }
                };
                Some(Shares {
                    bus,
                    low: pct(SHARE_RANGES[0]),
                    low_mid: pct(SHARE_RANGES[1]),
                    mid: pct(SHARE_RANGES[2]),
                    high_mid: pct(SHARE_RANGES[3]),
                    high: pct(SHARE_RANGES[4]),
                })
            });
            Some(TrackSection {
                section: sec.name.clone(),
                span: sec.span,
                top_bands,
                share,
                stale: v.stale(id, sec.span),
                stats: s,
            })
        })
        .collect()
}

/// The features `frames` can give: the frame's scalar fields, `band:<Hz>`
/// for a third octave's mid level and `side:<Hz>` for an octave's side
/// level, both in dB.
pub const SCALARS: [&str; 11] = [
    "momentary_lufs",
    "short_term_lufs",
    "sample_peak_dbfs",
    "true_peak_dbtp",
    "rms_dbfs",
    "crest_db",
    "correlation",
    "correlation_low",
    "centroid_hz",
    "flatness",
    "onset",
];

/// How a scalar is averaged over a window.
#[derive(Clone, Copy)]
enum Avg {
    /// Levels: as power.
    Power,
    /// Peaks: the highest.
    Max,
    /// The rest: the mean.
    Mean,
}

enum Feature {
    Scalar(fn(&Frame) -> f32, Avg),
    Band(usize),
    Side(usize),
}

fn feature(name: &str) -> Result<Feature, String> {
    let near = |hz: f64, centres: &[f32]| -> usize {
        (0..centres.len())
            .min_by(|a, b| {
                (f64::from(centres[*a]).ln() - hz.ln())
                    .abs()
                    .total_cmp(&(f64::from(centres[*b]).ln() - hz.ln()).abs())
            })
            .unwrap_or(0)
    };
    let hz = |s: &str| -> Result<f64, String> {
        let t = s.trim().trim_end_matches("Hz").trim();
        let (n, k) = match t.strip_suffix(['k', 'K']) {
            Some(n) => (n, 1000.0),
            None => (t, 1.0),
        };
        n.parse::<f64>()
            .map(|x| x * k)
            .ok()
            .filter(|x| *x > 0.0)
            .ok_or_else(|| format!("`{s}` is not a frequency"))
    };
    if let Some(f) = name.strip_prefix("band:") {
        return Ok(Feature::Band(near(
            hz(f)?,
            &mort_features::THIRD_OCTAVE_CENTRES_HZ,
        )));
    }
    if let Some(f) = name.strip_prefix("side:") {
        return Ok(Feature::Side(near(hz(f)?, &OCTAVE_CENTRES_HZ)));
    }
    let scalar: (fn(&Frame) -> f32, Avg) = match name {
        "momentary_lufs" => (|f| f.momentary_lufs, Avg::Power),
        "short_term_lufs" => (|f| f.short_term_lufs, Avg::Power),
        "sample_peak_dbfs" => (|f| f.sample_peak_dbfs, Avg::Max),
        "true_peak_dbtp" => (|f| f.true_peak_dbtp, Avg::Max),
        "rms_dbfs" => (|f| f.rms_dbfs, Avg::Power),
        "crest_db" => (|f| f.crest_db, Avg::Mean),
        "correlation" => (|f| f.correlation, Avg::Mean),
        "correlation_low" => (|f| f.correlation_low, Avg::Mean),
        "centroid_hz" => (|f| f.centroid_hz, Avg::Mean),
        "flatness" => (|f| f.flatness, Avg::Mean),
        "onset" => (|f| f.onset, Avg::Mean),
        _ => {
            return Err(format!(
                "no feature `{name}`; use {}, band:<Hz> or side:<Hz>",
                SCALARS.join(", ")
            ));
        }
    };
    Ok(Feature::Scalar(scalar.0, scalar.1))
}

/// `frames(id, range, features, step)`: raw frames resampled to `step`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Resampled {
    /// The feature names, in column order.
    pub columns: Vec<String>,
    /// Rows: the window's start, then one value per column.
    pub rows: Vec<(f64, Vec<f64>)>,
}

/// Resamples a track's frames over `span` into windows of `step` seconds
/// (at least one frame). Levels are averaged as power, peaks take the
/// highest, the rest the mean.
///
/// # Errors
///
/// An unknown feature name, with the names that exist.
pub fn frames(
    v: &View<'_>,
    id: TrackId,
    span: Span,
    features: &[String],
    step: f64,
) -> Result<Resampled, String> {
    let feats: Vec<Feature> = features
        .iter()
        .map(|f| feature(f))
        .collect::<Result<_, _>>()?;
    let step = step.max(0.05);
    let mut windows: BTreeMap<i64, Vec<&Frame>> = BTreeMap::new();
    for (t, f) in v.frames(id, span) {
        #[expect(clippy::cast_possible_truncation, reason = "window indices are small")]
        let w = ((t - span.start) / step + 1e-6).floor() as i64;
        windows.entry(w).or_default().push(f);
    }
    #[expect(clippy::cast_precision_loss, reason = "window indices are small")]
    let rows = windows
        .into_iter()
        .map(|(w, fs)| {
            let n = fs.len() as f64;
            let vals = feats
                .iter()
                .map(|feat| {
                    let x = match feat {
                        Feature::Band(k) => {
                            db(fs.iter().map(|f| f64::from(f.mid_bands[*k])).sum::<f64>() / n)
                        }
                        Feature::Side(k) => db(fs
                            .iter()
                            .map(|f| f64::from(f.side_octaves[*k]))
                            .sum::<f64>()
                            / n),
                        Feature::Scalar(get, Avg::Power) => db(fs
                            .iter()
                            .map(|f| 10f64.powf(f64::from(get(f)) / 10.0))
                            .sum::<f64>()
                            / n),
                        Feature::Scalar(get, Avg::Max) => fs
                            .iter()
                            .map(|f| f64::from(get(f)))
                            .fold(f64::MIN, f64::max),
                        Feature::Scalar(get, Avg::Mean) => {
                            fs.iter().map(|f| f64::from(get(f))).sum::<f64>() / n
                        }
                    };
                    (x * 100.0).round() / 100.0
                })
                .collect();
            (span.start + w as f64 * step, vals)
        })
        .collect();
    Ok(Resampled {
        columns: features.to_vec(),
        rows,
    })
}
