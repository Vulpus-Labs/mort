//! The song sheet (ADR 0013 §3): `<project>.mort/sheet.kdl`.
//!
//! This reads the sheet's form and harmony:
//!
//! ```kdl
//! meter "4/4"                         // the default; sections may differ
//! section "verse" bars=16 {
//!     harmony "D dorian" bars=8       // a scale held for 8 bars
//!     harmony "Gm9" "C13" bars=2      // each chord lasts 2 bars
//!     harmony "Fmaj7" beats=2
//!     harmony "Dm7" scale="aeolian" beats=2
//! }
//! section "intro" bars=4              // no harmony: drums only, say
//! ```
//!
//! Sections are named by their regions in the project. Times inside a
//! section are ticks from its start, at [`TICKS_PER_QUARTER`].

use std::fmt;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

use crate::harmony::Harmony;
use crate::source::{args, line_col, parse_kdl, props};

/// Ticks per quarter note, as REAPER's MIDI uses by default.
pub const TICKS_PER_QUARTER: u32 = 960;

/// A time signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meter {
    /// Beats per bar: the upper number.
    pub beats: u32,
    /// The beat's note value: the lower number (4 is a quarter).
    pub unit: u32,
}

impl Meter {
    /// Ticks in one beat.
    #[must_use]
    pub fn beat_ticks(self) -> u32 {
        TICKS_PER_QUARTER * 4 / self.unit
    }

    /// Ticks in one bar.
    #[must_use]
    pub fn bar_ticks(self) -> u32 {
        self.beats * self.beat_ticks()
    }

    fn parse(text: &str) -> Option<Self> {
        let (b, u) = text.trim().split_once('/')?;
        let beats: u32 = b.trim().parse().ok()?;
        let unit: u32 = u.trim().parse().ok()?;
        (beats > 0 && matches!(unit, 1 | 2 | 4 | 8 | 16 | 32)).then_some(Self { beats, unit })
    }

    /// A length in bars and beats, for messages.
    #[must_use]
    pub fn describe(self, ticks: u32) -> String {
        let bars = ticks / self.bar_ticks();
        let rest = ticks % self.bar_ticks();
        let beats = f64::from(rest) / f64::from(self.beat_ticks());
        let plural = |n: f64, word: &str| {
            if (n - 1.0).abs() < f64::EPSILON {
                format!("1 {word}")
            } else {
                format!("{n} {word}s")
            }
        };
        match (bars, rest) {
            (_, 0) => plural(f64::from(bars), "bar"),
            (0, _) => plural(beats, "beat"),
            _ => format!(
                "{} {}",
                plural(f64::from(bars), "bar"),
                plural(beats, "beat")
            ),
        }
    }
}

impl Default for Meter {
    fn default() -> Self {
        Self { beats: 4, unit: 4 }
    }
}

impl fmt::Display for Meter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.beats, self.unit)
    }
}

/// One harmony and when it sounds within its section.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    /// Ticks from the section's start.
    pub start: u32,
    /// Its length in ticks.
    pub length: u32,
    /// The text it was written as.
    pub text: String,
    /// The harmony.
    pub harmony: Harmony,
}

/// A section of the form.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// Its name: the name of its region in the project.
    pub name: String,
    /// Its length in bars.
    pub bars: u32,
    /// Its time signature.
    pub meter: Meter,
    /// Its harmony, in order, covering the section exactly; empty if it
    /// has none.
    pub spans: Vec<Span>,
}

impl Section {
    /// Its length in ticks.
    #[must_use]
    pub fn length(&self) -> u32 {
        self.bars * self.meter.bar_ticks()
    }

    /// The harmony sounding `ticks` after the section starts.
    #[must_use]
    pub fn harmony_at(&self, ticks: u32) -> Option<&Span> {
        self.spans
            .iter()
            .find(|s| s.start <= ticks && ticks < s.start + s.length)
    }
}

/// A song sheet.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sheet {
    /// The sections, in the order of the form.
    pub sections: Vec<Section>,
}

impl Sheet {
    /// The section named `name`.
    #[must_use]
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }
}

