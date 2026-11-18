//! Positions in time (ADR 0003 §4).
//!
//! The protocol carries project seconds. People and agents write musical
//! positions, which are converted through a [`TempoMap`]:
//!
//! | Text      | Meaning                                                  |
//! |-----------|----------------------------------------------------------|
//! | `9`       | bar 9, beat 1                                            |
//! | `9.2`     | bar 9, beat 2 (beats of the bar's time signature)        |
//! | `9.2.240` | bar 9, beat 2, plus 240 ticks at 960 per quarter note    |
//! | `12.5s`   | 12.5 seconds from the project start                      |
//! | `+4b`     | 4 quarter notes after the previous position              |
//! | `+1.5s`   | 1.5 seconds after the previous position                  |
//!
//! A [`Range`] is two positions joined by `..`, as in `9.1..17.1` or
//! `9.1..+8b`. A relative end is taken from the start.

use std::fmt;
use std::str::FromStr;

/// Ticks per quarter note, as in REAPER's default MIDI resolution.
pub const TICKS_PER_QN: u32 = 960;

/// A position as written, before conversion to seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
    /// A musical position. `bar` and `beat` count from 1. `ticks` are at
    /// [`TICKS_PER_QN`] per quarter note, added after the beat.
    BarBeat {
        /// Bar, from 1.
        bar: u32,
        /// Beat within the bar, from 1, in units of the time signature's
        /// denominator.
        beat: u32,
        /// Ticks after the beat.
        ticks: u32,
    },
    /// Seconds from the project start.
    Seconds(f64),
    /// Quarter notes after the previous position.
    AfterBeats(f64),
    /// Seconds after the previous position.
    AfterSeconds(f64),
}

/// Why a position could not be parsed or resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionError {
    /// The position as written.
    pub input: String,
    /// What is wrong with it.
    pub problem: String,
}

impl fmt::Display for PositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "position `{}`: {}. Write `9.1` (bar 9, beat 1), `9.2.240` (plus ticks at \
             960 per quarter note), `12.5s` (seconds), `+4b` (quarter notes after the \
             previous position) or `+1.5s`; for example `17.3` or `9.1..+8b`",
            self.input, self.problem
        )
    }
}

impl std::error::Error for PositionError {}

fn error(input: &str, problem: impl Into<String>) -> PositionError {
    PositionError {
        input: input.to_owned(),
        problem: problem.into(),
    }
}

fn parse_amount(input: &str, text: &str, unit: &str) -> Result<f64, PositionError> {
    let v: f64 = text
        .parse()
        .map_err(|_| error(input, format!("`{text}` is not a number of {unit}")))?;
    if !v.is_finite() || v < 0.0 {
        return Err(error(
            input,
            format!("{unit} must be a finite number, zero or more"),
        ));
    }
    Ok(v)
}

impl FromStr for Position {
    type Err = PositionError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let s = input.trim();
        if s.is_empty() {
            return Err(error(input, "it is empty"));
        }
        if let Some(rel) = s.strip_prefix('+') {
            if let Some(n) = rel.strip_suffix('b') {
                return parse_amount(input, n, "beats").map(Position::AfterBeats);
            }
            if let Some(n) = rel.strip_suffix('s') {
                return parse_amount(input, n, "seconds").map(Position::AfterSeconds);
            }
            return Err(error(
                input,
                "a relative position needs a unit: `b` for quarter notes or `s` for seconds",
            ));
        }
        if let Some(n) = s.strip_suffix('s') {
            return parse_amount(input, n, "seconds").map(Position::Seconds);
        }
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() > 3 {
            return Err(error(input, "a musical position has at most three parts"));
        }
        let mut nums = [1u32, 1, 0];
        for (i, p) in parts.iter().enumerate() {
            nums[i] = p
                .parse()
                .map_err(|_| error(input, format!("`{p}` is not a whole number")))?;
        }
        let [bar, beat, ticks] = nums;
        if bar == 0 || beat == 0 {
            return Err(error(input, "bars and beats count from 1"));
        }
        Ok(Position::BarBeat { bar, beat, ticks })
    }
}

/// A tempo map: what mort needs from REAPER's `TimeMap` functions.
///
/// Quarter-note positions (`qn`) count from 0 at the project start.
pub trait TempoMap {
    /// The quarter-note position where `bar` (from 1) starts.
    fn bar_start_qn(&self, bar: u32) -> f64;
    /// The time signature in force in `bar`, as (numerator, denominator).
    fn time_sig(&self, bar: u32) -> (u32, u32);
    /// The bar (from 1) containing quarter-note position `qn`.
    fn bar_at_qn(&self, qn: f64) -> u32;
    /// Seconds at quarter-note position `qn`.
    fn qn_to_seconds(&self, qn: f64) -> f64;
    /// Quarter-note position at `seconds`.
    fn seconds_to_qn(&self, seconds: f64) -> f64;
}

