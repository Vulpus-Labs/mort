//! Voicing vocabularies (ADR 0013 §5): rules, kept as KDL data, that build
//! the candidate voicings of a harmony within a range.
//!
//! The format is described at the top of `voicings/builtin.kdl`, which
//! holds the built-in vocabularies. Vocabularies in the user's directory
//! ([`user_dir`]) and the project's (`<project>.mort/voicings/`) are
//! loaded after them and replace any with the same name.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use kdl::{KdlEntry, KdlNode, KdlValue};

use crate::chord::Role;
use crate::harmony::{Harmony, Note};
use crate::pitch::{Pitch, PitchClass};
use crate::source::{args, line_col, parse_kdl, props};

/// The built-in vocabularies' source.
pub const BUILTIN: &str = include_str!("../voicings/builtin.kdl");

/// A range of pitches, both ends included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    /// The lowest pitch allowed.
    pub low: Pitch,
    /// The highest pitch allowed.
    pub high: Pitch,
}

impl Range {
    /// Reads `C3..A5`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (a, b) = text.split_once("..")?;
        let (low, high) = (Pitch::parse(a)?, Pitch::parse(b)?);
        (low <= high).then_some(Self { low, high })
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.low, self.high)
    }
}

/// A note of a voicing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoicedNote {
    /// Its pitch.
    pub pitch: Pitch,
    /// What it is in the harmony.
    pub role: Role,
}

/// A voicing: notes from the bottom up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voicing {
    /// The notes, lowest first.
    pub notes: Vec<VoicedNote>,
}

impl Voicing {
    /// Its pitches, lowest first.
    #[must_use]
    pub fn pitches(&self) -> Vec<u8> {
        self.notes.iter().map(|n| n.pitch.get()).collect()
    }
}

impl fmt::Display for Voicing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self
            .notes
            .iter()
            .map(|n| format!("{}({})", n.pitch, n.role))
            .collect();
        f.write_str(&parts.join(" "))
    }
}

/// Where a vocabulary came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Built into mort.
    Builtin,
    /// A file.
    File(PathBuf),
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Builtin => f.write_str("built in"),
            Source::File(p) => write!(f, "{}", p.display()),
        }
    }
}

/// A role name as written in a vocabulary: `root`, `3rd`, `9th`,
/// `tension` and so on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoleName {
    Exact(Role),
    AnyTension,
}

