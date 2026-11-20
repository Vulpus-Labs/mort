//! Articulation patterns and drum kit maps (ADR 0013 §8), kept as KDL
//! data. The format is described at the top of `patterns/builtin.kdl`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use kdl::{KdlEntry, KdlNode, KdlValue};

use crate::source::{args, line_col, parse_kdl, props};
use crate::voicing::Source;

/// The built-in patterns' and kits' source.
pub const BUILTIN: &str = include_str!("../patterns/builtin.kdl");

/// Where a hit falls.
#[derive(Debug, Clone, PartialEq)]
pub enum At {
    /// At the start of every harmony.
    Change,
    /// At these places in the pattern: bar from 0, beats from the bar's
    /// start (0 is the downbeat).
    Positions(Vec<(u32, f64)>),
}

/// A hit: notes selected by label, struck at positions.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// Voice and role names, or kit pieces in a drum pattern.
    pub select: Vec<String>,
    /// Where it falls.
    pub at: At,
    /// How long it lasts, in beats; until the line's next position if
    /// unset.
    pub length: Option<f64>,
    /// Its velocity.
    pub velocity: u8,
    /// Octaves to move the notes.
    pub octave: i8,
    /// Copies of each note, these many semitones away.
    pub double: Vec<i8>,
    /// The hand or instrument that plays it.
    pub hand: Option<String>,
    /// Whether it takes the next block's notes.
    pub anticipate: bool,
}

/// The order an arpeggio plays a block's notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// Lowest to highest.
    Up,
    /// Highest to lowest.
    Down,
    /// Up then down, not repeating the ends.
    UpDown,
}

/// An arpeggio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arp {
    /// The order.
    pub order: Order,
    /// Beats between notes.
    pub every: f64,
    /// Velocity.
    pub velocity: u8,
}

/// How a walking line approaches the next change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approach {
    /// From a semitone away.
    Chromatic,
    /// From the nearest scale note.
    Scale,
}

/// A walking line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Walk {
    /// How it approaches each change.
    pub approach: Approach,
    /// Velocity.
    pub velocity: u8,
}

/// An articulation pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    /// Its name.
    pub name: String,
    /// What it is.
    pub description: Option<String>,
    /// Where it came from.
    pub source: Source,
    /// How many bars before it repeats.
    pub bars: u32,
    /// Whether its hits name kit pieces.
    pub drums: bool,
    /// Whether non-chord tones sound.
    pub sound_nct: bool,
    /// Hand spans, in semitones.
    pub hands: BTreeMap<String, u8>,
    /// Its hits.
    pub hits: Vec<Hit>,
    /// Its arpeggio, if it has one.
    pub arp: Option<Arp>,
    /// Its walking line, if it has one.
    pub walk: Option<Walk>,
}

impl Pattern {
    /// The span a hand can reach, a tenth unless the pattern says.
    #[must_use]
    pub fn span(&self, hand: &str) -> u8 {
        self.hands.get(hand).copied().unwrap_or(16)
    }
}

/// A drum kit map: piece names to MIDI notes.
#[derive(Debug, Clone, PartialEq)]
pub struct Kit {
    /// Its name.
    pub name: String,
    /// Where it came from.
    pub source: Source,
    /// The pieces.
    pub pieces: BTreeMap<String, u8>,
}

/// Why a pattern file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternError {
    /// The file, or `built in`.
    pub source: String,
    /// The line, from 1.
    pub line: usize,
    /// The column, from 1.
    pub column: usize,
    /// What went wrong, and what to do.
    pub message: String,
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} line {}, column {}: {}",
            self.source, self.line, self.column, self.message
        )
    }
}

impl std::error::Error for PatternError {}

/// The patterns and kits available, by name.
#[derive(Debug, Clone, Default)]
pub struct Patterns {
    patterns: BTreeMap<String, Pattern>,
    kits: BTreeMap<String, Kit>,
}

/// The user's pattern directory: `~/Library/Application Support/mort/patterns`.
#[must_use]
pub fn user_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| {
        PathBuf::from(h)
            .join("Library")
            .join("Application Support")
            .join("mort")
            .join("patterns")
    })
}

