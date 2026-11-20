//! Parts in the song sheet (ADR 0013 §3, §6): how each part is voiced.
//!
//! ```kdl
//! part "Pad" track="Pad" {
//!     voicing "quartal" voices=4 range="C3..A4"
//!     weights movement=1 leaps=2 common=1 spacing=3 parallels=4 crossing=4 centre=0.05
//!     nct "passing" "suspension" beats=1   // in order of preference
//!     pedal "D2"                           // the bass holds D2
//!     in "verse" "chorus"                  // where it plays; all sections by default
//!     section "chorus" {                   // overrides for one section
//!         voicing "drop-2" voices=4 range="G3..D5"
//!     }
//!     top "chorus" "D5" "-" "C5"           // top notes, one per harmony; - is free
//!     fix "verse" bar=3 "D3 G3 C4 F4"      // a literal voicing at bar 3
//! }
//! ```

use std::collections::BTreeMap;

use kdl::{KdlEntry, KdlNode, KdlValue};

use crate::pitch::Pitch;
use crate::source::{args, props};
use crate::voicing::Range;

/// How much each voice-leading cost counts (ADR 0013 §6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    /// Per semitone each voice moves.
    pub movement: f64,
    /// Per semitone a voice leaps beyond a major third.
    pub leaps: f64,
    /// Taken off for each voice that holds its note.
    pub common: f64,
    /// Per semitone an interval sits below its low-interval limit.
    pub spacing: f64,
    /// Per pair of voices moving in parallel fifths or octaves.
    pub parallels: f64,
    /// Per voice that crosses or overlaps a neighbour's previous note.
    pub crossing: f64,
    /// Per semitone the voicing's middle sits from the range's middle.
    pub centre: f64,
}

impl Default for Weights {
    /// Starting values, to be set by listening (ADR 0013, Consequences).
    fn default() -> Self {
        Self {
            movement: 1.0,
            leaps: 2.0,
            common: 1.0,
            spacing: 3.0,
            parallels: 4.0,
            crossing: 4.0,
            centre: 0.05,
        }
    }
}

/// A kind of non-chord tone voice-leading may add (ADR 0013 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Nct {
    /// A step between two notes a third apart.
    Passing,
    /// A step away from a held note and back.
    Neighbour,
    /// The previous note held into the next harmony, resolving down a step.
    Suspension,
    /// The next harmony's note played early.
    Anticipation,
    /// A bass note held under changing harmony.
    Pedal,
}

impl Nct {
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "passing" => Self::Passing,
            "neighbour" | "neighbor" => Self::Neighbour,
            "suspension" => Self::Suspension,
            "anticipation" => Self::Anticipation,
            "pedal" => Self::Pedal,
            _ => return None,
        })
    }
}

/// A part's voicing settings, any of which a section may override.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spec {
    /// The vocabulary's name.
    pub vocabulary: Option<String>,
    /// How many voices.
    pub voices: Option<usize>,
    /// The range.
    pub range: Option<Range>,
    /// Cost weights; unset ones keep the part's or the defaults.
    pub weights: BTreeMap<String, f64>,
    /// Non-chord tones to add, in order of preference.
    pub nct: Option<Vec<Nct>>,
    /// How long a non-chord tone lasts, in beats.
    pub nct_beats: Option<f64>,
    /// A pedal pitch for the bass voice.
    pub pedal: Option<Pitch>,
}

impl Spec {
    /// This spec with `over`'s settings on top.
    #[must_use]
    pub fn merged(&self, over: &Spec) -> Spec {
        let mut weights = self.weights.clone();
        weights.extend(over.weights.iter().map(|(k, v)| (k.clone(), *v)));
        Spec {
            vocabulary: over.vocabulary.clone().or_else(|| self.vocabulary.clone()),
            voices: over.voices.or(self.voices),
            range: over.range.or(self.range),
            weights,
            nct: over.nct.clone().or_else(|| self.nct.clone()),
            nct_beats: over.nct_beats.or(self.nct_beats),
            pedal: over.pedal.or(self.pedal),
        }
    }

    /// The weights, defaults filled in.
    #[must_use]
    pub fn weights(&self) -> Weights {
        let mut w = Weights::default();
        for (k, v) in &self.weights {
            match k.as_str() {
                "movement" => w.movement = *v,
                "leaps" => w.leaps = *v,
                "common" => w.common = *v,
                "spacing" => w.spacing = *v,
                "parallels" => w.parallels = *v,
                "crossing" => w.crossing = *v,
                "centre" | "center" => w.centre = *v,
                _ => {}
            }
        }
        w
    }
}

/// A literal voicing for the harmony starting at a bar and beat.
#[derive(Debug, Clone, PartialEq)]
pub struct Fix {
    /// The section.
    pub section: String,
    /// The bar, from 1.
    pub bar: u32,
    /// The beat, from 1.
    pub beat: f64,
    /// The pitches, lowest first.
    pub pitches: Vec<Pitch>,
}

/// Required top notes for a section's harmonies, in order; `None` is free.
#[derive(Debug, Clone, PartialEq)]
pub struct Top {
    /// The section.
    pub section: String,
    /// One per harmony.
    pub notes: Vec<Option<Pitch>>,
}