/// Why a sheet could not be read: where, and what to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetError {
    /// The line, from 1.
    pub line: usize,
    /// The column, from 1.
    pub column: usize,
    /// What went wrong, and what to do.
    pub message: String,
}

impl fmt::Display for SheetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "sheet line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for SheetError {}

/// Reads a sheet.
///
/// # Errors
///
/// [`SheetError`] with the line and column of the first problem.
pub fn parse(src: &str) -> Result<Sheet, SheetError> {
    let at = |offset: usize, message: String| {
        let (line, column) = line_col(src, offset);
        SheetError {
            line,
            column,
            message,
        }
    };
    let doc = parse_kdl(src).map_err(|(o, m)| at(o, m))?;
    let node_err = |n: &KdlNode, m: String| at(n.span().offset(), m);
    let entry_err = |e: &KdlEntry, m: String| at(e.span().offset(), m);

    let mut default_meter = Meter::default();
    for n in doc.nodes().iter().filter(|n| n.name().value() == "meter") {
        let a = args(n);
        let [arg] = a.as_slice() else {
            return Err(node_err(
                n,
                "`meter` takes one time signature, such as \"6/8\"".into(),
            ));
        };
        default_meter = meter_of(arg).map_err(|m| entry_err(arg, m))?;
    }

    let mut sheet = Sheet::default();
    for n in doc.nodes() {
        match n.name().value() {
            "meter" => {}
            "section" => {
                let s = section(n, default_meter, &node_err, &entry_err)?;
                if sheet.section(&s.name).is_some() {
                    return Err(node_err(
                        n,
                        format!(
                            "there are two sections named `{}`; name each after its own region, such as `verse 1` and `verse 2`",
                            s.name
                        ),
                    ));
                }
                sheet.sections.push(s);
            }
            other => {
                return Err(node_err(
                    n,
                    format!(
                        "`{other}` is not a sheet node; a sheet holds `meter` and `section` nodes"
                    ),
                ));
            }
        }
    }
    Ok(sheet)
}

fn meter_of(e: &KdlEntry) -> Result<Meter, String> {
    e.value().as_string().and_then(Meter::parse).ok_or_else(|| {
        format!(
            "`{}` is not a time signature; write one such as \"4/4\" or \"6/8\"",
            e.value()
        )
    })
}

fn positive(e: &KdlEntry, what: &str) -> Result<f64, String> {
    let v = match e.value() {
        KdlValue::Integer(i) => {
            #[expect(clippy::cast_precision_loss, reason = "bar and beat counts are small")]
            let f = *i as f64;
            f
        }
        KdlValue::Float(f) => *f,
        other => return Err(format!("`{what}` must be a number, not {other}")),
    };
    if v > 0.0 {
        Ok(v)
    } else {
        Err(format!("`{what}` must be more than 0"))
    }
}

fn section(
    n: &KdlNode,
    default_meter: Meter,
    node_err: &dyn Fn(&KdlNode, String) -> SheetError,
    entry_err: &dyn Fn(&KdlEntry, String) -> SheetError,
) -> Result<Section, SheetError> {
    let (name, bars, meter) = section_header(n, default_meter, node_err, entry_err)?;
    let mut s = Section {
        name,
        bars,
        meter,
        spans: Vec::new(),
    };
    let mut t = 0u32;
    for h in n.children().map(KdlDocument::nodes).unwrap_or_default() {
        if h.name().value() != "harmony" {
            return Err(node_err(
                h,
                format!(
                    "`{}` is not part of a section; a section holds `harmony` nodes",
                    h.name().value()
                ),
            ));
        }
        let (length, scale) = harmony_props(h, meter, node_err, entry_err)?;
        let texts = args(h);
        if texts.is_empty() {
            return Err(node_err(
                h,
                "a harmony needs a chord symbol or a scale, such as \"Dm7\" or \"D dorian\"".into(),
            ));
        }
        for e in texts {
            let text = e
                .value()
                .as_string()
                .ok_or_else(|| entry_err(e, "a harmony is a string, such as \"Dm7\"".into()))?;
            let harmony = Harmony::parse(text, scale).map_err(|err| entry_err(e, err.message))?;
            s.spans.push(Span {
                start: t,
                length,
                text: text.to_string(),
                harmony,
            });
            t += length;
        }
    }
    if !s.spans.is_empty() && t != s.length() {
        return Err(node_err(
            n,
            format!(
                "section `{}` is {} long but its harmony lasts {}; make them agree",
                s.name,
                meter.describe(s.length()),
                meter.describe(t)
            ),
        ));
    }
    Ok(s)
}