/// Names a hit may select in a pitched pattern.
fn is_label(token: &str) -> bool {
    matches!(
        token,
        "all"
            | "bass"
            | "inner"
            | "top"
            | "root"
            | "2nd"
            | "3rd"
            | "4th"
            | "5th"
            | "6th"
            | "7th"
            | "9th"
            | "11th"
            | "13th"
            | "tension"
    ) || token
        .strip_prefix("inner")
        .is_some_and(|n| n.parse::<u8>().is_ok_and(|n| n > 0))
}

/// Reads a position: `1`, `2&`, `3e`, `4a`, `2.5`, or `2:4&` for bar 2.
/// Returns the bar from 0 and beats from the bar's start.
fn position(text: &str) -> Option<(u32, f64)> {
    let (bar, beat) = match text.split_once(':') {
        Some((b, rest)) => (b.parse::<u32>().ok().filter(|b| *b > 0)?, rest),
        None => (1, text),
    };
    let (num, frac) = match beat.char_indices().last()? {
        (i, '&') => (&beat[..i], 0.5),
        (i, 'e') => (&beat[..i], 0.25),
        (i, 'a') => (&beat[..i], 0.75),
        _ => (beat, 0.0),
    };
    let n: f64 = num.parse().ok().filter(|n: &f64| *n >= 1.0)?;
    Some((bar - 1, n - 1.0 + frac))
}

impl Patterns {
    /// The built-in patterns and kits.
    ///
    /// # Panics
    ///
    /// If the built-in file does not read, which its tests rule out.
    #[must_use]
    pub fn builtin() -> Self {
        let mut p = Self::default();
        if let Err(e) = p.load_str(BUILTIN, &Source::Builtin) {
            panic!("the built-in patterns do not read: {e}");
        }
        p
    }

    /// The built-ins, then the user's, then the project's (in
    /// `sidecar/patterns`), each replacing earlier ones by name.
    ///
    /// # Errors
    ///
    /// [`PatternError`] for the first file that does not read.
    pub fn standard(sidecar: Option<&Path>) -> Result<Self, PatternError> {
        let mut p = Self::builtin();
        if let Some(d) = user_dir() {
            p.load_dir(&d)?;
        }
        if let Some(s) = sidecar {
            p.load_dir(&s.join("patterns"))?;
        }
        Ok(p)
    }

    /// The pattern named `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Pattern> {
        self.patterns.get(name)
    }

    /// The kit named `name`.
    #[must_use]
    pub fn kit(&self, name: &str) -> Option<&Kit> {
        self.kits.get(name)
    }

    /// Every pattern, by name.
    pub fn iter(&self) -> impl Iterator<Item = &Pattern> {
        self.patterns.values()
    }

    /// Loads every `.kdl` file in `dir`, in name order; a missing
    /// directory loads nothing. Returns how many patterns and kits were
    /// read.
    ///
    /// # Errors
    ///
    /// [`PatternError`] for the first file that does not read.
    pub fn load_dir(&mut self, dir: &Path) -> Result<usize, PatternError> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Ok(0);
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "kdl"))
            .collect();
        files.sort();
        let mut n = 0;
        for f in files {
            let src = std::fs::read_to_string(&f).map_err(|e| PatternError {
                source: f.display().to_string(),
                line: 1,
                column: 1,
                message: format!("cannot read it: {e}"),
            })?;
            n += self.load_str(&src, &Source::File(f))?;
        }
        Ok(n)
    }

    /// Loads the patterns and kits in `src`, replacing any with the same
    /// name. Returns how many were read.
    ///
    /// # Errors
    ///
    /// [`PatternError`] with the line and column of the first problem.
    pub fn load_str(&mut self, src: &str, source: &Source) -> Result<usize, PatternError> {
        let at = |offset: usize, message: String| {
            let (line, column) = line_col(src, offset);
            PatternError {
                source: source.to_string(),
                line,
                column,
                message,
            }
        };
        let doc = parse_kdl(src).map_err(|(o, m)| at(o, m))?;
        let mut patterns = Vec::new();
        let mut kits = Vec::new();
        for n in doc.nodes() {
            let a = args(n);
            let name = match a.as_slice() {
                [e] => e.value().as_string().map(str::to_string),
                _ => None,
            };
            match (n.name().value(), name) {
                ("pattern", Some(name)) => patterns.push(read_pattern(n, name, source, &at)?),
                ("kit", Some(name)) => kits.push(read_kit(n, name, source, &at)?),
                ("pattern" | "kit", None) => {
                    return Err(at(
                        n.span().offset(),
                        format!(
                            "a {} takes one name, such as `{} \"bossa piano\"`",
                            n.name().value(),
                            n.name().value()
                        ),
                    ));
                }
                (other, _) => {
                    return Err(at(
                        n.span().offset(),
                        format!(
                            "`{other}` is not a pattern or kit; a pattern file holds `pattern` and `kit` nodes"
                        ),
                    ));
                }
            }
        }
        let count = patterns.len() + kits.len();
        for p in patterns {
            self.patterns.insert(p.name.clone(), p);
        }
        for k in kits {
            self.kits.insert(k.name.clone(), k);
        }
        Ok(count)
    }
}

