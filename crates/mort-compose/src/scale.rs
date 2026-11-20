//! Scales and modes: the note sets harmony is built from (ADR 0013 §4).

use std::fmt;

use crate::pitch::PitchClass;

/// A scale's definition: its name, other names it goes by, and its notes
/// as semitones above the root, rising from 0.
#[derive(Debug, PartialEq, Eq)]
pub struct ScaleDef {
    /// The name mort prints.
    pub name: &'static str,
    /// Other names it is known by.
    pub aliases: &'static [&'static str],
    /// Semitones above the root, rising, starting at 0.
    pub steps: &'static [u8],
}

macro_rules! scale {
    ($name:literal, [$($alias:literal),*], [$($step:literal),*]) => {
        ScaleDef { name: $name, aliases: &[$($alias),*], steps: &[$($step),*] }
    };
}

/// Every scale mort knows.
pub static SCALES: &[ScaleDef] = &[
    // The modes of the major scale.
    scale!("ionian", ["major"], [0, 2, 4, 5, 7, 9, 11]),
    scale!("dorian", [], [0, 2, 3, 5, 7, 9, 10]),
    scale!("phrygian", [], [0, 1, 3, 5, 7, 8, 10]),
    scale!("lydian", [], [0, 2, 4, 6, 7, 9, 11]),
    scale!("mixolydian", [], [0, 2, 4, 5, 7, 9, 10]),
    scale!(
        "aeolian",
        ["minor", "natural minor"],
        [0, 2, 3, 5, 7, 8, 10]
    ),
    scale!("locrian", [], [0, 1, 3, 5, 6, 8, 10]),
    // The modes of melodic minor.
    scale!("melodic minor", ["jazz minor"], [0, 2, 3, 5, 7, 9, 11]),
    scale!("dorian b2", ["phrygian #6"], [0, 1, 3, 5, 7, 9, 10]),
    scale!("lydian augmented", ["lydian #5"], [0, 2, 4, 6, 8, 9, 11]),
    scale!(
        "lydian dominant",
        ["lydian b7", "overtone"],
        [0, 2, 4, 6, 7, 9, 10]
    ),
    scale!(
        "mixolydian b6",
        ["aeolian dominant", "melodic major"],
        [0, 2, 4, 5, 7, 8, 10]
    ),
    scale!(
        "locrian #2",
        ["half diminished", "aeolian b5"],
        [0, 2, 3, 5, 6, 8, 10]
    ),
    scale!("altered", ["super locrian"], [0, 1, 3, 4, 6, 8, 10]),
    // The modes of harmonic minor.
    scale!("harmonic minor", [], [0, 2, 3, 5, 7, 8, 11]),
    scale!("locrian #6", [], [0, 1, 3, 5, 6, 9, 10]),
    scale!("ionian #5", [], [0, 2, 4, 5, 8, 9, 11]),
    scale!("dorian #4", ["ukrainian dorian"], [0, 2, 3, 6, 7, 9, 10]),
    scale!("phrygian dominant", ["freygish"], [0, 1, 4, 5, 7, 8, 10]),
    scale!("lydian #2", [], [0, 3, 4, 6, 7, 9, 11]),
    scale!("ultralocrian", ["super locrian bb7"], [0, 1, 3, 4, 6, 8, 9]),
    // Symmetric scales.
    scale!("whole tone", [], [0, 2, 4, 6, 8, 10]),
    scale!(
        "diminished half-whole",
        ["half-whole", "dominant diminished"],
        [0, 1, 3, 4, 6, 7, 9, 10]
    ),
    scale!(
        "diminished whole-half",
        ["whole-half", "diminished"],
        [0, 2, 3, 5, 6, 8, 9, 11]
    ),
    // Pentatonic and blues.
    scale!("major pentatonic", ["pentatonic"], [0, 2, 4, 7, 9]),
    scale!("minor pentatonic", [], [0, 3, 5, 7, 10]),
    scale!("blues", ["minor blues"], [0, 3, 5, 6, 7, 10]),
];

