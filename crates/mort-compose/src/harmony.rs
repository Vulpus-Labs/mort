//! Harmony (ADR 0013 §4): what may sound at a moment.
//!
//! A harmony has a root, a scale, chord tones with their roles, the
//! tensions available over them and the notes to avoid. It comes from a
//! chord symbol (with a default scale, or one the sheet names), or from a
//! bare scale such as `D dorian`, whose chord tones are its own root,
//! third, fifth and seventh.

use std::fmt;

use crate::chord::{ChordError, ChordSymbol, Role, Seventh, Triad};
use crate::pitch::{Pitch, PitchClass, Spelling};
use crate::scale::{self, SCALES, Scale, ScaleDef};

/// The kind of chord, for choosing a default scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Major triads, sixths and major sevenths.
    Major,
    /// Major third and minor seventh.
    Dominant,
    /// `alt` chords.
    Altered,
    /// A minor triad with nothing above it.
    MinorTriad,
    /// Minor chords with a sixth or seventh.
    Minor,
    /// Minor third, diminished fifth, minor seventh.
    HalfDiminished,
    /// A diminished triad with nothing above it.
    DiminishedTriad,
    /// Diminished seventh chords.
    Diminished,
    /// An augmented triad, with no seventh.
    Augmented,
    /// Suspended chords.
    Sus,
    /// Power chords.
    Power,
}

/// A candidate default scale and why it is there.
#[derive(Debug, Clone, Copy)]
pub struct Choice {
    /// The scale's name, as in [`SCALES`].
    pub scale: &'static str,
    /// Why it is a candidate, and why in this place.
    pub reason: &'static str,
}

const fn choice(scale: &'static str, reason: &'static str) -> Choice {
    Choice { scale, reason }
}

/// The default scales for each family, in order of preference. A chord
/// takes the first that holds every note its symbol asks for, so the
/// alterations in the symbol pick the scale: `G7` is mixolydian, `G7b9`
/// half-whole diminished, `G7b9b13` phrygian dominant. The sheet can
/// always name a scale instead.
pub static DEFAULT_SCALES: &[(Family, &[Choice])] = &[
    (
        Family::Major,
        &[
            choice(
                "ionian",
                "the plain major scale; its 4th is the one avoid note",
            ),
            choice("lydian", "when the symbol asks for #11"),
            choice(
                "lydian augmented",
                "when the symbol asks for #5 with a major 7th",
            ),
            choice("ionian #5", "for #5 with a natural 11"),
        ],
    ),
    (
        Family::Dominant,
        &[
            choice("mixolydian", "the plain dominant scale"),
            choice("lydian dominant", "when the symbol asks for #11"),
            choice(
                "whole tone",
                "for #5 (or b13) with no perfect 5th and natural 9ths",
            ),
            choice("mixolydian b6", "for b13 with a perfect 5th and natural 9"),
            choice("diminished half-whole", "for b9 or #9 with a natural 13"),
            choice("phrygian dominant", "for b9 with b13"),
            choice(
                "altered",
                "for altered 9ths with #5 or b13 and no perfect 5th",
            ),
        ],
    ),
    (
        Family::Altered,
        &[choice("altered", "`alt` names the altered scale")],
    ),
    (
        Family::MinorTriad,
        &[
            choice(
                "aeolian",
                "a plain minor triad is most often i in a minor key",
            ),
            choice("dorian", "when the symbol asks for a major 6th"),
            choice("phrygian", "when the symbol asks for b9"),
            choice(
                "melodic minor",
                "for a major 7th or a natural 9 with a major 6th",
            ),
            choice("harmonic minor", "for a major 7th with b13"),
        ],
    ),
    (
        Family::Minor,
        &[
            choice(
                "dorian",
                "the usual minor-seventh scale in jazz and modal writing",
            ),
            choice("aeolian", "when the symbol asks for b13"),
            choice("phrygian", "when the symbol asks for b9"),
            choice("melodic minor", "for a minor-major 7th"),
            choice("harmonic minor", "for a minor-major 7th with b13"),
        ],
    ),
    (
        Family::HalfDiminished,
        &[
            choice(
                "locrian #2",
                "the natural 9 is the usual tension on m7b5 in jazz",
            ),
            choice("locrian", "when the symbol asks for b9"),
        ],
    ),
    (
        Family::DiminishedTriad,
        &[
            choice(
                "locrian",
                "a diminished triad is most often vii in a major key",
            ),
            choice(
                "diminished whole-half",
                "when the symbol asks for a diminished 7th",
            ),
        ],
    ),
    (
        Family::Diminished,
        &[choice(
            "diminished whole-half",
            "the symmetric scale built on the diminished seventh",
        )],
    ),
    (
        Family::Augmented,
        &[
            choice(
                "whole tone",
                "the symmetric scale built on the augmented triad",
            ),
            choice("lydian augmented", "when the symbol asks for a major 7th"),
        ],
    ),
    (
        Family::Sus,
        &[
            choice("mixolydian", "a sus chord usually stands for a dominant"),
            choice("ionian", "when the symbol asks for a major 7th"),
            choice("phrygian", "when the symbol asks for b9"),
            choice("dorian b2", "for b9 with a natural 13"),
        ],
    ),
    (
        Family::Power,
        &[
            choice(
                "ionian",
                "a power chord implies no third; major is the plainest guess, so name the scale if it is minor",
            ),
            choice("aeolian", "when the symbol asks for b13"),
        ],
    ),
];

