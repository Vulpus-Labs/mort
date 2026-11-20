//! Articulation and performance (ticket 0027).

use mort_compose::articulate::{Event, Performance, play_part};
use mort_compose::lead::{Voice, voice_part};
use mort_compose::pattern::Patterns;
use mort_compose::sheet::{self, Sheet};
use mort_compose::voicing::{Source, Vocabularies};

fn sheet(src: &str) -> Sheet {
    sheet::parse(src).unwrap_or_else(|e| panic!("{e}"))
}

fn play_with(s: &Sheet, part: &str, patterns: &Patterns) -> Performance {
    play_part(s, s.part(part).unwrap(), &Vocabularies::builtin(), patterns)
        .unwrap_or_else(|e| panic!("{e}"))
}

fn play(s: &Sheet, part: &str) -> Performance {
    play_with(s, part, &Patterns::builtin())
}

/// Pitches struck at `tick` in `section`.
fn at(events: &[Event], section: &str, tick: u32) -> Vec<u8> {
    events
        .iter()
        .filter(|e| e.section == section && e.start == tick)
        .map(|e| e.pitch)
        .collect()
}

const BEAT: u32 = 960;
const BAR: u32 = 4 * BEAT;

const BOSSA: &str = r#"
section "a" bars=4 {
    harmony "Dm7" bars=2
    harmony "G7" bars=2
}
part "Keys" {
    voicing "drop-2" voices=4 range="C3..C5"
    articulation "bossa piano"
}
"#;

#[test]
fn bossa_anticipates_the_next_chord_on_the_and_of_four() {
    let s = sheet(BOSSA);
    let blocks = voice_part(&s, s.part("Keys").unwrap(), &Vocabularies::builtin()).unwrap();
    let upper = |b: usize| -> Vec<u8> {
        blocks[b]
            .notes
            .iter()
            .filter(|n| matches!(n.voice, Voice::Inner(_) | Voice::Top))
            .map(|n| n.pitch.get())
            .collect()
    };
    let p = play(&s, "Keys");
    // Bar 2, beat 4&: still under Dm7, but it plays G7's upper voices.
    let push = at(&p.events, "a", BAR + 3 * BEAT + BEAT / 2);
    assert_eq!(push, upper(1), "{push:?}");
    // Bar 1, beat 2&: Dm7's own upper voices.
    assert_eq!(at(&p.events, "a", BEAT + BEAT / 2), upper(0));
}

#[test]
fn a_two_chord_bar_with_an_anticipating_hit() {
    let mut patterns = Patterns::builtin();
    patterns
        .load_str(
            "pattern \"push\" {\n    hit \"top\" at=\"1 3\"\n    hit \"top\" at=\"2&\" anticipate=#true\n}\n",
            &Source::Builtin,
        )
        .unwrap();
    let s = sheet(
        r#"
        section "a" bars=1 {
            harmony "C" "F" beats=2
        }
        part "Keys" {
            voicing "close" voices=3 range="C4..C5"
            fix "a" beat=1 "C4 E4 G4"
            fix "a" beat=3 "C4 F4 A4"
            articulation "push"
        }
        "#,
    );
    let p = play_with(&s, "Keys", &patterns);
    // 2& is under C, but takes F's top, A4; it rings into beat 3 and is
    // struck again there.
    assert_eq!(at(&p.events, "a", 0), [67]);
    assert_eq!(at(&p.events, "a", BEAT + BEAT / 2), [69]);
    assert_eq!(at(&p.events, "a", 2 * BEAT), [69]);
}

#[test]
fn bass_in_octaves_doubles_only_the_bass_voice() {
    let s = sheet(BOSSA);
    let blocks = voice_part(&s, s.part("Keys").unwrap(), &Vocabularies::builtin()).unwrap();
    let bass = blocks[0]
        .notes
        .iter()
        .find(|n| n.voice == Voice::Bass)
        .unwrap()
        .pitch
        .get();
    let p = play(&s, "Keys");
    // Beat 1: the left hand strikes the bass and its octave below; the
    // right hand strikes the upper voices once each.
    let mut struck = at(&p.events, "a", 0);
    struck.sort_unstable();
    assert_eq!(struck[0], bass - 12);
    assert_eq!(struck[1], bass);
    assert_eq!(struck.len(), 2 + 3, "{struck:?}");
    assert!(
        !struck[2..].iter().any(|x| struck.contains(&(x - 12))),
        "{struck:?}"
    );
}

#[test]
fn a_hand_that_cannot_reach_is_reported_not_moved() {
    let mut patterns = Patterns::builtin();
    patterns
        .load_str(
            "pattern \"stretch\" {\n    hand \"left\" span=12\n    hit \"all\" at=\"1\" hand=\"left\"\n}\n",
            &Source::Builtin,
        )
        .unwrap();
    let s = sheet(
        r#"
        section "a" bars=1 {
            harmony "Cmaj7" bars=1
        }
        part "Keys" {
            voicing "spread" voices=4 range="C2..C5"
            articulation "stretch" range="A0..C8"
        }
        "#,
    );
    let p = play_with(&s, "Keys", &patterns);
    assert_eq!(p.problems.len(), 1, "{:?}", p.problems);
    let m = p.problems[0].to_string();
    assert!(
        m.starts_with("section `a`, bar 1 beat 1: the left hand would span"),
        "{m}"
    );
    assert!(m.contains("more than its 12"), "{m}");
    assert_eq!(
        at(&p.events, "a", 0).len(),
        4,
        "the notes are still written"
    );

    // The instrument's range is checked too.
    let s = sheet(
        r#"
        section "a" bars=1 {
            harmony "C" bars=1
        }
        part "Keys" {
            voicing "close" voices=3 range="C4..C5"
            articulation "sustain" range="C4..F4"
        }
        "#,
    );
    let p = play(&s, "Keys");
    assert!(
        p.problems
            .iter()
            .any(|x| x.message.contains("outside the instrument's range C4..F4")),
        "{:?}",
        p.problems
    );
}

