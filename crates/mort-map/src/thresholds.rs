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
    /// The lowest value an override may take.
    pub min: f64,
    /// The highest value an override may take.
    pub max: f64,
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

/// [`MASK_MIN_BANDS`] as a number, for [`Limits`].
#[expect(clippy::cast_precision_loss, reason = "a small count")]
const MASK_MIN_BANDS_F: f64 = MASK_MIN_BANDS as f64;

/// The thresholds findings use, by the names in [`TABLE`]. A project can
/// override them (ADR 0012 §7); [`Limits::ALL`] lets everything through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// `mask_spread`: see [`TABLE`].
    pub mask_spread: f64,
    /// `mask_bus`: see [`TABLE`].
    pub mask_bus: f64,
    /// `mask_fraction`: see [`TABLE`].
    pub mask_fraction: f64,
    /// `mask_bands`: see [`TABLE`].
    pub mask_bands: f64,
    /// `mask_floor`: see [`TABLE`].
    pub mask_floor: f64,
    /// `mud_excess`: see [`TABLE`].
    pub mud_excess: f64,
    /// `harsh_excess`: see [`TABLE`].
    pub harsh_excess: f64,
    /// `tilt_band_floor`: see [`TABLE`].
    pub tilt_band_floor: f64,
    /// `blame_share`: see [`TABLE`].
    pub blame_share: f64,
    /// `headroom`: see [`TABLE`].
    pub headroom: f64,
    /// `bus_clip`: see [`TABLE`].
    pub bus_clip: f64,
    /// `loudness_jump`: see [`TABLE`].
    pub loudness_jump: f64,
    /// `low_corr`: see [`TABLE`].
    pub low_corr: f64,
    /// `low_fraction`: see [`TABLE`].
    pub low_fraction: f64,
    /// `low_floor`: see [`TABLE`].
    pub low_floor: f64,
    /// `clash_min`: see [`TABLE`].
    pub clash_min: f64,
}

impl Limits {
    /// The calibrated thresholds.
    pub const DEFAULT: Limits = Limits {
        mask_spread: MASK_SPREAD_DB,
        mask_bus: MASK_BUS_DB,
        mask_fraction: MASK_FRACTION,
        mask_bands: MASK_MIN_BANDS_F,
        mask_floor: MASK_FLOOR_DB,
        mud_excess: MUD_EXCESS_DB,
        harsh_excess: HARSH_EXCESS_DB,
        tilt_band_floor: TILT_BAND_FLOOR_DB,
        blame_share: BLAME_SHARE,
        headroom: HEADROOM_DBTP,
        bus_clip: BUS_CLIP_DBFS,
        loudness_jump: LOUDNESS_JUMP_LU,
        low_corr: LOW_CORR,
        low_fraction: LOW_FRACTION,
        low_floor: LOW_FLOOR_DB,
        clash_min: CLASH_MIN_S,
    };

    /// Limits that let every measurement through as a finding, so its
    /// evidence shows how far it is from the real threshold: for
    /// calibration, and for asking how close something is. What defines a
    /// measurement (spreads, floors) keeps its default.
    pub const ALL: Limits = Limits {
        mask_spread: MASK_SPREAD_DB,
        mask_bus: MASK_BUS_DB,
        mask_fraction: -1.0,
        mask_bands: 0.0,
        mask_floor: MASK_FLOOR_DB,
        mud_excess: f64::NEG_INFINITY,
        harsh_excess: f64::NEG_INFINITY,
        tilt_band_floor: TILT_BAND_FLOOR_DB,
        blame_share: BLAME_SHARE,
        headroom: f64::NEG_INFINITY,
        bus_clip: f64::NEG_INFINITY,
        loudness_jump: -1.0,
        low_corr: LOW_CORR,
        low_fraction: -1.0,
        low_floor: LOW_FLOOR_DB,
        clash_min: 0.0,
    };

