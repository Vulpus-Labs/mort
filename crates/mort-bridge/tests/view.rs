//! The map view and the change feed over the fake (ticket 0018, ADR 0012
//! §6 and §8).

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use common::Bench;
use mort_bridge::dispatch::Dispatcher;
use mort_bridge::feed::Feed;
use mort_bridge::server::Server;
use mort_proto::TrackId;
use mort_session::{FakeSession, NewTrack, Session, Source, TrackChange};

/// Drums (Kick inside), Bass, and a muted Pad; regions A (0–4 s) and
/// B (4–8 s). Bass and Pad 2 are stacked so they mask each other.
fn session() -> (FakeSession, TrackId, TrackId) {
    let mut s = FakeSession::new();
    let add = |s: &mut FakeSession, name: &str, parent, src: Option<Source>| {
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
    let drums = add(&mut s, "Drums", None, None);
    let kick = add(
        &mut s,
        "Kick",
        Some(drums),
        Some(Source::Sine {
            hz: 55.0,
            db: -14.0,
        }),
    );
    let pink = |lo, hi, db| Some(Source::Pink { lo, hi, db });
    add(&mut s, "Pad", None, pink(150.0, 600.0, -26.0));
    add(&mut s, "Pad 2", None, pink(150.0, 600.0, -27.0));
    let muted = add(&mut s, "Muted", None, pink(1000.0, 4000.0, -30.0));
    s.set_track(
        muted,
        &TrackChange {
            mute: Some(true),
            ..TrackChange::default()
        },
    )
    .unwrap();
    s.add_region("A", 0.0, Some(4.0));
    s.add_region("B", 4.0, Some(8.0));
    (s, kick, muted)
}

fn listen(b: &mut Bench, from: f64, to: f64) {
    b.s.play(from, to).unwrap();
    while b.s.is_playing() {
        b.s.advance_by(0.1);
        b.tick();
    }
    b.tick();
    b.tick();
}

#[test]
fn the_view_is_a_grid_of_freshness_staleness_and_findings() {
    let (s, kick, _) = session();
    let mut b = Bench::new(s);
    b.measure(json!({ "start": 0.0, "end": 4.0 }));
    let v = b.ok("map.view", json!({ "start": 0.0, "end": 8.0, "step": 1.0 }));
    assert_eq!(v["columns"], 8);
    let rows = v["rows"].as_array().unwrap();
    let names: Vec<(&str, u64)> = rows
        .iter()
        .map(|r| (r["name"].as_str().unwrap(), r["depth"].as_u64().unwrap()))
        .collect();
    assert_eq!(
        names,
        [
            ("MASTER", 0),
            ("Drums", 0),
            ("Kick", 1),
            ("Pad", 0),
            ("Pad 2", 0),
            ("Muted", 0)
        ]
    );
    let kick_row = &rows[2];
    assert_eq!(kick_row["cells"][0]["c"], 1.0);
    assert_eq!(kick_row["cells"][0]["s"], "render");
    assert!(kick_row["cells"][0]["t"].as_u64().unwrap() > 0);
    assert!(kick_row["cells"][4].is_null(), "B was never measured");
    assert_eq!(rows[5]["unmeasured"], "muted");
    assert!(rows[5]["cells"][0].is_null());
    // Pad and Pad 2 mask each other in A: both rows carry the marker.
    for r in [&rows[3], &rows[4]] {
        let kinds = r["cells"][1]["k"].as_array().unwrap();
        assert!(
            kinds.contains(&json!("masking")),
            "{}: {kinds:?}",
            r["name"]
        );
        assert!(r["cells"][1]["f"].as_f64().unwrap() > 0.5);
    }

    // An edit makes the kick and its buses stale, with the reason.
    b.s.set_track(
        kick,
        &TrackChange {
            volume: Some(0.5),
            ..TrackChange::default()
        },
    )
    .unwrap();
    let v = b.ok("map.view", json!({ "start": 0.0, "end": 8.0, "step": 1.0 }));
    let rows = v["rows"].as_array().unwrap();
    for i in [0, 1, 2] {
        assert_eq!(
            rows[i]["cells"][0]["x"]["why"], "changed",
            "{}",
            rows[i]["name"]
        );
    }
    assert!(rows[3]["cells"][0].get("x").is_none(), "Pad is still fresh");
}

#[test]
fn findings_are_recomputed_only_where_data_changed() {
    let (s, _, _) = session();
    let mut b = Bench::new(s);
    b.measure(json!({ "start": 0.0, "end": 8.0 }));
    let r = b.ok("map.findings", json!({}));
    assert_eq!(r["recomputed"], json!(["A", "B"]));
    let r = b.ok("map.findings", json!({}));
    assert_eq!(r["recomputed"], json!([]), "nothing changed");
    listen(&mut b, 4.5, 8.0);
    let r = b.ok("map.findings", json!({}));
    assert_eq!(r["recomputed"], json!(["B"]), "only the section heard");
    b.ok(
        "thresholds.set",
        json!({ "name": "mud_excess", "value": 5.0 }),
    );
    let r = b.ok("map.findings", json!({}));
    assert_eq!(
        r["recomputed"],
        json!(["A", "B"]),
        "thresholds touch every section"
    );
}

/// A bridge on a real socket, with the dispatcher ticked by the test.
struct Live {
    s: FakeSession,
    d: Dispatcher<FakeSession>,
    _server: Server,
    path: std::path::PathBuf,
}

impl Live {
    fn new(name: &str) -> (Live, TrackId) {
        let dir = std::env::temp_dir().join(format!("mort-view-{}-{name}", std::process::id()));
        let (tx, rx) = mpsc::channel();
        let feed = Arc::new(Feed::new());
        let server = Server::start_with_feed(&dir, "bridge", tx, Some(Arc::clone(&feed))).unwrap();
        let path = server.path().to_owned();
        let mut d = Dispatcher::new(rx);
        mort_bridge::methods::register_reads(&mut d);
        mort_bridge::methods::register_edits(&mut d);
        mort_bridge::methods::register_probes(&mut d);
        mort_bridge::measure::register_measure_with_feed(&mut d, feed);
        let (s, kick, _) = session();
        let mut live = Live {
            s,
            d,
            _server: server,
            path,
        };
        live.call(&json!({ "method": "probes.ensure", "params": { "summary": "probes" } }));
        (live, kick)
    }

    fn tick(&mut self) {
        self.d
            .tick(&mut self.s, Instant::now(), Duration::from_millis(20));
    }

    /// Calls through the socket, ticking until the answer comes.
    fn call(&mut self, req: &Value) -> Value {
        let mut c = UnixStream::connect(&self.path).unwrap();
        let mut msg = req.clone();
        msg["jsonrpc"] = json!("2.0");
        msg["id"] = json!(1);
        writeln!(c, "{msg}").unwrap();
        c.set_nonblocking(true).unwrap();
        let mut r = BufReader::new(c);
        let mut line = String::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.tick();
            if r.read_line(&mut line).is_ok_and(|n| n > 0) && line.ends_with('\n') {
                return serde_json::from_str(&line).unwrap();
            }
            assert!(Instant::now() < deadline, "no answer to {req}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Starts a watch on another thread, as a client would.
    fn watch(&self, since: u64, timeout_ms: u64) -> mpsc::Receiver<Value> {
        let path = self.path.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut c = UnixStream::connect(path).unwrap();
            let msg = json!({ "jsonrpc": "2.0", "id": 7, "method": "map.watch",
                "params": { "since": since, "timeout_ms": timeout_ms } });
            writeln!(c, "{msg}").unwrap();
            let mut line = String::new();
            BufReader::new(c).read_line(&mut line).unwrap();
            let v: Value = serde_json::from_str(&line).unwrap();
            let _ = tx.send(v["result"].clone());
        });
        rx
    }

    /// Ticks, advancing the transport if it plays, until `rx` answers.
    fn until(&mut self, rx: &mpsc::Receiver<Value>) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(v) = rx.try_recv() {
                return v;
            }
            assert!(Instant::now() < deadline, "the watch never answered");
            self.s.advance_by(0.1);
            self.tick();
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn kinds(v: &Value) -> Vec<String> {
    v["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn watch_wakes_on_auditions_edits_and_thresholds_and_otherwise_times_out() {
    let (mut live, kick) = Live::new("watch");
    // Let the first staleness baseline settle.
    for _ in 0..5 {
        live.tick();
    }
    let v0 = live.watch(0, 0);
    let base = v0.recv().unwrap()["version"].as_u64().unwrap();

    // Nothing happens: the watch times out, answered without a single
    // tick of the dispatcher.
    let t = Instant::now();
    let v = live.watch(base, 150).recv().unwrap();
    assert!(t.elapsed() >= Duration::from_millis(150));
    assert_eq!(v["version"].as_u64().unwrap(), base);
    assert_eq!(v["changes"], json!([]));

    // An audition.
    let rx = live.watch(base, 5000);
    live.s.play(0.0, 2.0).unwrap();
    let v = live.until(&rx);
    assert_eq!(kinds(&v), ["capture"]);
    assert!(
        v["changes"][0]["tracks"]
            .as_array()
            .unwrap()
            .contains(&json!(kick))
    );
    let base = v["version"].as_u64().unwrap();

    // An edit: the kick and its buses go stale.
    let rx = live.watch(base, 5000);
    live.s
        .set_track(
            kick,
            &TrackChange {
                volume: Some(0.25),
                ..TrackChange::default()
            },
        )
        .unwrap();
    let v = live.until(&rx);
    assert_eq!(kinds(&v), ["stale"]);
    assert_eq!(v["changes"][0]["tracks"].as_array().unwrap().len(), 3);
    let base = v["version"].as_u64().unwrap();

    // A threshold change.
    let rx = live.watch(base, 5000);
    live.call(
        &json!({ "method": "thresholds.set", "params": { "name": "headroom", "value": -3.0 } }),
    );
    let v = live.until(&rx);
    assert_eq!(kinds(&v), ["thresholds"]);
}
