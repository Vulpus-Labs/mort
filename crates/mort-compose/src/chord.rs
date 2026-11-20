//! Chord symbols (ADR 0013 §4): parsing, printing, and the chord tones
//! each one asks for, with their roles.
//!
//! The grammar covers the usual jazz and pop symbols:
//! - a root, `C`, `F#`, `Bb`;
//! - a quality: major (nothing), `m`/`min`/`-`, `dim`/`°`/`o`, `aug`/`+`,
//!   `ø` (half-diminished), `5` (power chord);
//! - a seventh or sixth: `7`, `maj7`/`M7`/`Δ`, `mMaj7`, `6`, `6/9`;
//! - extensions, `9`, `11`, `13`, which stack the ones below them;
//! - `sus`, `sus2`, `sus4`;
//! - alterations, `b5`, `#5`, `b9`, `#9`, `#11`, `b13` (or with `+`/`-`),
//!   bare or in brackets;
//! - `alt`, `add9`/`add11`/`add13`, `omit3`/`omit5` (or `no3`);
//! - a slash bass, `/E`.

use std::fmt;

use crate::pitch::Spelling;

/// The chord's triad, or what stands in for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Triad {
    /// Root, major third, fifth.
    Major,
    /// Root, minor third, fifth.
    Minor,
    /// Root, minor third, diminished fifth.
    Diminished,
    /// Root, major third, augmented fifth.
    Augmented,
    /// Root, second, fifth.
    Sus2,
    /// Root, fourth, fifth.
    Sus4,
    /// Root and fifth.
    Power,
}

/// The chord's seventh, or the sixth in its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seventh {
    /// No seventh.
    None,
    /// A minor seventh (the dominant seventh on a major triad).
    Minor,
    /// A major seventh.
    Major,
    /// A diminished seventh.
    Diminished,
    /// A major sixth instead of a seventh.
    Sixth,
}

/// A chromatic alteration, in the order they are printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Alteration {
    /// b5
    Flat5,
    /// #5
    Sharp5,
    /// b9
    Flat9,
    /// #9
    Sharp9,
    /// #11
    Sharp11,
    /// b13
    Flat13,
}

impl Alteration {
    fn text(self) -> &'static str {
        match self {
            Alteration::Flat5 => "b5",
            Alteration::Sharp5 => "#5",
            Alteration::Flat9 => "b9",
            Alteration::Sharp9 => "#9",
            Alteration::Sharp11 => "#11",
            Alteration::Flat13 => "b13",
        }
    }
}

/// What a note is in a chord or harmony.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    /// The root.
    Root,
    /// A suspended second, in place of the third.
    Second,
    /// The third.
    Third,
    /// A suspended fourth, in place of the third.
    Fourth,
    /// The fifth, altered or not.
    Fifth,
    /// A sixth in place of the seventh.
    Sixth,
    /// The seventh.
    Seventh,
    /// A tension: its degree (7, 9, 11 or 13; 7 is a major seventh over a
    /// chord that has none).
    Tension(u8),
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::Root => f.write_str("root"),
            Role::Second => f.write_str("2nd"),
            Role::Third => f.write_str("3rd"),
            Role::Fourth => f.write_str("4th"),
            Role::Fifth => f.write_str("5th"),
            Role::Sixth => f.write_str("6th"),
            Role::Seventh => f.write_str("7th"),
            Role::Tension(d) => write!(f, "{d}th"),
        }
    }
}

/// A note of a chord: its role and its distance above the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChordTone {
    /// What it is.
    pub role: Role,
    /// Semitones above the root, 0 to 11.
    pub interval: u8,
}

