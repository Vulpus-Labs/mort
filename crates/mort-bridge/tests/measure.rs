//! `measure` and `map.status` over the fake, which renders through the
//! real feature code (ticket 0012, ADR 0006, ADR 0011 §2).

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use mort_bridge::dispatch::{Dispatcher, Inbound};
use mort_proto::{ErrorKind, ErrorObject, Message, Request, RequestId, SteppedTempoMap, TrackId};
use mort_session::{FakeSession, NewTrack, Session, Source};

struct Bench {
    s: FakeSession,
    d: Dispatcher<FakeSession>,
    tx: Sender<Inbound>,
    replies: Receiver<String>,
    n: i64,
}

impl Bench {
    fn new(s: FakeSession) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut d = Dispatcher::new(rx);
        mort_bridge::methods::register_reads(&mut d);
        mort_bridge::methods::register_edits(&mut d);
        mort_bridge::methods::register_transport(&mut d);
        mort_bridge::methods::register_probes(&mut d);
        mort_bridge::measure::register_measure(&mut d);
        let (out, replies) = mpsc::channel();
        tx.send(Inbound::Opened { conn: 1, tx: out }).unwrap();
        Self {
            s,
            d,
            tx,
            replies,
            n: 0,
        }
    }

    fn tick(&mut self) {
        self.d
            .tick(&mut self.s, Instant::now(), Duration::from_millis(50));
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, ErrorObject> {
        self.n += 1;
        let id = RequestId::Number(self.n);
        self.tx
            .send(Inbound::Request {
                conn: 1,
                request: Request::new(id.clone(), method, params),
            })
            .unwrap();
        self.tick();
        while let Ok(line) = self.replies.try_recv() {
            if let Ok(Message::Response(r)) = Message::parse(&line)
                && r.id == Some(id.clone())
            {
                return r.outcome;
            }
        }
        panic!("no reply to {method}");
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        self.call(method, params)
            .unwrap_or_else(|e| panic!("{method}: {e:?}"))
    }

    /// Runs a measure to the end and returns its result.
    fn measure(&mut self, params: Value) -> Value {
        let r = self.ok("measure", params);
        if r.get("nothing_stale").is_some() {
            return r;
        }
        let job = r["job"].clone();
        for _ in 0..1000 {
            self.s.advance_by(0.25);
            let st = self.ok("job.status", json!({ "id": job }));
            match st["state"].as_str() {
                Some("done") => return st["result"].clone(),
                Some("failed") => panic!("measure failed: {st}"),
                _ => {}
            }
        }
        panic!("measure never finished");
    }

    fn stale(&mut self) -> Vec<String> {
        let m = self.ok("map.status", json!({}));
        m["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| {
                t["spans"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|s| !s["stale"].is_null())
            })
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect()
    }
}

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
    assert_eq!(r["dropped"].as_array().unwrap().len(), 2);

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

    // A new measurement is written as it is made.
    let r = b.measure(json!({ "start": 0.0, "end": 2.0 }));
    assert_eq!(r["capture"], 2);
    assert!(sidecar.join("2.frames").exists() && !sidecar.join("1.frames").exists());

    // A bridge started on the saved project finds the map.
    let mut again = Bench::new(b.s.clone());
    let m = again.ok("map.status", json!({}));
    assert_eq!(m["captures"][0]["id"], 2);
    assert_eq!(m["tracks"].as_array().unwrap().len(), 5);
    assert!(again.stale().is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}
