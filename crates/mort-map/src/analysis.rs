//! Reading the map by section (ADR 0007 §2, §4): sections from regions,
//! per-section statistics, and the project facts findings need.

use serde::{Deserialize, Serialize};

use mort_features::{Frame, OCTAVES, SILENCE_DB, THIRD_OCTAVE_CENTRES_HZ, THIRD_OCTAVES};
use mort_proto::TrackId;
use mort_proto::model::{Note, Region};

use crate::thresholds::{ONSET_FLOOR, ONSET_MEDIAN_K};
use crate::{MixMap, Now, Span, Stale};

/// Energy (mean square) to dB, floored at [`SILENCE_DB`].
#[must_use]
pub fn db(e: f64) -> f64 {
    if e > 0.0 {
        (10.0 * e.log10()).max(f64::from(SILENCE_DB))
    } else {
        f64::from(SILENCE_DB)
    }
}

/// A band's centre in Hz.
#[must_use]
pub fn centre(band: usize) -> f64 {
    f64::from(THIRD_OCTAVE_CENTRES_HZ[band])
}

/// A part of the range: a region, or a stretch no region covers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Section {
    /// The region's name; `(no region)` for gaps, or `range` when the
    /// project has no regions there.
    pub name: String,
    /// Where it is, clipped to the range.
    #[serde(flatten)]
    pub span: Span,
}

/// Splits `range` into sections by the project's regions. Overlapping
/// regions are cut where the next one starts.
#[must_use]
pub fn sections(regions: &[Region], range: Span) -> Vec<Section> {
    let mut rs: Vec<(String, Span)> = regions
        .iter()
        .filter_map(|r| {
            let end = r.end?;
            Span::new(r.start, end)
                .intersect(range)
                .map(|s| (r.name.clone(), s))
        })
        .collect();
    rs.sort_by(|a, b| a.1.start.total_cmp(&b.1.start));
    if rs.is_empty() {
        return vec![Section {
            name: "range".into(),
            span: range,
        }];
    }
    let mut out = Vec::new();
    let mut at = range.start;
    for (i, (name, s)) in rs.iter().enumerate() {
        if s.start > at + 1e-9 {
            out.push(Section {
                name: "(no region)".into(),
                span: Span::new(at, s.start),
            });
        }
        let end = rs
            .get(i + 1)
            .map_or(s.end, |n| s.end.min(n.1.start.max(s.start)));
        let start = s.start.max(at);
        if end > start {
            out.push(Section {
                name: name.clone(),
                span: Span::new(start, end),
            });
            at = end;
        }
    }
    if range.end > at + 1e-9 {
        out.push(Section {
            name: "(no region)".into(),
            span: Span::new(at, range.end),
        });
    }
    out
}

/// What analysis needs to know about a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackMeta {
    /// The track.
    pub id: TrackId,
    /// Its name.
    pub name: String,
    /// Linear gain from its probe to the bus it feeds: fader, pan's effect
    /// on mid, and mute.
    pub gain: f64,
    /// The bus its main send feeds (its folder, or the master); `None`
    /// for the master or a track whose main send is off.
    pub bus: Option<TrackId>,
    /// Its MIDI notes, for `register_clash`.
    pub notes: Vec<Note>,
}

/// Statistics of one track over one section.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stats {
    /// Frames counted.
    pub frames: usize,
    /// Mean momentary loudness, power-averaged over frames above −70
    /// LUFS.
    pub loudness: f64,
    /// Highest short-term loudness.
    pub short_term_max: f64,
    /// Highest true peak, dBTP.
    pub true_peak: f64,
    /// Highest sample peak, dBFS.
    pub sample_peak: f64,
    /// RMS over the section, dBFS.
    pub rms: f64,
    /// Mean crest factor, dB.
    pub crest: f64,
    /// Mean L/R correlation.
    pub correlation: f64,
    /// Mean correlation below 150 Hz.
    pub correlation_low: f64,
    /// Energy-weighted mean centroid, Hz.
    pub centroid: f64,
    /// Mean flatness.
    pub flatness: f64,
    /// Onsets per second.
    pub density: f64,
    /// Spectral tilt of the mean third-octave spectrum, 63 Hz to 8 kHz,
    /// dB per octave: pink noise reads 0, white noise +3.
    pub tilt: f64,
    /// Side energy relative to mid, dB.
    pub side: f64,
    /// Mean mid energy per third octave (linear).
    #[serde(skip)]
    pub bands: [f64; THIRD_OCTAVES],
}

