//! `measure` and `map.status` over the fake, which renders through the
//! real feature code (ticket 0012, ADR 0006, ADR 0011 §2).

mod common;

use serde_json::{Value, json};

use common::Bench;
use mort_proto::{ErrorKind, SteppedTempoMap, TrackId};
use mort_session::{FakeSession, NewTrack, Session, Source};

fn names(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect()
}

struct Band {
    s: FakeSession,
    kick: TrackId,
    bass: TrackId,
}

/// Drums (a folder holding Kick), Bass, and Verb, which Bass sends to.
fn band() -> Band {
    let mut s = FakeSession::new();
    let add = |s: &mut FakeSession, name: &str, parent| {
        s.add_track(NewTrack {
            name: name.into(),
            parent,
            ..NewTrack::default()
        })
        .unwrap()
    };
    let drums = add(&mut s, "Drums", None);
    let kick = add(&mut s, "Kick", Some(drums));
    let bass = add(&mut s, "Bass", None);
    let verb = add(&mut s, "Verb", None);
    s.add_send(bass, verb).unwrap();
    s.add_sound(kick, Source::Sine { hz: 55.0, db: -6.0 }, None)
        .unwrap();
    s.add_sound(
        bass,
        Source::Band {
            lo: 80.0,
            hi: 300.0,
            db: -18.0,
        },
        None,
    )
    .unwrap();
    Band { s, kick, bass }
}

#[test]
fn measure_edit_and_remeasure_what_is_stale() {
    let Band { s, kick, bass } = band();
    let mut b = Bench::new(s);

    let r = b.measure(json!({ "start": 0.0, "end": 4.0 }));
    assert_eq!(
        names(&r["measured"]),
        ["MASTER", "Drums", "Kick", "Bass", "Verb"]
    );
    assert!(
        r["measured"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["frames"] == 80),
        "4 s of frames, the render's tail clipped: {r}"
    );
    assert_eq!(
        r["saved"],
        Value::Null,
        "an unsaved project keeps the map in memory"
    );
    assert!(b.stale().is_empty());

    // Renaming is not a change to the sound.
    b.ok("track.set", json!({ "track": bass, "name": "Bass 2" }));
    assert!(b.stale().is_empty());

    // Turning the kick down makes it and its buses stale, and nothing else.
    b.ok("track.set", json!({ "track": kick, "volume": 0.5 }));
    assert_eq!(b.stale(), ["MASTER", "Drums", "Kick"]);
    let m = b.ok("map.status", json!({}));
    let drums = &m["tracks"][1];
    assert_eq!(drums["name"], "Drums");
    assert_eq!(drums["spans"][0]["stale"]["why"], "changed");
    assert_eq!(drums["spans"][0]["stale"]["track"], json!(kick));

    let r = b.measure(json!({ "stale": true }));
    assert_eq!(names(&r["measured"]), ["MASTER", "Drums", "Kick"]);
    assert!(b.stale().is_empty());
    assert_eq!(b.measure(json!({ "stale": true }))["nothing_stale"], true);

    // Two captures now cover the map; a full re-measure replaces both.
    assert_eq!(
        b.ok("map.status", json!({}))["captures"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let r = b.measure(json!({ "start": 0.0, "end": 4.0 }));
    assert_eq!(r["superseded"].as_array().unwrap().len(), 2);

    // A tempo change makes everything stale.
    b.s.set_tempo_map(SteppedTempoMap::constant(100.0, 4, 4));
    assert_eq!(b.stale().len(), 5);
}

#[test]
fn measuring_some_tracks_takes_their_buses_and_play_works() {
    let Band { s, bass, .. } = band();
    let mut b = Bench::new(s);
    let r = b.measure(json!({ "start": 1.0, "end": 3.0, "tracks": [bass], "mode": "play" }));
    assert_eq!(names(&r["measured"]), ["MASTER", "Bass", "Verb"]);
    assert_eq!(r["mode"], "play");
    assert_eq!(r["measured"][1]["frames"], 40);
    let m = b.ok("map.status", json!({}));
    assert_eq!(names(&m["unmeasured"]), ["Drums", "Kick"]);

    // Measuring is an edit, as it may add probes.
    b.d.set_held(true);
    let e = b
        .call("measure", json!({ "start": 0.0, "end": 1.0 }))
        .unwrap_err();
    assert_eq!(e.kind(), Some(ErrorKind::Busy));
    b.d.set_held(false);

    let e = b.call("measure", json!({ "start": 1.0 })).unwrap_err();
    assert!(e.message.contains("both a start and an end"), "{e:?}");
    let e = b.call("measure", json!({})).unwrap_err();
    assert!(e.message.contains("needs a range"), "{e:?}");
}

#[test]
fn the_map_is_saved_beside_the_project_and_reloads() {
    let Band { s, .. } = band();
    let mut b = Bench::new(s);
    b.measure(json!({ "start": 0.0, "end": 2.0 }));

    let dir = std::env::temp_dir().join(format!("mort-measure-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.join("song.rpp");
    b.s.set_path(Some(project.to_str().unwrap()));
    // The first look after saving writes the map out.
    let m = b.ok("map.status", json!({}));
    let sidecar = dir.join("song.mort/captures");
    assert_eq!(m["saved"], json!(sidecar));
    assert!(sidecar.join("index.json").exists());
    assert!(sidecar.join("1.frames").exists());

    // A new measurement is written as it is made; the one it supersedes
    // is kept for comparing.
    let r = b.measure(json!({ "start": 0.0, "end": 2.0 }));
    assert_eq!(r["capture"], 2);
    assert!(sidecar.join("2.frames").exists() && sidecar.join("1.frames").exists());

    // A bridge started on the saved project finds the map.
    let mut again = Bench::new(b.s.clone());
    let m = again.ok("map.status", json!({}));
    assert_eq!(m["captures"][0]["id"], 2);
    assert_eq!(m["tracks"].as_array().unwrap().len(), 5);
    assert!(again.stale().is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_measure_starts_early_so_its_first_frames_are_right() {
    let mut s = FakeSession::new();
    let t = s
        .add_track(NewTrack {
            name: "Bass".into(),
            ..NewTrack::default()
        })
        .unwrap();
    s.add_sound(
        t,
        Source::Pink {
            lo: 40.0,
            hi: 150.0,
            db: -20.0,
        },
        None,
    )
    .unwrap();
    let mut b = Bench::new(s);
    let read = |b: &mut Bench| {
        b.ok(
            "map.frames",
            json!({ "track": t, "start": 4.0, "end": 6.0, "features": ["band:63", "short_term_lufs"], "step": 0.5 }),
        )["rows"]
            .clone()
    };
    // A render rolls in 3 s early: every feature is right from the start.
    b.measure(json!({ "start": 4.0, "end": 6.0 }));
    let rows = read(&mut b);
    let low: Vec<f64> = (0..4).map(|i| rows[i][1][0].as_f64().unwrap()).collect();
    assert!(
        (low[0] - low[3]).abs() < 3.0,
        "the first half second reads like the rest: {low:?}"
    );
    assert!(rows[0][1][1].as_f64().unwrap() > -30.0, "{rows}");

    // Playing rolls in 0.7 s early: the low bands are right, short-term
    // loudness is unknown until its window is full.
    b.measure(json!({ "start": 4.0, "end": 6.0, "mode": "play" }));
    let rows = read(&mut b);
    let low: Vec<f64> = (0..4).map(|i| rows[i][1][0].as_f64().unwrap()).collect();
    assert!((low[0] - low[3]).abs() < 3.0, "{low:?}");
    assert!(rows[0][1][1].is_null(), "{rows}");
}