/// The family a chord belongs to.
#[must_use]
pub fn family(c: &ChordSymbol) -> Family {
    if c.alt {
        return Family::Altered;
    }
    let flat5 = c.alterations.contains(&crate::chord::Alteration::Flat5);
    match (c.triad, c.seventh) {
        (Triad::Power, _) => Family::Power,
        (Triad::Sus2 | Triad::Sus4, _) => Family::Sus,
        (Triad::Diminished, Seventh::Diminished) => Family::Diminished,
        (Triad::Diminished, Seventh::Minor) => Family::HalfDiminished,
        (Triad::Diminished, _) => Family::DiminishedTriad,
        (Triad::Minor, Seventh::Minor) if flat5 => Family::HalfDiminished,
        (Triad::Minor, Seventh::None) if flat5 => Family::DiminishedTriad,
        (Triad::Minor, Seventh::None) => Family::MinorTriad,
        (Triad::Minor, _) => Family::Minor,
        (Triad::Major | Triad::Augmented, Seventh::Minor) => Family::Dominant,
        (Triad::Augmented, Seventh::None) => Family::Augmented,
        (Triad::Major | Triad::Augmented, _) => Family::Major,
    }
}

/// A note of a harmony: its role and pitch class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Note {
    /// What it is.
    pub role: Role,
    /// Its pitch class.
    pub pc: PitchClass,
}

/// Why a harmony could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarmonyError {
    /// What went wrong, and what to do.
    pub message: String,
}

impl fmt::Display for HarmonyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HarmonyError {}

impl From<ChordError> for HarmonyError {
    fn from(e: ChordError) -> Self {
        Self {
            message: e.to_string(),
        }
    }
}

/// What may sound at a moment.
#[derive(Debug, Clone, PartialEq)]
pub struct Harmony {
    /// The root.
    pub root: PitchClass,
    /// The root as written, for spelling.
    pub name: Spelling,
    /// The scale.
    pub scale: Scale,
    /// Why this scale, when it was chosen by default.
    pub scale_reason: Option<&'static str>,
    /// The chord symbol, if it came from one.
    pub chord: Option<ChordSymbol>,
    /// The lowest note: the slash bass, or the root.
    pub bass: PitchClass,
    /// The chord tones (including tensions the symbol names), by role.
    pub tones: Vec<Note>,
    /// Scale notes that may be added: not chord tones, not avoided.
    pub tensions: Vec<Note>,
    /// Scale notes to avoid: a semitone above a chord tone.
    pub avoid: Vec<Note>,
}