impl RoleName {
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "root" => Self::Exact(Role::Root),
            "2nd" => Self::Exact(Role::Second),
            "3rd" => Self::Exact(Role::Third),
            "4th" => Self::Exact(Role::Fourth),
            "5th" => Self::Exact(Role::Fifth),
            "6th" => Self::Exact(Role::Sixth),
            "7th" => Self::Exact(Role::Seventh),
            "9th" => Self::Exact(Role::Tension(9)),
            "11th" => Self::Exact(Role::Tension(11)),
            "13th" => Self::Exact(Role::Tension(13)),
            "tension" => Self::AnyTension,
            _ => return None,
        })
    }

    fn matches(self, r: Role) -> bool {
        match self {
            Self::Exact(x) => x == r,
            Self::AnyTension => matches!(r, Role::Tension(_)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    Close,
    /// Drop these voices, counted from the top, an octave.
    Drop(&'static [usize]),
}

/// Chord tones picked by role and arranged.
#[derive(Debug, Clone)]
struct ChordRule {
    roles: Vec<RoleName>,
    tensions: bool,
    double: Vec<RoleName>,
    layout: Layout,
    bass: Option<(RoleName, u8)>,
}

#[derive(Debug, Clone)]
enum Build {
    Chord(ChordRule),
    /// Each step any of these semitones.
    Step(Vec<u8>),
    /// Fixed stacks, as alternatives.
    Steps(Vec<Vec<u8>>),
    Layered(Box<Layer>, Box<Layer>),
}

#[derive(Debug, Clone)]
struct Require {
    roles: Vec<RoleName>,
    count: Option<usize>,
}

impl Require {
    fn holds(&self, roles: &[Role]) -> bool {
        match self.count {
            Some(n) => {
                roles
                    .iter()
                    .filter(|r| self.roles.iter().any(|x| x.matches(**r)))
                    .count()
                    >= n
            }
            None => self
                .roles
                .iter()
                .all(|x| roles.iter().any(|r| x.matches(*r))),
        }
    }
}

#[derive(Debug, Clone)]
struct Layer {
    build: Build,
    voices: Vec<usize>,
    from: Vec<RoleName>,
    require: Vec<Require>,
}

/// A shape: notes as semitones on an absolute scale (C-1 is 0), not yet
/// placed in a range, with their roles. Sorted, lowest first.
type Shape = Vec<(i32, Role)>;

/// A voicing vocabulary.
#[derive(Debug, Clone)]
pub struct Vocabulary {
    /// Its name.
    pub name: String,
    /// What it is for.
    pub description: Option<String>,
    /// Where it came from.
    pub source: Source,
    top: Layer,
}

/// Why a vocabulary built no voicing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateError {
    /// What went wrong, and what to do.
    pub message: String,
}

impl fmt::Display for CandidateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CandidateError {}

impl Vocabulary {
    /// The voicings it builds for `harmony` with `voices` notes inside
    /// `range`, lowest first, then by pitches.
    ///
    /// # Errors
    ///
    /// [`CandidateError`] if it builds none, saying why and what to try.
    pub fn candidates(
        &self,
        harmony: &Harmony,
        range: Range,
        voices: usize,
    ) -> Result<Vec<Voicing>, CandidateError> {
        if !self.top.voices.is_empty() && !self.top.voices.contains(&voices) {
            let allowed: Vec<String> = self.top.voices.iter().map(ToString::to_string).collect();
            return Err(CandidateError {
                message: format!(
                    "`{}` voices {} notes, not {voices}; change the part's voice count or its vocabulary",
                    self.name,
                    allowed.join(" or ")
                ),
            });
        }
        let mut out: Vec<Voicing> = place_layer(&self.top, harmony, range, voices)
            .into_iter()
            .map(|pitches| Voicing {
                notes: pitches
                    .into_iter()
                    .filter_map(|(p, role)| {
                        Some(VoicedNote {
                            pitch: Pitch::new(p)?,
                            role,
                        })
                    })
                    .collect(),
            })
            .collect();
        out.sort_by_key(Voicing::pitches);
        out.dedup_by(|a, b| a.pitches() == b.pitches());
        if out.is_empty() {
            return Err(CandidateError {
                message: format!(
                    "`{}` builds no {voices}-voice voicing of {harmony} within {range}; try a wider range, another voice count or another vocabulary",
                    self.name
                ),
            });
        }
        Ok(out)
    }
}

/// The notes a layer may use: chord tones, and the available tensions if
/// `tensions`.
fn pool(h: &Harmony, tensions: bool) -> Vec<Note> {
    if tensions {
        h.available()
    } else {
        h.tones.clone()
    }
}

/// A layer's voicings, placed in the range, as (pitch, role) lists.
fn place_layer(layer: &Layer, h: &Harmony, range: Range, voices: usize) -> Vec<Vec<(i32, Role)>> {
    if !layer.voices.is_empty() && !layer.voices.contains(&voices) {
        return Vec::new();
    }
    if let Build::Layered(lower, upper) = &layer.build {
        let Some(&n_low) = lower.voices.first() else {
            return Vec::new();
        };
        if n_low >= voices {
            return Vec::new();
        }
        let lows = place_layer(lower, h, range, n_low);
        let ups = place_layer(upper, h, range, voices - n_low);
        let mut out = Vec::new();
        for l in &lows {
            for u in &ups {
                let top = l.last().map_or(i32::MIN, |n| n.0);
                if u.first().is_some_and(|n| n.0 > top) {
                    let v: Vec<(i32, Role)> = l.iter().chain(u).copied().collect();
                    if keeps(layer, &v) {
                        out.push(v);
                    }
                }
            }
        }
        return out;
    }
    let mut out = Vec::new();
    for shape in shapes(layer, h, voices) {
        if !keeps(layer, &shape) {
            continue;
        }
        let (Some(lo), Some(hi)) = (shape.first(), shape.last()) else {
            continue;
        };
        let low = i32::from(range.low.get());
        let high = i32::from(range.high.get());
        let first = (low - lo.0).div_euclid(12) + i32::from((low - lo.0).rem_euclid(12) != 0);
        let last = (high - hi.0).div_euclid(12);
        for k in first..=last {
            out.push(shape.iter().map(|(p, r)| (p + 12 * k, *r)).collect());
        }
    }
    out
}

fn keeps(layer: &Layer, notes: &[(i32, Role)]) -> bool {
    let roles: Vec<Role> = notes.iter().map(|n| n.1).collect();
    layer.require.iter().all(|r| r.holds(&roles))
}

fn shapes(layer: &Layer, h: &Harmony, voices: usize) -> Vec<Shape> {
    match &layer.build {
        Build::Chord(rule) => rule.shapes(h, voices),
        Build::Step(choices) => {
            let alts = sequences(choices, voices.saturating_sub(1));
            stack_shapes(h, &layer.from, &alts)
        }
        Build::Steps(alts) => {
            let fit: Vec<Vec<u8>> = alts
                .iter()
                .filter(|a| a.len() + 1 == voices)
                .cloned()
                .collect();
            stack_shapes(h, &layer.from, &fit)
        }
        Build::Layered(..) => Vec::new(),
    }
}

/// Every sequence of `n` steps, each one of `choices`.
fn sequences(choices: &[u8], n: usize) -> Vec<Vec<u8>> {
    let mut out = vec![Vec::new()];
    for _ in 0..n {
        out = out
            .into_iter()
            .flat_map(|s| {
                choices.iter().map(move |c| {
                    let mut t = s.clone();
                    t.push(*c);
                    t
                })
            })
            .collect();
    }
    out
}

/// Stacks from every allowed bottom note, keeping those whose notes are
/// all available.
fn stack_shapes(h: &Harmony, from: &[RoleName], alts: &[Vec<u8>]) -> Vec<Shape> {
    let avail = h.available();
    let role = |pc: PitchClass| avail.iter().find(|n| n.pc == pc).map(|n| n.role);
    let mut out = Vec::new();
    for start in &avail {
        if !from.is_empty() && !from.iter().any(|f| f.matches(start.role)) {
            continue;
        }
        'alt: for steps in alts {
            let mut p = i32::from(start.pc.get());
            let mut shape = vec![(p, start.role)];
            for s in steps {
                p += i32::from(*s);
                match role(PitchClass::new(p)) {
                    Some(r) => shape.push((p, r)),
                    None => continue 'alt,
                }
            }
            out.push(shape);
        }
    }
    out
}

impl ChordRule {
    fn shapes(&self, harmony: &Harmony, voices: usize) -> Vec<Shape> {
        let pool = pool(harmony, self.tensions);
        let bass_note = match self.bass {
            Some((name, gap)) => match pool.iter().find(|n| name.matches(n.role)) {
                Some(n) => Some((*n, gap)),
                None => return Vec::new(),
            },
            None => None,
        };
        let wanted = voices - usize::from(bass_note.is_some());
        let Some((mut distinct, doubles)) =
            self.pick(harmony, &pool, wanted, bass_note.map(|(n, _)| n.pc))
        else {
            return Vec::new();
        };
        distinct.sort_by_key(|n| n.pc.above(harmony.root));
        let mut out = Vec::new();
        for inversion in 0..distinct.len() {
            let Some(stack) = self.arrange(&distinct, &doubles, inversion) else {
                continue;
            };
            match bass_note {
                None => out.push(stack),
                Some((low, gap)) => {
                    let bottom = stack[0].0;
                    let mut below = i32::from(gap.max(1));
                    while PitchClass::new(bottom - below) != low.pc {
                        below += 1;
                    }
                    for extra in [0, 12] {
                        let mut with_bass = vec![(bottom - below - extra, low.role)];
                        with_bass.extend(stack.iter().copied());
                        out.push(with_bass);
                    }
                }
            }
        }
        out
    }

    /// Picks `wanted` notes: distinct ones by role in order of preference,
    /// never `skip`'s pitch class, first from the notes the symbol names
    /// and then, if allowed, from the scale's tensions (so `Dm9` gives
    /// F A C E before reaching for B); then doubles in order.
    fn pick(
        &self,
        harmony: &Harmony,
        pool: &[Note],
        wanted: usize,
        skip: Option<PitchClass>,
    ) -> Option<(Vec<Note>, Vec<Note>)> {
        if wanted == 0 {
            return None;
        }
        let mut picked: Vec<Note> = Vec::new();
        for from in [harmony.tones.as_slice(), pool] {
            for name in &self.roles {
                if picked.len() == wanted {
                    break;
                }
                if let Some(n) = from.iter().find(|n| {
                    name.matches(n.role)
                        && !picked.iter().any(|p| p.pc == n.pc)
                        && skip != Some(n.pc)
                }) {
                    picked.push(*n);
                }
            }
        }
        if picked.is_empty() {
            return None;
        }
        let mut doubles = Vec::new();
        let mut tries = 0;
        while picked.len() + doubles.len() < wanted && tries < wanted * self.double.len() {
            let name = self.double[tries % self.double.len()];
            if let Some(n) = picked.iter().find(|n| name.matches(n.role)) {
                doubles.push(*n);
            }
            tries += 1;
        }
        (picked.len() + doubles.len() == wanted).then_some((picked, doubles))
    }

    /// One inversion in close position, doubled notes an octave above
    /// their first appearance, then the layout's drops. `None` if the
    /// layout needs more voices.
    fn arrange(&self, distinct: &[Note], doubles: &[Note], inversion: usize) -> Option<Shape> {
        let mut stack: Shape = Vec::new();
        for n in distinct.iter().cycle().skip(inversion).take(distinct.len()) {
            let mut v = i32::from(n.pc.get());
            if let Some(&(prev, _)) = stack.last() {
                while v <= prev {
                    v += 12;
                }
            }
            stack.push((v, n.role));
        }
        for d in doubles {
            let first = stack
                .iter()
                .find(|(v, _)| PitchClass::new(*v) == d.pc)
                .map_or(i32::from(d.pc.get()), |(v, _)| *v);
            let mut v = first + 12;
            while stack.iter().any(|(x, _)| *x == v) {
                v += 12;
            }
            stack.push((v, d.role));
        }
        stack.sort_unstable_by_key(|n| n.0);
        if let Layout::Drop(which) = self.layout {
            let m = stack.len();
            if which.iter().any(|d| *d > m) {
                return None;
            }
            for d in which {
                stack[m - d].0 -= 12;
            }
            stack.sort_unstable_by_key(|n| n.0);
        }
        Some(stack)
    }
}

/// Why a vocabulary file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabError {
    /// The file, or `built in`.
    pub source: String,
    /// The line, from 1.
    pub line: usize,
    /// The column, from 1.
    pub column: usize,
    /// What went wrong, and what to do.
    pub message: String,
}

