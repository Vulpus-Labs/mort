//! Findings over the fake (ticket 0013): each kind planted in a synthetic
//! mix and found, and a clean mix that finds nothing. `calibrate` (run
//! with `--ignored --nocapture`) prints every measurement, flagged or not,
//! which is where the thresholds in `mort_map::thresholds` come from.

mod common;

use std::collections::BTreeMap;

use serde_json::{Value, json};

use common::Bench;
use mort_proto::TrackId;
use mort_proto::model::Note;
use mort_session::{FakeSession, NewTrack, Session, Source};

fn pink(lo: f64, hi: f64, db: f64) -> Source {
    Source::Pink { lo, hi, db }
}

struct Mix {
    s: FakeSession,
    ids: BTreeMap<String, TrackId>,
}

impl Mix {
    fn new() -> Self {
        Self {
            s: FakeSession::new(),
            ids: BTreeMap::new(),
        }
    }

    fn add(&mut self, name: &str, parent: Option<&str>, sources: &[Source]) -> &mut Self {
        let parent = parent.map(|p| self.ids[p]);
        let id = self
            .s
            .add_track(NewTrack {
                name: name.into(),
                parent,
                ..NewTrack::default()
            })
            .unwrap();
        for src in sources {
            self.s.add_sound(id, src.clone(), None).unwrap();
        }
        self.ids.insert(name.into(), id);
        self
    }

    fn during(&mut self, name: &str, src: Source, from: f64, to: f64) -> &mut Self {
        let id = self.ids[name];
        self.s.add_sound(id, src, Some((from, to))).unwrap();
        self
    }

    fn notes(&mut self, name: &str, notes: &[(u8, f64, f64)]) -> &mut Self {
        let id = self.ids[name];
        let end = notes.iter().map(|n| n.2).fold(0.0, f64::max);
        let item = self.s.add_midi_item(id, 0.0, end).unwrap();
        let ns: Vec<Note> = notes
            .iter()
            .map(|&(pitch, start, end)| Note {
                pitch,
                start,
                end,
                velocity: 100,
                channel: 0,
            })
            .collect();
        self.s.insert_midi(item, &ns, &[]).unwrap();
        self.s
            .add_sound(id, Source::Voice { db: -24.0 }, None)
            .unwrap();
        self
    }

    /// Measures `0..end` and returns the findings.
    fn findings(self, end: f64, all: bool) -> Vec<Value> {
        let mut b = Bench::new(self.s);
        b.measure(json!({ "start": 0.0, "end": end }));
        b.ok("map.findings", json!({ "all": all }))["findings"]
            .as_array()
            .unwrap()
            .clone()
    }
}

/// A clean arrangement: five parts in neighbouring ranges, each two
/// octaves or so wide at the same level, so the whole is close to pink.
fn base() -> Mix {
    let mut m = Mix::new();
    m.add("Bass", None, &[pink(40.0, 150.0, -26.0)])
        .add("Pad", None, &[pink(150.0, 600.0, -26.0)])
        .add("Keys", None, &[pink(600.0, 2500.0, -26.0)])
        .add("Bright", None, &[pink(2500.0, 8000.0, -26.0)])
        .add("Air", None, &[pink(8000.0, 16_000.0, -32.0)]);
    m
}

fn kinds(fs: &[Value]) -> Vec<String> {
    let mut k: Vec<String> = fs
        .iter()
        .map(|f| f["kind"].as_str().unwrap().to_owned())
        .collect();
    k.dedup();
    k
}

fn of<'a>(fs: &'a [Value], kind: &str) -> Vec<&'a Value> {
    fs.iter().filter(|f| f["kind"] == kind).collect()
}

#[test]
fn a_clean_mix_finds_nothing() {
    let fs = base().findings(4.0, false);
    assert!(
        fs.is_empty(),
        "{}",
        serde_json::to_string_pretty(&fs).unwrap()
    );
}