/// The tilt of a third-octave spectrum in dB per octave, by least squares
/// over bands `lo..=hi`, leaving out `skip`.
#[must_use]
pub fn tilt_fit(bands: &[f64; THIRD_OCTAVES], lo: usize, hi: usize, skip: &[usize]) -> (f64, f64) {
    let pts: Vec<(f64, f64)> = (lo..=hi)
        .filter(|k| !skip.contains(k))
        .map(|k| (centre(k).log2(), db(bands[k])))
        .collect();
    #[expect(clippy::cast_precision_loss, reason = "a few dozen points")]
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let sxy: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    (slope, my - slope * mx)
}

/// Band range of the tilt fit: 63 Hz to 8 kHz.
pub const TILT_BANDS: (usize, usize) = (5, 26);

#[expect(clippy::cast_precision_loss, reason = "frame counts are small")]
fn stats_of(frames: &[&Frame], seconds: f64) -> Stats {
    let n = frames.len() as f64;
    let mean = |f: &dyn Fn(&Frame) -> f64| frames.iter().map(|x| f(x)).sum::<f64>() / n;
    let max = |f: &dyn Fn(&Frame) -> f64| {
        frames
            .iter()
            .map(|x| f(x))
            .fold(f64::from(SILENCE_DB), f64::max)
    };
    let loud: Vec<f64> = frames
        .iter()
        .map(|f| f64::from(f.momentary_lufs))
        .filter(|l| *l > -70.0)
        .collect();
    let loudness = if loud.is_empty() {
        f64::from(SILENCE_DB)
    } else {
        db(loud.iter().map(|l| 10f64.powf(l / 10.0)).sum::<f64>() / loud.len() as f64)
    };
    let mut bands = [0.0; THIRD_OCTAVES];
    for (k, b) in bands.iter_mut().enumerate() {
        *b = mean(&|f| f64::from(f.mid_bands[k]));
    }
    let mut side = 0.0;
    for k in 0..OCTAVES {
        side += mean(&|f| f64::from(f.side_octaves[k]));
    }
    let mid: f64 = bands.iter().sum();
    let energy = |f: &Frame| f.mid_bands.iter().map(|b| f64::from(*b)).sum::<f64>();
    let total: f64 = frames.iter().map(|f| energy(f)).sum();
    let centroid = if total > 0.0 {
        frames
            .iter()
            .map(|f| f64::from(f.centroid_hz) * energy(f))
            .sum::<f64>()
            / total
    } else {
        0.0
    };
    let mut onsets: Vec<f64> = frames.iter().map(|f| f64::from(f.onset)).collect();
    let mut sorted = onsets.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let bar = (median * ONSET_MEDIAN_K).max(ONSET_FLOOR);
    // The low bands' 683 ms window spreads an onset's rise over several
    // frames, so a peak must stand two frames clear on either side.
    onsets.splice(0..0, [0.0, 0.0]);
    onsets.extend([0.0, 0.0]);
    let peaks = onsets
        .windows(5)
        .filter(|w| w[2] > bar && w[2] >= w[0] && w[2] >= w[1] && w[2] > w[3] && w[2] > w[4])
        .count();
    Stats {
        frames: frames.len(),
        loudness,
        short_term_max: max(&|f| f64::from(f.short_term_lufs)),
        true_peak: max(&|f| f64::from(f.true_peak_dbtp)),
        sample_peak: max(&|f| f64::from(f.sample_peak_dbfs)),
        rms: db(mean(&|f| 10f64.powf(f64::from(f.rms_dbfs) / 10.0))),
        crest: mean(&|f| f64::from(f.crest_db)),
        correlation: mean(&|f| f64::from(f.correlation)),
        correlation_low: mean(&|f| f64::from(f.correlation_low)),
        centroid,
        flatness: mean(&|f| f64::from(f.flatness)),
        density: peaks as f64 / seconds.max(1e-9),
        tilt: tilt_fit(&bands, TILT_BANDS.0, TILT_BANDS.1, &[]).0,
        side: (db(side) - db(mid)).max(-60.0),
        bands,
    }
}