impl fmt::Display for VocabError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} line {}, column {}: {}",
            self.source, self.line, self.column, self.message
        )
    }
}

impl std::error::Error for VocabError {}

/// The vocabularies available, by name.
#[derive(Debug, Clone, Default)]
pub struct Vocabularies {
    map: BTreeMap<String, Vocabulary>,
}

/// The user's vocabulary directory:
/// `~/Library/Application Support/mort/voicings`.
#[must_use]
pub fn user_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| {
        PathBuf::from(h)
            .join("Library")
            .join("Application Support")
            .join("mort")
            .join("voicings")
    })
}

impl Vocabularies {
    /// The built-in vocabularies.
    ///
    /// # Panics
    ///
    /// If the built-in file does not read, which its tests rule out.
    #[must_use]
    pub fn builtin() -> Self {
        let mut v = Self::default();
        if let Err(e) = v.load_str(BUILTIN, &Source::Builtin) {
            panic!("the built-in vocabularies do not read: {e}");
        }
        v
    }

    /// The built-in vocabularies, then the user's, then the project's
    /// (in `sidecar/voicings`), each replacing earlier ones by name.
    ///
    /// # Errors
    ///
    /// [`VocabError`] for the first file that does not read.
    pub fn standard(sidecar: Option<&Path>) -> Result<Self, VocabError> {
        let mut v = Self::builtin();
        if let Some(d) = user_dir() {
            v.load_dir(&d)?;
        }
        if let Some(s) = sidecar {
            v.load_dir(&s.join("voicings"))?;
        }
        Ok(v)
    }

