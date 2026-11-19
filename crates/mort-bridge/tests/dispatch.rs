//! The dispatcher over `FakeSession`, through a real socket (ADR 0011 §4).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use mort_bridge::dispatch::{Dispatcher, Job, JobPoll, Outcome};
use mort_bridge::server::Server;
use mort_proto::ErrorObject;
use mort_session::{FakeSession, NewTrack, Session, TrackChange};

/// A job that finishes after `ticks` polls.
struct CountJob {
    ticks: u32,
    polled: u32,
    plays: bool,
}

impl Job<FakeSession> for CountJob {
    fn start(&mut self, s: &mut FakeSession) -> Result<(), ErrorObject> {
        if self.plays {
            s.play(0.0, 10.0)
        } else {
            Ok(())
        }
    }
    fn poll(&mut self, _: &mut FakeSession) -> JobPoll {
        self.polled += 1;
        if self.polled >= self.ticks {
            JobPoll::Done(json!({ "polled": self.polled }))
        } else {
            JobPoll::Running(Some(json!(self.polled)))
        }
    }
    fn cancel(&mut self, s: &mut FakeSession) {
        s.stop();
    }
}

struct Rig {
    session: FakeSession,
    dispatcher: Dispatcher<FakeSession>,
    _server: Server,
    path: PathBuf,
    now: Instant,
}

fn rig(name: &str) -> Rig {
    let dir = std::env::temp_dir().join(format!("mort-test-{}-{name}", std::process::id()));
    let (tx, rx) = mpsc::channel();
    let server = Server::start(&dir, "bridge", tx).unwrap();
    let path = server.path().to_owned();
    let mut d = Dispatcher::new(rx);
    d.register("test.count", false, |_, p: Value| {
        let ticks = u32::try_from(p["ticks"].as_u64().unwrap_or(1)).unwrap();
        let plays = p["play"].as_bool().unwrap_or(false);
        Outcome::Job(Box::new(CountJob {
            ticks,
            polled: 0,
            plays,
        }))
    });
    d.register("track.add", true, |ctx, p: Value| {
        let name = p["name"].as_str().unwrap_or("track").to_owned();
        Outcome::Value(
            ctx.session
                .add_track(NewTrack {
                    name,
                    ..NewTrack::default()
                })
                .map(|id| json!(id)),
        )
    });
    d.register("track.mute", true, |ctx, p: Value| {
        let id = serde_json::from_value(p["track"].clone()).unwrap();
        Outcome::Value(
            ctx.session
                .set_track(
                    id,
                    &TrackChange {
                        mute: Some(true),
                        ..TrackChange::default()
                    },
                )
                .map(|()| Value::Null),
        )
    });
    d.register("tracks.count", false, |ctx, _| {
        Outcome::Value(Ok(json!(ctx.session.tracks().len())))
    });
    Rig {
        session: FakeSession::new(),
        dispatcher: d,
        _server: server,
        path,
        now: Instant::now(),
    }
}

struct Client {
    w: UnixStream,
    r: BufReader<UnixStream>,
    next: i64,
}

impl Rig {
    fn tick(&mut self) {
        self.dispatcher
            .tick(&mut self.session, self.now, Duration::from_millis(8));
    }

    fn connect(&mut self) -> Client {
        let w = UnixStream::connect(&self.path).unwrap();
        w.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let r = BufReader::new(w.try_clone().unwrap());
        Client { w, r, next: 1 }
    }

    /// Sends a request, ticks until a line arrives, returns it.
    fn call(&mut self, c: &mut Client, method: &str, params: Value) -> Value {
        c.send(method, params);
        self.read(c)
    }

