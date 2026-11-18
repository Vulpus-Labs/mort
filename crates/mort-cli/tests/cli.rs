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
                d.set_held(true);
                let mut session = FakeSession::new();
                ready_tx.send(()).unwrap();
                while !stop.load(Ordering::SeqCst) {
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