    /// The vocabulary named `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Vocabulary> {
        self.map.get(name)
    }

    /// Every vocabulary, by name.
    pub fn iter(&self) -> impl Iterator<Item = &Vocabulary> {
        self.map.values()
    }

    /// Loads every `.kdl` file in `dir`, in name order. A missing
    /// directory loads nothing. Returns how many vocabularies were read.
    ///
    /// # Errors
    ///
    /// [`VocabError`] for the first file that does not read.
    pub fn load_dir(&mut self, dir: &Path) -> Result<usize, VocabError> {
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
            let src = std::fs::read_to_string(&f).map_err(|e| VocabError {
                source: f.display().to_string(),
                line: 1,
                column: 1,
                message: format!("cannot read it: {e}"),
            })?;
            n += self.load_str(&src, &Source::File(f))?;
        }
        Ok(n)
    }

    /// Loads the vocabularies in `src`, replacing any with the same name.
    /// Returns how many were read.
    ///
    /// # Errors
    ///
    /// [`VocabError`] with the line and column of the first problem.
    pub fn load_str(&mut self, src: &str, source: &Source) -> Result<usize, VocabError> {
        let at = |offset: usize, message: String| {
            let (line, column) = line_col(src, offset);
            VocabError {
                source: source.to_string(),
                line,
                column,
                message,
            }
        };
        let doc = parse_kdl(src).map_err(|(o, m)| at(o, m))?;
        let mut read = Vec::new();
        for n in doc.nodes() {
            if n.name().value() != "voicing" {
                return Err(at(
                    n.span().offset(),
                    format!(
                        "`{}` is not a vocabulary; a vocabulary file holds `voicing` nodes",
                        n.name().value()
                    ),
                ));
            }
            let a = args(n);
            let name = match a.as_slice() {
                [e] => e.value().as_string().map(str::to_string),
                _ => None,
            }
            .ok_or_else(|| {
                at(
                    n.span().offset(),
                    "a voicing takes one name, such as `voicing \"quartal\"`".into(),
                )
            })?;
            let description = n
                .children()
                .and_then(|c| c.get("description"))
                .and_then(|d| d.entries().first())
                .and_then(|e| e.value().as_string())
                .map(str::to_string);
            let top = layer(n, &at)?;
            read.push(Vocabulary {
                name,
                description,
                source: source.clone(),
                top,
            });
        }
        let count = read.len();
        for v in read {
            self.map.insert(v.name.clone(), v);
        }
        Ok(count)
    }
}