/// A section node's name, length in bars and meter.
fn section_header(
    n: &KdlNode,
    default_meter: Meter,
    node_err: &dyn Fn(&KdlNode, String) -> SheetError,
    entry_err: &dyn Fn(&KdlEntry, String) -> SheetError,
) -> Result<(String, u32, Meter), SheetError> {
    let name = match args(n).as_slice() {
        [e] => e
            .value()
            .as_string()
            .ok_or_else(|| {
                entry_err(
                    e,
                    "a section's name is a string, such as \"verse 1\"".into(),
                )
            })?
            .to_string(),
        _ => {
            return Err(node_err(
                n,
                "a section takes one name, its region's, such as `section \"verse 1\" bars=8`"
                    .into(),
            ));
        }
    };
    let mut bars = None;
    let mut meter = default_meter;
    for (k, e) in props(n) {
        match k {
            "bars" => {
                let v = positive(e, "bars").map_err(|m| entry_err(e, m))?;
                if v.fract() != 0.0 {
                    return Err(entry_err(e, "`bars` must be a whole number".into()));
                }
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a positive whole number"
                )]
                let b = v as u32;
                bars = Some(b);
            }
            "meter" => meter = meter_of(e).map_err(|m| entry_err(e, m))?,
            other => {
                return Err(entry_err(
                    e,
                    format!("a section has `bars` and `meter`, not `{other}`"),
                ));
            }
        }
    }
    let bars = bars.ok_or_else(|| {
        node_err(
            n,
            format!("section `{name}` needs its length, such as `bars=8`"),
        )
    })?;

    Ok((name, bars, meter))
}

/// A harmony node's length in ticks and its `scale`, if any.
fn harmony_props<'a>(
    h: &'a KdlNode,
    meter: Meter,
    node_err: &dyn Fn(&KdlNode, String) -> SheetError,
    entry_err: &dyn Fn(&KdlEntry, String) -> SheetError,
) -> Result<(u32, Option<&'a str>), SheetError> {
    let mut length = None;
    let mut scale = None;
    for (k, e) in props(h) {
        match k {
            "bars" | "beats" => {
                if length.is_some() {
                    return Err(entry_err(e, "give `bars` or `beats`, not both".into()));
                }
                let v = positive(e, k).map_err(|m| entry_err(e, m))?;
                let unit = if k == "bars" {
                    meter.bar_ticks()
                } else {
                    meter.beat_ticks()
                };
                let ticks = v * f64::from(unit);
                if ticks.fract() != 0.0 {
                    return Err(entry_err(
                        e,
                        format!("{v} {k} is not a whole number of ticks; use a simpler length"),
                    ));
                }
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a positive whole number of ticks"
                )]
                let ticks = ticks as u32;
                length = Some(ticks);
            }
            "scale" => {
                scale = Some(e.value().as_string().ok_or_else(|| {
                    entry_err(e, "`scale` is a string, such as \"aeolian\"".into())
                })?);
            }
            other => {
                return Err(entry_err(
                    e,
                    format!("a harmony has `bars`, `beats` and `scale`, not `{other}`"),
                ));
            }
        }
    }
    let length = length.ok_or_else(|| {
        node_err(
            h,
            "a harmony needs its length, such as `bars=2` or `beats=2`".into(),
        )
    })?;
    Ok((length, scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meters() {
        let m = Meter::parse("6/8").unwrap();
        assert_eq!((m.beat_ticks(), m.bar_ticks()), (480, 2880));
        assert_eq!(
            Meter::default().describe(3840 * 2 + 960 * 3 / 2),
            "2 bars 1.5 beats"
        );
        assert_eq!(Meter::default().describe(960), "1 beat");
        assert!(Meter::parse("4/3").is_none());
    }
}