#[test]
fn masking() {
    let mut m = base();
    m.add("Pad 2", None, &[pink(150.0, 600.0, -27.0)]);
    let ids = m.ids.clone();
    let fs = m.findings(4.0, false);
    let mask = of(&fs, "masking");
    assert_eq!(mask.len(), 1, "{fs:#?}");
    assert_eq!(mask[0]["tracks"], json!([ids["Pad"], ids["Pad 2"]]));
    let bands: Vec<f64> = serde_json::from_value(mask[0]["bands"].clone()).unwrap();
    assert!(
        bands.contains(&250.0) && bands.contains(&400.0),
        "{bands:?}"
    );
    assert!(
        mask[0]["suggest"]
            .as_str()
            .unwrap()
            .contains("Pad and Pad 2")
    );
}

#[test]
fn mud_and_harsh() {
    let mut m = base();
    m.add("Warm", None, &[pink(200.0, 500.0, -26.0)]);
    let ids = m.ids.clone();
    let fs = m.findings(4.0, false);
    let mud = of(&fs, "mud");
    assert_eq!(mud.len(), 1, "{fs:#?}");
    assert_eq!(
        mud[0]["evidence"]["bus"],
        json!(
            ids.values()
                .next()
                .map(|_| mud[0]["tracks"][0].clone())
                .unwrap()
        )
    );
    assert!(
        mud[0]["tracks"]
            .as_array()
            .unwrap()
            .contains(&json!(ids["Warm"])),
        "{:#}",
        mud[0]
    );
    assert!(of(&fs, "harsh").is_empty(), "{fs:#?}");

    let mut m = base();
    m.add("Edge", None, &[pink(2000.0, 5000.0, -26.0)]);
    let fs = m.findings(4.0, false);
    assert_eq!(of(&fs, "harsh").len(), 1, "{fs:#?}");
    assert!(of(&fs, "mud").is_empty(), "{fs:#?}");
}

#[test]
fn headroom_on_the_master_and_a_bus() {
    let mut m = base();
    m.add(
        "Lead",
        None,
        &[Source::Sine {
            hz: 1000.0,
            db: -0.5,
        }],
    );
    let fs = m.findings(2.0, false);
    let h = of(&fs, "headroom");
    assert_eq!(h.len(), 1, "{fs:#?}");
    assert!(h[0]["evidence"]["peak"].as_f64().unwrap() > -1.0);

    let mut m = Mix::new();
    m.add("Drums", None, &[])
        .add(
            "Kick",
            Some("Drums"),
            &[Source::Sine { hz: 60.0, db: -4.0 }],
        )
        .add(
            "Kick 2",
            Some("Drums"),
            &[Source::Sine { hz: 60.0, db: -4.0 }],
        );
    let drums = m.ids["Drums"];
    let fs = m.findings(2.0, false);
    let h = of(&fs, "headroom");
    assert!(
        h.iter().any(
            |f| f["tracks"] == json!([drums]) && f["evidence"]["measure"] == "sample_peak_dbfs"
        ),
        "{fs:#?}"
    );
}

/// The base mix quiet in a verse, then a chorus that adds `lift`.
fn verse_chorus(lift: f64) -> Mix {
    let mut m = base();
    m.s.add_region("verse", 0.0, Some(6.0));
    m.s.add_region("chorus", 6.0, Some(12.0));
    m.add("Lift", None, &[])
        .during("Lift", pink(20.0, 20_000.0, lift), 6.0, 12.0);
    m
}

#[test]
fn loudness_jump() {
    let fs = verse_chorus(-10.0).findings(12.0, false);
    let j = of(&fs, "loudness_jump");
    assert_eq!(j.len(), 1, "{fs:#?}");
    assert_eq!(j[0]["section"], "chorus");
    assert!(j[0]["evidence"]["jump_lu"].as_f64().unwrap() > 6.0);

    // A build of a few LU is music, not a problem.
    let fs = verse_chorus(-24.0).findings(12.0, false);
    assert!(of(&fs, "loudness_jump").is_empty(), "{fs:#?}");
}

/// The base mix with its bass made wide.
fn wide_bass() -> Mix {
    let mut m = base();
    let bass = m.ids["Bass"];
    m.s.clear_sounds(bass).unwrap();
    m.s.add_sound(
        bass,
        Source::Wide {
            lo: 40.0,
            hi: 150.0,
            db: -26.0,
        },
        None,
    )
    .unwrap();
    m
}