type At<'a> = &'a dyn Fn(usize, String) -> VocabError;

fn err_n(at: At<'_>, n: &KdlNode, m: String) -> VocabError {
    at(n.span().offset(), m)
}

fn err_e(at: At<'_>, e: &KdlEntry, m: String) -> VocabError {
    at(e.span().offset(), m)
}

fn role_names(at: At<'_>, n: &KdlNode) -> Result<Vec<RoleName>, VocabError> {
    args(n)
        .into_iter()
        .map(|e| {
            e.value().as_string().and_then(RoleName::parse).ok_or_else(|| {
                let shown = e
                    .value()
                    .as_string()
                    .map_or_else(|| e.value().to_string(), str::to_string);
                err_e(
                    at,
                    e,
                    format!(
                        "`{shown}` is not a role; use root, 2nd, 3rd, 4th, 5th, 6th, 7th, 9th, 11th, 13th or tension"
                    ),
                )
            })
        })
        .collect()
}

/// A node's arguments as whole numbers from 1 to `max`.
fn numbers(at: At<'_>, n: &KdlNode, what: &str, max: u8) -> Result<Vec<u8>, VocabError> {
    args(n)
        .into_iter()
        .map(|e| match e.value() {
            KdlValue::Integer(i) if (1..=i128::from(max)).contains(i) => {
                Ok(u8::try_from(*i).unwrap_or(max))
            }
            _ => Err(err_e(
                at,
                e,
                format!("{what} must be whole numbers from 1 to {max}"),
            )),
        })
        .collect()
}