impl Harmony {
    /// Reads a harmony: a chord symbol (`Dm7`) or a root and scale
    /// (`D dorian`). `scale` overrides a chord's default scale: a scale
    /// name on the chord's root, or a root and scale.
    ///
    /// # Errors
    ///
    /// [`HarmonyError`] if the text is neither, or the scale is unknown.
    pub fn parse(text: &str, scale: Option<&str>) -> Result<Self, HarmonyError> {
        let text = text.trim();
        if let Some((root, def)) = root_and_scale(text) {
            if scale.is_some() {
                return Err(HarmonyError {
                    message: format!("`{text}` is already a scale; drop `scale=`"),
                });
            }
            return Ok(Self::from_scale(root, def));
        }
        let chord = ChordSymbol::parse(text)?;
        let over = match scale {
            None => None,
            Some(s) => Some(match root_and_scale(s) {
                Some((r, d)) => (r.pitch_class(), d),
                None => (
                    chord.root.pitch_class(),
                    scale::find(s).ok_or_else(|| unknown_scale(s))?,
                ),
            }),
        };
        Self::from_chord(chord, over)
    }

    /// A harmony from a chord symbol, on its default scale or on `scale`
    /// (a root and a scale).
    ///
    /// # Errors
    ///
    /// [`HarmonyError`] if no scale mort knows holds every note of the
    /// symbol, and none was given.
    pub fn from_chord(
        chord: ChordSymbol,
        scale: Option<(PitchClass, &'static ScaleDef)>,
    ) -> Result<Self, HarmonyError> {
        let root = chord.root.pitch_class();
        let tones: Vec<Note> = chord
            .tones()
            .into_iter()
            .map(|t| Note {
                role: t.role,
                pc: root.plus(i32::from(t.interval)),
            })
            .collect();
        let (scale, scale_reason) = if let Some((r, def)) = scale {
            (Scale { root: r, def }, None)
        } else {
            let (def, reason) = default_scale(&chord)?;
            (Scale { root, def }, Some(reason))
        };
        let bass = chord.bass.map_or(root, Spelling::pitch_class);
        Ok(Self::complete(
            chord.root,
            scale,
            scale_reason,
            Some(chord),
            bass,
            tones,
        ))
    }

    /// A harmony from a bare scale: its chord tones are the scale's own
    /// root, third, fifth and seventh.
    #[must_use]
    pub fn from_scale(name: Spelling, def: &'static ScaleDef) -> Self {
        let root = name.pitch_class();
        let scale = Scale { root, def };
        let tones = scale_chord(def)
            .into_iter()
            .map(|(role, i)| Note {
                role,
                pc: root.plus(i32::from(i)),
            })
            .collect();
        Self::complete(name, scale, None, None, root, tones)
    }

    fn complete(
        name: Spelling,
        scale: Scale,
        scale_reason: Option<&'static str>,
        chord: Option<ChordSymbol>,
        bass: PitchClass,
        tones: Vec<Note>,
    ) -> Self {
        let root = name.pitch_class();
        let has_third = tones.iter().any(|t| t.role == Role::Third);
        let has_seventh = tones
            .iter()
            .any(|t| matches!(t.role, Role::Seventh | Role::Sixth));
        let mut tensions = Vec::new();
        let mut avoid = Vec::new();
        for pc in scale.pitch_classes() {
            if tones.iter().any(|t| t.pc == pc) {
                continue;
            }
            let note = Note {
                role: tension_role(pc.above(root), has_seventh, has_third),
                pc,
            };
            if tones.iter().any(|t| pc.above(t.pc) == 1) {
                avoid.push(note);
            } else {
                tensions.push(note);
            }
        }
        Self {
            root,
            name,
            scale,
            scale_reason,
            chord,
            bass,
            tones,
            tensions,
            avoid,
        }
    }

    /// `p` spelled for this harmony: a note with a role takes its role's
    /// letter (A7alt's 3rd is C#, its #9 B#); another scale note takes its
    /// degree's letter (B flat, not A sharp, in E flat lydian); any other
    /// note is sharp, or flat if the root is.
    #[must_use]
    pub fn spell(&self, p: Pitch) -> String {
        let pc = p.class();
        let flats = self.name.accidental < 0
            || (self.name.letter == crate::pitch::Letter::F && self.name.accidental == 0);
        let steps = self.scale.def.steps;
        let scale_root = if self.scale.root == self.root {
            Some(self.name)
        } else {
            plain_spelling(self.scale.root, flats)
        };
        let by_degree = scale_root.filter(|_| steps.len() == 7).and_then(|r| {
            let i = steps
                .iter()
                .position(|s| self.scale.root.plus(i32::from(*s)) == pc)?;
            Spelling::with_letter(pc, r.letter_up(i))
        });
        // A note with a role takes that role's letter: A7alt's 3rd is C#
        // and its #9 B#, though the altered scale's degrees would say D
        // flat and C.
        let role = self
            .available()
            .iter()
            .chain(&self.avoid)
            .find(|n| n.pc == pc)
            .map(|n| n.role);
        let by_role = role.and_then(|r| {
            let steps_up = match r {
                Role::Root => 0,
                Role::Second | Role::Tension(9) => 1,
                Role::Third => 2,
                Role::Fourth | Role::Tension(11) => 3,
                Role::Fifth => 4,
                Role::Sixth | Role::Tension(13) => 5,
                Role::Seventh | Role::Tension(_) => 6,
            };
            Spelling::with_letter(pc, self.name.letter_up(steps_up))
        });
        let Some(sp) = by_role.or(by_degree).or_else(|| plain_spelling(pc, flats)) else {
            return p.to_string();
        };
        let natural = i32::from(sp.letter.natural().get()) + i32::from(sp.accidental);
        let octave = (i32::from(p.get()) - natural).div_euclid(12) - 1;
        format!("{sp}{octave}")
    }

    /// Every note that may sound: chord tones and available tensions.
    #[must_use]
    pub fn available(&self) -> Vec<Note> {
        self.tones.iter().chain(&self.tensions).copied().collect()
    }

    /// The role of `pc` here, if it may sound.
    #[must_use]
    pub fn role_of(&self, pc: PitchClass) -> Option<Role> {
        self.available().iter().find(|n| n.pc == pc).map(|n| n.role)
    }
}

impl fmt::Display for Harmony {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.chord {
            Some(c) => write!(f, "{c} ({})", self.scale),
            None => write!(f, "{}", self.scale),
        }
    }
}