type AtFn<'a> = &'a dyn Fn(usize, String) -> PatternError;

fn err_n(at: AtFn<'_>, n: &KdlNode, m: String) -> PatternError {
    at(n.span().offset(), m)
}

fn err_e(at: AtFn<'_>, e: &KdlEntry, m: String) -> PatternError {
    at(e.span().offset(), m)
}

fn number(e: &KdlEntry) -> Option<f64> {
    match e.value() {
        #[expect(clippy::cast_precision_loss, reason = "small numbers")]
        KdlValue::Integer(i) => Some(*i as f64),
        KdlValue::Float(f) => Some(*f),
        _ => None,
    }
}

fn velocity(at: AtFn<'_>, e: &KdlEntry) -> Result<u8, PatternError> {
    match e.value() {
        KdlValue::Integer(v) if (1..=127).contains(v) => Ok(u8::try_from(*v).unwrap_or(127)),
        _ => Err(err_e(
            at,
            e,
            "`velocity` is a whole number from 1 to 127".into(),
        )),
    }
}

fn read_kit(n: &KdlNode, name: String, source: &Source, at: AtFn<'_>) -> Result<Kit, PatternError> {
    let mut pieces = BTreeMap::new();
    for c in n
        .children()
        .map(kdl::KdlDocument::nodes)
        .unwrap_or_default()
    {
        match c.name().value() {
            "description" => {}
            "piece" => {
                let a = args(c);
                let (Some(piece), Some(note)) = (
                    a.first().and_then(|e| e.value().as_string()),
                    a.get(1).and_then(|e| match e.value() {
                        KdlValue::Integer(i) => u8::try_from(*i).ok().filter(|n| *n <= 127),
                        _ => None,
                    }),
                ) else {
                    return Err(err_n(
                        at,
                        c,
                        "`piece` takes a name and a MIDI note, such as `piece \"kick\" 36`".into(),
                    ));
                };
                pieces.insert(piece.to_string(), note);
            }
            other => {
                return Err(err_n(
                    at,
                    c,
                    format!("a kit holds `piece` nodes, not `{other}`"),
                ));
            }
        }
    }
    Ok(Kit {
        name,
        source: source.clone(),
        pieces,
    })
}