/// A voicing or layer node's contents as read so far.
#[derive(Default)]
struct Draft {
    chord: Option<(Vec<RoleName>, bool)>,
    double: Vec<RoleName>,
    layout: Option<Layout>,
    bass: Option<(RoleName, u8)>,
    step: Option<Vec<u8>>,
    steps: Vec<Vec<u8>>,
    lower: Option<Layer>,
    upper: Option<Layer>,
    voices: Vec<usize>,
    from: Vec<RoleName>,
    require: Vec<Require>,
}

impl Draft {
    /// Reads one child node into the draft.
    fn read(&mut self, at: At<'_>, c: &KdlNode) -> Result<(), VocabError> {
        match c.name().value() {
            "description" => {}
            "chord" => {
                let mut tensions = false;
                for (k, e) in props(c) {
                    match (k, e.value()) {
                        ("tensions", KdlValue::Bool(b)) => tensions = *b,
                        _ => {
                            return Err(err_e(
                                at,
                                e,
                                format!("`chord` takes `tensions=#true` or `#false`, not `{k}`"),
                            ));
                        }
                    }
                }
                self.chord = Some((role_names(at, c)?, tensions));
            }
            "double" => self.double = role_names(at, c)?,
            "layout" => self.layout = Some(read_layout(at, c)?),
            "bass" => self.bass = Some(read_bass(at, c)?),
            "step" => self.step = Some(numbers(at, c, "steps", 12)?),
            "steps" => self.steps.push(numbers(at, c, "steps", 12)?),
            "voices" => {
                self.voices = numbers(at, c, "voice counts", 16)?
                    .into_iter()
                    .map(usize::from)
                    .collect();
            }
            "from" => self.from = role_names(at, c)?,
            "require" => self.require.push(read_require(at, c)?),
            "lower" => self.lower = Some(layer(c, at)?),
            "upper" => self.upper = Some(layer(c, at)?),
            other => {
                return Err(err_n(
                    at,
                    c,
                    format!(
                        "`{other}` is not part of a voicing; see the format at the top of voicings/builtin.kdl"
                    ),
                ));
            }
        }
        Ok(())
    }