fn unknown_scale(name: &str) -> HarmonyError {
    HarmonyError {
        message: format!(
            "`{name}` is not a scale mort knows; use one such as `dorian`, `lydian dominant` or `altered` (see mort_compose::scale::SCALES)"
        ),
    }
}

/// A pitch class with a natural if it has one, else a sharp, or a flat
/// if `flats`.
fn plain_spelling(pc: PitchClass, flats: bool) -> Option<Spelling> {
    use crate::pitch::Letter;
    let letters = [
        Letter::C,
        Letter::D,
        Letter::E,
        Letter::F,
        Letter::G,
        Letter::A,
        Letter::B,
    ];
    let with = |acc: i8| {
        letters
            .iter()
            .find(|l| l.natural().plus(i32::from(acc)) == pc)
            .map(|l| Spelling {
                letter: *l,
                accidental: acc,
            })
    };
    with(0).or_else(|| if flats { with(-1) } else { with(1) })
}

/// `D dorian`: a root, a space, and a scale name.
fn root_and_scale(text: &str) -> Option<(Spelling, &'static ScaleDef)> {
    let (root, rest) = Spelling::parse_prefix(text)?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    scale::find(rest).map(|d| (root, d))
}

/// The default scale for a chord: the first in its family's list that
/// holds every note of the symbol, else the first scale mort knows that
/// does.
///
/// # Errors
///
/// [`HarmonyError`] if no scale holds them all.
pub fn default_scale(
    chord: &ChordSymbol,
) -> Result<(&'static ScaleDef, &'static str), HarmonyError> {
    let steps: Vec<u8> = chord.tones().iter().map(|t| t.interval).collect();
    let holds = |d: &ScaleDef| steps.iter().all(|s| d.steps.contains(s));
    let fam = family(chord);
    let list = DEFAULT_SCALES
        .iter()
        .find(|(f, _)| *f == fam)
        .map_or(&[][..], |(_, l)| *l);
    for c in list {
        if let Some(d) = scale::find(c.scale)
            && holds(d)
        {
            return Ok((d, c.reason));
        }
    }
    SCALES
        .iter()
        .find(|d| holds(d))
        .map(|d| {
            (
                d,
                "no scale in the chord's family holds every note of the symbol; this is the first that does",
            )
        })
        .ok_or_else(|| HarmonyError {
            message: format!(
                "no scale mort knows holds every note of `{chord}`; name one with `scale=`"
            ),
        })
}