fn read_pattern(
    n: &KdlNode,
    name: String,
    source: &Source,
    at: AtFn<'_>,
) -> Result<Pattern, PatternError> {
    let mut p = Pattern {
        name,
        description: None,
        source: source.clone(),
        bars: 1,
        drums: false,
        sound_nct: false,
        hands: BTreeMap::new(),
        hits: Vec::new(),
        arp: None,
        walk: None,
    };
    for (k, e) in props(n) {
        match (k, e.value()) {
            ("bars", KdlValue::Integer(b)) if (1..=64).contains(b) => {
                p.bars = u32::try_from(*b).unwrap_or(1);
            }
            ("drums", KdlValue::Bool(b)) => p.drums = *b,
            ("nct", KdlValue::String(s)) if s == "skip" || s == "sound" => {
                p.sound_nct = s == "sound";
            }
            _ => {
                return Err(err_e(
                    at,
                    e,
                    format!(
                        "a pattern takes bars=1..64, drums=#true|#false and nct=\"skip\"|\"sound\"; `{k}` is not one of them"
                    ),
                ));
            }
        }
    }
    for c in n
        .children()
        .map(kdl::KdlDocument::nodes)
        .unwrap_or_default()
    {
        match c.name().value() {
            "description" => {
                p.description = args(c)
                    .first()
                    .and_then(|e| e.value().as_string())
                    .map(str::to_string);
            }
            "hand" => {
                let a = args(c);
                let Some(hand) = a.first().and_then(|e| e.value().as_string()) else {
                    return Err(err_n(
                        at,
                        c,
                        "`hand` takes a name, such as `hand \"left\" span=16`".into(),
                    ));
                };
                let mut span = 16;
                for (k, e) in props(c) {
                    match (k, e.value()) {
                        ("span", KdlValue::Integer(s)) if (0..=36).contains(s) => {
                            span = u8::try_from(*s).unwrap_or(16);
                        }
                        _ => {
                            return Err(err_e(
                                at,
                                e,
                                "`hand` takes `span=N`, semitones from 0 to 36".into(),
                            ));
                        }
                    }
                }
                p.hands.insert(hand.to_string(), span);
            }
            "hit" => p.hits.push(read_hit(c, p.drums, p.bars, at)?),
            "arp" => p.arp = Some(read_arp(c, at)?),
            "walk" => p.walk = Some(read_walk(c, at)?),
            other => {
                return Err(err_n(
                    at,
                    c,
                    format!(
                        "`{other}` is not part of a pattern; see the format at the top of patterns/builtin.kdl"
                    ),
                ));
            }
        }
    }
    let kinds = usize::from(!p.hits.is_empty())
        + usize::from(p.arp.is_some())
        + usize::from(p.walk.is_some());
    if kinds != 1 {
        return Err(err_n(
            at,
            n,
            "give a pattern `hit` lines, or one `arp`, or one `walk`".into(),
        ));
    }
    if p.drums && (p.arp.is_some() || p.walk.is_some()) {
        return Err(err_n(at, n, "a drum pattern has `hit` lines only".into()));
    }
    Ok(p)
}

/// A hit's selection: voices and roles, or kit pieces in a drum pattern.
fn read_select(c: &KdlNode, drums: bool, at: AtFn<'_>) -> Result<Vec<String>, PatternError> {
    let mut select = Vec::new();
    for e in args(c) {
        let Some(s) = e.value().as_string() else {
            return Err(err_e(
                at,
                e,
                "a hit selects with strings, such as `hit \"inner top\"`".into(),
            ));
        };
        for token in s.split_whitespace() {
            if !drums && !is_label(token) {
                return Err(err_e(
                    at,
                    e,
                    format!(
                        "`{token}` is not a voice or role; use bass, inner, inner1…, top, all, or a role such as 3rd or tension"
                    ),
                ));
            }
            select.push(token.to_string());
        }
    }
    if select.is_empty() {
        return Err(err_n(
            at,
            c,
            "a hit needs something to strike, such as `hit \"bass\"`".into(),
        ));
    }
    Ok(select)
}

/// A hit's `at`: `change`, or positions within a pattern of `bars` bars.
fn read_at(e: &KdlEntry, bars: u32, at: AtFn<'_>) -> Result<At, PatternError> {
    let text = e.value().as_string().unwrap_or("");
    if text == "change" {
        return Ok(At::Change);
    }
    let mut v = Vec::new();
    for t in text.split_whitespace() {
        let pos = position(t).filter(|(b, _)| *b < bars).ok_or_else(|| {
            err_e(
                at,
                e,
                format!(
                    "`{t}` is not a position in a {bars}-bar pattern; write beats such as `1`, `2&`, `3e` or `2:4&`"
                ),
            )
        })?;
        v.push(pos);
    }
    if v.is_empty() {
        return Err(err_e(at, e, "`at` needs positions, such as \"1 3\"".into()));
    }
    Ok(At::Positions(v))
}

