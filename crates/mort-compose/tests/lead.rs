//! Voice-leading and blocks (ticket 0026).

use mort_compose::chord::Role;
use mort_compose::lead::{Block, Tone, Voice, voice_part};
use mort_compose::part::Nct;
use mort_compose::sheet::{self, Sheet};
use mort_compose::voicing::Vocabularies;

fn sheet(src: &str) -> Sheet {
    sheet::parse(src).unwrap_or_else(|e| panic!("{e}"))
}

fn blocks(src: &str, part: &str) -> Vec<Block> {
    let s = sheet(src);
    voice_part(&s, s.part(part).unwrap(), &Vocabularies::builtin())
        .unwrap_or_else(|e| panic!("{e}"))
}

/// The chord-tone pitches of each block, lowest first.
fn chords(bs: &[Block]) -> Vec<Vec<u8>> {
    bs.iter()
        .map(|b| {
            b.notes
                .iter()
                .filter(|n| n.tone == Tone::Chord)
                .map(|n| n.pitch.get())
                .collect()
        })
        .collect()
}

const TWO_FIVE_ONE: &str = r#"
section "a" bars=3 {
    harmony "Dm7" "G7" "Cmaj7" bars=1
}
part "Keys" {
    voicing "close" voices=4 range="C4..C5"
}
"#;

#[test]
fn a_two_five_one_keeps_common_tones() {
    let bs = blocks(TWO_FIVE_ONE, "Keys");
    let c = chords(&bs);
    let held = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x == y).count();
    // Dm7 and G7 share D and F; G7 and Cmaj7 share G and B.
    assert!(held(&c[0], &c[1]) >= 2, "{c:?}");
    assert!(held(&c[1], &c[2]) >= 2, "{c:?}");
    // Every voice moves by a step at most.
    for w in c.windows(2) {
        for (x, y) in w[0].iter().zip(&w[1]) {
            assert!(x.abs_diff(*y) <= 2, "{c:?}");
        }
    }
    // Voices and roles are labelled.
    let first = &bs[0].notes;
    assert_eq!(first[0].voice, Voice::Bass);
    assert_eq!(first[1].voice, Voice::Inner(1));
    assert_eq!(first[3].voice, Voice::Top);
    assert!(first.iter().all(|n| n.role.is_some()));
}

#[test]
fn the_same_sheet_voices_the_same_way() {
    assert_eq!(blocks(TWO_FIVE_ONE, "Keys"), blocks(TWO_FIVE_ONE, "Keys"));
}

#[test]
fn a_top_line_is_honoured() {
    let bs = blocks(
        r#"
        section "a" bars=4 {
            harmony "Dm7" "G7" "Cmaj7" "A7" bars=1
        }
        part "Keys" {
            // The only drop-2 Dm7 with A4 on top is F3 C4 D4 A4.
            voicing "drop-2" voices=4 range="E3..D5"
            top "a" "A4" "-" "B4" "C#5"
        }
        "#,
        "Keys",
    );
    let tops: Vec<String> = bs
        .iter()
        .map(|b| {
            b.notes
                .iter()
                .find(|n| n.voice == Voice::Top)
                .unwrap()
                .pitch
                .to_string()
        })
        .collect();
    assert_eq!(tops[0], "A4");
    assert_eq!(tops[2], "B4");
    assert_eq!(tops[3], "C#5");
}

#[test]
fn constraints_that_cannot_be_met_say_where_and_what_to_do() {
    let s = sheet(
        r#"
        section "a" bars=2 {
            harmony "Dm7" "G7" bars=1
        }
        part "Keys" {
            voicing "drop-2" voices=4 range="C4..E4"
        }
        part "Pad" {
            voicing "close" voices=4 range="C4..C5"
            top "a" "-" "C#5"
        }
        part "None" {
            in "a"
        }
        "#,
    );
    let v = Vocabularies::builtin();
    let e = voice_part(&s, s.part("Keys").unwrap(), &v)
        .unwrap_err()
        .to_string();
    assert!(
        e.starts_with("part `Keys`, section `a`, bar 1 beat 1:"),
        "{e}"
    );
    assert!(e.contains("wider range"), "{e}");
    let e = voice_part(&s, s.part("Pad").unwrap(), &v)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("bar 2 beat 1") && e.contains("has C#5 on top"),
        "{e}"
    );
    let e = voice_part(&s, s.part("None").unwrap(), &v)
        .unwrap_err()
        .to_string();
    assert!(e.contains("has no voicing here"), "{e}");
}

