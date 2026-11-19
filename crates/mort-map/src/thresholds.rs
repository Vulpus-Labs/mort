//! Every finding threshold, in one table with its reason (ADR 0007 §3).
//!
//! Ticket 0013 calibrated these on synthetic mixes with planted problems
//! (`cargo test -p mort-bridge --test findings -- --ignored --nocapture`
//! prints the measurements). They
//! are provisional until calibrated on real material (Fenland). Each reason
//! gives the measured clean and planted values the threshold sits between.

/// A threshold and why it has its value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Threshold {
    /// Its name, as used in evidence.
    pub name: &'static str,
    /// The value.
    pub value: f64,
    /// The unit.
    pub unit: &'static str,
    /// Why.
    pub reason: &'static str,
}

/// `masking`: two inputs of a bus count as competing in a band when their
/// levels there are within this of each other.
pub const MASK_SPREAD_DB: f64 = 6.0;
/// `masking`: … and each is within this of the bus's level in the band.
pub const MASK_BUS_DB: f64 = 12.0;
/// `masking`: … for more than this fraction of the section's frames.
pub const MASK_FRACTION: f64 = 0.3;
/// `masking`: a pair must compete in at least this many third octaves.
pub const MASK_MIN_BANDS: usize = 3;
/// `masking`: bands quieter than this in the bus are ignored.
pub const MASK_FLOOR_DB: f64 = -70.0;

/// `mud`: 200–500 Hz on a bus or the master above the bus's own fitted
/// tilt by more than this.
pub const MUD_EXCESS_DB: f64 = 2.0;
/// `harsh`: 2–5 kHz above the fitted tilt by more than this.
pub const HARSH_EXCESS_DB: f64 = 2.0;
/// `mud` and `harsh`: the band must be louder than this to count.
pub const TILT_BAND_FLOOR_DB: f64 = -60.0;
/// `mud` and `harsh`: inputs with at least this share of the band's
/// energy are named as causes.
pub const BLAME_SHARE: f64 = 0.25;

/// `headroom`: master true peak above this.
pub const HEADROOM_DBTP: f64 = -1.0;
/// `headroom`: a bus's sample peak above this counts as clipping
/// internally.
pub const BUS_CLIP_DBFS: f64 = 0.0;

/// `loudness_jump`: master short-term loudness moving more than this
/// across a section boundary.
pub const LOUDNESS_JUMP_LU: f64 = 6.0;

/// `low_end_width`: correlation below 150 Hz under this…
pub const LOW_CORR: f64 = 0.5;
/// `low_end_width`: … in more than this fraction of the frames with low
/// end…
pub const LOW_FRACTION: f64 = 0.3;
/// `low_end_width`: … where frames with low end have energy below 141 Hz
/// above this.
pub const LOW_FLOOR_DB: f64 = -50.0;

/// `register_clash`: notes a minor second apart below C4 overlapping for
/// at least this long.
pub const CLASH_MIN_S: f64 = 0.1;
/// `register_clash`: notes below this MIDI pitch (C4).
pub const CLASH_BELOW: u8 = 60;

/// Onset density: a frame is an onset when its onset strength is the
/// highest within two frames either side, and above this multiple of the
/// section's median…
pub const ONSET_MEDIAN_K: f64 = 4.0;
/// … and above this absolute floor (mean square).
pub const ONSET_FLOOR: f64 = 1e-6;

/// The thresholds findings use, so a caller can loosen them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// [`MASK_FRACTION`].
    pub mask_fraction: f64,
    /// [`MASK_MIN_BANDS`].
    pub mask_bands: usize,
    /// [`MUD_EXCESS_DB`].
    pub mud_excess: f64,
    /// [`HARSH_EXCESS_DB`].
    pub harsh_excess: f64,
    /// [`HEADROOM_DBTP`].
    pub headroom: f64,
    /// [`BUS_CLIP_DBFS`].
    pub bus_clip: f64,
    /// [`LOUDNESS_JUMP_LU`].
    pub loudness_jump: f64,
    /// [`LOW_FRACTION`].
    pub low_fraction: f64,
    /// [`CLASH_MIN_S`].
    pub clash_min: f64,
}

impl Limits {
    /// The calibrated thresholds.
    pub const DEFAULT: Limits = Limits {
        mask_fraction: MASK_FRACTION,
        mask_bands: MASK_MIN_BANDS,
        mud_excess: MUD_EXCESS_DB,
        harsh_excess: HARSH_EXCESS_DB,
        headroom: HEADROOM_DBTP,
        bus_clip: BUS_CLIP_DBFS,
        loudness_jump: LOUDNESS_JUMP_LU,
        low_fraction: LOW_FRACTION,
        clash_min: CLASH_MIN_S,
    };