impl Position {
    /// Converts to project seconds. `prev` is the previous position in
    /// seconds, needed by relative positions.
    ///
    /// # Errors
    ///
    /// A relative position with no `prev`, or a beat beyond the bar's time
    /// signature.
    pub fn resolve(
        &self,
        map: &dyn TempoMap,
        prev: Option<f64>,
        input: &str,
    ) -> Result<f64, PositionError> {
        let need_prev =
            || prev.ok_or_else(|| error(input, "a relative position needs a position before it"));
        match *self {
            Position::Seconds(s) => Ok(s),
            Position::AfterSeconds(s) => Ok(need_prev()? + s),
            Position::AfterBeats(b) => {
                let qn = map.seconds_to_qn(need_prev()?) + b;
                Ok(map.qn_to_seconds(qn))
            }
            Position::BarBeat { bar, beat, ticks } => {
                let (num, den) = map.time_sig(bar);
                if beat > num {
                    return Err(error(
                        input,
                        format!("bar {bar} is in {num}/{den}, so its beats run 1 to {num}"),
                    ));
                }
                let beat_qn = 4.0 / f64::from(den);
                let qn = map.bar_start_qn(bar)
                    + f64::from(beat - 1) * beat_qn
                    + f64::from(ticks) / f64::from(TICKS_PER_QN);
                Ok(map.qn_to_seconds(qn))
            }
        }
    }

    /// Formats project seconds as `bar.beat` or `bar.beat.ticks`, to the
    /// nearest tick.
    #[must_use]
    pub fn format_musical(map: &dyn TempoMap, seconds: f64) -> String {
        let qn = map.seconds_to_qn(seconds);
        let mut bar = map.bar_at_qn(qn);
        let (num, den) = map.time_sig(bar);
        let ticks_per_beat = TICKS_PER_QN * 4 / den;
        let offset = (qn - map.bar_start_qn(bar)) * f64::from(TICKS_PER_QN);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "offset within a bar is small and not negative"
        )]
        let total = offset.round().max(0.0) as u32;
        let mut beat = total / ticks_per_beat + 1;
        let ticks = total % ticks_per_beat;
        if beat > num {
            bar += 1;
            beat = 1;
        }
        if ticks == 0 {
            format!("{bar}.{beat}")
        } else {
            format!("{bar}.{beat}.{ticks}")
        }
    }
}

/// A range written as `start..end`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    /// Where it starts. Must not be relative.
    pub start: Position,
    /// Where it ends. A relative end is taken from `start`.
    pub end: Position,
}

impl FromStr for Range {
    type Err = PositionError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (a, b) = input
            .split_once("..")
            .ok_or_else(|| error(input, "a range is written `start..end`"))?;
        let start: Position = a
            .parse()
            .map_err(|e: PositionError| error(input, e.problem))?;
        if matches!(start, Position::AfterBeats(_) | Position::AfterSeconds(_)) {
            return Err(error(
                input,
                "a range cannot start with a relative position",
            ));
        }
        let end = b
            .parse()
            .map_err(|e: PositionError| error(input, e.problem))?;
        Ok(Range { start, end })
    }
}

impl Range {
    /// Converts to project seconds `(start, end)`.
    ///
    /// # Errors
    ///
    /// Either end fails to resolve, or the end is not after the start.
    pub fn resolve(&self, map: &dyn TempoMap, input: &str) -> Result<(f64, f64), PositionError> {
        let start = self.start.resolve(map, None, input)?;
        let end = self.end.resolve(map, Some(start), input)?;
        if end <= start {
            return Err(error(input, "the end must be after the start"));
        }
        Ok((start, end))
    }
}

/// A tempo and time-signature change at the start of a bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoChange {
    /// The bar where it takes effect, from 1.
    pub bar: u32,
    /// Quarter notes per minute.
    pub bpm: f64,
    /// Time-signature numerator.
    pub num: u32,
    /// Time-signature denominator: 1, 2, 4, 8, 16 or 32.
    pub den: u32,
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    change: TempoChange,
    start_qn: f64,
    start_seconds: f64,
}

impl Segment {
    fn qn_per_bar(&self) -> f64 {
        f64::from(self.change.num) * 4.0 / f64::from(self.change.den)
    }
}

/// A tempo map with step changes at bar starts. Used by the fake session
/// and in tests. REAPER's own map (with ramps and changes mid-bar) is
/// reached through the bridge.
#[derive(Debug, Clone)]
pub struct SteppedTempoMap {
    segments: Vec<Segment>,
}