#[test]
fn passing_tones_fill_thirds_when_switched_on() {
    let src = r#"
        section "a" bars=2 {
            harmony "C" "Em" bars=1
        }
        part "Keys" {
            voicing "close" voices=3 range="C4..C5"
            fix "a" bar=1 "C4 E4 G4"
            fix "a" bar=2 "E4 G4 B4"
            nct "passing"
        }
        part "Plain" {
            voicing "close" voices=3 range="C4..C5"
            fix "a" bar=1 "C4 E4 G4"
            fix "a" bar=2 "E4 G4 B4"
        }
    "#;
    let bs = blocks(src, "Keys");
    let passing: Vec<(String, Voice, u32)> = bs[0]
        .notes
        .iter()
        .filter(|n| n.tone == Tone::Nct(Nct::Passing))
        .map(|n| (n.pitch.to_string(), n.voice, n.start))
        .collect();
    // E→G fills with F, G→B with A, on the last beat; the bass is left.
    assert_eq!(
        passing,
        [
            ("F4".to_string(), Voice::Inner(1), 2880),
            ("A4".to_string(), Voice::Top, 2880)
        ]
    );
    let top = bs[0]
        .notes
        .iter()
        .find(|n| n.voice == Voice::Top && n.tone == Tone::Chord)
        .unwrap();
    assert_eq!(top.length, 2880, "the chord tone gives up its last beat");
    let a = bs[0]
        .notes
        .iter()
        .find(|n| n.pitch.to_string() == "A4")
        .unwrap();
    assert_eq!(a.role, Some(Role::Tension(13)));
    assert!(
        blocks(src, "Plain")
            .iter()
            .all(|b| b.notes.iter().all(|n| n.tone == Tone::Chord))
    );
}

#[test]
fn suspensions_anticipations_neighbours_and_pedals() {
    let bs = blocks(
        r#"
        // Em7 as iii in C: phrygian, so F is in its scale.
        section "a" bars=2 {
            harmony "Fmaj7" bars=1
            harmony "Em7" scale="phrygian" bars=1
        }
        part "Keys" {
            voicing "close" voices=3 range="C4..C5"
            fix "a" bar=1 "C4 F4 A4"
            fix "a" bar=2 "B3 E4 G4"
            nct "suspension" beats=2
        }
        "#,
        "Keys",
    );
    // F→E and A→G descend by step into Em7, where F and A are not chord
    // tones: both are held over the bar line for two beats.
    let sus: Vec<(String, u32, u32)> = bs[1]
        .notes
        .iter()
        .filter(|n| n.tone == Tone::Nct(Nct::Suspension))
        .map(|n| (n.pitch.to_string(), n.start, n.length))
        .collect();
    assert_eq!(
        sus,
        [
            ("F4".to_string(), 3840, 1920),
            ("A4".to_string(), 3840, 1920)
        ]
    );

    let bs = blocks(
        r#"
        section "a" bars=2 {
            harmony "C" "G" bars=1
        }
        part "Keys" {
            voicing "close" voices=3 range="C4..C5"
            fix "a" bar=1 "C4 E4 G4"
            fix "a" bar=2 "B3 D4 G4"
            nct "anticipation" "neighbour" beats=0.5
            pedal "C3"
        }
        "#,
        "Keys",
    );
    let tones: Vec<(String, String)> = bs[0]
        .notes
        .iter()
        .map(|n| (n.pitch.to_string(), n.tone.to_string()))
        .collect();
    assert!(
        tones.contains(&("D4".into(), "anticipation".into())),
        "{tones:?}"
    );
    // G is held in the top voice: an upper neighbour, A.
    assert!(
        tones.contains(&("A4".into(), "neighbour".into())),
        "{tones:?}"
    );
    // The pedal replaces the bass in both bars.
    for b in &bs {
        let bass: Vec<_> = b.notes.iter().filter(|n| n.voice == Voice::Bass).collect();
        assert!(
            bass.iter()
                .all(|n| n.pitch.to_string() == "C3" && n.tone == Tone::Nct(Nct::Pedal))
        );
    }
}