fn read_hit(c: &KdlNode, drums: bool, bars: u32, at: AtFn<'_>) -> Result<Hit, PatternError> {
    let select = read_select(c, drums, at)?;
    let mut hit = Hit {
        select,
        at: At::Change,
        length: None,
        velocity: 90,
        octave: 0,
        double: Vec::new(),
        hand: None,
        anticipate: false,
    };
    let mut accent = false;
    let mut placed = false;
    for (k, e) in props(c) {
        match k {
            "at" => {
                hit.at = read_at(e, bars, at)?;
                placed = true;
            }
            "length" => {
                hit.length = Some(
                    number(e)
                        .filter(|l| *l > 0.0)
                        .ok_or_else(|| err_e(at, e, "`length` is a number of beats".into()))?,
                );
            }
            "velocity" => hit.velocity = velocity(at, e)?,
            "accent" => accent = matches!(e.value(), KdlValue::Bool(true)),
            "octave" => {
                hit.octave = match e.value() {
                    KdlValue::Integer(o) if (-4..=4).contains(o) => i8::try_from(*o).unwrap_or(0),
                    _ => {
                        return Err(err_e(
                            at,
                            e,
                            "`octave` is a whole number from -4 to 4".into(),
                        ));
                    }
                };
            }
            "double" => {
                let d = match e.value() {
                    KdlValue::Integer(d) if (-36..=36).contains(d) && *d != 0 => {
                        i8::try_from(*d).unwrap_or(0)
                    }
                    _ => {
                        return Err(err_e(
                            at,
                            e,
                            "`double` is semitones from -36 to 36, such as -12".into(),
                        ));
                    }
                };
                hit.double.push(d);
            }
            "hand" => hit.hand = e.value().as_string().map(str::to_string),
            "anticipate" => hit.anticipate = matches!(e.value(), KdlValue::Bool(true)),
            other => {
                return Err(err_e(
                    at,
                    e,
                    format!(
                        "a hit takes at, length, velocity, accent, octave, double, hand and anticipate, not `{other}`"
                    ),
                ));
            }
        }
    }
    if !placed {
        return Err(err_n(
            at,
            c,
            "a hit needs `at`, such as `at=\"1 3\"` or `at=\"change\"`".into(),
        ));
    }
    if accent {
        hit.velocity = hit.velocity.saturating_add(20).min(127);
    }
    Ok(hit)
}

fn read_arp(c: &KdlNode, at: AtFn<'_>) -> Result<Arp, PatternError> {
    let order = match args(c).first().and_then(|e| e.value().as_string()) {
        Some("up") => Order::Up,
        Some("down") => Order::Down,
        Some("updown") => Order::UpDown,
        _ => return Err(err_n(at, c, "`arp` takes up, down or updown".into())),
    };
    let mut arp = Arp {
        order,
        every: 0.5,
        velocity: 85,
    };
    for (k, e) in props(c) {
        match k {
            "every" => {
                arp.every = number(e)
                    .filter(|v| *v > 0.0)
                    .ok_or_else(|| err_e(at, e, "`every` is a number of beats".into()))?;
            }
            "velocity" => arp.velocity = velocity(at, e)?,
            other => {
                return Err(err_e(
                    at,
                    e,
                    format!("`arp` takes every and velocity, not `{other}`"),
                ));
            }
        }
    }
    Ok(arp)
}

fn read_walk(c: &KdlNode, at: AtFn<'_>) -> Result<Walk, PatternError> {
    let mut walk = Walk {
        approach: Approach::Chromatic,
        velocity: 85,
    };
    for (k, e) in props(c) {
        match (k, e.value().as_string()) {
            ("approach", Some("chromatic")) => walk.approach = Approach::Chromatic,
            ("approach", Some("scale")) => walk.approach = Approach::Scale,
            ("velocity", _) => walk.velocity = velocity(at, e)?,
            _ => {
                return Err(err_e(
                    at,
                    e,
                    "`walk` takes approach=\"chromatic\"|\"scale\" and velocity".into(),
                ));
            }
        }
    }
    Ok(walk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions() {
        assert_eq!(position("1"), Some((0, 0.0)));
        assert_eq!(position("2&"), Some((0, 1.5)));
        assert_eq!(position("3e"), Some((0, 2.25)));
        assert_eq!(position("4a"), Some((0, 3.75)));
        assert_eq!(position("2.5"), Some((0, 1.5)));
        assert_eq!(position("2:4&"), Some((1, 3.5)));
        assert_eq!(position("0"), None);
        assert_eq!(position("x"), None);
    }

    #[test]
    fn the_builtins_read() {
        let p = Patterns::builtin();
        let names: Vec<&str> = p.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "arpeggio down",
                "arpeggio up",
                "block",
                "bossa drums",
                "bossa piano",
                "rock drums",
                "sustain",
                "walking"
            ]
        );
        assert_eq!(p.kit("gm").unwrap().pieces["kick"], 36);
        let bossa = p.get("bossa piano").unwrap();
        assert_eq!(bossa.bars, 2);
        assert!(bossa.hits.iter().any(|h| h.anticipate));
    }
}
