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
    let first: u64 = t
        .split("run ")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();

    let t = ok(&b.mort(&["play", "1.1..2.1"]));
    assert!(
        t.starts_with(&format!("played 1.1..2.1, run {}", first + 1)),
        "runs count up: {t}"
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