    fn read(&mut self, c: &mut Client) -> Value {
        c.r.get_ref().set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut line = String::new();
        loop {
            self.tick();
            match c.r.read_line(&mut line) {
                Ok(n) if n > 0 && line.ends_with('\n') => break,
                _ if Instant::now() > deadline => panic!("no reply; partial: {line:?}"),
                _ => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        c.r.get_ref().set_nonblocking(false).unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

impl Client {
    fn send(&mut self, method: &str, params: Value) {
        let mut msg = json!({"jsonrpc": "2.0", "id": self.next, "method": method});
        msg["params"] = params;
        self.next += 1;
        writeln!(self.w, "{msg}").unwrap();
    }
}

fn error_kind(v: &Value) -> &str {
    v["error"]["data"]["kind"].as_str().unwrap_or("none")
}

#[test]
fn hello_and_status() {
    let mut r = rig("hello");
    let mut c = r.connect();
    let v = r.call(&mut c, "bridge.hello", json!({ "protocol": [0, 1] }));
    assert_eq!(v["result"]["protocol"], json!([0, 1]));
    assert!(
        v["result"]["bridge"]
            .as_str()
            .unwrap()
            .starts_with("0.1.0+")
    );

    let v = r.call(&mut c, "bridge.hello", json!({ "protocol": [7, 0] }));
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("7.0") && msg.contains("0.1"), "{msg}");

    let v = r.call(&mut c, "bridge.status", Value::Null);
    assert_eq!(v["result"]["project"], "untitled");
    assert_eq!(v["result"]["held"], false);
}

#[test]
fn protocol_errors() {
    let mut r = rig("proto");
    let mut c = r.connect();
    writeln!(c.w, "not json").unwrap();
    let v = r.read(&mut c);
    assert_eq!(v["error"]["code"], -32700);
    assert_eq!(v["id"], Value::Null);
    let v = r.call(&mut c, "no.such", Value::Null);
    assert_eq!(v["error"]["code"], -32601);
    let v = r.call(&mut c, "job.status", json!({ "wrong": 1 }));
    assert_eq!(v["error"]["code"], -32602);
}

#[test]
fn edits_are_undo_points_and_check_state() {
    let mut r = rig("edits");
    let mut c = r.connect();
    let v = r.call(
        &mut c,
        "track.add",
        json!({ "name": "bass", "summary": "add bass" }),
    );
    let bass = v["result"].clone();
    assert_eq!(r.session.undo_label().as_deref(), Some("mort: add bass"));

    let state = r.session.state();
    let v = r.call(
        &mut c,
        "track.mute",
        json!({ "track": bass, "expect_state": state - 1 }),
    );
    assert_eq!(error_kind(&v), "stale");
    let v = r.call(
        &mut c,
        "track.mute",
        json!({ "track": bass, "expect_state": state }),
    );
    assert_eq!(v["result"], Value::Null);
    assert_eq!(r.session.undo_label().as_deref(), Some("mort: track.mute"));

    r.dispatcher.set_held(true);
    let v = r.call(&mut c, "track.add", json!({}));
    assert_eq!(error_kind(&v), "busy");
    let v = r.call(&mut c, "tracks.count", Value::Null);
    assert_eq!(v["result"], 1, "reads work while held");
}

#[test]
fn batches_are_one_undo_point_and_all_or_nothing() {
    let mut r = rig("batch");
    let mut c = r.connect();
    let v = r.call(
        &mut c,
        "batch",
        json!({ "summary": "two tracks", "calls": [
            { "method": "track.add", "params": { "name": "a" } },
            { "method": "track.add", "params": { "name": "b" } },
            { "method": "tracks.count" },
        ]}),
    );
    assert_eq!(v["result"][2], 2);
    assert_eq!(r.session.undo_label().as_deref(), Some("mort: two tracks"));
    r.session.undo();
    assert_eq!(r.session.tracks().len(), 0, "one undo removes both");
    r.session.redo();

    let ghost = "{00000000-0000-0000-0000-00000000002A}";
    let v = r.call(
        &mut c,
        "batch",
        json!({ "calls": [
            { "method": "track.add", "params": { "name": "c" } },
            { "method": "track.mute", "params": { "track": ghost } },
        ]}),
    );
    assert_eq!(error_kind(&v), "not_found");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("batch call 1")
    );
    assert_eq!(
        r.session.tracks().len(),
        2,
        "the failed batch was rolled back"
    );
    assert_eq!(r.session.undo_label().as_deref(), Some("mort: two tracks"));

    let v = r.call(
        &mut c,
        "batch",
        json!({ "calls": [{ "method": "test.count" }] }),
    );
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot start a job")
    );
}

#[test]
fn jobs_queue_run_wait_and_cancel() {
    let mut r = rig("jobs");
    let mut c = r.connect();
    // A job that runs until cancelled, and one queued behind it.
    let long = r.call(
        &mut c,
        "test.count",
        json!({ "ticks": 1_000_000, "play": true }),
    )["result"]["job"]
        .clone();
    let b = r.call(&mut c, "test.count", json!({ "ticks": 3 }))["result"]["job"].clone();
    let v = r.call(&mut c, "job.status", json!({ "id": long }));
    assert_eq!(v["result"]["state"], "running");
    assert!(r.session.is_playing());
    let v = r.call(&mut c, "job.status", json!({ "id": b }));
    assert_eq!(v["result"]["state"], "queued", "one job runs at a time");

    // Waiting with no time left answers at once with the current state.
    let v = r.call(&mut c, "job.wait", json!({ "id": b, "timeout_ms": 0 }));
    assert_eq!(v["result"]["state"], "queued");

    let v = r.call(&mut c, "job.cancel", json!({ "id": long }));
    assert_eq!(v["result"]["state"], "cancelled");
    assert!(!r.session.is_playing(), "cancel stopped the transport");

    let v = r.call(&mut c, "job.wait", json!({ "id": b, "timeout_ms": 5000 }));
    assert_eq!(v["result"]["state"], "done");
    assert_eq!(v["result"]["result"]["polled"], 3);

    // Results expire after ten minutes.
    r.now += Duration::from_secs(601);
    let v = r.call(&mut c, "job.status", json!({ "id": b }));
    assert_eq!(error_kind(&v), "not_found");
}

#[test]
fn notifications() {
    let mut r = rig("notify");
    let mut c = r.connect();
    let v = r.call(
        &mut c,
        "bridge.subscribe",
        json!({ "events": ["project.changed", "job.finished"] }),
    );
    assert!(v["result"].is_object());
    let v = r.call(&mut c, "bridge.subscribe", json!({ "events": ["nope"] }));
    assert!(
        v["error"]["data"]["fix"]
            .as_str()
            .unwrap()
            .contains("project.changed")
    );

    // mort's own edit: the reply, then a notification saying it was mort.
    r.call(&mut c, "track.add", json!({ "name": "x" }));
    let n = r.read(&mut c);
    assert_eq!(n["method"], "project.changed");
    assert_eq!(n["params"]["by_mort"], true);

    // Someone else's edit.
    r.session.add_track(NewTrack::default()).unwrap();
    let n = r.read(&mut c);
    assert_eq!(n["params"]["by_mort"], false);
    assert_eq!(n["params"]["state"], r.session.state());

    let job = r.call(&mut c, "test.count", json!({ "ticks": 1 }))["result"]["job"].clone();
    let n = r.read(&mut c);
    assert_eq!(n["method"], "job.finished");
    assert_eq!(n["params"]["id"], job);
}

#[test]
fn socket_is_private_and_cleaned_up() {
    use std::os::unix::fs::PermissionsExt;
    let r = rig("perm");
    let mode = std::fs::metadata(&r.path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let current = r.path.parent().unwrap().join("current");
    assert_eq!(
        std::fs::read_to_string(&current).unwrap(),
        r.path.to_string_lossy()
    );
    let path = r.path.clone();
    drop(r);
    assert!(!path.exists());
    assert!(!current.exists());
}

#[test]
fn a_failed_edit_never_undoes_the_one_before() {
    let mut r = rig("rollback");
    let mut c = r.connect();
    let bass = r.call(
        &mut c,
        "track.add",
        json!({ "name": "bass", "summary": "add bass" }),
    )["result"]
        .clone();
    let ghost = "{00000000-0000-0000-0000-00000000002A}";
    let v = r.call(
        &mut c,
        "track.mute",
        json!({ "track": ghost, "summary": "mute ghost" }),
    );
    assert_eq!(error_kind(&v), "not_found");
    assert_eq!(r.session.undo_label().as_deref(), Some("mort: add bass"));
    assert_eq!(r.session.tracks().len(), 1, "the earlier edit stands");

    // A failure after a real change is rolled back through its own point.
    let v = r.call(
        &mut c,
        "batch",
        json!({ "summary": "half", "calls": [
            { "method": "track.mute", "params": { "track": bass } },
            { "method": "track.mute", "params": { "track": ghost } },
        ]}),
    );
    assert_eq!(error_kind(&v), "not_found");
    assert!(
        !r.session.tracks()[0].mute,
        "the first mute was rolled back"
    );
    assert_eq!(r.session.undo_label().as_deref(), Some("mort: add bass"));
}

#[test]
fn edits_say_whether_they_left_an_undo_point() {
    let mut r = rig("undo-point");
    r.dispatcher.register("test.add", true, |ctx, _| {
        Outcome::Value(
            ctx.session
                .add_track(NewTrack::default())
                .map(|id| json!({ "track": id })),
        )
    });
    // An edit that changes nothing leaves no undo point, and says so.
    r.dispatcher
        .register("test.nothing", true, |_, _| Outcome::Value(Ok(json!({}))));
    let mut c = r.connect();
    let v = r.call(&mut c, "test.add", json!({}));
    assert_eq!(v["result"]["undo_point"], true);
    let v = r.call(&mut c, "test.nothing", json!({}));
    assert_eq!(v["result"]["undo_point"], false);
}
