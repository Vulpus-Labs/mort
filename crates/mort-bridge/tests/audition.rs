//! Auditions over the fake (ticket 0017, ADR 0012 §1–5): playback nobody
//! asked mort to measure feeds the map.

mod common;

use serde_json::{Value, json};

use common::Bench;
use mort_proto::TrackId;
use mort_session::{FakeSession, NewTrack, Session, Source, TrackChange};

struct Kit {
    b: Bench,
    kick: TrackId,
    snare: TrackId,
    bass: TrackId,
}

/// Drums (a folder holding Kick and Snare) and Bass, all probed.
fn kit() -> Kit {
    let mut s = FakeSession::new();
    let mut add = |name: &str, parent: Option<TrackId>, src: Option<Source>| {
        let id = s
            .add_track(NewTrack {
                name: name.into(),
                parent,
                ..NewTrack::default()
            })
            .unwrap();
        if let Some(src) = src {
            s.add_sound(id, src, None).unwrap();
        }
        id
    };
    let drums = add("Drums", None, None);
    let kick = add(
        "Kick",
        Some(drums),
        Some(Source::Sine {
            hz: 55.0,
            db: -12.0,
        }),
    );
    let snare = add(
        "Snare",
        Some(drums),
        Some(Source::Pink {
            lo: 200.0,
            hi: 8000.0,
            db: -24.0,
        }),
    );
    let bass = add(
        "Bass",
        None,
        Some(Source::Pink {
            lo: 40.0,
            hi: 150.0,
            db: -20.0,
        }),
    );
    let mut b = Bench::new(s);
    b.ok("probes.ensure", json!({ "summary": "probes" }));
    Kit {
        b,
        kick,
        snare,
        bass,
    }
}

/// Plays `from..to` in steps of 0.1 s, ticking the bridge as REAPER would,
/// calling `during` at each step with the position.
fn listen(b: &mut Bench, from: f64, to: f64, mut during: impl FnMut(&mut Bench, f64)) {
    b.s.play(from, to).unwrap();
    while b.s.is_playing() {
        b.s.advance_by(0.1);
        let at = b.s.position();
        during(b, at);
        b.tick();
    }
    b.tick();
    b.tick();
}

fn status(b: &mut Bench) -> Value {
    b.ok("map.status", json!({}))
}