    /// The value of the threshold `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<f64> {
        Some(match name {
            "mask_spread" => self.mask_spread,
            "mask_bus" => self.mask_bus,
            "mask_fraction" => self.mask_fraction,
            "mask_bands" => self.mask_bands,
            "mask_floor" => self.mask_floor,
            "mud_excess" => self.mud_excess,
            "harsh_excess" => self.harsh_excess,
            "tilt_band_floor" => self.tilt_band_floor,
            "blame_share" => self.blame_share,
            "headroom" => self.headroom,
            "bus_clip" => self.bus_clip,
            "loudness_jump" => self.loudness_jump,
            "low_corr" => self.low_corr,
            "low_fraction" => self.low_fraction,
            "low_floor" => self.low_floor,
            "clash_min" => self.clash_min,
            _ => return None,
        })
    }

    /// Sets the threshold `name`, checking it against [`TABLE`]'s range.
    ///
    /// # Errors
    ///
    /// An unknown name, or a value out of range, with what is allowed.
    pub fn set(&mut self, name: &str, value: f64) -> Result<(), String> {
        let t = threshold(name)?;
        if !(t.min..=t.max).contains(&value) {
            return Err(format!(
                "`{name}` must be from {} to {} {}; {value} is outside that",
                t.min, t.max, t.unit
            ));
        }
        let slot = match name {
            "mask_spread" => &mut self.mask_spread,
            "mask_bus" => &mut self.mask_bus,
            "mask_fraction" => &mut self.mask_fraction,
            "mask_bands" => &mut self.mask_bands,
            "mask_floor" => &mut self.mask_floor,
            "mud_excess" => &mut self.mud_excess,
            "harsh_excess" => &mut self.harsh_excess,
            "tilt_band_floor" => &mut self.tilt_band_floor,
            "blame_share" => &mut self.blame_share,
            "headroom" => &mut self.headroom,
            "bus_clip" => &mut self.bus_clip,
            "loudness_jump" => &mut self.loudness_jump,
            "low_corr" => &mut self.low_corr,
            "low_fraction" => &mut self.low_fraction,
            "low_floor" => &mut self.low_floor,
            "clash_min" => &mut self.clash_min,
            _ => return Err(unknown(name)),
        };
        *slot = value;
        Ok(())
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

fn unknown(name: &str) -> String {
    let names: Vec<&str> = TABLE
        .iter()
        .filter(|t| t.overridable())
        .map(|t| t.name)
        .collect();
    format!(
        "no threshold `{name}`; the thresholds are {}, and `min_severity`",
        names.join(", ")
    )
}

/// The table entry for `name`, if a project may override it.
///
/// # Errors
///
/// An unknown name, listing the known ones.
pub fn threshold(name: &str) -> Result<&'static Threshold, String> {
    TABLE
        .iter()
        .find(|t| t.name == name && t.overridable())
        .ok_or_else(|| unknown(name))
}

impl Threshold {
    /// Whether a project may override it: every finding threshold may;
    /// the onset detector's is a feature definition, not a finding's.
    #[must_use]
    pub fn overridable(&self) -> bool {
        Limits::DEFAULT.get(self.name).is_some()
    }
}

/// Which thresholds each kind of finding uses, to say which were
/// overridden.
#[must_use]
pub fn used_by(kind: crate::findings::Kind) -> &'static [&'static str] {
    use crate::findings::Kind;
    match kind {
        Kind::Masking => &[
            "mask_spread",
            "mask_bus",
            "mask_fraction",
            "mask_bands",
            "mask_floor",
        ],
        Kind::Mud => &["mud_excess", "tilt_band_floor", "blame_share"],
        Kind::Harsh => &["harsh_excess", "tilt_band_floor", "blame_share"],
        Kind::Headroom => &["headroom", "bus_clip"],
        Kind::LoudnessJump => &["loudness_jump"],
        Kind::LowEndWidth => &["low_corr", "low_fraction", "low_floor"],
        Kind::RegisterClash => &["clash_min"],
    }
}

/// A project's overrides (ADR 0012 §7): thresholds by name, and the lowest
/// severity a finding needs to be shown.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overrides {
    /// Overridden thresholds.
    pub values: std::collections::BTreeMap<String, f64>,
    /// Findings below this severity are not shown. 0 by default.
    pub min_severity: Option<f64>,
}

impl Overrides {
    /// The limits with these overrides applied.
    #[must_use]
    pub fn limits(&self) -> Limits {
        let mut l = Limits::DEFAULT;
        for (k, v) in &self.values {
            let _ = l.set(k, *v);
        }
        l
    }

    /// The lowest severity shown.
    #[must_use]
    pub fn min_severity(&self) -> f64 {
        self.min_severity.unwrap_or(0.0)
    }