/// The map, the project and the range, ready to read by section.
pub struct View<'a> {
    /// The map.
    pub map: &'a MixMap,
    /// The project now, for staleness.
    pub now: &'a Now<'a>,
    /// The master.
    pub master: TrackId,
    /// Every track, the master first.
    pub tracks: Vec<TrackMeta>,
    /// The sections of the range, in order.
    pub sections: Vec<Section>,
}

impl View<'_> {
    /// A track's facts.
    #[must_use]
    pub fn meta(&self, id: TrackId) -> Option<&TrackMeta> {
        self.tracks.iter().find(|t| t.id == id)
    }

    /// The tracks whose main send feeds `bus`.
    #[must_use]
    pub fn inputs(&self, bus: TrackId) -> Vec<&TrackMeta> {
        self.tracks.iter().filter(|t| t.bus == Some(bus)).collect()
    }

    /// The tracks that are buses: the master, and every track something
    /// feeds.
    #[must_use]
    pub fn buses(&self) -> Vec<&TrackMeta> {
        self.tracks
            .iter()
            .filter(|t| t.id == self.master || !self.inputs(t.id).is_empty())
            .collect()
    }

    /// Gain from a track's probe to the master, following main sends.
    #[must_use]
    pub fn gain_to_master(&self, id: TrackId) -> f64 {
        let mut g = 1.0;
        let mut at = id;
        for _ in 0..64 {
            if at == self.master {
                return g;
            }
            let Some(m) = self.meta(at) else { return 0.0 };
            g *= m.gain;
            match m.bus {
                Some(b) => at = b,
                None => return 0.0,
            }
        }
        g
    }

    /// A track's frames starting inside `span`, in time order.
    #[must_use]
    pub fn frames(&self, id: TrackId, span: Span) -> Vec<(f64, &Frame)> {
        self.map
            .segments(id)
            .iter()
            .filter(|s| s.span.intersect(span).is_some())
            .flat_map(|s| s.frames.iter())
            .filter(|f| span.holds_frame(f.start))
            .map(|f| (f.start, &f.frame))
            .collect()
    }

    /// The first reason a track's data over `span` is stale.
    #[must_use]
    pub fn stale(&self, id: TrackId, span: Span) -> Option<Stale> {
        self.map
            .segments(id)
            .iter()
            .filter(|s| s.span.intersect(span).is_some())
            .find_map(|s| self.map.staleness(id, s.capture, self.now))
    }

    /// A track's statistics over `span`, or `None` without frames there.
    #[must_use]
    pub fn stats(&self, id: TrackId, span: Span) -> Option<Stats> {
        let fs: Vec<&Frame> = self.frames(id, span).into_iter().map(|(_, f)| f).collect();
        (!fs.is_empty()).then(|| stats_of(&fs, span.end - span.start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(name: &str, start: f64, end: f64) -> Region {
        Region {
            number: 0,
            name: name.into(),
            start,
            end: Some(end),
        }
    }

    #[test]
    fn sections_follow_regions_and_fill_gaps() {
        let rs = [
            region("verse", 0.0, 16.0),
            region("chorus", 20.0, 36.0),
            Region {
                number: 3,
                name: "drop".into(),
                start: 8.0,
                end: None,
            },
        ];
        let s = sections(&rs, Span::new(4.0, 40.0));
        let got: Vec<(&str, f64, f64)> = s
            .iter()
            .map(|x| (x.name.as_str(), x.span.start, x.span.end))
            .collect();
        assert_eq!(
            got,
            [
                ("verse", 4.0, 16.0),
                ("(no region)", 16.0, 20.0),
                ("chorus", 20.0, 36.0),
                ("(no region)", 36.0, 40.0)
            ]
        );
        assert_eq!(sections(&[], Span::new(0.0, 4.0))[0].name, "range");
    }

    #[test]
    fn tilt_of_pink_and_white() {
        let pink = [1e-3; THIRD_OCTAVES];
        assert!(tilt_fit(&pink, 5, 26, &[]).0.abs() < 1e-9);
        let mut white = [0.0; THIRD_OCTAVES];
        for (k, b) in white.iter_mut().enumerate() {
            *b = 1e-6 * centre(k);
        }
        // Nominal centres are rounded, so the slope is a little off 3.01.
        assert!((tilt_fit(&white, 5, 26, &[]).0 - 3.01).abs() < 0.05);
    }
}