/// The chord on a bare scale: root, third, fifth and seventh. A
/// seven-note scale uses its 1st, 3rd, 5th and 7th notes; other scales
/// take the nearest notes to those degrees that they have.
fn scale_chord(def: &ScaleDef) -> Vec<(Role, u8)> {
    let s = def.steps;
    let mut out = vec![(Role::Root, 0)];
    if s.len() == 7 {
        let third = match s[2] {
            2 => Role::Second,
            5 => Role::Fourth,
            _ => Role::Third,
        };
        out.push((third, s[2]));
        out.push((Role::Fifth, s[4]));
        out.push((Role::Seventh, s[6]));
        return out;
    }
    let pick = |prefs: &[u8]| prefs.iter().copied().find(|p| s.contains(p));
    let third = pick(&[4, 3]);
    // Without a perfect fifth, a major third takes the augmented one
    // (whole tone) and a minor third the diminished one.
    let fifth = if third == Some(4) {
        pick(&[7, 8, 6])
    } else {
        pick(&[7, 6, 8])
    };
    if let Some(t) = third {
        out.push((Role::Third, t));
    }
    if let Some(f) = fifth {
        out.push((Role::Fifth, f));
    }
    let seventh = if third == Some(3) && fifth == Some(6) {
        pick(&[9, 10, 11])
    } else {
        pick(&[10, 11])
    };
    if let Some(v) = seventh {
        out.push((Role::Seventh, v));
    }
    out
}

