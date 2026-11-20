//! Voicing vocabularies and their candidates (ticket 0025).

use std::collections::BTreeSet;

use mort_compose::chord::Role;
use mort_compose::harmony::Harmony;
use mort_compose::pitch::Pitch;
use mort_compose::voicing::{Range, Source, Vocabularies, Voicing};

fn h(text: &str) -> Harmony {
    Harmony::parse(text, None).unwrap_or_else(|e| panic!("{e}"))
}

fn range(text: &str) -> Range {
    Range::parse(text).unwrap()
}

fn names(v: &Voicing) -> Vec<String> {
    v.notes.iter().map(|n| n.pitch.to_string()).collect()
}

/// The pitch classes of each voicing's bottom note.
fn bottoms(vs: &[Voicing]) -> BTreeSet<u8> {
    vs.iter().map(|v| v.notes[0].pitch.get() % 12).collect()
}

#[test]
fn quartal_on_d_dorian() {
    let v = Vocabularies::builtin();
    let q = v.get("quartal").unwrap();
    let vs = q.candidates(&h("D dorian"), range("C3..C6"), 4).unwrap();
    // Perfect-fourth stacks inside D dorian start on D, E, A and B; the
    // ones on C, F and G need a B flat.
    assert_eq!(bottoms(&vs), BTreeSet::from([2, 4, 9, 11]));
    for c in &vs {
        let p = c.pitches();
        assert!(p.windows(2).all(|w| w[1] - w[0] == 5), "{c}");
        assert!(p[0] >= 48 && p[3] <= 84, "{c}");
    }
    let d = vs
        .iter()
        .find(|c| names(c) == ["D3", "G3", "C4", "F4"])
        .unwrap();
    let roles: Vec<Role> = d.notes.iter().map(|n| n.role).collect();
    assert_eq!(
        roles,
        [Role::Root, Role::Tension(11), Role::Seventh, Role::Third]
    );
}

#[test]
fn quartal_on_gm9_under_its_default_scale() {
    let v = Vocabularies::builtin();
    let harmony = h("Gm9");
    assert_eq!(harmony.scale.to_string(), "G dorian");
    let vs = v
        .get("quartal")
        .unwrap()
        .candidates(&harmony, range("C3..C6"), 4)
        .unwrap();
    assert_eq!(bottoms(&vs), BTreeSet::from([2, 4, 7, 9]));
}

#[test]
fn drop2_of_cmaj7_in_every_inversion() {
    let v = Vocabularies::builtin();
    let vs = v
        .get("drop-2")
        .unwrap()
        .candidates(&h("Cmaj7"), range("C3..C6"), 4)
        .unwrap();
    // One shape per inversion, each placed at every octave that fits.
    let shapes: BTreeSet<Vec<u8>> = vs
        .iter()
        .map(|c| {
            let p = c.pitches();
            p.iter().map(|x| x - p[0]).collect()
        })
        .collect();
    assert_eq!(shapes.len(), 4, "{shapes:?}");
    assert!(vs.iter().any(|c| names(c) == ["G3", "C4", "E4", "B4"]));
    // Close E4 G4 B4 C5 with B4 dropped.
    assert!(vs.iter().any(|c| names(c) == ["B3", "E4", "G4", "C5"]));
    for c in &vs {
        let p = c.pitches();
        // The dropped voice leaves a gap of more than an octave nowhere,
        // and the top three are within an octave.
        assert!(p[3] - p[1] < 12, "{c}");
    }
    // Five voices are not drop-2 here: Cmaj7 has four chord tones, and
    // the fifth doubles the root.
    let five = v
        .get("drop-2")
        .unwrap()
        .candidates(&h("Cmaj7"), range("C2..C6"), 5)
        .unwrap();
    assert!(five.iter().all(|c| c.notes.len() == 5));
}

#[test]
fn shells_put_the_root_in_the_bass() {
    let v = Vocabularies::builtin();
    let vs = v
        .get("shell")
        .unwrap()
        .candidates(&h("G7"), range("E2..C4"), 3)
        .unwrap();
    for c in &vs {
        assert_eq!(c.notes[0].role, Role::Root, "{c}");
        let upper: BTreeSet<Role> = c.notes[1..].iter().map(|n| n.role).collect();
        assert_eq!(upper, BTreeSet::from([Role::Third, Role::Seventh]), "{c}");
    }
    assert!(vs.iter().any(|c| names(c) == ["G2", "B2", "F3"]));
    assert!(vs.iter().any(|c| names(c) == ["G2", "F3", "B3"]));
    let e = v
        .get("shell")
        .unwrap()
        .candidates(&h("G7"), range("E2..C4"), 4)
        .unwrap_err();
    assert!(e.message.contains("voices 3 notes, not 4"), "{e}");
}