#[test]
fn walking_bass_follows_another_parts_bass_voice() {
    let s = sheet(
        r#"
        section "a" bars=3 {
            harmony "Dm7" "G7" "Cmaj7" bars=1
        }
        part "Keys" {
            voicing "shell" voices=3 range="C2..C5"
        }
        part "Bass" {
            from "Keys" voice="bass"
            articulation "walking" range="E1..G3"
        }
        "#,
    );
    let p = play(&s, "Bass");
    assert_eq!(p.events.len(), 12, "a note a beat");
    let pcs: Vec<u8> = p.events.iter().map(|e| e.pitch % 12).collect();
    // Each bar starts on its chord's root.
    assert_eq!((pcs[0], pcs[4], pcs[8]), (2, 7, 0));
    // Beat 4 approaches the next root by a semitone.
    for bar in 0..2 {
        let approach = i32::from(p.events[bar * 4 + 3].pitch);
        let next = i32::from(p.events[bar * 4 + 4].pitch);
        assert_eq!((approach - next).abs(), 1, "{:?}", p.events);
    }
    assert!(p.events.iter().all(|e| (28..=55).contains(&e.pitch)));
}

#[test]
fn drums_use_the_kit_map() {
    let s = sheet(
        r#"
        section "intro" bars=2
        part "Drums" {
            articulation "rock drums" kit="gm"
        }
        "#,
    );
    let p = play(&s, "Drums");
    assert_eq!(at(&p.events, "intro", 0), [36, 42]);
    assert_eq!(at(&p.events, "intro", BEAT), [38, 42]);
    assert_eq!(p.events.iter().filter(|e| e.pitch == 42).count(), 16);
    let snare = p.events.iter().find(|e| e.pitch == 38).unwrap();
    assert_eq!(snare.velocity, 110, "accented");

    let mut patterns = Patterns::builtin();
    patterns
        .load_str(
            "pattern \"odd\" drums=#true {\n    hit \"cowbel\" at=\"1\"\n}\n",
            &Source::Builtin,
        )
        .unwrap();
    let s = sheet("section \"a\" bars=1\npart \"D\" {\n    articulation \"odd\"\n}\n");
    let e = play_part(
        &s,
        s.part("D").unwrap(),
        &Vocabularies::builtin(),
        &patterns,
    )
    .unwrap_err()
    .to_string();
    assert!(
        e.contains("strikes `cowbel`, which kit `gm` has no note for"),
        "{e}"
    );
}

#[test]
fn feel_is_seeded() {
    let src = |seed: u32| {
        format!(
            "section \"a\" bars=2\npart \"D\" {{\n    articulation \"rock drums\"\n    feel swing=0.66 timing=12 velocity=10 seed={seed}\n}}\n"
        )
    };
    let one = play(&sheet(&src(1)), "D");
    assert_eq!(
        one,
        play(&sheet(&src(1)), "D"),
        "the same seed plays the same"
    );
    assert_ne!(
        one,
        play(&sheet(&src(2)), "D"),
        "another seed plays differently"
    );
    // Swing moves the offbeat eighths late: 1& lands near two thirds of
    // the beat, give or take the timing.
    let offbeat = one
        .events
        .iter()
        .find(|e| e.pitch == 42 && e.start > 300 && e.start < BEAT)
        .unwrap();
    assert!((622..=646).contains(&offbeat.start), "{}", offbeat.start);
}

#[test]
fn pattern_errors_say_where() {
    let mut p = Patterns::default();
    let src = Source::File("mine.kdl".into());
    let e = p
        .load_str("pattern \"x\" {\n    hit \"bass\" at=\"5:1\"\n}\n", &src)
        .unwrap_err();
    assert!(e.to_string().starts_with("mine.kdl line 2"), "{e}");
    assert!(
        e.message.contains("not a position in a 1-bar pattern"),
        "{e}"
    );
    let e = p
        .load_str("pattern \"x\" {\n    hit \"lefthand\" at=\"1\"\n}\n", &src)
        .unwrap_err();
    assert!(
        e.message.contains("`lefthand` is not a voice or role"),
        "{e}"
    );
    let e = p
        .load_str("pattern \"x\" {\n    hit \"bass\"\n}\n", &src)
        .unwrap_err();
    assert!(e.message.contains("needs `at`"), "{e}");
    let e = p
        .load_str("pattern \"x\" drums=#true {\n    walk\n}\n", &src)
        .unwrap_err();
    assert!(e.message.contains("hit` lines only"), "{e}");
    let e = sheet::parse("part \"B\" {\n    from \"Keys\"\n}\n").unwrap_err();
    assert!(e.message.contains("`from` part `Keys`"), "{e}");
}