/// A parsed chord symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChordSymbol {
    /// The root.
    pub root: Spelling,
    /// The triad.
    pub triad: Triad,
    /// The seventh or sixth.
    pub seventh: Seventh,
    /// The highest stacked extension, 9, 11 or 13, if any.
    pub extension: Option<u8>,
    /// Alterations, sorted.
    pub alterations: Vec<Alteration>,
    /// Added tensions (9, 11, 13), sorted.
    pub adds: Vec<u8>,
    /// Omitted degrees (3, 5), sorted.
    pub omits: Vec<u8>,
    /// `alt`: an altered dominant.
    pub alt: bool,
    /// A slash bass.
    pub bass: Option<Spelling>,
}

/// Why a chord symbol could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChordError {
    /// The symbol.
    pub symbol: String,
    /// What went wrong.
    pub message: String,
}

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a chord symbol: {}; write one such as `Dm7`, `G7b9`, `Cmaj9#11` or `F/A`",
            self.symbol, self.message
        )
    }
}

impl std::error::Error for ChordError {}

/// A cursor over the text after the root.
struct Cursor<'a> {
    rest: &'a str,
}

impl Cursor<'_> {
    fn eat(&mut self, token: &str) -> bool {
        if let Some(r) = self.rest.strip_prefix(token) {
            self.rest = r;
            true
        } else {
            false
        }
    }

    fn eat_any(&mut self, tokens: &[&str]) -> bool {
        tokens.iter().any(|t| self.eat(t))
    }

    /// A number, if one starts here.
    fn number(&mut self) -> Option<u8> {
        let n = self.rest.bytes().take_while(u8::is_ascii_digit).count();
        let (d, r) = self.rest.split_at(n);
        let v = d.parse().ok()?;
        self.rest = r;
        Some(v)
    }
}

impl ChordSymbol {
    /// Reads a chord symbol.
    ///
    /// # Errors
    ///
    /// [`ChordError`] naming the part that is not understood.
    pub fn parse(symbol: &str) -> Result<Self, ChordError> {
        let text = symbol.trim();
        let err = |message: String| ChordError {
            symbol: text.to_string(),
            message,
        };
        let (root, mut rest) = Spelling::parse_prefix(text)
            .ok_or_else(|| err("it must start with a root, A to G".into()))?;

        // A slash bass: the part after the last `/` if it is a note name
        // (so `6/9` is not one).
        let mut bass = None;
        if let Some(i) = rest.rfind('/')
            && let Some((b, "")) = Spelling::parse_prefix(&rest[i + 1..])
        {
            bass = Some(b);
            rest = &rest[..i];
        }

        let mut c = ChordSymbol {
            root,
            triad: Triad::Major,
            seventh: Seventh::None,
            extension: None,
            alterations: Vec::new(),
            adds: Vec::new(),
            omits: Vec::new(),
            alt: false,
            bass,
        };
        let mut cur = Cursor { rest };

        if cur.rest == "5" {
            c.triad = Triad::Power;
            cur.rest = "";
        } else if cur.eat_any(&[
            "mMaj", "m(maj", "mmaj", "minmaj", "-maj", "mΔ", "-Δ", "m∆", "mM",
        ]) {
            c.triad = Triad::Minor;
            c.read_major_seventh(&mut cur).map_err(err)?;
            cur.eat(")");
        } else if cur.eat_any(&["maj", "Maj", "MAJ", "M"])
            || (cur.rest.starts_with("ma")
                && cur.rest[2..].starts_with(|ch: char| ch.is_ascii_digit())
                && cur.eat("ma"))
        {
            // `Cmaj` alone is a major triad; `Cmaj7`, `CM9` have sevenths.
            if cur.rest.starts_with(|ch: char| ch.is_ascii_digit()) {
                c.read_major_seventh(&mut cur).map_err(err)?;
            }
        } else if cur.eat_any(&["Δ", "∆"]) {
            c.read_major_seventh(&mut cur).map_err(err)?;
        } else if cur.eat_any(&["ø", "Ø"]) {
            c.triad = Triad::Minor;
            c.seventh = Seventh::Minor;
            c.alterations.push(Alteration::Flat5);
            cur.eat("7");
        } else if cur.eat_any(&["dim", "°", "o"]) {
            c.triad = Triad::Diminished;
            if cur.eat("7") {
                c.seventh = Seventh::Diminished;
            }
        } else if cur.eat_any(&["aug", "+"]) {
            c.triad = Triad::Augmented;
            c.read_number(&mut cur).map_err(err)?;
        } else if cur.eat_any(&["min", "mi", "m", "-"]) {
            c.triad = Triad::Minor;
            if cur.eat_any(&["maj", "Maj", "M", "Δ"]) {
                c.read_major_seventh(&mut cur).map_err(err)?;
            } else {
                c.read_number(&mut cur).map_err(err)?;
            }
        } else {
            c.read_number(&mut cur).map_err(err)?;
        }

        c.read_modifiers(&mut cur).map_err(err)?;
        c.alterations.sort_unstable();
        c.alterations.dedup();
        c.adds.sort_unstable();
        c.adds.dedup();
        c.omits.sort_unstable();
        c.omits.dedup();
        Ok(c)
    }