/// A part.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// Its name.
    pub name: String,
    /// The track it writes to.
    pub track: String,
    /// Its settings.
    pub spec: Spec,
    /// The sections it plays in; empty means every section with harmony.
    pub sections: Vec<String>,
    /// Settings for particular sections.
    pub overrides: BTreeMap<String, Spec>,
    /// Literal voicings.
    pub fixes: Vec<Fix>,
    /// Top-note lines.
    pub tops: Vec<Top>,
}

impl Part {
    /// Its settings in `section`.
    #[must_use]
    pub fn spec_in(&self, section: &str) -> Spec {
        match self.overrides.get(section) {
            Some(o) => self.spec.merged(o),
            None => self.spec.clone(),
        }
    }

    /// Whether it plays in `section`.
    #[must_use]
    pub fn plays_in(&self, section: &str) -> bool {
        self.sections.is_empty() || self.sections.iter().any(|s| s == section)
    }
}

pub(crate) type NodeErr<'a> = &'a dyn Fn(&KdlNode, String) -> crate::sheet::SheetError;
pub(crate) type EntryErr<'a> = &'a dyn Fn(&KdlEntry, String) -> crate::sheet::SheetError;

fn string<'a>(
    e: &'a KdlEntry,
    what: &str,
    entry_err: EntryErr<'_>,
) -> Result<&'a str, crate::sheet::SheetError> {
    e.value()
        .as_string()
        .ok_or_else(|| entry_err(e, format!("{what} is a string")))
}

fn number(e: &KdlEntry) -> Option<f64> {
    match e.value() {
        #[expect(clippy::cast_precision_loss, reason = "small numbers")]
        KdlValue::Integer(i) => Some(*i as f64),
        KdlValue::Float(f) => Some(*f),
        _ => None,
    }
}

/// Reads a `part` node.
pub(crate) fn parse_part(
    n: &KdlNode,
    node_err: NodeErr<'_>,
    entry_err: EntryErr<'_>,
) -> Result<Part, crate::sheet::SheetError> {
    let a = args(n);
    let [name] = a.as_slice() else {
        return Err(node_err(
            n,
            "a part takes one name, such as `part \"Pad\" track=\"Pad\"`".into(),
        ));
    };
    let name = string(name, "a part's name", entry_err)?.to_string();
    let mut track = None;
    for (k, e) in props(n) {
        match k {
            "track" => track = Some(string(e, "`track`", entry_err)?.to_string()),
            other => return Err(entry_err(e, format!("a part has `track`, not `{other}`"))),
        }
    }
    let mut part = Part {
        track: track.unwrap_or_else(|| name.clone()),
        name,
        spec: Spec::default(),
        sections: Vec::new(),
        overrides: BTreeMap::new(),
        fixes: Vec::new(),
        tops: Vec::new(),
    };
    for c in n
        .children()
        .map(kdl::KdlDocument::nodes)
        .unwrap_or_default()
    {
        match c.name().value() {
            "in" => {
                part.sections = args(c)
                    .into_iter()
                    .map(|e| string(e, "a section name", entry_err).map(str::to_string))
                    .collect::<Result<_, _>>()?;
            }
            "section" => {
                let a = args(c);
                let [s] = a.as_slice() else {
                    return Err(node_err(c, "`section` takes the section's name".into()));
                };
                let s = string(s, "a section name", entry_err)?.to_string();
                let mut spec = Spec::default();
                for g in c
                    .children()
                    .map(kdl::KdlDocument::nodes)
                    .unwrap_or_default()
                {
                    if !read_spec(&mut spec, g, node_err, entry_err)? {
                        return Err(node_err(
                            g,
                            format!(
                                "`{}` cannot be set per section; use voicing, weights, nct or pedal",
                                g.name().value()
                            ),
                        ));
                    }
                }
                part.overrides.insert(s, spec);
            }
            "top" => part.tops.push(read_top(c, node_err, entry_err)?),
            "fix" => part.fixes.push(read_fix(c, node_err, entry_err)?),
            _ => {
                if !read_spec(&mut part.spec, c, node_err, entry_err)? {
                    return Err(node_err(
                        c,
                        format!(
                            "`{}` is not part of a part; a part holds voicing, weights, nct, pedal, in, section, top and fix",
                            c.name().value()
                        ),
                    ));
                }
            }
        }
    }
    Ok(part)
}