/// The role of a scale note `interval` semitones above the root that is
/// not a chord tone.
fn tension_role(interval: u8, has_seventh: bool, has_third: bool) -> Role {
    match interval {
        // A sus or power chord has no third: the scale's third is still a
        // third (D over Bbsus2), not an 11th.
        3 | 4 if !has_third => Role::Third,
        1..=3 => Role::Tension(9),
        4..=6 => Role::Tension(11),
        7 => Role::Fifth,
        8 | 9 => Role::Tension(13),
        _ if !has_seventh => Role::Tension(7),
        _ => Role::Seventh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcs(notes: &[Note]) -> Vec<u8> {
        let mut v: Vec<u8> = notes.iter().map(|n| n.pc.get()).collect();
        v.sort_unstable();
        v
    }

    fn scale_of(symbol: &str) -> &'static str {
        Harmony::parse(symbol, None)
            .unwrap_or_else(|e| panic!("{e}"))
            .scale
            .def
            .name
    }

    #[test]
    fn default_scales_follow_the_symbol() {
        let cases = [
            ("C", "ionian"),
            ("C6/9", "ionian"),
            ("Cmaj7", "ionian"),
            ("Cmaj7#11", "lydian"),
            ("Cmaj7#5", "lydian augmented"),
            ("G7", "mixolydian"),
            ("G13", "mixolydian"),
            ("G7#11", "lydian dominant"),
            ("G7#5", "whole tone"),
            ("G7b13", "mixolydian b6"),
            ("G7b9", "diminished half-whole"),
            ("G13b9", "diminished half-whole"),
            ("G7b9b13", "phrygian dominant"),
            ("G7#9#5", "altered"),
            ("G7alt", "altered"),
            ("Am", "aeolian"),
            ("Am6", "dorian"),
            ("Dm7", "dorian"),
            ("Dm11", "dorian"),
            ("Em7b9", "phrygian"),
            ("CmMaj7", "melodic minor"),
            ("Bm7b5", "locrian #2"),
            ("Bdim", "locrian"),
            ("Bdim7", "diminished whole-half"),
            ("Caug", "whole tone"),
            ("G7sus4", "mixolydian"),
            ("C5", "ionian"),
        ];
        for (s, want) in cases {
            assert_eq!(scale_of(s), want, "{s}");
        }
    }

    #[test]
    fn every_listed_scale_exists() {
        for (_, list) in DEFAULT_SCALES {
            for c in *list {
                assert!(scale::find(c.scale).is_some(), "{}", c.scale);
            }
        }
    }

    #[test]
    fn tensions_and_avoid_notes() {
        // G7 on mixolydian: C (a semitone above B) is the avoid note; A
        // and E are tensions.
        let h = Harmony::parse("G7", None).unwrap();
        assert_eq!(pcs(&h.tones), vec![2, 5, 7, 11]);
        assert_eq!(pcs(&h.avoid), vec![0]);
        assert_eq!(pcs(&h.tensions), vec![4, 9]);
        assert_eq!(h.role_of(PitchClass::new(9)), Some(Role::Tension(9)));
        assert_eq!(h.role_of(PitchClass::new(4)), Some(Role::Tension(13)));
        // Dm7 on dorian: nothing to avoid.
        let h = Harmony::parse("Dm7", None).unwrap();
        assert!(h.avoid.is_empty());
        assert_eq!(pcs(&h.tensions), vec![4, 7, 11]);
        // Cmaj7 on ionian: F.
        let h = Harmony::parse("Cmaj7", None).unwrap();
        assert_eq!(pcs(&h.avoid), vec![5]);
    }

    #[test]
    fn bare_scales() {
        let h = Harmony::parse("D dorian", None).unwrap();
        assert!(h.chord.is_none());
        assert_eq!(pcs(&h.tones), vec![0, 2, 5, 9]);
        assert_eq!(h.to_string(), "D dorian");
        let h = Harmony::parse("C whole tone", None).unwrap();
        assert_eq!(pcs(&h.tones), vec![0, 4, 8, 10]);
        let h = Harmony::parse("A minor pentatonic", None).unwrap();
        assert_eq!(pcs(&h.tones), vec![0, 4, 7, 9]);
        let h = Harmony::parse("C diminished", None).unwrap();
        assert_eq!(pcs(&h.tones), vec![0, 3, 6, 9]);
    }

    #[test]
    fn spelling_follows_the_harmony() {
        let p = |t: &str| Pitch::parse(t).unwrap();
        let h = Harmony::parse("Eb lydian", None).unwrap();
        assert_eq!(h.spell(p("A#3")), "Bb3");
        assert_eq!(h.spell(p("A4")), "A4", "the #11");
        let h = Harmony::parse("A7alt", None).unwrap();
        let names: Vec<String> = ["A3", "C#4", "G4", "A#4", "C5", "D#5", "F5"]
            .iter()
            .map(|n| h.spell(p(n)))
            .collect();
        assert_eq!(names, ["A3", "C#4", "G4", "Bb4", "B#4", "D#5", "F5"]);
        let h = Harmony::parse("F#m7", None).unwrap();
        assert_eq!(h.spell(p("A4")), "A4");
        assert_eq!(h.spell(p("D#4")), "D#4");
        let h = Harmony::parse("Cb lydian", None).unwrap();
        assert_eq!(h.spell(p("B3")), "Cb4", "octave follows the letter");
    }

    #[test]
    fn a_sus_chord_spells_the_scale_third_as_a_third() {
        let p = |t: &str| Pitch::parse(t).unwrap();
        let h = Harmony::parse("Bbmaj7sus2", Some("D aeolian")).unwrap();
        assert_eq!(h.spell(p("D2")), "D2");
        assert_eq!(h.role_of(p("D2").class()), Some(Role::Third));
        let h = Harmony::parse("Csus2", Some("D aeolian")).unwrap();
        assert_eq!(h.spell(p("E4")), "E4");
        let h = Harmony::parse("G7#9", None).unwrap();
        assert_eq!(
            h.role_of(p("A#4").class()),
            Some(Role::Tension(9)),
            "with a 3rd, it stays a #9"
        );
    }

    #[test]
    fn scale_overrides_and_slash_bass() {
        let h = Harmony::parse("Dm7", Some("aeolian")).unwrap();
        assert_eq!(h.scale.to_string(), "D aeolian");
        assert!(h.scale_reason.is_none());
        let h = Harmony::parse("G7sus4", Some("D dorian")).unwrap();
        assert_eq!(h.scale.to_string(), "D dorian");
        let h = Harmony::parse("F/A", None).unwrap();
        assert_eq!(h.bass.get(), 9);
        assert!(
            Harmony::parse("Dm7", Some("bebop"))
                .unwrap_err()
                .message
                .contains("`bebop`")
        );
        assert!(Harmony::parse("D dorian", Some("aeolian")).is_err());
    }
}