    /// After `maj`: an optional 7, 9, 11 or 13 (`Δ` alone is `maj7`).
    fn read_major_seventh(&mut self, cur: &mut Cursor<'_>) -> Result<(), String> {
        self.seventh = Seventh::Major;
        match cur.number() {
            None | Some(7) => Ok(()),
            Some(n @ (9 | 11 | 13)) => {
                self.extension = Some(n);
                Ok(())
            }
            Some(n) => Err(format!("`maj{n}` is not a major-seventh chord")),
        }
    }

    /// A 6, 6/9, 7, 9, 11 or 13 after the quality, if there is one.
    fn read_number(&mut self, cur: &mut Cursor<'_>) -> Result<(), String> {
        if !cur.rest.starts_with(|ch: char| ch.is_ascii_digit()) {
            return Ok(());
        }
        match cur.number() {
            Some(6) => {
                self.seventh = Seventh::Sixth;
                if cur.eat("/9") {
                    self.adds.push(9);
                }
            }
            Some(69) => {
                self.seventh = Seventh::Sixth;
                self.adds.push(9);
            }
            Some(7) => self.seventh = Seventh::Minor,
            Some(n @ (9 | 11 | 13)) => {
                self.seventh = Seventh::Minor;
                self.extension = Some(n);
            }
            Some(n) => return Err(format!("`{n}` is not a sixth, seventh or extension")),
            None => {}
        }
        Ok(())
    }

    fn read_modifiers(&mut self, cur: &mut Cursor<'_>) -> Result<(), String> {
        loop {
            while cur.eat_any(&["(", ")", ",", " "]) {}
            if cur.rest.is_empty() {
                return Ok(());
            }
            if cur.eat("sus") {
                self.triad = match cur.number() {
                    None | Some(4) => Triad::Sus4,
                    Some(2) => Triad::Sus2,
                    Some(n) => return Err(format!("`sus{n}` is not sus2 or sus4")),
                };
            } else if cur.eat("alt") {
                self.alt = true;
                if self.seventh == Seventh::None {
                    self.seventh = Seventh::Minor;
                }
            } else if cur.eat("add") {
                let n = cur.number().ok_or("`add` needs a degree, such as add9")?;
                match n {
                    2 | 9 => self.adds.push(9),
                    4 | 11 => self.adds.push(11),
                    6 | 13 if self.seventh == Seventh::None => self.seventh = Seventh::Sixth,
                    13 => self.adds.push(13),
                    _ => return Err(format!("`add{n}` is not add9, add11 or add13")),
                }
            } else if cur.eat_any(&["omit", "no"]) {
                match cur.number() {
                    Some(n @ (3 | 5)) => self.omits.push(n),
                    _ => return Err("only the 3rd or 5th can be omitted".into()),
                }
            } else if let Some(a) = Self::read_alteration(cur)? {
                self.alterations.push(a);
            } else {
                return Err(format!("`{}` is not understood", cur.rest));
            }
        }
    }