/// Each track's covered spans as (start, end, stale), by name.
fn spans(m: &Value, name: &str) -> Vec<(f64, f64, bool)> {
    m["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .map(|t| {
            t["spans"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| {
                    (
                        (s["start"].as_f64().unwrap() * 100.0).round() / 100.0,
                        (s["end"].as_f64().unwrap() * 100.0).round() / 100.0,
                        !s["stale"].is_null(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn sources(m: &Value) -> Vec<String> {
    m["captures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["source"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn playing_fills_the_map_after_the_warm_up() {
    let mut k = kit();
    listen(&mut k.b, 0.0, 4.0, |_, _| {});
    let m = status(&mut k.b);
    assert_eq!(sources(&m), ["audition"]);
    for name in ["MASTER", "Drums", "Kick", "Snare", "Bass"] {
        assert_eq!(spans(&m, name), [(0.7, 4.0, false)], "{name}");
    }
    // Short-term loudness is unknown for the first 3 s, and skipped.
    let f = k.b.ok(
        "map.frames",
        json!({ "track": k.bass, "start": 0.0, "end": 4.0, "features": ["short_term_lufs"], "step": 1.0 }),
    );
    let rows = f["rows"].as_array().unwrap();
    assert!(rows[0][1][0].is_null() && rows[1][1][0].is_null(), "{f}");
    assert!(rows[3][1][0].as_f64().unwrap() > -40.0, "{f}");
}

#[test]
fn a_seek_or_a_loop_splits_the_stretch() {
    let mut k = kit();
    let mut sought = false;
    listen(&mut k.b, 0.0, 8.0, |b, at| {
        if at >= 3.0 && !sought {
            sought = true;
            b.s.seek(5.0);
        }
    });
    let m = status(&mut k.b);
    assert_eq!(sources(&m), ["audition", "audition"]);
    assert_eq!(spans(&m, "Kick"), [(0.7, 3.0, false), (5.7, 8.0, false)]);

    // A loop wraps back: the second time round replaces the first.
    let mut k = kit();
    k.b.s.set_loop(Some((1.0, 3.0)));
    let mut steps = 0;
    listen(&mut k.b, 1.0, 10.0, |b, _| {
        steps += 1;
        if steps == 45 {
            b.s.stop();
        }
    });
    let m = status(&mut k.b);
    assert_eq!(spans(&m, "Kick"), [(1.7, 3.0, false)], "{m:#}");
}

#[test]
fn an_edit_mid_play_splits_and_each_part_keeps_its_state() {
    let mut k = kit();
    let kick = k.kick;
    let mut edited = false;
    listen(&mut k.b, 0.0, 6.0, |b, at| {
        if at >= 2.5 && !edited {
            edited = true;
            b.s.set_track(
                kick,
                &TrackChange {
                    volume: Some(0.5),
                    ..TrackChange::default()
                },
            )
            .unwrap();
        }
    });
    let m = status(&mut k.b);
    assert_eq!(sources(&m), ["audition", "audition"]);
    // Before the edit the kick was played under the old state: stale now,
    // as are its buses. After it, fresh.
    let kick = spans(&m, "Kick");
    assert_eq!(kick.len(), 2, "{kick:?}");
    assert!(kick[0].2 && !kick[1].2, "{kick:?}");
    assert!(spans(&m, "Drums")[0].2 && !spans(&m, "Drums")[1].2);
    assert!(
        spans(&m, "Bass").iter().all(|s| !s.2),
        "Bass did not change"
    );
}

#[test]
fn solo_keeps_tracks_and_drops_buses_with_silenced_inputs() {
    let mut k = kit();
    k.b.s
        .set_track(
            k.kick,
            &TrackChange {
                solo: Some(true),
                ..TrackChange::default()
            },
        )
        .unwrap();
    listen(&mut k.b, 0.0, 3.0, |_, _| {});
    let m = status(&mut k.b);
    // Every track's own probe is pre-fader, so it measures fairly; the
    // folder and the master lost Snare and Bass to the solo.
    for name in ["Kick", "Snare", "Bass"] {
        assert_eq!(spans(&m, name).len(), 1, "{name} kept");
    }
    for name in ["Drums", "MASTER"] {
        assert!(spans(&m, name).is_empty(), "{name} dropped");
    }

    // Solo is not audio state: soloing after a measure leaves it fresh.
    let mut k = kit();
    k.b.measure(json!({ "start": 0.0, "end": 2.0 }));
    k.b.s
        .set_track(
            k.snare,
            &TrackChange {
                solo: Some(true),
                ..TrackChange::default()
            },
        )
        .unwrap();
    assert!(k.b.stale().is_empty());
}

#[test]
fn what_is_never_kept() {
    // Play rate other than 1.
    let mut k = kit();
    k.b.s.set_play_rate(1.5);
    listen(&mut k.b, 0.0, 3.0, |_, _| {});
    assert!(sources(&status(&mut k.b)).is_empty());

    // Shorter than the warm-up.
    let mut k = kit();
    listen(&mut k.b, 0.0, 0.6, |_, _| {});
    assert!(sources(&status(&mut k.b)).is_empty());

    // A record-monitored track and everything it reaches.
    let mut k = kit();
    k.b.s.set_monitored(k.bass, true);
    listen(&mut k.b, 0.0, 3.0, |_, _| {});
    let m = status(&mut k.b);
    assert!(spans(&m, "Bass").is_empty() && spans(&m, "MASTER").is_empty());
    assert_eq!(spans(&m, "Kick").len(), 1);
}

#[test]
fn one_playback_is_one_frames_file() {
    let mut k = kit();
    let dir = std::env::temp_dir().join(format!("mort-audition-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    k.b.s.set_path(Some(dir.join("song.rpp").to_str().unwrap()));
    let mut sought = false;
    listen(&mut k.b, 0.0, 8.0, |b, at| {
        if at >= 3.0 && !sought {
            sought = true;
            b.s.seek(5.0);
        }
    });
    let side = dir.join("song.mort/captures");
    let files: Vec<String> = std::fs::read_dir(&side)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".frames"))
        .collect();
    assert_eq!(files, ["1.frames"], "two stretches, one file");
    let again = status(&mut Bench::new(k.b.s.clone()));
    assert_eq!(sources(&again), ["audition", "audition"]);
    std::fs::remove_dir_all(&dir).unwrap();
}