impl SteppedTempoMap {
    /// A map with one tempo and time signature throughout.
    ///
    /// # Panics
    ///
    /// If `bpm` is not positive or `num/den` is not a time signature.
    #[must_use]
    pub fn constant(bpm: f64, num: u32, den: u32) -> Self {
        Self::new(vec![TempoChange {
            bar: 1,
            bpm,
            num,
            den,
        }])
        .expect("a single change at bar 1 is valid")
    }

    /// A map from changes in bar order, the first at bar 1.
    ///
    /// # Errors
    ///
    /// No change at bar 1, bars out of order, a tempo that is not positive,
    /// or a time signature REAPER would not accept.
    pub fn new(changes: Vec<TempoChange>) -> Result<Self, String> {
        if changes.first().map(|c| c.bar) != Some(1) {
            return Err("the first tempo change must be at bar 1".into());
        }
        let mut segments: Vec<Segment> = Vec::with_capacity(changes.len());
        for c in changes {
            if !(c.bpm.is_finite() && c.bpm > 0.0) {
                return Err(format!(
                    "bar {}: tempo {} is not a positive number",
                    c.bar, c.bpm
                ));
            }
            if c.num == 0 || ![1, 2, 4, 8, 16, 32].contains(&c.den) {
                return Err(format!(
                    "bar {}: {}/{} is not a time signature",
                    c.bar, c.num, c.den
                ));
            }
            let (start_qn, start_seconds) = match segments.last() {
                None => (0.0, 0.0),
                Some(p) => {
                    if c.bar <= p.change.bar {
                        return Err(format!(
                            "tempo changes must be in bar order: bar {} follows bar {}",
                            c.bar, p.change.bar
                        ));
                    }
                    let qn = p.start_qn + f64::from(c.bar - p.change.bar) * p.qn_per_bar();
                    (
                        qn,
                        p.start_seconds + (qn - p.start_qn) * 60.0 / p.change.bpm,
                    )
                }
            };
            segments.push(Segment {
                change: c,
                start_qn,
                start_seconds,
            });
        }
        Ok(Self { segments })
    }

    fn by_bar(&self, bar: u32) -> &Segment {
        self.segments
            .iter()
            .rev()
            .find(|s| s.change.bar <= bar)
            .unwrap_or(&self.segments[0])
    }

    fn by_qn(&self, qn: f64) -> &Segment {
        self.segments
            .iter()
            .rev()
            .find(|s| s.start_qn <= qn)
            .unwrap_or(&self.segments[0])
    }

    fn by_seconds(&self, seconds: f64) -> &Segment {
        self.segments
            .iter()
            .rev()
            .find(|s| s.start_seconds <= seconds)
            .unwrap_or(&self.segments[0])
    }
}

impl TempoMap for SteppedTempoMap {
    fn bar_start_qn(&self, bar: u32) -> f64 {
        let s = self.by_bar(bar);
        s.start_qn + (f64::from(bar) - f64::from(s.change.bar)) * s.qn_per_bar()
    }

    fn time_sig(&self, bar: u32) -> (u32, u32) {
        let s = self.by_bar(bar);
        (s.change.num, s.change.den)
    }