    fn read_alteration(cur: &mut Cursor<'_>) -> Result<Option<Alteration>, String> {
        let flat = if cur.eat_any(&["b", "♭", "-"]) {
            true
        } else if cur.eat_any(&["#", "♯", "+"]) {
            false
        } else {
            return Ok(None);
        };
        let a = match (flat, cur.number()) {
            (true, Some(5)) => Alteration::Flat5,
            (false, Some(5)) => Alteration::Sharp5,
            (true, Some(9)) => Alteration::Flat9,
            (false, Some(9)) => Alteration::Sharp9,
            (false, Some(11)) => Alteration::Sharp11,
            (true, Some(13)) => Alteration::Flat13,
            (_, n) => {
                let sign = if flat { "b" } else { "#" };
                let n = n.map_or(String::new(), |n| n.to_string());
                return Err(format!(
                    "`{sign}{n}` is not an alteration (b5, #5, b9, #9, #11, b13)"
                ));
            }
        };
        Ok(Some(a))
    }

    /// The notes the symbol asks for, with their roles, from the root up
    /// in role order. Tensions the symbol names are included; the slash
    /// bass is not (see [`ChordSymbol::bass`]).
    #[must_use]
    pub fn tones(&self) -> Vec<ChordTone> {
        let mut out = vec![ChordTone {
            role: Role::Root,
            interval: 0,
        }];
        let mut push = |role, interval| out.push(ChordTone { role, interval });
        let has = |a| self.alterations.contains(&a);

        match self.triad {
            Triad::Major | Triad::Augmented => push(Role::Third, 4),
            Triad::Minor | Triad::Diminished => push(Role::Third, 3),
            Triad::Sus2 => push(Role::Second, 2),
            Triad::Sus4 => push(Role::Fourth, 5),
            Triad::Power => {}
        }
        let fifth = if has(Alteration::Flat5) || self.triad == Triad::Diminished {
            Some(6)
        } else if has(Alteration::Sharp5) || self.triad == Triad::Augmented {
            Some(8)
        } else if self.alt {
            None
        } else {
            Some(7)
        };
        if let Some(f) = fifth {
            push(Role::Fifth, f);
        }
        if has(Alteration::Flat5) && has(Alteration::Sharp5) {
            push(Role::Tension(13), 8);
        }
        match self.seventh {
            Seventh::None => {}
            Seventh::Minor => push(Role::Seventh, 10),
            Seventh::Major => push(Role::Seventh, 11),
            Seventh::Diminished => push(Role::Seventh, 9),
            Seventh::Sixth => push(Role::Sixth, 9),
        }

        // Stacked extensions. A dominant or major 13 leaves out the
        // natural 11, which clashes with the major third.
        let minorish = matches!(self.triad, Triad::Minor | Triad::Sus4 | Triad::Sus2);
        let ext = self.extension.unwrap_or(0);
        let mut nine = ext >= 9 || self.adds.contains(&9);
        let eleven = (ext == 11 || (ext == 13 && minorish) || self.adds.contains(&11))
            && self.triad != Triad::Sus4;
        let thirteen = ext >= 13 || self.adds.contains(&13);
        if self.alt {
            nine = false;
            push(Role::Tension(9), 1);
            push(Role::Tension(9), 3);
            push(Role::Tension(11), 6);
            push(Role::Tension(13), 8);
        }
        if nine && !has(Alteration::Flat9) && !has(Alteration::Sharp9) {
            push(Role::Tension(9), 2);
        }
        if has(Alteration::Flat9) {
            push(Role::Tension(9), 1);
        }
        if has(Alteration::Sharp9) {
            push(Role::Tension(9), 3);
        }
        if eleven && !has(Alteration::Sharp11) {
            push(Role::Tension(11), 5);
        }
        if has(Alteration::Sharp11) {
            push(Role::Tension(11), 6);
        }
        if thirteen && !has(Alteration::Flat13) {
            push(Role::Tension(13), 9);
        }
        if has(Alteration::Flat13) {
            push(Role::Tension(13), 8);
        }

        for o in &self.omits {
            let role = if *o == 3 { Role::Third } else { Role::Fifth };
            out.retain(|t| t.role != role);
        }
        let mut seen = Vec::new();
        out.retain(|t| {
            let new = !seen.contains(&t.interval);
            seen.push(t.interval);
            new
        });
        out
    }
}

