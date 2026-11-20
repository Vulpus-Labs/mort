//! Reading song sheets (ticket 0024).

use mort_compose::chord::Role;
use mort_compose::sheet::{self, TICKS_PER_QUARTER};

const BAR: u32 = 4 * TICKS_PER_QUARTER;

#[test]
fn a_modal_section_and_a_section_of_chords() {
    let sheet = sheet::parse(
        r#"
        // A modal verse, then a chorus of chords.
        section "verse" bars=16 {
            harmony "D dorian" bars=8
            harmony "Eb lydian" bars=8
        }
        section "chorus" bars=7 {
            harmony "Gm9" "C13" "Fmaj9" bars=2
            harmony "Bbmaj7#11" beats=2
            harmony "A7alt" beats=2
        }
        section "intro" bars=4 meter="6/8"
        "#,
    )
    .unwrap_or_else(|e| panic!("{e}"));

    let names: Vec<&str> = sheet.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["verse", "chorus", "intro"]);

    let verse = sheet.section("verse").unwrap();
    assert_eq!(verse.length(), 16 * BAR);
    let first = verse.harmony_at(0).unwrap();
    assert!(first.harmony.chord.is_none());
    assert_eq!(first.harmony.scale.to_string(), "D dorian");
    assert_eq!(verse.harmony_at(8 * BAR).unwrap().text, "Eb lydian");

    let chorus = sheet.section("chorus").unwrap();
    let texts: Vec<&str> = chorus.spans.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(texts, ["Gm9", "C13", "Fmaj9", "Bbmaj7#11", "A7alt"]);
    let starts: Vec<u32> = chorus.spans.iter().map(|s| s.start).collect();
    assert_eq!(
        starts,
        [
            0,
            2 * BAR,
            4 * BAR,
            6 * BAR,
            6 * BAR + 2 * TICKS_PER_QUARTER
        ]
    );
    let bb = &chorus.spans[3].harmony;
    assert_eq!(bb.scale.to_string(), "Bb lydian");
    assert!(
        bb.tones
            .iter()
            .any(|t| t.role == Role::Tension(11) && t.pc.get() == 4)
    );
    assert_eq!(chorus.spans[4].harmony.scale.def.name, "altered");

    let intro = sheet.section("intro").unwrap();
    assert!(intro.spans.is_empty());
    assert_eq!(intro.length(), 4 * 3 * TICKS_PER_QUARTER);
}

#[test]
fn meters_and_scale_overrides() {
    let sheet = sheet::parse(
        r#"
        meter "3/4"
        section "waltz" bars=2 {
            harmony "Dm7" scale="aeolian" bars=1
            harmony "G7" beats=3
        }
        "#,
    )
    .unwrap();
    let s = &sheet.sections[0];
    assert_eq!(s.meter.to_string(), "3/4");
    assert_eq!(s.spans[0].harmony.scale.to_string(), "D aeolian");
    assert_eq!(s.spans[1].start, 3 * TICKS_PER_QUARTER);
}

fn error(src: &str) -> String {
    sheet::parse(src).unwrap_err().to_string()
}

#[test]
fn errors_say_where_and_what_to_do() {
    let e = error("section \"verse\" bars=4 {\n    harmony \"Dm7\" bars=2\n}\n");
    assert!(e.starts_with("sheet line 1, column 1:"), "{e}");
    assert!(
        e.contains("4 bars long but its harmony lasts 2 bars"),
        "{e}"
    );

    let e = error("section \"verse\" bars=4 {\n    harmony \"Dm8\" bars=4\n}\n");
    assert!(e.starts_with("sheet line 2, column 13:"), "{e}");
    assert!(e.contains("`8`"), "{e}");

    let e = error("section \"verse\" bars=4 {\n    harmony \"Dm7\" scale=\"bebop\" bars=4\n}\n");
    assert!(e.contains("`bebop` is not a scale"), "{e}");

    assert!(error("section \"v\" {\n}\n").contains("needs its length"));
    assert!(
        error("section \"v\" bars=2\nsection \"v\" bars=2\n").contains("two sections named `v`")
    );
    assert!(error("part \"Pad\"\n").contains("`part` is not a sheet node"));
    assert!(
        error("section \"v\" bars=1 {\n harmony \"C\" bars=1 beats=4\n}\n").contains("not both")
    );
    assert!(error("meter \"4/3\"\n").contains("not a time signature"));
    assert!(
        error("section \"v\" bars=1 {\n harmony \"C\" beats=0.33\n}\n")
            .contains("whole number of ticks")
    );
    assert!(error("section \"v\" bars=2 {\n").contains("not KDL"));
}
