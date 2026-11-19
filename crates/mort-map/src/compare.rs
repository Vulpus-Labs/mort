//! Comparing the map before and after an edit (ADR 0007 §5): changes in
//! each track's features by section, and the findings that appeared,
//! disappeared or changed.

use serde::Serialize;

use mort_features::THIRD_OCTAVES;
use mort_proto::TrackId;

use crate::Span;
use crate::analysis::{View, centre, db};
use crate::findings::{Finding, Kind, findings};
use crate::thresholds::Limits;

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// How one track changed over one section, as `b` minus `a`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackDelta {
    /// The track.
    pub track: TrackId,
    /// Its name.
    pub name: String,
    /// Loudness before and after, LUFS.
    pub loudness: (f64, f64),
    /// Change in RMS, dB.
    pub rms: f64,
    /// Change in true peak, dB.
    pub true_peak: f64,
    /// Change in centroid, Hz.
    pub centroid: f64,
    /// Change in tilt, dB per octave.
    pub tilt: f64,
    /// Change in side level, dB.
    pub side: f64,
    /// Change in correlation below 150 Hz.
    pub correlation_low: f64,
    /// The three third octaves that moved most: centre in Hz and change
    /// in dB.
    pub bands: Vec<(f64, f64)>,
    /// The largest change of level in any third octave, or of loudness,
    /// dB: how much the track changed at all.
    pub size: f64,
}

/// One section of a comparison.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SectionDelta {
    /// The section.
    pub section: String,
    /// Where.
    #[serde(flatten)]
    pub span: Span,
    /// Tracks measured in both, most changed first.
    pub tracks: Vec<TrackDelta>,
}

/// A finding in both, whose severity moved.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Moved {
    /// The finding after.
    pub finding: Finding,
    /// Its severity before.
    pub was: f64,
}

/// What [`compare`] found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comparison {
    /// By section.
    pub sections: Vec<SectionDelta>,
    /// Findings in `b` and not in `a`.
    pub appeared: Vec<Finding>,
    /// Findings in `a` and not in `b`.
    pub gone: Vec<Finding>,
    /// Findings in both.
    pub kept: Vec<Moved>,
}

/// What makes two findings the same problem: kind, section, and the
/// tracks it is about (the pair, or the bus or track).
fn key(f: &Finding) -> (Kind, String, Vec<TrackId>) {
    let mut tracks = match f.kind {
        Kind::Masking | Kind::RegisterClash => f.tracks.clone(),
        _ => f.tracks.iter().take(1).copied().collect(),
    };
    tracks.sort();
    (f.kind, f.section.clone(), tracks)
}

/// Compares two views of the same sections and tracks: `a` before, `b`
/// after.
#[must_use]
pub fn compare(before: &View<'_>, after: &View<'_>, lim: &Limits) -> Comparison {
    let sections = after
        .sections
        .iter()
        .map(|sec| {
            let mut tracks: Vec<TrackDelta> = after
                .tracks
                .iter()
                .filter_map(|t| {
                    let x = before.stats(t.id, sec.span)?;
                    let y = after.stats(t.id, sec.span)?;
                    let mut bands: Vec<(f64, f64)> = (0..THIRD_OCTAVES)
                        .filter(|k| db(x.bands[*k]).max(db(y.bands[*k])) > -90.0)
                        .map(|k| (centre(k), r1(db(y.bands[k]) - db(x.bands[k]))))
                        .collect();
                    bands.sort_by(|p, q| q.1.abs().total_cmp(&p.1.abs()));
                    let loud = y.loudness - x.loudness;
                    let size = bands
                        .first()
                        .map_or(0.0, |band| band.1.abs())
                        .max(loud.abs());
                    bands.truncate(3);
                    Some(TrackDelta {
                        track: t.id,
                        name: t.name.clone(),
                        loudness: (r1(x.loudness), r1(y.loudness)),
                        rms: r1(y.rms - x.rms),
                        true_peak: r1(y.true_peak - x.true_peak),
                        centroid: (y.centroid - x.centroid).round(),
                        tilt: r1(y.tilt - x.tilt),
                        side: r1(y.side - x.side),
                        correlation_low: (100.0 * (y.correlation_low - x.correlation_low)).round()
                            / 100.0,
                        bands,
                        size: r1(size),
                    })
                })
                .collect();
            tracks.sort_by(|p, q| q.size.total_cmp(&p.size));
            SectionDelta {
                section: sec.name.clone(),
                span: sec.span,
                tracks,
            }
        })
        .collect();
    let was = findings(before, lim);
    let now = findings(after, lim);
    let mut gone = Vec::new();
    let mut kept = Vec::new();
    let mut appeared: Vec<Finding> = now;
    for f in was {
        let id = key(&f);
        match appeared.iter().position(|g| key(g) == id) {
            Some(i) => {
                let g = appeared.remove(i);
                kept.push(Moved {
                    was: f.severity,
                    finding: g,
                });
            }
            None => gone.push(f),
        }
    }
    Comparison {
        sections,
        appeared,
        gone,
        kept,
    }
}