#[test]
fn so_what_and_upper_structures() {
    let v = Vocabularies::builtin();
    let sw = v
        .get("so what")
        .unwrap()
        .candidates(&h("E dorian"), range("C3..C6"), 5)
        .unwrap();
    assert!(
        sw.iter()
            .any(|c| names(c) == ["E3", "A3", "D4", "G4", "B4"])
    );
    // G7#11 on lydian dominant: an A major triad (9, #11, 13) over B and F.
    let us = v
        .get("upper structure")
        .unwrap()
        .candidates(&h("G7#11"), range("A2..C6"), 5)
        .unwrap();
    assert!(
        us.iter()
            .any(|c| names(c) == ["B3", "F4", "A4", "C#5", "E5"]),
        "{}",
        us.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
    for c in &us {
        let tensions = c.notes[2..]
            .iter()
            .filter(|n| matches!(n.role, Role::Tension(_)))
            .count();
        assert!(tensions >= 2, "{c}");
    }
}

#[test]
fn rootless_voicings_prefer_the_notes_the_symbol_names() {
    let v = Vocabularies::builtin();
    let r = v.get("rootless").unwrap();
    let pcs = |text: &str| -> BTreeSet<u8> {
        let vs = r.candidates(&h(text), range("C3..C5"), 4).unwrap();
        vs[0].notes.iter().map(|n| n.pitch.get() % 12).collect()
    };
    // Dm9: F A C E. G13: B E F A. Dm7 takes the 9th from the scale.
    assert_eq!(pcs("Dm9"), BTreeSet::from([5, 9, 0, 4]));
    assert_eq!(pcs("G13"), BTreeSet::from([11, 4, 5, 9]));
    assert_eq!(pcs("Dm7"), BTreeSet::from([5, 9, 0, 4]));
}

#[test]
fn every_builtin_builds_something() {
    let v = Vocabularies::builtin();
    let cases = [
        ("close", "Cmaj7", 4),
        ("cluster", "D dorian", 3),
        ("drop-2", "Dm7", 4),
        ("drop-3", "G7", 4),
        ("quartal", "D dorian", 3),
        ("rootless", "Dm9", 4),
        ("shell", "Fmaj7", 3),
        ("so what", "D dorian", 5),
        ("spread", "Cmaj9", 5),
        ("upper structure", "G7#11", 5),
        ("root fifth", "A7alt", 2),
        ("guide tones", "G13", 3),
        ("guide tones", "Csus2", 3),
    ];
    for (name, harmony, voices) in cases {
        let vs = v
            .get(name)
            .unwrap()
            .candidates(&h(harmony), range("C2..C6"), voices)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(vs.iter().all(|c| c.notes.len() == voices), "{name}");
        // Nothing avoided, nothing outside the harmony.
        let harm = h(harmony);
        for c in &vs {
            for n in &c.notes {
                assert!(harm.role_of(n.pitch.class()).is_some(), "{name}: {c}");
            }
        }
    }
}

#[test]
fn a_user_vocabulary_replaces_a_builtin() {
    let mut v = Vocabularies::builtin();
    let dir = std::env::temp_dir().join(format!("mort-voicings-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("mine.kdl"),
        "// Quartal with tritones allowed.\nvoicing \"quartal\" {\n    description \"fourths, tritones allowed\"\n    step 5 6\n}\n",
    )
    .unwrap();
    assert_eq!(v.load_dir(&dir).unwrap(), 1);
    let q = v.get("quartal").unwrap();
    assert_eq!(q.source, Source::File(dir.join("mine.kdl")));
    let vs = q.candidates(&h("D dorian"), range("C3..C6"), 3).unwrap();
    assert!(
        vs.iter().any(|c| names(c) == ["F3", "B3", "E4"]),
        "a tritone F-B"
    );
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        v.load_dir(&dir).unwrap(),
        0,
        "a missing directory loads nothing"
    );
}

#[test]
fn refuses_when_nothing_can_be_built() {
    let v = Vocabularies::builtin();
    // Whole tone holds no perfect fourths.
    let e = v
        .get("quartal")
        .unwrap()
        .candidates(&h("C whole tone"), range("C3..C6"), 4)
        .unwrap_err();
    assert!(
        e.message
            .contains("`quartal` builds no 4-voice voicing of C whole tone within C3..C6"),
        "{e}"
    );
    let e = v
        .get("so what")
        .unwrap()
        .candidates(&h("D dorian"), range("C3..C6"), 4)
        .unwrap_err();
    assert!(e.message.contains("voices 5 notes, not 4"), "{e}");
    // A range too narrow for any stack.
    let e = v
        .get("drop-2")
        .unwrap()
        .candidates(&h("Cmaj7"), range("C4..E4"), 4)
        .unwrap_err();
    assert!(e.message.contains("wider range"), "{e}");
}

#[test]
fn file_errors_say_where() {
    let mut v = Vocabularies::default();
    let src = Source::File("bad.kdl".into());
    let e = v
        .load_str("voicing \"x\" {\n    chord \"root\" \"8th\"\n}\n", &src)
        .unwrap_err();
    assert_eq!((e.line, e.column), (2, 18), "{e}");
    assert!(e.to_string().starts_with("bad.kdl line 2"), "{e}");
    assert!(e.message.contains("`8th` is not a role"), "{e}");
    let e = v
        .load_str("voicing \"x\" {\n    step 5\n    steps 5 4\n}\n", &src)
        .unwrap_err();
    assert!(e.message.contains("exactly one of"), "{e}");
    let e = v
        .load_str("voicing \"x\" {\n    stack 5\n}\n", &src)
        .unwrap_err();
    assert!(
        e.message.contains("`stack` is not part of a voicing"),
        "{e}"
    );
    let e = v.load_str("chord \"root\"\n", &src).unwrap_err();
    assert!(e.message.contains("holds `voicing` nodes"), "{e}");
    let _ = Pitch::parse("C4");
}
