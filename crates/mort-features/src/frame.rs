//! The feature frame: one per 50 ms of audio.

/// Number of third-octave bands in [`Frame::mid_bands`].
pub const THIRD_OCTAVES: usize = 31;
/// Number of octave bands in [`Frame::side_octaves`].
pub const OCTAVES: usize = 10;

/// ISO 266 nominal centres of [`Frame::mid_bands`], in Hz. The bands are
/// measured at the exact base-10 centres `1000 · 10^(n/10)` for
/// `n = -17..=13`, with edges a twentieth of a decade either side.
pub const THIRD_OCTAVE_CENTRES_HZ: [f32; THIRD_OCTAVES] = [
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0,
    500.0, 630.0, 800.0, 1_000.0, 1_250.0, 1_600.0, 2_000.0, 2_500.0, 3_150.0, 4_000.0, 5_000.0,
    6_300.0, 8_000.0, 10_000.0, 12_500.0, 16_000.0, 20_000.0,
];

/// ISO 266 nominal centres of [`Frame::side_octaves`], in Hz. Each octave
/// spans exactly three of the third-octave bands.
pub const OCTAVE_CENTRES_HZ: [f32; OCTAVES] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
];

/// The floor for every value in dB, used for silence. No field is ever NaN
/// or infinite.
pub const SILENCE_DB: f32 = -150.0;

/// Frame size in bytes: three `u32` and 52 `f32`.
pub const FRAME_BYTES: usize = 220;

/// The features of one 50 ms frame (ADR 0007 §1).
///
/// The layout is fixed: `#[repr(C)]`, 4-byte fields only, no padding,
/// [`FRAME_BYTES`] (220) bytes. Fields appear in declaration order.
///
/// Levels are in dB, floored at [`SILENCE_DB`]. Energies (bands, side,
/// onset) are linear mean squares relative to full scale, where a
/// full-scale sine has energy 0.5. Mid is `(L + R) / 2` and side is
/// `(L - R) / 2`.
///
/// Every windowed feature describes the window that ends with the frame's
/// last sample: loudness over 400 ms and 3 s; bands from 800 Hz up,
/// centroid, flatness and onset over about 85 ms; bands up to 630 Hz over
/// about 683 ms. The rest describe the frame's own samples.
///
/// Frames say where they are in the caller's blocks, not on the timeline.
/// The probe stamps the project position: the frame started
/// `len - end_offset` samples before the start of the block it ended in
/// (a negative value means it started inside that block).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    /// Frames since construction or the last reset, from 0.
    pub index: u32,
    /// Length in samples. Frame `k` covers samples
    /// `floor(k · rate / 20)` to `floor((k + 1) · rate / 20)`, exclusive,
    /// counted from construction or the last reset, so every frame is
    /// exactly 50 ms when the rate is a multiple of 20 Hz and the lengths
    /// alternate to keep the boundaries exact otherwise.
    pub len: u32,
    /// How many samples into the block passed to `process` the frame
    /// ended: the frame's last sample is at `end_offset - 1`.
    pub end_offset: u32,
    /// Momentary loudness, 400 ms, LUFS (BS.1770-4, K-weighted, L and R
    /// with unit weights).
    pub momentary_lufs: f32,
    /// Short-term loudness, 3 s, LUFS.
    pub short_term_lufs: f32,
    /// Largest absolute sample of either channel, dBFS.
    pub sample_peak_dbfs: f32,
    /// Largest absolute value of either channel after 4× oversampling,
    /// dBTP (BS.1770-4 Annex 2). Never below the sample peak.
    pub true_peak_dbtp: f32,
    /// RMS of both channels together, dBFS: a full-scale sine reads -3.01.
    pub rms_dbfs: f32,
    /// Sample peak minus RMS, dB. Zero for silence.
    pub crest_db: f32,
    /// L/R correlation, `Σ L·R / sqrt(Σ L² · Σ R²)`, from -1 to 1. Reads 1
    /// when both channels are silent and 0 when only one is.
    pub correlation: f32,
    /// The same, after a fourth-order Butterworth low-pass at 150 Hz.
    pub correlation_low: f32,
    /// Power-weighted mean frequency of mid from 20 Hz to 20 kHz (or
    /// Nyquist), Hz. Zero for silence.
    pub centroid_hz: f32,
    /// Geometric over arithmetic mean of mid's power spectrum over the same
    /// range, from 0 (tonal) to 1 (flat). White noise reads about 0.56,
    /// `e^-γ`, because each bin of one periodogram is exponentially
    /// distributed. Zero for silence.
    pub flatness: f32,
    /// Onset strength: spectral flux of mid, the sum over bins from 20 Hz
    /// to 20 kHz of the rise in power since the previous frame, as a mean
    /// square. A sine starting from silence gives its energy, `a² / 2`,
    /// spread over the frames its onset spans; a steady signal gives 0.
    pub onset: f32,
    /// Mid energy per third octave, 20 Hz to 20 kHz (see
    /// [`THIRD_OCTAVE_CENTRES_HZ`]). Bands above Nyquist read 0.
    pub mid_bands: [f32; THIRD_OCTAVES],
    /// Side energy per octave, 31.5 Hz to 16 kHz (see
    /// [`OCTAVE_CENTRES_HZ`]). Bands above Nyquist read 0.
    pub side_octaves: [f32; OCTAVES],
}

const _: () = assert!(std::mem::size_of::<Frame>() == FRAME_BYTES);
const _: () = assert!(std::mem::align_of::<Frame>() == 4);