/// A name made comparable: lower case, `♭`/`♯` as `b`/`#`, and `-`, `_`
/// and runs of spaces as one space.
fn normalise(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '♭' => 'b',
            '♯' => '#',
            '-' | '_' => ' ',
            c => c.to_ascii_lowercase(),
        })
        .collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The scale named `name`, by its name or an alias, ignoring case.
#[must_use]
pub fn find(name: &str) -> Option<&'static ScaleDef> {
    let n = normalise(name);
    SCALES
        .iter()
        .find(|s| normalise(s.name) == n || s.aliases.iter().any(|a| normalise(a) == n))
}

/// A scale on a root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    /// The root.
    pub root: PitchClass,
    /// Which scale.
    pub def: &'static ScaleDef,
}

impl Scale {
    /// Its pitch classes, from the root up.
    #[must_use]
    pub fn pitch_classes(&self) -> Vec<PitchClass> {
        self.def
            .steps
            .iter()
            .map(|s| self.root.plus(i32::from(*s)))
            .collect()
    }

    /// Whether it holds `pc`.
    #[must_use]
    pub fn contains(&self, pc: PitchClass) -> bool {
        self.def.steps.contains(&pc.above(self.root))
    }

    /// Whether it holds a note `semitones` above its root.
    #[must_use]
    pub fn has_step(&self, semitones: u8) -> bool {
        self.def.steps.contains(&(semitones % 12))
    }
}

impl fmt::Display for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [&str; 12] = [
            "C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B",
        ];
        write!(
            f,
            "{} {}",
            NAMES[usize::from(self.root.get())],
            self.def.name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scale_is_well_formed_and_named_once() {
        let mut names = Vec::new();
        for s in SCALES {
            assert_eq!(s.steps[0], 0, "{}", s.name);
            assert!(
                s.steps.windows(2).all(|w| w[0] < w[1] && w[1] < 12),
                "{}",
                s.name
            );
            names.push(normalise(s.name));
            names.extend(s.aliases.iter().map(|a| normalise(a)));
        }
        let n = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), n, "a name or alias is used twice");
    }

    #[test]
    fn modes_are_rotations_of_their_parent() {
        let rotate = |parent: &[u8], k: usize| -> Vec<u8> {
            let r = parent[k];
            let mut v: Vec<u8> = parent.iter().map(|s| (s + 12 - r) % 12).collect();
            v.sort_unstable();
            v
        };
        let families = [
            [
                "ionian",
                "dorian",
                "phrygian",
                "lydian",
                "mixolydian",
                "aeolian",
                "locrian",
            ],
            [
                "melodic minor",
                "dorian b2",
                "lydian augmented",
                "lydian dominant",
                "mixolydian b6",
                "locrian #2",
                "altered",
            ],
            [
                "harmonic minor",
                "locrian #6",
                "ionian #5",
                "dorian #4",
                "phrygian dominant",
                "lydian #2",
                "ultralocrian",
            ],
        ];
        for fam in families {
            let parent = find(fam[0]).unwrap().steps;
            for (k, name) in fam.iter().enumerate() {
                assert_eq!(
                    find(name).unwrap().steps,
                    rotate(parent, k).as_slice(),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn finds_by_alias_and_spelling() {
        assert_eq!(find("Super-Locrian").unwrap().name, "altered");
        assert_eq!(find("lydian ♭7").unwrap().name, "lydian dominant");
        assert_eq!(find("whole_half").unwrap().name, "diminished whole-half");
        assert!(find("bebop").is_none());
        let d = Scale {
            root: PitchClass::new(2),
            def: find("dorian").unwrap(),
        };
        assert_eq!(d.to_string(), "D dorian");
        assert!(d.contains(PitchClass::new(11)) && !d.contains(PitchClass::new(10)));
    }
}