    /// The layer the draft describes.
    fn finish(self, at: At<'_>, n: &KdlNode) -> Result<Layer, VocabError> {
        let kinds = usize::from(self.chord.is_some())
            + usize::from(self.step.is_some())
            + usize::from(!self.steps.is_empty())
            + usize::from(self.lower.is_some() || self.upper.is_some());
        if kinds != 1 {
            return Err(err_n(
                at,
                n,
                "give exactly one of `chord`, `step`, `steps`, or `lower` with `upper`".into(),
            ));
        }
        let mut voices = self.voices;
        let build = if let Some((roles, tensions)) = self.chord {
            Build::Chord(ChordRule {
                roles,
                tensions,
                double: self.double,
                layout: self.layout.unwrap_or(Layout::Close),
                bass: self.bass,
            })
        } else if let Some(s) = self.step {
            Build::Step(s)
        } else if !self.steps.is_empty() {
            if voices.is_empty() {
                voices = self.steps.iter().map(|s| s.len() + 1).collect();
                voices.sort_unstable();
                voices.dedup();
            }
            Build::Steps(self.steps)
        } else {
            let (Some(l), Some(u)) = (self.lower, self.upper) else {
                return Err(err_n(
                    at,
                    n,
                    "a layered voicing needs both `lower` and `upper`".into(),
                ));
            };
            if l.voices.len() != 1 {
                return Err(err_n(
                    at,
                    n,
                    "`lower` needs one voice count, such as `voices 2`".into(),
                ));
            }
            Build::Layered(Box::new(l), Box::new(u))
        };
        Ok(Layer {
            build,
            voices,
            from: self.from,
            require: self.require,
        })
    }
}

fn read_layout(at: At<'_>, c: &KdlNode) -> Result<Layout, VocabError> {
    let a = args(c);
    let name = match a.as_slice() {
        [e] => e.value().as_string(),
        _ => None,
    };
    match name {
        Some("close") => Ok(Layout::Close),
        Some("drop2") => Ok(Layout::Drop(&[2])),
        Some("drop3") => Ok(Layout::Drop(&[3])),
        Some("drop24") => Ok(Layout::Drop(&[2, 4])),
        _ => Err(err_n(
            at,
            c,
            "`layout` is close, drop2, drop3 or drop24".into(),
        )),
    }
}

fn read_bass(at: At<'_>, c: &KdlNode) -> Result<(RoleName, u8), VocabError> {
    let names = role_names(at, c)?;
    let [role] = names.as_slice() else {
        return Err(err_n(
            at,
            c,
            "`bass` takes one role, such as `bass \"root\"`".into(),
        ));
    };
    let mut gap = 0;
    for (k, e) in props(c) {
        match (k, e.value()) {
            ("gap", KdlValue::Integer(g)) if (0..=24).contains(g) => {
                gap = u8::try_from(*g).unwrap_or(0);
            }
            _ => {
                return Err(err_e(
                    at,
                    e,
                    "`bass` takes `gap=N`, N semitones from 0 to 24".into(),
                ));
            }
        }
    }
    Ok((*role, gap))
}

fn read_require(at: At<'_>, c: &KdlNode) -> Result<Require, VocabError> {
    let mut count = None;
    for (k, e) in props(c) {
        match (k, e.value()) {
            ("count", KdlValue::Integer(i)) if (1..=16).contains(i) => {
                count = usize::try_from(*i).ok();
            }
            _ => {
                return Err(err_e(
                    at,
                    e,
                    "`require` takes `count=N`, N from 1 to 16".into(),
                ));
            }
        }
    }
    Ok(Require {
        roles: role_names(at, c)?,
        count,
    })
}

/// Reads a voicing node, or a `lower`/`upper` layer within one.
fn layer(n: &KdlNode, at: At<'_>) -> Result<Layer, VocabError> {
    let mut draft = Draft::default();
    for c in n
        .children()
        .map(kdl::KdlDocument::nodes)
        .unwrap_or_default()
    {
        draft.read(at, c)?;
    }
    draft.finish(at, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtins_read() {
        let v = Vocabularies::builtin();
        let names: Vec<&str> = v.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "close",
                "cluster",
                "drop-2",
                "drop-3",
                "guide tones",
                "quartal",
                "root fifth",
                "rootless",
                "shell",
                "so what",
                "spread",
                "upper structure"
            ]
        );
        assert!(
            v.iter()
                .all(|x| x.description.is_some() && x.source == Source::Builtin)
        );
    }

    #[test]
    fn ranges() {
        let r = Range::parse("C3..A5").unwrap();
        assert_eq!((r.low.get(), r.high.get()), (48, 81));
        assert!(Range::parse("A5..C3").is_none());
        assert_eq!(r.to_string(), "C3..A5");
    }
}