#[test]
fn low_end_width() {
    let m = wide_bass();
    let (bass, master) = (m.ids["Bass"], m.s.master());
    let fs = m.findings(4.0, false);
    let w = of(&fs, "low_end_width");
    let mut who: Vec<&Value> = w.iter().map(|f| &f["tracks"][0]).collect();
    who.sort_by_key(|v| v.as_str().unwrap().to_owned());
    assert_eq!(who, [&json!(master), &json!(bass)], "{fs:#?}");
    assert_eq!(kinds(&fs), ["low_end_width"], "{fs:#?}");
}

#[test]
fn register_clash() {
    let mut m = Mix::new();
    m.add("Bass line", None, &[]).add("Left hand", None, &[]);
    m.notes("Bass line", &[(40, 0.0, 2.0)])
        .notes("Left hand", &[(41, 0.5, 2.5)]);
    let fs = m.findings(3.0, false);
    let c = of(&fs, "register_clash");
    assert_eq!(c.len(), 1, "{fs:#?}");
    assert_eq!(c[0]["evidence"]["first"]["notes"], json!(["E2", "F2"]));
    assert!((c[0]["evidence"]["seconds"].as_f64().unwrap() - 1.5).abs() < 0.01);

    let mut m = Mix::new();
    m.add("Bass line", None, &[]).add("Left hand", None, &[]);
    m.notes("Bass line", &[(40, 0.0, 2.0)])
        .notes("Left hand", &[(43, 0.5, 2.5)]);
    assert!(of(&m.findings(3.0, false), "register_clash").is_empty());
}

/// Prints every measurement behind every kind, flagged or not, for the
/// clean mix and each planted problem.
#[test]
#[ignore = "calibration: prints measurements"]
fn calibrate() {
    let show = |label: &str, m: Mix, end: f64| {
        let fs = m.findings(end, true);
        println!("== {label}");
        for f in &fs {
            let e = &f["evidence"];
            let k = f["kind"].as_str().unwrap();
            let line = match k {
                "masking" => {
                    let bands = e["bands"].as_array().unwrap();
                    let over: Vec<String> = bands
                        .iter()
                        .filter(|b| b["fraction"].as_f64().unwrap() > 0.0)
                        .map(|b| format!("{}:{}", b["hz"], b["fraction"]))
                        .collect();
                    format!("{} bands with any hits [{}]", over.len(), over.join(" "))
                }
                "mud" | "harsh" => format!(
                    "excess {} dB, tilt {}",
                    e["excess_db"], e["tilt_db_per_octave"]
                ),
                "headroom" => format!("{} {}", e["measure"], e["peak"]),
                "loudness_jump" => format!("{} LU", e["jump_lu"]),
                "low_end_width" => {
                    format!("corr {} fraction {}", e["correlation_low"], e["fraction"])
                }
                _ => e.to_string(),
            };
            println!("  {k:14} {:24} {line}", f["section"].as_str().unwrap());
        }
    };
    show("clean", base(), 4.0);
    let mut m = base();
    m.add("Pad 2", None, &[pink(150.0, 600.0, -27.0)]);
    show("masking: Pad 2 1 dB under Pad", m, 4.0);
    let mut m = base();
    m.add("Pad 2", None, &[pink(150.0, 600.0, -41.0)]);
    show("masking: Pad 2 15 dB under Pad", m, 4.0);
    let mut m = base();
    m.add("Warm", None, &[pink(200.0, 500.0, -26.0)]);
    show("mud: Warm 200–500 at -26", m, 4.0);
    let mut m = base();
    m.add("Warm", None, &[pink(200.0, 500.0, -32.0)]);
    show("mud: Warm 200–500 at -32", m, 4.0);
    let mut m = base();
    m.add("Edge", None, &[pink(2000.0, 5000.0, -26.0)]);
    show("harsh: Edge 2–5k at -26", m, 4.0);
    let mut m = base();
    m.add("Edge", None, &[pink(2000.0, 5000.0, -32.0)]);
    show("harsh: Edge 2–5k at -32", m, 4.0);
    show("jump: chorus adds pink at -10", verse_chorus(-10.0), 12.0);
    show("jump: chorus adds pink at -14", verse_chorus(-14.0), 12.0);
    show("jump: chorus adds pink at -24", verse_chorus(-24.0), 12.0);
    show("jump: none", verse_chorus(-150.0), 12.0);
    show("width: wide bass", wide_bass(), 4.0);
    let mut m = Mix::new();
    m.add("Voice", None, &[]).notes(
        "Voice",
        &(0..8)
            .map(|i| (48, f64::from(i) * 0.5, f64::from(i) * 0.5 + 0.4))
            .collect::<Vec<_>>(),
    );
    let mut b = Bench::new(m.s);
    b.measure(json!({ "start": 0.0, "end": 4.0 }));
    let t = b.ok("map.summary", json!({}));
    println!(
        "== density: quarter notes at 120 bpm: master {}",
        t["sections"][0]["master"]["density"]
    );
    let mut b = Bench::new(base().s);
    b.measure(json!({ "start": 0.0, "end": 4.0 }));
    let t = b.ok("map.summary", json!({}));
    println!(
        "== density: steady noise: master {}",
        t["sections"][0]["master"]["density"]
    );
}