    /// Overrides `name` (a threshold or `min_severity`) with `value`.
    ///
    /// # Errors
    ///
    /// An unknown name or a value out of range, naming what is allowed.
    pub fn set(&mut self, name: &str, value: f64) -> Result<(), String> {
        if name == "min_severity" {
            if !(0.0..=1.0).contains(&value) {
                return Err(format!(
                    "`min_severity` must be from 0 to 1; {value} is outside that"
                ));
            }
            self.min_severity = Some(value);
            return Ok(());
        }
        Limits::DEFAULT.clone().set(name, value)?;
        self.values.insert(name.to_owned(), value);
        Ok(())
    }

    /// Goes back to the default for `name`.
    ///
    /// # Errors
    ///
    /// An unknown name.
    pub fn reset(&mut self, name: &str) -> Result<(), String> {
        if name == "min_severity" {
            self.min_severity = None;
            return Ok(());
        }
        threshold(name)?;
        self.values.remove(name);
        Ok(())
    }

    /// Reads `thresholds.toml`: `key = number` lines, one per override.
    ///
    /// # Errors
    ///
    /// Text that is not TOML, a value that is not a number, an unknown key
    /// or a value out of range, naming the key and what it allows.
    pub fn from_toml(text: &str) -> Result<Overrides, String> {
        let table: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| e.message().to_owned())?;
        let mut o = Overrides::default();
        for (k, v) in table {
            let n = match v {
                toml::Value::Float(f) => f,
                #[expect(clippy::cast_precision_loss, reason = "small numbers")]
                toml::Value::Integer(i) => i as f64,
                other => {
                    return Err(format!("`{k}` must be a number, not {}", other.type_str()));
                }
            };
            o.set(&k, n)?;
        }
        Ok(o)
    }

    /// Writes the overrides as `thresholds.toml`.
    #[must_use]
    pub fn to_toml(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::from(
            "# Finding thresholds this project overrides (mort, ADR 0012 §7).\n\
             # `mort thresholds` lists them all with their defaults and reasons.\n",
        );
        if let Some(m) = self.min_severity {
            let _ = writeln!(out, "min_severity = {m:?}");
        }
        for (k, v) in &self.values {
            let _ = writeln!(out, "{k} = {v:?}");
        }
        out
    }
}

const _: () = assert!(MASK_MIN_BANDS == 3, "keep TABLE's mask_bands in step");

