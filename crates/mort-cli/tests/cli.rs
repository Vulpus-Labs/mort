//! The `mort` binary against a fake bridge (ADR 0011 §4).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use mort_bridge::dispatch::{Dispatcher, Outcome};
use mort_bridge::server::Server;
use mort_proto::{ErrorKind, ErrorObject};
use mort_session::FakeSession;

struct FakeBridge {
    socket: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeBridge {
    fn start(name: &str) -> FakeBridge {
        Self::start_held(name, true, |_| {})
    }

    fn start_with(name: &str, setup: impl FnOnce(&mut FakeSession) + Send + 'static) -> FakeBridge {
        Self::start_held(name, false, setup)
    }

    fn start_held(
        name: &str,
        held: bool,
        setup: impl FnOnce(&mut FakeSession) + Send + 'static,
    ) -> FakeBridge {
        let dir = std::env::temp_dir().join(format!("mort-cli-{}-{name}", std::process::id()));
        let socket = dir.join("bridge.sock");
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let (tx, rx) = mpsc::channel();
                let _server = Server::start(&dir, "bridge", tx).unwrap();
                let mut d: Dispatcher<FakeSession> = Dispatcher::new(rx);
                d.register("test.missing", false, |_, _| {
                    Outcome::Value(Err(ErrorObject::new(
                        ErrorKind::NotFound,
                        "no such track",
                        Some("bass".into()),
                        "list tracks with `mort tracks`",
                    )))
                });
                d.register("test.echo", false, |_, p| Outcome::Value(Ok(p)));
                d.set_held(held);
                mort_bridge::methods::register_reads(&mut d);
                mort_bridge::methods::register_edits(&mut d);
                mort_bridge::methods::register_automation(&mut d);
                mort_bridge::methods::register_midi(&mut d);
                mort_bridge::methods::register_transport(&mut d);
                mort_bridge::methods::register_probes(&mut d);
                mort_bridge::measure::register_measure(&mut d);
                let mut session = FakeSession::new();
                setup(&mut session);
                ready_tx.send(()).unwrap();
                let mut last = Instant::now();
                while !stop.load(Ordering::SeqCst) {
                    // Fake playback runs at fifty times real time.
                    session.advance_by(last.elapsed().as_secs_f64() * 50.0);
                    last = Instant::now();
                    d.tick(&mut session, Instant::now(), Duration::from_millis(8));
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
        };
        ready_rx.recv().unwrap();
        FakeBridge {
            socket,
            stop,
            thread: Some(thread),
        }
    }

    fn mort(&self, args: &[&str]) -> Output {
        let mut a = vec!["--socket", self.socket.to_str().unwrap()];
        a.extend_from_slice(args);
        mort(&a)
    }
}

impl Drop for FakeBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn mort(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mort"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn status_text_and_json() {
    let b = FakeBridge::start("status");
    let o = b.mort(&["status"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let text = stdout(&o);
    assert!(text.starts_with("bridge   0.1.0+"), "{text}");
    assert!(text.contains("project  untitled"), "{text}");
    assert!(text.contains("held     yes"), "{text}");
    assert!(
        text.trim_end().ends_with("state    1"),
        "state is the last line: {text}"
    );

    let o = b.mort(&["status", "--json"]);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["project"], "untitled");
    assert_eq!(v["held"], true);
    assert!(v["bridge"].as_str().unwrap().starts_with("0.1.0+"));
}

#[test]
fn call_sends_any_rpc() {
    let b = FakeBridge::start("call");
    let o = b.mort(&["call", "test.echo", r#"{"a": [1, 2]}"#]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v, json!({ "a": [1, 2] }));
    let o = b.mort(&["--json", "call", "test.echo", "[3]"]);
    assert_eq!(stdout(&o), "[3]\n");
}

#[test]
fn bridge_errors_exit_1_with_kind_and_fix() {
    let b = FakeBridge::start("errors");
    let o = b.mort(&["call", "test.missing"]);
    assert_eq!(o.status.code(), Some(1));
    let e = stderr(&o);
    assert!(
        e.contains("not_found: no such track (bass). list tracks with `mort tracks`"),
        "{e}"
    );

    let o = b.mort(&["--json", "call", "test.missing"]);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["error"]["data"]["kind"], "not_found");

    let o = b.mort(&["call", "no.such"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("no method `no.such`"));
}

#[test]
fn usage_errors_exit_2() {
    let b = FakeBridge::start("usage");
    assert_eq!(b.mort(&["frobnicate"]).status.code(), Some(2));
    let o = b.mort(&["call", "test.echo", "{not json"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("parameters are not JSON"));
}

#[test]
fn no_bridge_exits_3() {
    let o = mort(&["--socket", "/nonexistent/mort.sock", "status"]);
    assert_eq!(o.status.code(), Some(3));
    let e = stderr(&o);
    assert!(
        e.contains("no bridge running") && e.contains("mort install"),
        "{e}"
    );
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mort-install-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn install(from: &Path, plugins: &Path, clap: &Path) -> Output {
    mort(&[
        "install",
        "--from",
        from.to_str().unwrap(),
        "--user-plugins",
        plugins.to_str().unwrap(),
        "--clap-dir",
        clap.to_str().unwrap(),
    ])
}

#[test]
fn install_copies_bridge_and_probe() {
    let root = tmp("ok");
    let (from, plugins, clap) = (
        root.join("target"),
        root.join("UserPlugins"),
        root.join("CLAP"),
    );
    std::fs::create_dir_all(from.join("mort-probe.clap/Contents/MacOS")).unwrap();
    std::fs::write(from.join("libmort_bridge.dylib"), "bridge v1").unwrap();
    std::fs::write(
        from.join("mort-probe.clap/Contents/MacOS/mort-probe"),
        "probe v1",
    )
    .unwrap();

    let o = install(&from, &plugins, &clap);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(
        std::fs::read_to_string(plugins.join("reaper_mort.dylib")).unwrap(),
        "bridge v1"
    );
    let probe = clap.join("mort-probe.clap/Contents/MacOS/mort-probe");
    assert_eq!(std::fs::read_to_string(&probe).unwrap(), "probe v1");

    // A second install replaces both, leaving no temporary files.
    std::fs::write(from.join("libmort_bridge.dylib"), "bridge v2").unwrap();
    std::fs::write(
        from.join("mort-probe.clap/Contents/MacOS/mort-probe"),
        "probe v2",
    )
    .unwrap();
    assert_eq!(install(&from, &plugins, &clap).status.code(), Some(0));
    assert_eq!(
        std::fs::read_to_string(plugins.join("reaper_mort.dylib")).unwrap(),
        "bridge v2"
    );
    assert_eq!(std::fs::read_to_string(&probe).unwrap(), "probe v2");
    let leftovers = |d: &Path| {
        std::fs::read_dir(d)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with('.')
            })
            .count()
    };
    assert_eq!(leftovers(&plugins) + leftovers(&clap), 0);
}

#[test]
fn install_without_a_build_says_how() {
    let root = tmp("missing");
    let o = install(
        &root.join("target"),
        &root.join("UserPlugins"),
        &root.join("CLAP"),
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("cargo build -p mort-bridge"),
        "{}",
        stderr(&o)
    );

    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("target/libmort_bridge.dylib"), "b").unwrap();
    let o = install(
        &root.join("target"),
        &root.join("UserPlugins"),
        &root.join("CLAP"),
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).contains("probe   not built"), "{}", stdout(&o));
}

fn band(s: &mut FakeSession) {
    use mort_session::{NewTrack, Session, TrackChange};
    let drums = s
        .add_track(NewTrack {
            name: "Drums".into(),
            ..NewTrack::default()
        })
        .unwrap();
    let kick = s
        .add_track(NewTrack {
            name: "Kick".into(),
            parent: Some(drums),
            ..NewTrack::default()
        })
        .unwrap();
    let bass = s
        .add_track(NewTrack {
            name: "Bass".into(),
            ..NewTrack::default()
        })
        .unwrap();
    s.add_track(NewTrack {
        name: "Bass DI".into(),
        ..NewTrack::default()
    })
    .unwrap();
    let verb = s
        .add_track(NewTrack {
            name: "Verb".into(),
            ..NewTrack::default()
        })
        .unwrap();
    s.set_track(
        bass,
        &TrackChange {
            volume: Some(0.5),
            pan: Some(-0.2),
            mute: Some(true),
            ..TrackChange::default()
        },
    )
    .unwrap();
    s.add_send(kick, verb).unwrap();
    s.add_fx(bass, "ReaEQ", None).unwrap();
    let comp = s.add_fx(bass, "ReaComp", None).unwrap();
    s.set_fx_enabled(comp, false).unwrap();
    s.add_region("verse", 0.0, Some(16.0));
}

#[test]
fn tracks_listing() {
    let b = FakeBridge::start_with("tracks", band);
    let o = b.mort(&["tracks"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let t = stdout(&o);
    let line = |name: &str| {
        t.lines()
            .find(|l| l.contains(name))
            .unwrap_or_else(|| panic!("{name} in {t}"))
            .to_owned()
    };
    assert!(
        line("  Kick").contains("→ Verb 0.0 dB"),
        "kick is indented under drums, with its send: {t}"
    );
    let bass = line("Bass  ");
    assert!(
        bass.starts_with("3 ")
            && bass.contains("-6.0 dB")
            && bass.contains("L20")
            && bass.contains(" M "),
        "{bass}"
    );
    assert!(
        bass.contains("VST: ReaEQ (Cockos), (VST: ReaComp (Cockos))"),
        "bypassed FX in brackets: {bass}"
    );
    assert!(line("MASTER").starts_with("   "), "master has no number");
    assert!(t.trim_end().lines().last().unwrap().starts_with("state "));

    let o = b.mort(&["--json", "tracks"]);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["tracks"].as_array().unwrap().len(), 5);
    assert!(v["state"].as_u64().unwrap() > 1);
}

#[test]
fn fx_and_params_by_name() {
    let b = FakeBridge::start_with("params", band);
    let o = b.mort(&["fx", "bass"]);
    let t = stdout(&o);
    assert!(
        t.starts_with("FX on Bass (2)"),
        "exact name beats `Bass DI`: {t}"
    );
    assert!(t.contains("2  VST: ReaComp (Cockos)  bypassed"), "{t}");

    let o = b.mort(&["params", "3", "eq"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let t = stdout(&o);
    assert!(
        t.starts_with("VST: ReaEQ (Cockos) on Bass: 3 of 3 parameters"),
        "{t}"
    );
    assert!(t.contains("0  Freq-Band 1  0.500  632 Hz"), "{t}");

    let o = b.mort(&["params", "bass", "1", "gain"]);
    let t = stdout(&o);
    assert!(
        t.contains("1 of 3") && t.contains("Gain-Band 1") && !t.contains("Freq"),
        "{t}"
    );
}

#[test]
fn ambiguous_names_list_the_matches() {
    let b = FakeBridge::start_with("ambiguous", band);
    let o = b.mort(&["fx", "ba"]);
    assert_eq!(o.status.code(), Some(1));
    let e = stderr(&o);
    assert!(
        e.contains("matches 2 tracks") && e.contains("3 Bass") && e.contains("4 Bass DI"),
        "{e}"
    );
    let o = b.mort(&["params", "bass", "delay"]);
    assert!(
        stderr(&o).contains("no FX matches `delay`"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn time_convert_uses_the_tempo_map() {
    let b = FakeBridge::start_with("time", band);
    let o = b.mort(&[
        "--json",
        "call",
        "time.convert",
        r#"{"to_seconds": ["9.1", "+4b", "1.1..+8b"], "to_musical": [16.5]}"#,
    ]);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["seconds"], json!([16.0, 18.0, [0.0, 4.0]]));
    assert_eq!(v["musical"], json!(["9.2"]));
    let o = b.mort(&["call", "time.convert", r#"{"to_seconds": ["9.9"]}"#]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("bar 9 is in 4/4"), "{}", stderr(&o));
}

#[test]
fn project_get_has_regions() {
    let b = FakeBridge::start_with("project", band);
    let o = b.mort(&["--json", "call", "project.get"]);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["project"]["name"], "untitled");
    assert_eq!(v["regions"][0]["name"], "verse");
    assert!(v["state"].is_u64());
}

fn ok(o: &Output) -> String {
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(o));
    stdout(o)
}

#[test]
fn track_edits_and_undo() {
    let b = FakeBridge::start_with("track-edits", band);
    let t = ok(&b.mort(&["track", "add", "Lead", "--at", "1"]));
    assert!(t.starts_with("added Lead at 1  {"), "{t}");
    let t = ok(&b.mort(&["track", "add", "Snare", "--in", "drums"]));
    assert!(t.starts_with("added Snare in Drums"), "{t}");
    let t = ok(&b.mort(&[
        "track", "set", "lead", "--volume", "-6", "--pan", "R35", "--solo", "on",
    ]));
    assert!(
        t.starts_with("set Lead volume -6.0 dB, pan R35, solo on"),
        "{t}"
    );

    let list = ok(&b.mort(&["tracks"]));
    let lead = list.lines().find(|l| l.contains("Lead")).unwrap();
    assert!(
        lead.starts_with("1 ")
            && lead.contains("-6.0 dB")
            && lead.contains("R35")
            && lead.contains('S'),
        "{lead}"
    );
    assert!(list.contains("    Snare"), "snare is inside drums: {list}");

    let t = ok(&b.mort(&["undo"]));
    assert!(
        t.starts_with("undid mort: Lead volume -6.0 dB, pan R35, solo on"),
        "{t}"
    );
    ok(&b.mort(&["redo"]));
    let t = ok(&b.mort(&["track", "rm", "lead"]));
    assert!(t.starts_with("removed Lead"), "{t}");
    assert!(!ok(&b.mort(&["tracks"])).contains("Lead"));

    let o = b.mort(&["track", "set", "bass"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("say what to change"));
}

#[test]
fn fx_edits() {
    let b = FakeBridge::start_with("fx-edits", band);
    let t = ok(&b.mort(&["fx", "add", "verb", "ReaComp"]));
    assert!(
        t.starts_with("added VST: ReaComp (Cockos) to Verb at 1"),
        "{t}"
    );
    ok(&b.mort(&["fx", "add", "verb", "ReaEQ", "--at", "1"]));
    let t = ok(&b.mort(&["fx", "verb"]));
    assert!(
        t.contains("1  VST: ReaEQ") && t.contains("2  VST: ReaComp"),
        "{t}"
    );
    ok(&b.mort(&["fx", "move", "verb", "comp", "1"]));
    ok(&b.mort(&["fx", "bypass", "verb", "eq"]));
    let t = ok(&b.mort(&["fx", "verb"]));
    let eq = t
        .lines()
        .find(|l| l.contains("2  VST: ReaEQ"))
        .unwrap_or_default();
    assert!(
        t.contains("1  VST: ReaComp") && eq.contains("bypassed"),
        "{t}"
    );
    ok(&b.mort(&["fx", "enable", "verb", "eq"]));
    let t = ok(&b.mort(&["fx", "preset", "verb", "eq", "Mud cut"]));
    assert!(
        t.starts_with("loaded VST: ReaEQ (Cockos) on Verb: preset Mud cut"),
        "{t}"
    );
    let t = ok(&b.mort(&["params", "verb", "eq", "gain"]));
    assert!(t.contains("-3.00 dB"), "{t}");
    ok(&b.mort(&["fx", "rm", "verb", "comp"]));
    assert!(!ok(&b.mort(&["fx", "verb"])).contains("ReaComp"));

    let o = b.mort(&["fx", "add", "verb", "Serum"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("no such plugin (Serum)"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn set_params_by_text_and_value() {
    let b = FakeBridge::start_with("set", band);
    let t = ok(&b.mort(&["set", "bass", "eq", "gain", "-3 dB"]));
    assert!(t.contains("Gain-Band 1 = -3.00 dB"), "{t}");
    let t = ok(&b.mort(&["set", "bass", "eq", "freq", "2.4k"]));
    assert!(t.contains("Freq-Band 1 = 2400 Hz"), "{t}");
    let t = ok(&b.mort(&["set", "bass", "eq", "2", "0.25", "--normalised"]));
    assert!(t.contains("Q-Band 1") && t.contains("(0.2500)"), "{t}");
    let o = b.mort(&["set", "bass", "eq", "gain", "+40 dB"]);
    assert_eq!(o.status.code(), Some(1));
    let e = stderr(&o);
    assert!(e.contains("runs from `-24.00 dB` to `24.00 dB`"), "{e}");
}

#[test]
fn edits_check_state_and_hold() {
    let b = FakeBridge::start_with("stale", band);
    let o = b.mort(&["call", "track.add", r#"{"name": "x", "expect_state": 1}"#]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).starts_with("mort: stale:"), "{}", stderr(&o));

    let held = FakeBridge::start_held("held", true, band);
    let o = held.mort(&["track", "add", "x"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("busy: edits are held"),
        "{}",
        stderr(&o)
    );
    assert!(
        ok(&held.mort(&["tracks"])).contains("Bass"),
        "reads still work"
    );
}

#[test]
fn undo_only_undoes_mort() {
    let b = FakeBridge::start_with("undo", |s| {
        use mort_session::{NewTrack, Session};
        band(s);
        s.begin_undo();
        s.add_track(NewTrack {
            name: "Theirs".into(),
            ..NewTrack::default()
        })
        .unwrap();
        s.end_undo("Add track");
    });
    let o = b.mort(&["undo"]);
    assert_eq!(o.status.code(), Some(1));
    let e = stderr(&o);
    assert!(
        e.contains("the next undo is not mort's: `Add track`")
            && e.contains("undo this one in REAPER"),
        "{e}"
    );
    assert!(ok(&b.mort(&["tracks"])).contains("Theirs"));
}

#[test]
fn automate_volume_pan_and_clear() {
    let b = FakeBridge::start_with("automate", band);
    let t = ok(&b.mort(&["automate", "bass", "volume"]));
    assert!(t.starts_with("Bass volume: no envelope"), "{t}");

    let t = ok(&b.mort(&[
        "automate", "bass", "volume", "1.1=-6", "+4b=0", "3.1=-inf", "--shape", "slow",
    ]));
    assert!(t.starts_with("Bass volume: wrote 3 points"), "{t}");
    assert!(t.contains("1.1  0.000s  -6.00dB  slow"), "{t}");
    assert!(
        t.contains("2.1  2.000s  0.00dB"),
        "+4b chains from the point before: {t}"
    );
    assert!(t.contains("3.1  4.000s  -inf"), "{t}");

    // Writing again over 1.1..2.1 replaces the points there, not the one after.
    let t = ok(&b.mort(&["automate", "bass", "volume", "1.1=-12", "2.1=-3"]));
    assert!(
        t.contains("3 points") && t.contains("-12.00dB") && t.contains("-inf"),
        "{t}"
    );
    let t = ok(&b.mort(&["automate", "bass", "volume", "1.3=-1", "--add"]));
    assert!(t.contains("4 points"), "{t}");

    let t = ok(&b.mort(&["automate", "bass", "volume", "--clear", "1.1..2.1"]));
    assert!(
        t.contains("cleared 1.1..2.1") && t.contains("2 points"),
        "{t}"
    );

    let t = ok(&b.mort(&["automate", "bass", "pan", "1.1=L30", "+8b=R30"]));
    assert!(t.contains("30%L") && t.contains("30%R"), "{t}");
}

#[test]
fn automate_params_and_sends() {
    let b = FakeBridge::start_with("automate-params", band);
    let t = ok(&b.mort(&["automate", "bass", "eq/gain", "1.1=0.25", "2.1=0.75"]));
    assert!(
        t.starts_with("Bass VST: ReaEQ (Cockos) Gain-Band 1: wrote 2 points"),
        "{t}"
    );
    let t = ok(&b.mort(&[
        "automate",
        "bass",
        "eq/freq",
        "1.1=300 Hz",
        "2.1=2.4k",
        "--text",
    ]));
    assert!(t.contains("2 points"), "{t}");
    let v: Value =
        serde_json::from_str(&stdout(&b.mort(&["--json", "automate", "bass", "eq/freq"]))).unwrap();
    let vals: Vec<f64> = v["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["value"].as_f64().unwrap())
        .collect();
    // ReaEQ's fake frequency runs 20 Hz to 20 kHz on a log scale.
    let hz = |n: f64| 20.0 * 1000f64.powf(n);
    assert!(
        (hz(vals[0]) - 300.0).abs() < 1.0 && (hz(vals[1]) - 2400.0).abs() < 5.0,
        "{vals:?}"
    );

    let t = ok(&b.mort(&["automate", "kick", "send:verb", "1.1=-inf", "2.1=0"]));
    assert!(t.starts_with("Kick send to Verb: wrote 2 points"), "{t}");
    let o = b.mort(&["automate", "bass", "send:verb", "1.1=0"]);
    assert!(
        stderr(&o).contains("Bass has no send to Verb"),
        "{}",
        stderr(&o)
    );

    let o = b.mort(&["automate", "bass", "eq/gain", "1.1=3"]);
    assert!(
        stderr(&o).contains("not a normalised value"),
        "{}",
        stderr(&o)
    );
    let o = b.mort(&["automate", "bass", "width"]);
    assert!(
        stderr(&o).contains("`width` is not a target"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn clear_and_write_are_one_undo_point() {
    let b = FakeBridge::start_with("automate-undo", band);
    ok(&b.mort(&["automate", "bass", "volume", "1.1=-6", "3.1=0"]));
    ok(&b.mort(&[
        "automate", "bass", "volume", "--clear", "1.1..4.1", "2.1=-3",
    ]));
    let t = ok(&b.mort(&["undo"]));
    assert!(
        t.starts_with("undid mort: automate Bass volume: cleared 1.1..4.1, wrote 1 point"),
        "{t}"
    );
    assert!(ok(&b.mort(&["automate", "bass", "volume"])).contains("2 points"));
}

fn smf_bytes() -> Vec<u8> {
    // Format 0, 480 ticks per quarter: C3 for a beat, then a G3 for half a beat.
    let track = [
        0x00, 0x90, 48, 100, 0x83, 0x60, 0x80, 48, 0, 0x00, 0x90, 55, 80, 0x81, 0x70, 55, 0, 0x00,
        0xFF, 0x2F, 0x00,
    ];
    let mut f = b"MThd".to_vec();
    f.extend(6u32.to_be_bytes());
    f.extend(0u16.to_be_bytes());
    f.extend(1u16.to_be_bytes());
    f.extend(480u16.to_be_bytes());
    f.extend(b"MTrk");
    f.extend(u32::try_from(track.len()).unwrap().to_be_bytes());
    f.extend(track);
    f
}

#[test]
fn midi_write_list_and_replace() {
    let b = FakeBridge::start_with("midi", band);
    assert!(ok(&b.mort(&["midi", "bass"])).starts_with("Bass: no MIDI"));

    // A chord at 2.1 (+0b stacks), then a note a beat later, and a CC sweep.
    let t = ok(&b.mort(&[
        "midi",
        "bass",
        "2.1..3.1",
        "C2:2.1:2b:90",
        "E2:+0b:2b",
        "G2:+1b:1b:70:3",
        "cc1:2.1=0",
        "cc1:+3b=127",
    ]));
    assert!(
        t.starts_with("Bass: wrote 3 notes, 2 CCs into a new item"),
        "{t}"
    );
    assert!(t.contains("item 2.1–3.1, 3 notes, 2 CCs"), "{t}");
    let has = |words: &[&str]| {
        t.lines()
            .any(|l| l.split_whitespace().collect::<Vec<_>>() == words)
    };
    assert!(has(&["2.1–2.3", "C2", "(36)", "v90", "ch1"]), "{t}");
    assert!(has(&["2.1–2.3", "E2", "(40)", "v100", "ch1"]), "{t}");
    assert!(has(&["2.2–2.3", "G2", "(43)", "v70", "ch3"]), "{t}");
    assert!(has(&["2.4", "cc1", "=", "127", "ch1"]), "{t}");

    // Replacing 2.1..2.2 removes the chord and the first CC, keeps G2, and
    // writes into the same item.
    let t = ok(&b.mort(&["midi", "bass", "2.1..2.2", "--replace", "A1:2.1:1b"]));
    assert!(t.starts_with("Bass: wrote 1 note, 0 CCs\n"), "{t}");
    let t = ok(&b.mort(&["midi", "bass"]));
    assert!(
        t.contains("2 notes, 1 CCs")
            && t.contains("A1 (33)")
            && t.contains("G2 (43)")
            && !t.contains("C2"),
        "{t}"
    );

    let t = ok(&b.mort(&["undo"]));
    assert!(
        t.starts_with("undid mort: MIDI on Bass: replace with 1 note, 0 CCs"),
        "{t}"
    );
    assert!(ok(&b.mort(&["midi", "bass"])).contains("3 notes, 2 CCs"));

    let t = ok(&b.mort(&["midi", "bass", "2.2..2.4"]));
    assert!(
        t.contains("1 notes, 0 CCs") && t.contains("G2"),
        "range filters by start: {t}"
    );
}

#[test]
fn midi_from_files() {
    let b = FakeBridge::start_with("midi-files", band);
    let dir = tmp("midi-files");
    let mid = dir.join("riff.mid");
    std::fs::write(&mid, smf_bytes()).unwrap();
    let t = ok(&b.mort(&["midi", "verb", "3.1..4.1", "--file", mid.to_str().unwrap()]));
    assert!(t.contains("wrote 2 notes, 0 CCs"), "{t}");
    let has = |words: &[&str]| {
        t.lines()
            .any(|l| l.split_whitespace().collect::<Vec<_>>() == words)
    };
    assert!(has(&["3.1–3.2", "C3", "(48)", "v100", "ch1"]), "{t}");
    assert!(has(&["3.2–3.2.480", "G3", "(55)", "v80", "ch1"]), "{t}");

    let txt = dir.join("chords.txt");
    std::fs::write(&txt, "# a chord\nC4:1.1:4b E4:+0b:4b\nG4:+0b:4b  # top\n").unwrap();
    let t = ok(&b.mort(&["midi", "verb", "--file", txt.to_str().unwrap()]));
    assert!(
        t.contains("wrote 3 notes") && t.contains("1.1–2.1  G4 (67)"),
        "{t}"
    );

    let o = b.mort(&["midi", "verb", "--file", mid.to_str().unwrap()]);
    assert!(
        stderr(&o).contains("a MIDI file needs a range"),
        "{}",
        stderr(&o)
    );
    let o = b.mort(&["midi", "verb", "1.1..2.1", "Q4:1.1"]);
    assert!(stderr(&o).contains("`Q4` is not a pitch"), "{}", stderr(&o));
    let o = b.mort(&["midi", "verb", "C4:1.1", "--replace"]);
    assert!(
        stderr(&o).contains("--replace needs a range"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn midi_follows_tempo_changes() {
    let b = FakeBridge::start_with("midi-tempo", |s| {
        band(s);
        s.set_tempo_map(
            mort_proto::SteppedTempoMap::new(vec![
                mort_proto::TempoChange {
                    bar: 1,
                    bpm: 120.0,
                    num: 4,
                    den: 4,
                },
                mort_proto::TempoChange {
                    bar: 3,
                    bpm: 60.0,
                    num: 4,
                    den: 4,
                },
            ])
            .unwrap(),
        );
    });
    let v: Value =
        serde_json::from_str(&stdout(&b.mort(&["--json", "midi", "bass", "C4:2.4:2b"]))).unwrap();
    let item = v["item"].clone();
    let read: Value = serde_json::from_str(&stdout(&b.mort(&["--json", "midi", "bass"]))).unwrap();
    let n = &read["items"][0]["notes"][0];
    // Beat 2.4 is 3.5 s at 120 bpm; a beat later the tempo halves, so the
    // second beat lasts a second: 3.5 + 0.5 + 1.0.
    assert_eq!(
        (n["start"].as_f64().unwrap(), n["end"].as_f64().unwrap()),
        (3.5, 5.0)
    );
    assert_eq!(read["items"][0]["item"]["id"], item);
}

#[test]
fn render_play_and_stop() {
    let b = FakeBridge::start_with("transport", band);
    let t = ok(&b.mort(&["render", "1.1..3.1"]));
    assert!(t.starts_with("rendered 1.1..3.1 (4.00 s of audio"), "{t}");
    let t = ok(&b.mort(&["play", "1.1..2.1"]));
    assert!(
        t.starts_with("played 1.1..2.1 (heard by the map as an audition)"),
        "{t}"
    );

    // A long play, stopped from outside: the job sees it stop early.
    let t = ok(&b.mort(&["play", "1.1..200.1", "--no-wait"]));
    let job = t
        .split("job ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    assert!(ok(&b.mort(&["status"])).contains("playing  yes"));
    let t = ok(&b.mort(&["stop"]));
    assert!(t.starts_with("stopped at "), "{t}");
    let v: Value = serde_json::from_str(&stdout(&b.mort(&[
        "--json",
        "call",
        "job.wait",
        &format!(r#"{{"id": {job}, "timeout_ms": 5000}}"#),
    ])))
    .unwrap();
    assert_eq!(v["state"], "done");
    assert_eq!(v["result"]["stopped_early"], true);

    let o = b.mort(&["render", "3.1..1.1"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("the end must be after the start"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn probes_ensure_and_list() {
    let b = FakeBridge::start_with("probes", band);
    let t = ok(&b.mort(&["probes"]));
    assert!(
        t.starts_with("0 probes")
            && t.contains("no probe: MASTER, Drums, Kick, Bass, Bass DI, Verb"),
        "{t}"
    );

    let t = ok(&b.mort(&["probes", "ensure", "bass", "verb", "--no-master"]));
    assert!(
        t.starts_with("probes: 2 added, 0 moved to the end, 0 duplicates removed"),
        "{t}"
    );
    let fx = ok(&b.mort(&["fx", "bass"]));
    assert!(
        fx.contains("3  CLAP: mort probe (Vulpus Labs)"),
        "the probe goes last: {fx}"
    );

    // An FX added after the probe pushes it off the end; ensure moves it back,
    // and removes a second probe.
    ok(&b.mort(&["fx", "add", "bass", "ReaEQ"]));
    ok(&b.mort(&[
        "fx",
        "add",
        "bass",
        "CLAP: mort probe (Vulpus Labs)",
        "--at",
        "1",
    ]));
    let t = ok(&b.mort(&["probes", "ensure"]));
    assert!(
        t.starts_with("probes: 4 added, 1 moved to the end, 1 duplicates removed"),
        "{t}"
    );
    let fx = ok(&b.mort(&["fx", "bass"]));
    assert!(
        fx.starts_with("FX on Bass (4)") && fx.contains("4  CLAP: mort probe"),
        "{fx}"
    );
    let t = ok(&b.mort(&["probes"]));
    assert!(t.starts_with("6 probes") && !t.contains("no probe:"), "{t}");
    assert!(
        t.contains("detached"),
        "the fake's probes have no bridge slot: {t}"
    );
    assert!(ok(&b.mort(&["undo"])).starts_with("undid mort: ensure probes"));
}

#[test]
fn install_bundles_the_probe_dylib() {
    let root = tmp("bundle");
    let (from, plugins, clap) = (
        root.join("target"),
        root.join("UserPlugins"),
        root.join("CLAP"),
    );
    std::fs::create_dir_all(&from).unwrap();
    std::fs::write(from.join("libmort_bridge.dylib"), "bridge").unwrap();
    std::fs::write(from.join("libmort_probe.dylib"), "probe").unwrap();
    let t = ok(&install(&from, &plugins, &clap));
    assert!(
        t.contains("probe   ") && t.contains("mort-probe.clap"),
        "{t}"
    );
    let b = clap.join("mort-probe.clap/Contents");
    assert_eq!(
        std::fs::read_to_string(b.join("MacOS/mort-probe")).unwrap(),
        "probe"
    );
    assert!(
        std::fs::read_to_string(b.join("Info.plist"))
            .unwrap()
            .contains("com.vulpuslabs.mort.probe")
    );
    assert_eq!(
        std::fs::read_to_string(b.join("PkgInfo")).unwrap(),
        "BNDL????"
    );
}

fn band_with_sound(s: &mut FakeSession) {
    use mort_session::{Session, Source};
    band(s);
    let kick = s
        .tracks()
        .into_iter()
        .find(|t| t.name == "Kick")
        .unwrap()
        .id;
    s.add_sound(kick, Source::Sine { hz: 55.0, db: -6.0 }, None)
        .unwrap();
}

#[test]
fn measure_and_map() {
    let b = FakeBridge::start_with("measure", band_with_sound);
    let t = ok(&b.mort(&["map"]));
    assert!(t.starts_with("the map is empty"), "{t}");

    let o = b.mort(&["measure", "kick"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("give a range"), "{}", stderr(&o));

    let t = ok(&b.mort(&["measure", "1.1..2.1"]));
    assert!(t.starts_with("measured 0.00–2.00s by render"), "{t}");
    assert!(t.contains("capture 1"), "{t}");
    for name in ["MASTER", "Drums", "Kick", "Bass DI", "Verb"] {
        assert!(
            t.lines()
                .any(|l| l.trim_start().starts_with(name) && l.ends_with("40")),
            "{name} measured: {t}"
        );
    }
    assert!(t.contains("not measured: Bass: it is muted"), "{t}");
    assert!(t.contains("in memory only"), "{t}");

    let t = ok(&b.mort(&["map"]));
    assert!(t.starts_with("1 captures"), "{t}");
    let kick = t
        .lines()
        .find(|l| l.trim_start().starts_with("Kick"))
        .unwrap();
    assert!(
        kick.contains("0.00–2.00s") && kick.trim_end().ends_with('-'),
        "{t}"
    );

    ok(&b.mort(&["track", "set", "kick", "--volume", "-6"]));
    let t = ok(&b.mort(&["map"]));
    for name in ["MASTER", "Drums", "Kick", "Verb"] {
        let l = t
            .lines()
            .find(|l| l.trim_start().starts_with(name))
            .unwrap();
        assert!(l.contains("(Kick changed)"), "{name} stale: {t}");
    }
    let bass = t
        .lines()
        .find(|l| l.trim_start().starts_with("Bass "))
        .unwrap();
    assert!(bass.trim_end().ends_with('-'), "{t}");
    assert!(t.contains("4 tracks stale"), "{t}");

    let t = ok(&b.mort(&["measure", "--stale"]));
    assert!(t.contains("capture 2"), "{t}");
    assert!(!t.contains("Bass"), "only what was stale: {t}");
    let t = ok(&b.mort(&["map"]));
    assert!(
        t.starts_with("2 captures") && !t.contains("changed)") && !t.contains("tracks stale"),
        "{t}"
    );
    let t = ok(&b.mort(&["measure", "--stale"]));
    assert!(t.starts_with("nothing is stale"), "{t}");
}

/// Twenty tracks of pink noise 0.85 octave wide, 0.43 octave apart, alternating 12 dB in level so that neighbours never compete; the
/// twentieth doubles the eleventh, so that those two mask each other. One
/// region.
fn twenty(s: &mut FakeSession) {
    use mort_session::{NewTrack, Session, Source};
    for i in 0..20 {
        let k = if i == 19 { 10 } else { i };
        let lo = 40.0 * 1.35f64.powi(k);
        let db = if k % 2 == 0 { -30.0 } else { -42.0 };
        let name = if i == 19 {
            "Double".to_owned()
        } else {
            format!("Part {i:02}")
        };
        let t = s
            .add_track(NewTrack {
                name,
                ..NewTrack::default()
            })
            .unwrap();
        s.add_sound(
            t,
            Source::Pink {
                lo,
                hi: lo * 1.8,
                db,
            },
            None,
        )
        .unwrap();
    }
    s.add_region("verse", 0.0, Some(2.0));
}

#[test]
fn summary_findings_report_and_frames() {
    let b = FakeBridge::start_with("report", twenty);
    let o = b.mort(&["findings"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("the map is empty"), "{}", stderr(&o));
    ok(&b.mort(&["measure", "1.1..2.1"]));

    let t = ok(&b.mort(&["summary"]));
    assert!(t.starts_with("verse 0.00–2.00s  "), "{t}");
    assert!(
        t.lines().count() <= 30,
        "a section of 20 tracks stays short:\n{t}"
    );
    assert!(t.contains("findings: masking"), "{t}");
    assert!(
        t.lines().any(|l| l.trim_start().starts_with("Double")),
        "{t}"
    );

    let t = ok(&b.mort(&["findings", "--kind", "masking"]));
    assert!(t.starts_with("verse 0.00–2.00s\n"), "{t}");
    assert!(t.contains("Part 10 and Double compete at"), "{t}");
    assert!(t.contains("\n1 finding\n"), "only the planted pair: {t}");

    // Everything, as if each measurement were a finding, is cut short.
    let t = ok(&b.mort(&["findings", "--all"]));
    assert!(t.contains("more: --limit 0 shows all"), "{t}");
    assert!(t.lines().count() < 30, "{t}");

    let t = ok(&b.mort(&["findings", "--thresholds"]));
    assert!(
        t.contains("mud_excess") && t.contains("200-500 Hz part"),
        "{t}"
    );

    let t = ok(&b.mort(&["report", "double"]));
    assert!(t.starts_with("Double\nverse 0.00–2.00s  "), "{t}");
    assert!(t.contains("share of MASTER:"), "{t}");

    let t = ok(&b.mort(&["frames", "double", "1.1..1.3", "-f", "rms_dbfs,band:1k"]));
    let rows: Vec<&str> = t.lines().collect();
    assert!(
        rows[0].contains("rms_dbfs") && rows[0].contains("band:1k"),
        "{t}"
    );
    assert_eq!(
        rows.len(),
        1 + 2 + 1,
        "a header, two half-second rows, state: {t}"
    );

    let o = b.mort(&["frames", "double", "1.1..1.3", "-f", "loudness"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("no feature `loudness`"),
        "{}",
        stderr(&o)
    );
}

/// Two pads stacked in the same range, so they mask each other, over a
/// bass and keys.
fn stacked_pads(s: &mut FakeSession) {
    use mort_session::{NewTrack, Session, Source};
    for (name, lo, hi, db) in [
        ("Bass", 40.0, 150.0, -26.0),
        ("Pad", 150.0, 600.0, -26.0),
        ("Keys", 600.0, 2500.0, -26.0),
        ("Pad 2", 150.0, 600.0, -27.0),
    ] {
        let t = s
            .add_track(NewTrack {
                name: name.into(),
                ..NewTrack::default()
            })
            .unwrap();
        s.add_sound(t, Source::Pink { lo, hi, db }, None).unwrap();
    }
}

fn script(name: &str, text: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("mort-script-{}-{name}.kdl", std::process::id()));
    std::fs::write(&p, text).unwrap();
    p
}

#[test]
fn a_script_fixes_masking_and_compare_shows_it() {
    let b = FakeBridge::start_with("script", stacked_pads);
    ok(&b.mort(&["measure", "1.1..3.1"]));
    let t = ok(&b.mort(&["findings"]));
    assert!(t.contains("Pad and Pad 2 compete at"), "{t}");

    let fix = script(
        "fix",
        r#"// Cut Pad 2 where it fights Pad, then measure again.
fx add "Pad 2" ReaEQ
set "Pad 2" ReaEQ "Freq-Band" "300 Hz"
set "Pad 2" ReaEQ "Gain-Band" "-24 dB"
set "Pad 2" ReaEQ "Q-Band" "0.3"
measure "1.1..3.1"
compare
"#,
    );
    let t = ok(&b.mort(&["run", fix.to_str().unwrap()]));
    assert!(
        t.contains("> fx add \"Pad 2\" ReaEQ\n  held for the batch"),
        "{t}"
    );
    assert!(
        t.contains("> measure 1.1..3.1\n  measured"),
        "the batch goes before the job: {t}"
    );
    assert!(t.contains("capture 1 → 2"), "{t}");
    let gone = t.split("findings gone:").nth(1).unwrap_or("");
    assert!(gone.contains("Pad and Pad 2 compete"), "{t}");
    assert!(!t.contains("findings new:"), "{t}");
    let pad2 = t
        .lines()
        .find(|l| l.trim_start().starts_with("Pad 2 ") && l.contains('→'))
        .unwrap();
    assert!(
        pad2.contains("315 Hz -"),
        "the cut shows in Pad 2's bands: {t}"
    );

    // The four edits were one undo point. (The EQ went in after Pad 2's
    // probe, so measuring moved the probe back to the end: that is an undo
    // point of its own, on top.)
    let fx = ok(&b.mort(&["fx", "pad 2"]));
    assert!(fx.contains("ReaEQ"), "{fx}");
    let t = ok(&b.mort(&["undo"]));
    assert!(t.contains("mort: add probes to measure"), "{t}");
    let t = ok(&b.mort(&["undo"]));
    assert!(t.contains("mort: script mort-script"), "{t}");
    let fx = ok(&b.mort(&["fx", "pad 2"]));
    assert!(
        !fx.contains("ReaEQ"),
        "one undo removes the whole batch: {fx}"
    );
}

#[test]
fn a_failed_step_stops_the_script_and_sends_nothing() {
    let b = FakeBridge::start_with("script-fail", stacked_pads);
    let bad = script(
        "bad",
        r#"track add Lead
fx add Lead ReaEQ
set Lead ReaEQ "Nonsense" "3 dB"
track add Never
"#,
    );
    let o = b.mort(&["run", bad.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    let e = stderr(&o);
    assert!(
        e.contains("step 3 (line 3: `set Lead ReaEQ Nonsense \"3 dB\"`)"),
        "{e}"
    );
    assert!(e.contains("no such parameter"), "{e}");
    let tracks = ok(&b.mort(&["tracks"]));
    assert!(
        !tracks.contains("Lead") && !tracks.contains("Never"),
        "nothing sent: {tracks}"
    );

    // A usage error is caught before anything runs.
    let typo = script("typo", "track add Lead\ntrack frobnicate x\n");
    let o = b.mort(&["run", typo.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).contains("line 2: `track frobnicate x`"),
        "{}",
        stderr(&o)
    );
    assert!(!ok(&b.mort(&["tracks"])).contains("Lead"));

    // `commit` sends what is held: two batches, two undo points.
    let two = script("two", "track add One\ncommit\ntrack add Two\n");
    let t = ok(&b.mort(&["run", two.to_str().unwrap(), "--summary", "two steps"]));
    assert_eq!(t.matches("as one batch").count(), 2, "{t}");
    ok(&b.mort(&["undo"]));
    let tracks = ok(&b.mort(&["tracks"]));
    assert!(
        tracks.contains("One") && !tracks.contains("Two"),
        "{tracks}"
    );
}

#[test]
fn regions_and_markers() {
    let b = FakeBridge::start_with("regions", |_| {});
    let t = ok(&b.mort(&["region", "add", "verse", "1.1..5.1"]));
    assert!(t.starts_with("added region 1 `verse` at 0.00–8.00s"), "{t}");
    let t = ok(&b.mort(&["region", "add", "drop", "3.1"]));
    assert!(t.starts_with("added marker 2 `drop` at 4.00s"), "{t}");
    let o = b.mort(&["call", "project.get"]);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["regions"][0]["name"], "verse");
    assert_eq!(v["regions"][0]["end"], 8.0);
    assert!(v["regions"][1]["end"].is_null());
    let t = ok(&b.mort(&["undo"]));
    assert!(t.contains("mort: add marker drop"), "{t}");
}

#[test]
fn thresholds_set_and_reset() {
    let b = FakeBridge::start_with("thresholds", |_| {});
    let t = ok(&b.mort(&["thresholds"]));
    assert!(
        t.contains("mud_excess") && t.contains("overrides in memory only"),
        "{t}"
    );
    let t = ok(&b.mort(&["thresholds", "set", "headroom", "-2"]));
    assert!(t.starts_with("headroom = -2\n"), "{t}");
    let line = t
        .lines()
        .find(|l| l.starts_with("headroom") && l.contains("dBTP"))
        .unwrap();
    assert!(line.contains("-2 dBTP *"), "{t}");
    let t = ok(&b.mort(&["thresholds", "reset", "headroom"]));
    assert!(
        t.lines()
            .any(|l| l.starts_with("headroom") && l.contains("-1 dBTP ")),
        "{t}"
    );
    let o = b.mort(&["thresholds", "set", "nonsense", "1"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("no threshold `nonsense`"),
        "{}",
        stderr(&o)
    );
}