    fn bar_at_qn(&self, qn: f64) -> u32 {
        let s = self.by_qn(qn);
        let bars = ((qn - s.start_qn) / s.qn_per_bar()).floor().max(0.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "bar counts are small and not negative"
        )]
        let bars = bars as u32;
        s.change.bar + bars
    }

    fn qn_to_seconds(&self, qn: f64) -> f64 {
        let s = self.by_qn(qn);
        s.start_seconds + (qn - s.start_qn) * 60.0 / s.change.bpm
    }

    fn seconds_to_qn(&self, seconds: f64) -> f64 {
        let s = self.by_seconds(seconds);
        s.start_qn + (seconds - s.start_seconds) * s.change.bpm / 60.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    fn at(map: &dyn TempoMap, text: &str) -> f64 {
        text.parse::<Position>()
            .unwrap()
            .resolve(map, None, text)
            .unwrap()
    }

    /// 120 bpm 4/4 for bars 1–4, then 90 bpm 3/4 from bar 5.
    fn changing() -> SteppedTempoMap {
        SteppedTempoMap::new(vec![
            TempoChange {
                bar: 1,
                bpm: 120.0,
                num: 4,
                den: 4,
            },
            TempoChange {
                bar: 5,
                bpm: 90.0,
                num: 3,
                den: 4,
            },
        ])
        .unwrap()
    }

    #[test]
    fn parses_all_forms() {
        use Position::*;
        let cases = [
            (
                "9",
                BarBeat {
                    bar: 9,
                    beat: 1,
                    ticks: 0,
                },
            ),
            (
                "9.2",
                BarBeat {
                    bar: 9,
                    beat: 2,
                    ticks: 0,
                },
            ),
            (
                "9.2.240",
                BarBeat {
                    bar: 9,
                    beat: 2,
                    ticks: 240,
                },
            ),
            ("12.5s", Seconds(12.5)),
            ("+4b", AfterBeats(4.0)),
            ("+0.5b", AfterBeats(0.5)),
            ("+1.5s", AfterSeconds(1.5)),
        ];
        for (text, want) in cases {
            assert_eq!(text.parse::<Position>().unwrap(), want, "{text}");
        }
    }

    #[test]
    fn bad_input_names_input_forms_and_example() {
        for bad in [
            "", "0.1", "9.0", "9.x", "1.2.3.4", "+4", "-3s", "+abcb", "9..1",
        ] {
            let e = bad.parse::<Position>().unwrap_err();
            let msg = e.to_string();
            assert!(msg.contains(&format!("`{bad}`")), "{msg}");
            assert!(
                msg.contains("`9.2.240`") && msg.contains("for example"),
                "{msg}"
            );
        }
    }

    #[test]
    fn constant_tempo() {
        let map = SteppedTempoMap::constant(120.0, 4, 4);
        close(at(&map, "1.1"), 0.0);
        close(at(&map, "2.1"), 2.0);
        close(at(&map, "9.2"), 16.5);
        close(at(&map, "9.2.480"), 16.75);
        close(at(&map, "3s"), 3.0);
    }

    #[test]
    fn eighth_note_beats() {
        let map = SteppedTempoMap::constant(120.0, 6, 8);
        // A 6/8 bar is 3 quarter notes = 1.5 s; each beat is an eighth.
        close(at(&map, "2.1"), 1.5);
        close(at(&map, "2.4"), 2.25);
        let e = "2.7"
            .parse::<Position>()
            .unwrap()
            .resolve(&map, None, "2.7")
            .unwrap_err();
        assert!(e.problem.contains("6/8"), "{e}");
    }

    #[test]
    fn tempo_change() {
        let map = changing();
        // Bars 1–4: 16 quarter notes at 0.5 s.
        close(at(&map, "5.1"), 8.0);
        // Bar 5 on: 3/4 at 90 bpm, a quarter note is 2/3 s, a bar is 2 s.
        close(at(&map, "5.2"), 8.0 + 2.0 / 3.0);
        close(at(&map, "6.1"), 10.0);
        // Four beats from bar 4 beat 3 cross the change: 2 at 0.5 s, 2 at 2/3 s.
        let p: Position = "+4b".parse().unwrap();
        close(
            p.resolve(&map, Some(at(&map, "4.3")), "+4b").unwrap(),
            8.0 + 4.0 / 3.0,
        );
    }

    #[test]
    fn relative_needs_previous() {
        let map = SteppedTempoMap::constant(120.0, 4, 4);
        let e = "+4b"
            .parse::<Position>()
            .unwrap()
            .resolve(&map, None, "+4b")
            .unwrap_err();
        assert!(e.problem.contains("position before it"));
    }

    #[test]
    fn formats_musical_positions() {
        let map = changing();
        for text in ["1.1", "4.3", "5.1", "5.2", "6.3.120", "9.1"] {
            let s = at(&map, text);
            assert_eq!(Position::format_musical(&map, s), text);
        }
    }

    #[test]
    fn ranges() {
        let map = changing();
        let r: Range = "1.1..+8b".parse().unwrap();
        assert_eq!(r.resolve(&map, "1.1..+8b").unwrap(), (0.0, 4.0));
        let r: Range = "5.1..6.1".parse().unwrap();
        assert_eq!(r.resolve(&map, "").unwrap(), (8.0, 10.0));
        for bad in ["5.1", "+4b..9.1", "9.1..5.1", "9.1..9.1"] {
            let parsed = bad.parse::<Range>();
            let e = match parsed {
                Err(e) => e,
                Ok(r) => r.resolve(&map, bad).unwrap_err(),
            };
            assert!(e.to_string().contains(&format!("`{bad}`")), "{e}");
        }
    }

    #[test]
    fn map_validation() {
        let c = |bar, bpm, num, den| TempoChange { bar, bpm, num, den };
        assert!(SteppedTempoMap::new(vec![c(2, 120.0, 4, 4)]).is_err());
        assert!(SteppedTempoMap::new(vec![c(1, 0.0, 4, 4)]).is_err());
        assert!(SteppedTempoMap::new(vec![c(1, 120.0, 4, 3)]).is_err());
        assert!(SteppedTempoMap::new(vec![c(1, 120.0, 4, 4), c(1, 90.0, 4, 4)]).is_err());
    }
}
