//! Pitch classes, spelled note names and MIDI pitches.
//!
//! Pitches use the same names as `mort midi`: `C4` is middle C, MIDI 60.

use std::fmt;

/// A pitch class: 0 is C, 11 is B.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PitchClass(u8);

impl PitchClass {
    /// The pitch class of `n` semitones above C, for any `n`.
    #[must_use]
    pub fn new(n: i32) -> Self {
        // `rem_euclid(12)` is in 0..12.
        Self(u8::try_from(n.rem_euclid(12)).unwrap_or(0))
    }

    /// Its number, 0 to 11.
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }

    /// The pitch class `semitones` above this one.
    #[must_use]
    pub fn plus(self, semitones: i32) -> Self {
        Self::new(i32::from(self.0) + semitones)
    }

    /// How many semitones above `below` this pitch class lies, 0 to 11.
    #[must_use]
    pub fn above(self, below: PitchClass) -> u8 {
        Self::new(i32::from(self.0) - i32::from(below.0)).0
    }
}

/// A letter name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Letter {
    /// C
    C,
    /// D
    D,
    /// E
    E,
    /// F
    F,
    /// G
    G,
    /// A
    A,
    /// B
    B,
}

impl Letter {
    const ALL: [Letter; 7] = [
        Letter::C,
        Letter::D,
        Letter::E,
        Letter::F,
        Letter::G,
        Letter::A,
        Letter::B,
    ];

    /// The pitch class of the letter without accidentals.
    #[must_use]
    pub fn natural(self) -> PitchClass {
        PitchClass(match self {
            Letter::C => 0,
            Letter::D => 2,
            Letter::E => 4,
            Letter::F => 5,
            Letter::G => 7,
            Letter::A => 9,
            Letter::B => 11,
        })
    }

    /// The letter for an upper- or lower-case character.
    #[must_use]
    pub fn from_char(c: char) -> Option<Self> {
        let i = "CDEFGAB".find(c.to_ascii_uppercase())?;
        Self::ALL.get(i).copied()
    }

    fn as_char(self) -> char {
        ['C', 'D', 'E', 'F', 'G', 'A', 'B'][self as usize]
    }
}

/// A spelled note name without an octave: a letter and an accidental,
/// from −2 (double flat) to +2 (double sharp).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Spelling {
    /// The letter.
    pub letter: Letter,
    /// Sharps (positive) or flats (negative).
    pub accidental: i8,
}

impl Spelling {
    /// Its pitch class.
    #[must_use]
    pub fn pitch_class(self) -> PitchClass {
        self.letter.natural().plus(i32::from(self.accidental))
    }

    /// Reads a spelling from the start of `text`: a letter, then up to two
    /// sharps (`#`, `♯`) or two flats (`b`, `♭`), not mixed. Returns it and
    /// the rest of the text.
    ///
    /// A `b` straight after the letter is a flat, so `Bb` is B flat and
    /// `bb` is B flat too; a chord's quality never starts with `b`.
    #[must_use]
    pub fn parse_prefix(text: &str) -> Option<(Self, &str)> {
        let mut chars = text.char_indices();
        let (_, first) = chars.next()?;
        let letter = Letter::from_char(first)?;
        let mut accidental: i8 = 0;
        let mut end = first.len_utf8();
        for (i, c) in chars {
            let step = match c {
                '#' | '♯' => 1,
                'b' | '♭' => -1,
                _ => break,
            };
            if accidental * step < 0 || (accidental + step).abs() > 2 {
                return None;
            }
            accidental += step;
            end = i + c.len_utf8();
        }
        Some((Self { letter, accidental }, &text[end..]))
    }

    /// The spelling of `pc` with `letter`, if it needs at most two
    /// accidentals.
    #[must_use]
    pub fn with_letter(pc: PitchClass, letter: Letter) -> Option<Self> {
        let up = i8::try_from(pc.above(letter.natural())).ok()?;
        let accidental = if up > 6 { up - 12 } else { up };
        (accidental.abs() <= 2).then_some(Self { letter, accidental })
    }

    /// The letter `steps` letters above this one (a third is 2 steps).
    #[must_use]
    pub fn letter_up(self, steps: usize) -> Letter {
        Letter::ALL[(self.letter as usize + steps) % 7]
    }
}

impl fmt::Display for Spelling {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.letter.as_char())?;
        let sign = if self.accidental > 0 { "#" } else { "b" };
        for _ in 0..self.accidental.unsigned_abs() {
            f.write_str(sign)?;
        }
        Ok(())
    }
}

/// A MIDI pitch, 0 to 127.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pitch(u8);

/// Sharps, as `mort midi` names pitches.
const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

impl Pitch {
    /// The pitch with MIDI number `n`, if it is 0 to 127.
    #[must_use]
    pub fn new(n: i32) -> Option<Self> {
        u8::try_from(n).ok().filter(|n| *n <= 127).map(Self)
    }

    /// Its MIDI number.
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }

    /// Its pitch class.
    #[must_use]
    pub fn class(self) -> PitchClass {
        PitchClass::new(i32::from(self.0))
    }

    /// Reads a pitch such as `C4` (60), `F#2` or `Bb-1`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (s, rest) = Spelling::parse_prefix(text.trim())?;
        let octave: i32 = rest.parse().ok()?;
        let n = (octave + 1) * 12 + i32::from(s.letter.natural().get()) + i32::from(s.accidental);
        Self::new(n)
    }
}

impl fmt::Display for Pitch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            NAMES[usize::from(self.0 % 12)],
            i32::from(self.0 / 12) - 1
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitches_match_mort_midi() {
        assert_eq!(Pitch::parse("C4").map(Pitch::get), Some(60));
        assert_eq!(Pitch::parse("Bb3").map(Pitch::get), Some(58));
        assert_eq!(Pitch::parse("B#3").map(Pitch::get), Some(60));
        assert_eq!(Pitch::parse("C-1").map(Pitch::get), Some(0));
        assert_eq!(Pitch::parse("G9").map(Pitch::get), Some(127));
        assert_eq!(Pitch::parse("G#9"), None);
        assert_eq!(Pitch::parse("H4"), None);
        assert_eq!(Pitch::new(61).unwrap().to_string(), "C#4");
    }

    #[test]
    fn spellings() {
        let (s, rest) = Spelling::parse_prefix("Ebmaj7").unwrap();
        assert_eq!((s.to_string().as_str(), rest), ("Eb", "maj7"));
        assert_eq!(s.pitch_class().get(), 3);
        let (s, _) = Spelling::parse_prefix("F♯").unwrap();
        assert_eq!(s.to_string(), "F#");
        assert_eq!(Spelling::parse_prefix("Cb###"), None);
        let gs = Spelling::with_letter(PitchClass::new(8), Letter::G).unwrap();
        assert_eq!(gs.to_string(), "G#");
        assert_eq!(Spelling::with_letter(PitchClass::new(8), Letter::C), None);
        assert_eq!(PitchClass::new(2).above(PitchClass::new(9)), 5);
    }
}