/// Reads a node that sets a [`Spec`] field; `false` if it is not one.
fn read_spec(
    spec: &mut Spec,
    c: &KdlNode,
    node_err: NodeErr<'_>,
    entry_err: EntryErr<'_>,
) -> Result<bool, crate::sheet::SheetError> {
    match c.name().value() {
        "voicing" => {
            let a = args(c);
            let [v] = a.as_slice() else {
                return Err(node_err(c, "`voicing` takes a vocabulary's name, such as `voicing \"drop-2\" voices=4 range=\"G3..D5\"`".into()));
            };
            spec.vocabulary = Some(string(v, "a vocabulary's name", entry_err)?.to_string());
            for (k, e) in props(c) {
                match k {
                    "voices" => {
                        let v = number(e).filter(|v| v.fract() == 0.0 && (1.0..=16.0).contains(v));
                        #[expect(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "checked 1 to 16"
                        )]
                        let v = v.map(|v| v as usize);
                        spec.voices = Some(v.ok_or_else(|| {
                            entry_err(e, "`voices` is a whole number from 1 to 16".into())
                        })?);
                    }
                    "range" => {
                        let r = string(e, "`range`", entry_err)?;
                        spec.range = Some(Range::parse(r).ok_or_else(|| {
                            entry_err(e, format!("`{r}` is not a range; write the lowest and highest pitch, such as \"C3..A4\""))
                        })?);
                    }
                    other => {
                        return Err(entry_err(
                            e,
                            format!("`voicing` has `voices` and `range`, not `{other}`"),
                        ));
                    }
                }
            }
        }
        "weights" => {
            for (k, e) in props(c) {
                if !matches!(
                    k,
                    "movement"
                        | "leaps"
                        | "common"
                        | "spacing"
                        | "parallels"
                        | "crossing"
                        | "centre"
                        | "center"
                ) {
                    return Err(entry_err(
                        e,
                        format!(
                            "`{k}` is not a weight; use movement, leaps, common, spacing, parallels, crossing or centre"
                        ),
                    ));
                }
                let v = number(e)
                    .filter(|v| *v >= 0.0)
                    .ok_or_else(|| entry_err(e, "a weight is a number, 0 or more".into()))?;
                spec.weights.insert(k.to_string(), v);
            }
        }
        "nct" => {
            let kinds = args(c)
                .into_iter()
                .map(|e| {
                    e.value().as_string().and_then(Nct::parse).ok_or_else(|| {
                        entry_err(e, "a non-chord tone is passing, neighbour, suspension, anticipation or pedal".into())
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            spec.nct = Some(kinds);
            for (k, e) in props(c) {
                match (k, number(e)) {
                    ("beats", Some(b)) if b > 0.0 => spec.nct_beats = Some(b),
                    _ => {
                        return Err(entry_err(
                            e,
                            "`nct` takes `beats=N`, how long each lasts".into(),
                        ));
                    }
                }
            }
        }
        "pedal" => {
            let a = args(c);
            let p = match a.as_slice() {
                [e] => e.value().as_string().and_then(Pitch::parse),
                _ => None,
            };
            spec.pedal =
                Some(p.ok_or_else(|| {
                    node_err(c, "`pedal` takes one pitch, such as \"D2\"".into())
                })?);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn read_top(
    c: &KdlNode,
    node_err: NodeErr<'_>,
    entry_err: EntryErr<'_>,
) -> Result<Top, crate::sheet::SheetError> {
    let a = args(c);
    let Some((first, rest)) = a.split_first() else {
        return Err(node_err(
            c,
            "`top` takes a section and its top notes, such as `top \"chorus\" \"D5\" \"-\" \"C5\"`"
                .into(),
        ));
    };
    let section = string(first, "a section name", entry_err)?.to_string();
    let notes = rest
        .iter()
        .map(|e| {
            let t = string(e, "a top note", entry_err)?;
            if t == "-" {
                Ok(None)
            } else {
                Pitch::parse(t)
                    .map(Some)
                    .ok_or_else(|| entry_err(e, format!("`{t}` is not a pitch; write one such as \"D5\", or \"-\" to leave it free")))
            }
        })
        .collect::<Result<_, _>>()?;
    Ok(Top { section, notes })
}

fn read_fix(
    node: &KdlNode,
    node_err: NodeErr<'_>,
    entry_err: EntryErr<'_>,
) -> Result<Fix, crate::sheet::SheetError> {
    let given = args(node);
    let [section_name, voicing] = given.as_slice() else {
        return Err(node_err(
            node,
            "`fix` takes a section and a voicing, such as `fix \"verse\" bar=3 \"D3 G3 C4 F4\"`"
                .into(),
        ));
    };
    let section = string(section_name, "a section name", entry_err)?.to_string();
    let text = string(voicing, "a voicing", entry_err)?;
    let mut pitches = text
        .split_whitespace()
        .map(|p| Pitch::parse(p).ok_or_else(|| entry_err(voicing, format!("`{p}` is not a pitch"))))
        .collect::<Result<Vec<_>, _>>()?;
    pitches.sort();
    let (mut bar, mut beat) = (1, 1.0);
    for (k, e) in props(node) {
        match (k, number(e)) {
            ("bar", Some(b)) if b >= 1.0 && b.fract() == 0.0 => {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a positive whole number"
                )]
                let b = b as u32;
                bar = b;
            }
            ("beat", Some(b)) if b >= 1.0 => beat = b,
            _ => {
                return Err(entry_err(
                    e,
                    "`fix` takes `bar=N` (from 1) and `beat=N` (from 1)".into(),
                ));
            }
        }
    }
    Ok(Fix {
        section,
        bar,
        beat,
        pitches,
    })
}