#[test]
#[expect(clippy::many_single_char_names, reason = "short test bindings")]
fn a_project_can_override_thresholds() {
    let mut m = base();
    m.add("Warm", None, &[pink(200.0, 500.0, -26.0)]);
    let mut b = Bench::new(m.s);
    b.measure(json!({ "start": 0.0, "end": 4.0 }));
    let mud = |b: &mut Bench| -> Vec<Value> {
        b.ok("map.findings", json!({ "kinds": ["mud"] }))["findings"]
            .as_array()
            .unwrap()
            .clone()
    };
    assert_eq!(mud(&mut b).len(), 1);

    // Raised past the measured 2.9 dB, the finding goes; a little less and
    // it stays, saying the threshold is the project's.
    let r = b.ok(
        "thresholds.set",
        json!({ "name": "mud_excess", "value": 4.0 }),
    );
    assert_eq!(
        r["file"],
        Value::Null,
        "an unsaved project keeps them in memory"
    );
    assert!(mud(&mut b).is_empty());
    b.ok(
        "thresholds.set",
        json!({ "name": "mud_excess", "value": 2.5 }),
    );
    let f = mud(&mut b);
    assert_eq!(f[0]["overridden"], json!(["mud_excess"]));
    b.ok(
        "thresholds.set",
        json!({ "name": "mud_excess", "reset": true }),
    );
    let f = mud(&mut b);
    assert_eq!(f.len(), 1);
    assert!(f[0].get("overridden").is_none());

    // The lowest severity shown hides weaker findings everywhere.
    b.ok(
        "thresholds.set",
        json!({ "name": "min_severity", "value": 0.9 }),
    );
    assert!(mud(&mut b).is_empty());
    let s = b.ok("map.summary", json!({}));
    assert!(s["sections"][0]["findings"].get("mud").is_none(), "{s}");

    // Mistakes name the key and what it allows.
    let e = b
        .call("thresholds.set", json!({ "name": "mud", "value": 1.0 }))
        .unwrap_err();
    assert!(e.message.contains("no threshold `mud`"), "{e:?}");
    let e = b
        .call(
            "thresholds.set",
            json!({ "name": "low_corr", "value": 3.0 }),
        )
        .unwrap_err();
    assert!(e.message.contains("from -1 to 1"), "{e:?}");

    // Saved with the project, read back by a bridge that opens it.
    let dir = std::env::temp_dir().join(format!("mort-thresholds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    b.s.set_path(Some(dir.join("song.rpp").to_str().unwrap()));
    let r = b.ok("thresholds.get", json!({}));
    let file = dir.join("song.mort/thresholds.toml");
    assert_eq!(r["file"], json!(file));
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("min_severity = 0.9")
    );
    let mut again = Bench::new(b.s.clone());
    let r = again.ok("thresholds.get", json!({}));
    assert!((r["min_severity"].as_f64().unwrap() - 0.9).abs() < 1e-9);

    // A broken file is reported, and the defaults used.
    std::fs::write(&file, "mud_excess = \"high\"\n").unwrap();
    let mut broken = Bench::new(b.s.clone());
    let r = broken.ok("thresholds.get", json!({}));
    assert!(
        r["error"]
            .as_str()
            .unwrap()
            .contains("`mud_excess` must be a number"),
        "{r}"
    );
    assert!(r["min_severity"].as_f64().unwrap().abs() < 1e-9);
    std::fs::remove_dir_all(&dir).unwrap();
}