/// The table, for `mort thresholds` and the docs. Measured values come from
/// the calibration test's synthetic mixes: five pink-noise parts in
/// neighbouring ranges at -26 dB RMS (the clean mix), with one problem
/// planted at a time.
pub const TABLE: &[Threshold] = &[
    Threshold {
        name: "mask_spread",
        value: MASK_SPREAD_DB,
        unit: "dB",
        min: 0.0,
        max: 24.0,
        reason: "ADR 0007's starting point. A second pad 1 dB under the first competes in 9 bands at 88-99 % of frames; 15 dB under, in none.",
    },
    Threshold {
        name: "mask_bus",
        value: MASK_BUS_DB,
        unit: "dB",
        min: 0.0,
        max: 40.0,
        reason: "ADR 0007's starting point. Two parts within 6 dB of each other sit 1-7 dB under their sum, so this only drops pairs that other parts bury.",
    },
    Threshold {
        name: "mask_fraction",
        value: MASK_FRACTION,
        unit: "of frames",
        min: 0.0,
        max: 1.0,
        reason: "ADR 0007's starting point. Planted overlaps reach 88-99 % per band; it keeps passing overlaps out.",
    },
    Threshold {
        name: "mask_bands",
        value: 3.0,
        unit: "third octaves",
        min: 1.0,
        max: 26.0,
        reason: "Clean crossovers between parts with 24 dB/octave skirts reach 2 bands over 30 %; planted overlaps 5-10. Three bands is an octave.",
    },
    Threshold {
        name: "mask_floor",
        value: MASK_FLOOR_DB,
        unit: "dB",
        min: -150.0,
        max: 0.0,
        reason: "Keeps silence and filter skirts out: about 35 dB below the quietest band of the synthetic parts.",
    },
    Threshold {
        name: "mud_excess",
        value: MUD_EXCESS_DB,
        unit: "dB",
        min: 0.0,
        max: 24.0,
        reason: "Clean mixes read -0.7 to +0.1 dB. A 200-500 Hz part at the others' level reads 2.9, 6 dB quieter 0.8; a pad stacked on another reads 2.0, left unflagged.",
    },
    Threshold {
        name: "harsh_excess",
        value: HARSH_EXCESS_DB,
        unit: "dB",
        min: 0.0,
        max: 24.0,
        reason: "The mud test, mirrored. Clean mixes read 0.3 to 1.2 dB; a 2-5 kHz part at the others' level reads 3.3, 6 dB quieter 1.7.",
    },
    Threshold {
        name: "tilt_band_floor",
        value: TILT_BAND_FLOOR_DB,
        unit: "dB",
        min: -150.0,
        max: 0.0,
        reason: "A near-silent bus has no tilt to speak of.",
    },
    Threshold {
        name: "blame_share",
        value: BLAME_SHARE,
        unit: "of band energy",
        min: 0.0,
        max: 1.0,
        reason: "Names every input carrying a quarter of the band, so at most four.",
    },
    Threshold {
        name: "headroom",
        value: HEADROOM_DBTP,
        unit: "dBTP",
        min: -24.0,
        max: 6.0,
        reason: "Streaming services' usual ceiling. Clean synthetic mixes peak at -6.2 to -4.2 dBTP.",
    },
    Threshold {
        name: "bus_clip",
        value: BUS_CLIP_DBFS,
        unit: "dBFS",
        min: -24.0,
        max: 12.0,
        reason: "Full scale. Floating point does not clip inside REAPER, but plugins after the bus often do.",
    },
    Threshold {
        name: "loudness_jump",
        value: LOUDNESS_JUMP_LU,
        unit: "LU",
        min: 0.0,
        max: 40.0,
        reason: "ADR 0007's starting point. A chorus adding full-band pink reads 0.0 LU with none, 1.1 at -24 dB, 5.7 at -14, 8.9 at -10; only the last is flagged.",
    },
    Threshold {
        name: "low_corr",
        value: LOW_CORR,
        unit: "",
        min: -1.0,
        max: 1.0,
        reason: "ADR 0007's starting point. Mono low end reads 1.0; a wide bass reads 0.01 alone and 0.11 on the master.",
    },
    Threshold {
        name: "low_fraction",
        value: LOW_FRACTION,
        unit: "of frames",
        min: 0.0,
        max: 1.0,
        reason: "A wide bass is under the correlation limit in 90-95 % of frames, a mono one in none.",
    },
    Threshold {
        name: "low_floor",
        value: LOW_FLOOR_DB,
        unit: "dB",
        min: -150.0,
        max: 0.0,
        reason: "Frames with no real low end have meaningless low correlation.",
    },
    Threshold {
        name: "clash_min",
        value: CLASH_MIN_S,
        unit: "s",
        min: 0.0,
        max: 10.0,
        reason: "A musical judgement, not measured: passing notes shorter than this are heard as movement, not a clash.",
    },
    Threshold {
        name: "onset_median_k",
        value: ONSET_MEDIAN_K,
        unit: "x median",
        min: 0.0,
        max: 100.0,
        reason: "Steady noise reads 0.0 onsets a second; a voice playing quarter notes at 120 bpm (2.0 a second) reads 2.3.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_read_write_and_refuse_what_they_cannot_use() {
        let o =
            Overrides::from_toml("mud_excess = 3\nmin_severity = 0.25\nmask_bands = 4\n").unwrap();
        assert!((o.limits().mud_excess - 3.0).abs() < 1e-12);
        assert!((o.limits().mask_bands - 4.0).abs() < 1e-12);
        assert!((o.limits().harsh_excess - HARSH_EXCESS_DB).abs() < 1e-12);
        assert!((o.min_severity() - 0.25).abs() < 1e-12);
        assert_eq!(Overrides::from_toml(&o.to_toml()).unwrap(), o);

        let e = Overrides::from_toml("mud = 3").unwrap_err();
        assert!(
            e.contains("no threshold `mud`") && e.contains("mud_excess"),
            "{e}"
        );
        let e = Overrides::from_toml("low_corr = 2").unwrap_err();
        assert!(e.contains("`low_corr` must be from -1 to 1"), "{e}");
        let e = Overrides::from_toml("headroom = \"loud\"").unwrap_err();
        assert!(e.contains("must be a number"), "{e}");
        assert!(
            Overrides::from_toml("onset_median_k = 3").is_err(),
            "not a finding threshold"
        );
        let e = Overrides::from_toml("min_severity = 2").unwrap_err();
        assert!(e.contains("from 0 to 1"), "{e}");
        assert!(Overrides::from_toml("= 3").is_err());
    }
}