    /// Limits that let every measurement through as a finding, so its
    /// evidence shows how far it is from the real threshold: for
    /// calibration, and for asking how close something is.
    pub const ALL: Limits = Limits {
        mask_fraction: -1.0,
        mask_bands: 0,
        mud_excess: f64::NEG_INFINITY,
        harsh_excess: f64::NEG_INFINITY,
        headroom: f64::NEG_INFINITY,
        bus_clip: f64::NEG_INFINITY,
        loudness_jump: -1.0,
        low_fraction: -1.0,
        clash_min: 0.0,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

const _: () = assert!(MASK_MIN_BANDS == 3, "keep TABLE's mask_bands in step");

/// The table, for `mort findings --thresholds` and the docs. Measured
/// values come from the calibration test's synthetic mixes: five pink-noise
/// parts in neighbouring ranges at -26 dB RMS (the clean mix), with one
/// problem planted at a time.
pub const TABLE: &[Threshold] = &[
    Threshold {
        name: "mask_spread",
        value: MASK_SPREAD_DB,
        unit: "dB",
        reason: "ADR 0007's starting point. A second pad 1 dB under the first competes in 9 bands at 88-99 % of frames; 15 dB under, in none.",
    },
    Threshold {
        name: "mask_bus",
        value: MASK_BUS_DB,
        unit: "dB",
        reason: "ADR 0007's starting point. Two parts within 6 dB of each other sit 1-7 dB under their sum, so this only drops pairs that other parts bury.",
    },
    Threshold {
        name: "mask_fraction",
        value: MASK_FRACTION,
        unit: "of frames",
        reason: "ADR 0007's starting point. Planted overlaps reach 88-99 % per band; it keeps passing overlaps out.",
    },
    Threshold {
        name: "mask_bands",
        value: 3.0,
        unit: "third octaves",
        reason: "Clean crossovers between parts with 24 dB/octave skirts reach 2 bands over 30 %; planted overlaps 5-10. Three bands is an octave.",
    },
    Threshold {
        name: "mask_floor",
        value: MASK_FLOOR_DB,
        unit: "dB",
        reason: "Keeps silence and filter skirts out: about 35 dB below the quietest band of the synthetic parts.",
    },
    Threshold {
        name: "mud_excess",
        value: MUD_EXCESS_DB,
        unit: "dB",
        reason: "Clean mixes read -0.7 to +0.1 dB. A 200-500 Hz part at the others' level reads 2.9, 6 dB quieter 0.8; a pad stacked on another reads 2.0, left unflagged.",
    },
    Threshold {
        name: "harsh_excess",
        value: HARSH_EXCESS_DB,
        unit: "dB",
        reason: "The mud test, mirrored. Clean mixes read 0.3 to 1.2 dB; a 2-5 kHz part at the others' level reads 3.3, 6 dB quieter 1.7.",
    },
    Threshold {
        name: "tilt_band_floor",
        value: TILT_BAND_FLOOR_DB,
        unit: "dB",
        reason: "A near-silent bus has no tilt to speak of.",
    },
    Threshold {
        name: "blame_share",
        value: BLAME_SHARE,
        unit: "of band energy",
        reason: "Names every input carrying a quarter of the band, so at most four.",
    },
    Threshold {
        name: "headroom",
        value: HEADROOM_DBTP,
        unit: "dBTP",
        reason: "Streaming services' usual ceiling. Clean synthetic mixes peak at -6.2 to -4.2 dBTP.",
    },
    Threshold {
        name: "bus_clip",
        value: BUS_CLIP_DBFS,
        unit: "dBFS",
        reason: "Full scale. Floating point does not clip inside REAPER, but plugins after the bus often do.",
    },
    Threshold {
        name: "loudness_jump",
        value: LOUDNESS_JUMP_LU,
        unit: "LU",
        reason: "ADR 0007's starting point. A chorus adding full-band pink reads 0.0 LU with none, 1.1 at -24 dB, 5.7 at -14, 8.9 at -10; only the last is flagged.",
    },
    Threshold {
        name: "low_corr",
        value: LOW_CORR,
        unit: "",
        reason: "ADR 0007's starting point. Mono low end reads 1.0; a wide bass reads 0.01 alone and 0.11 on the master.",
    },
    Threshold {
        name: "low_fraction",
        value: LOW_FRACTION,
        unit: "of frames",
        reason: "A wide bass is under the correlation limit in 90-95 % of frames, a mono one in none.",
    },
    Threshold {
        name: "low_floor",
        value: LOW_FLOOR_DB,
        unit: "dB",
        reason: "Frames with no real low end have meaningless low correlation.",
    },
    Threshold {
        name: "clash_min",
        value: CLASH_MIN_S,
        unit: "s",
        reason: "A musical judgement, not measured: passing notes shorter than this are heard as movement, not a clash.",
    },
    Threshold {
        name: "onset_median_k",
        value: ONSET_MEDIAN_K,
        unit: "x median",
        reason: "Steady noise reads 0.0 onsets a second; a voice playing quarter notes at 120 bpm (2.0 a second) reads 2.3.",
    },
];