impl fmt::Display for ChordSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.root)?;
        let n = self.extension.unwrap_or(7);
        let six_nine = self.seventh == Seventh::Sixth && self.adds.contains(&9);
        match self.triad {
            Triad::Power => return write!(f, "5"),
            Triad::Minor => f.write_str("m")?,
            Triad::Diminished => f.write_str("dim")?,
            Triad::Augmented => f.write_str("aug")?,
            Triad::Major | Triad::Sus2 | Triad::Sus4 => {}
        }
        match self.seventh {
            Seventh::None => {}
            Seventh::Minor if self.alt => f.write_str("7")?,
            Seventh::Minor => write!(f, "{n}")?,
            Seventh::Major if self.triad == Triad::Minor => write!(f, "Maj{n}")?,
            Seventh::Major => write!(f, "maj{n}")?,
            Seventh::Diminished => f.write_str("7")?,
            Seventh::Sixth if six_nine => f.write_str("6/9")?,
            Seventh::Sixth => f.write_str("6")?,
        }
        match self.triad {
            Triad::Sus2 => f.write_str("sus2")?,
            Triad::Sus4 => f.write_str("sus4")?,
            _ => {}
        }
        for a in &self.alterations {
            f.write_str(a.text())?;
        }
        if self.alt {
            f.write_str("alt")?;
        }
        for a in &self.adds {
            if !(six_nine && *a == 9) {
                write!(f, "add{a}")?;
            }
        }
        for o in &self.omits {
            write!(f, "omit{o}")?;
        }
        if let Some(b) = self.bass {
            write!(f, "/{b}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intervals(s: &str) -> Vec<u8> {
        let mut v: Vec<u8> = ChordSymbol::parse(s)
            .unwrap_or_else(|e| panic!("{e}"))
            .tones()
            .iter()
            .map(|t| t.interval)
            .collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn chord_tones() {
        let cases: &[(&str, &[u8])] = &[
            ("C", &[0, 4, 7]),
            ("Cm", &[0, 3, 7]),
            ("Cdim", &[0, 3, 6]),
            ("C+", &[0, 4, 8]),
            ("C5", &[0, 7]),
            ("Csus", &[0, 5, 7]),
            ("Csus2", &[0, 2, 7]),
            ("C6", &[0, 4, 7, 9]),
            ("C6/9", &[0, 2, 4, 7, 9]),
            ("Cm6", &[0, 3, 7, 9]),
            ("C7", &[0, 4, 7, 10]),
            ("Cmaj7", &[0, 4, 7, 11]),
            ("CΔ", &[0, 4, 7, 11]),
            ("CM9", &[0, 2, 4, 7, 11]),
            ("Cm7", &[0, 3, 7, 10]),
            ("C-7", &[0, 3, 7, 10]),
            ("CmMaj7", &[0, 3, 7, 11]),
            ("Cm(maj7)", &[0, 3, 7, 11]),
            ("Cm7b5", &[0, 3, 6, 10]),
            ("Cø", &[0, 3, 6, 10]),
            ("Cdim7", &[0, 3, 6, 9]),
            ("Caug7", &[0, 4, 8, 10]),
            ("C9", &[0, 2, 4, 7, 10]),
            ("Cm11", &[0, 2, 3, 5, 7, 10]),
            ("C13", &[0, 2, 4, 7, 9, 10]),
            ("Cm13", &[0, 2, 3, 5, 7, 9, 10]),
            ("C7sus4", &[0, 5, 7, 10]),
            ("C9sus4", &[0, 2, 5, 7, 10]),
            ("C7b9", &[0, 1, 4, 7, 10]),
            ("C7#9", &[0, 3, 4, 7, 10]),
            ("C7(b9,#11)", &[0, 1, 4, 6, 7, 10]),
            ("C13b9", &[0, 1, 4, 7, 9, 10]),
            ("Cmaj7#11", &[0, 4, 6, 7, 11]),
            ("C7+5", &[0, 4, 8, 10]),
            ("C7-9", &[0, 1, 4, 7, 10]),
            ("C7alt", &[0, 1, 3, 4, 6, 8, 10]),
            ("Cadd9", &[0, 2, 4, 7]),
            ("Cmadd9", &[0, 2, 3, 7]),
            ("Cma7", &[0, 4, 7, 11]),
            ("C7omit3", &[0, 7, 10]),
            ("C9no5", &[0, 2, 4, 10]),
        ];
        for (s, want) in cases {
            assert_eq!(intervals(s), *want, "{s}");
        }
    }

    #[test]
    fn roles() {
        let c = ChordSymbol::parse("G7b9").unwrap();
        let t = c.tones();
        assert!(t.contains(&ChordTone {
            role: Role::Third,
            interval: 4
        }));
        assert!(t.contains(&ChordTone {
            role: Role::Seventh,
            interval: 10
        }));
        assert!(t.contains(&ChordTone {
            role: Role::Tension(9),
            interval: 1
        }));
        let s = ChordSymbol::parse("Dsus4").unwrap().tones();
        assert!(
            s.iter().any(|t| t.role == Role::Fourth) && !s.iter().any(|t| t.role == Role::Third)
        );
    }

    #[test]
    fn slash_bass() {
        let c = ChordSymbol::parse("F/A").unwrap();
        assert_eq!(c.bass.unwrap().to_string(), "A");
        let c = ChordSymbol::parse("Bb6/9/D").unwrap();
        assert_eq!(c.bass.unwrap().to_string(), "D");
        assert_eq!(c.seventh, Seventh::Sixth);
        assert_eq!(c.adds, vec![9]);
    }

    #[test]
    fn prints_canonically_and_round_trips() {
        let cases = [
            ("C", "C"),
            ("Cmin7", "Cm7"),
            ("C-7b5", "Cm7b5"),
            ("Cø7", "Cm7b5"),
            ("CΔ9", "Cmaj9"),
            ("Cm(maj7)", "CmMaj7"),
            ("C°7", "Cdim7"),
            ("C+", "Caug"),
            ("Csus", "Csus4"),
            ("C9sus", "C9sus4"),
            ("C7(#11,b9)", "C7b9#11"),
            ("Calt", "C7alt"),
            ("C69", "C6/9"),
            ("Ebmaj7#11/G", "Ebmaj7#11/G"),
            ("F#m11", "F#m11"),
            ("Cadd2", "Cadd9"),
            ("C5", "C5"),
            ("C7no3", "C7omit3"),
        ];
        for (input, want) in cases {
            let c = ChordSymbol::parse(input).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(c.to_string(), want, "{input}");
            assert_eq!(ChordSymbol::parse(want).unwrap(), c, "{want} round trip");
        }
    }

    #[test]
    fn refuses_with_reasons() {
        for (s, part) in [
            ("H7", "root"),
            ("C8", "`8`"),
            ("Cmaj6", "major-seventh"),
            ("C7#13", "alteration"),
            ("C7sus3", "sus2 or sus4"),
            ("C7omit7", "3rd or 5th"),
            ("C7xyz", "`xyz`"),
        ] {
            let e = ChordSymbol::parse(s).unwrap_err().to_string();
            assert!(e.contains(part), "{s}: {e}");
        }
    }
}