#[test]
fn suspensions_hold_only_notes_of_the_next_scale() {
    let bs = blocks(
        r#"
        section "a" bars=2 {
            harmony "D dorian" "Eb lydian" bars=1
        }
        part "Keys" {
            voicing "close" voices=3 range="C4..C5"
            fix "a" bar=1 "C4 E4 A4"
            fix "a" bar=2 "Bb3 D4 G4"
            nct "suspension"
        }
        "#,
        "Keys",
    );
    let held: Vec<String> = bs[1]
        .notes
        .iter()
        .filter(|n| n.tone == Tone::Nct(Nct::Suspension))
        .map(|n| n.pitch.to_string())
        .collect();
    // E is not in E flat lydian, so it moves to D on the beat; A (the
    // #11) is held over and falls to G.
    assert_eq!(held, ["A4"]);
}

#[test]
fn sections_voice_in_form_order_with_overrides() {
    let bs = blocks(
        r#"
        section "verse" bars=2 {
            harmony "D dorian" bars=2
        }
        section "chorus" bars=2 {
            harmony "Gm9" "C13" bars=1
        }
        section "intro" bars=1
        part "Pad" {
            voicing "quartal" voices=4 range="C3..A4"
            section "chorus" {
                voicing "drop-2" voices=4 range="G3..D5"
            }
        }
        "#,
        "Pad",
    );
    let where_: Vec<(&str, &str)> = bs
        .iter()
        .map(|b| (b.section.as_str(), b.text.as_str()))
        .collect();
    assert_eq!(
        where_,
        [("verse", "D dorian"), ("chorus", "Gm9"), ("chorus", "C13")]
    );
    let p = bs[0]
        .notes
        .iter()
        .map(|n| n.pitch.get())
        .collect::<Vec<_>>();
    assert!(
        p.windows(2).all(|w| w[1] - w[0] == 5),
        "quartal in the verse: {p:?}"
    );
}

#[test]
fn part_errors_say_where() {
    let e = |src: &str| sheet::parse(src).unwrap_err().to_string();
    assert!(
        e("part \"P\" {\n  voicing \"close\" voices=40\n}\n")
            .contains("`voices` is a whole number from 1 to 16")
    );
    assert!(e("part \"P\" {\n  voicing \"close\" range=\"C5..C3\"\n}\n").contains("not a range"));
    assert!(e("part \"P\" {\n  weights loud=1\n}\n").contains("`loud` is not a weight"));
    assert!(e("part \"P\" {\n  nct \"trill\"\n}\n").contains("passing, neighbour"));
    assert!(e("part \"P\" {\n  in \"bridge\"\n}\n").contains("names section `bridge`"));
    assert!(e("part \"P\"\npart \"P\"\n").contains("two parts named `P`"));
    assert!(e("part \"P\" {\n  top \"a\" \"Q5\"\n}\n").contains("`Q5` is not a pitch"));
    let s = sheet(
        "section \"a\" bars=1 {\n harmony \"C\" bars=1\n}\npart \"P\" {\n voicing \"close\" voices=3 range=\"C4..C5\"\n fix \"a\" beat=2 \"C4 E4 G4\"\n}\n",
    );
    let e = voice_part(&s, s.part("P").unwrap(), &Vocabularies::builtin())
        .unwrap_err()
        .to_string();
    assert!(e.contains("must fall where a harmony starts"), "{e}");
}
