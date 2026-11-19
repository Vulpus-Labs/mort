//! The connection to the bridge (ADR 0002, ADR 0009).

use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use mort_proto::{ErrorObject, Message, PROTOCOL_VERSION, Request, RequestId};

/// How long to wait for a reply before giving up.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(15);

/// Why a command failed, and which exit code that means (ADR 0009 §6).
#[derive(Debug)]
pub enum Failure {
    /// The bridge answered with an error. Exit 1.
    Bridge(ErrorObject),
    /// The bridge could not be reached or stopped answering. Exit 3.
    NoBridge(String),
    /// Something else went wrong on this side. Exit 1.
    Local(String),
}

impl Failure {
    /// The process exit code.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Failure::Bridge(_) | Failure::Local(_) => 1,
            Failure::NoBridge(_) => 3,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Bridge(e) => write!(f, "{e}"),
            Failure::NoBridge(m) | Failure::Local(m) => write!(f, "{m}"),
        }
    }
}

/// The socket named by `$TMPDIR/mort/current`.
///
/// # Errors
///
/// [`Failure::NoBridge`] if no bridge has recorded itself.
pub fn current_socket() -> Result<PathBuf, Failure> {
    let current = std::env::temp_dir().join("mort").join("current");
    std::fs::read_to_string(&current)
        .map(|s| PathBuf::from(s.trim()))
        .map_err(|_| no_bridge(&format!("{} does not exist", current.display())))
}

fn no_bridge(why: &str) -> Failure {
    Failure::NoBridge(format!(
        "no bridge running ({why}). Start REAPER with the bridge installed (`mort install`)"
    ))
}

/// Edits held back to send as one `batch` (`mort run`, ADR 0009 §4).
#[derive(Debug, Default)]
struct Recording {
    /// The step being run, as the script names it.
    step: String,
    /// The undo label for batches sent before a job.
    label: String,
    /// Each recorded call: the step it came from, its method and params.
    calls: Vec<(String, String, Value)>,
    /// The state the first recorded edit read.
    expect_state: Option<Value>,
}

/// Methods that must see every earlier edit, so they send the batch first.
const FLUSH_FIRST: [&str; 6] = ["measure", "play", "render", "stop", "undo", "redo"];

/// A connection to the bridge.
pub struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next: i64,
    path: PathBuf,
    recording: Option<Recording>,
}

impl Client {
    /// Connects and checks the protocol version with `bridge.hello`.
    ///
    /// # Errors
    ///
    /// [`Failure::NoBridge`] if nothing answers at `path`;
    /// [`Failure::Bridge`] if the versions do not match.
    pub fn connect(path: &Path) -> Result<(Client, Value), Failure> {
        let writer = UnixStream::connect(path)
            .map_err(|e| no_bridge(&format!("cannot connect to {}: {e}", path.display())))?;
        writer
            .set_read_timeout(Some(REPLY_TIMEOUT))
            .map_err(|e| Failure::Local(e.to_string()))?;
        let reader = BufReader::new(
            writer
                .try_clone()
                .map_err(|e| Failure::Local(e.to_string()))?,
        );
        let mut c = Client {
            writer,
            reader,
            next: 1,
            path: path.to_owned(),
            recording: None,
        };
        let hello = c.call(
            "bridge.hello",
            json!({ "protocol": [PROTOCOL_VERSION.0, PROTOCOL_VERSION.1] }),
        )?;
        Ok((c, hello))
    }

    /// Calls a method and waits for its reply. Notifications that arrive
    /// meanwhile are skipped.
    ///
    /// # Errors
    ///
    /// [`Failure::Bridge`] for an error reply; [`Failure::NoBridge`] if the
    /// bridge goes away or does not answer in time.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, Failure> {
        self.call_within(method, params, REPLY_TIMEOUT)
    }

    /// From now on, holds edits back to send as one batch: each is
    /// recorded and answered with a placeholder result. Reads still go to
    /// the bridge, but `tracks.list` shows the tracks and FX the held
    /// edits add, with ids such as `$0.track` that the bridge fills in
    /// when it runs the batch. Jobs and undo send the batch first.
    pub fn record(&mut self, label: &str) {
        self.recording = Some(Recording {
            label: label.to_owned(),
            ..Recording::default()
        });
    }

    /// Names the step that the next calls belong to, for error messages.
    pub fn set_step(&mut self, step: &str) {
        if let Some(r) = self.recording.as_mut() {
            step.clone_into(&mut r.step);
        }
    }

    /// How many edits are held.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.recording.as_ref().map_or(0, |r| r.calls.len())
    }

    /// Sends the held edits as one batch with one undo point labelled
    /// `summary`. Returns its result, or `None` if nothing was held.
    ///
    /// # Errors
    ///
    /// The bridge's error, its message naming the step whose call failed.
    pub fn flush(&mut self, summary: &str) -> Result<Option<Value>, Failure> {
        let Some(rec) = self.recording.as_mut() else {
            return Ok(None);
        };
        if rec.calls.is_empty() {
            return Ok(None);
        }
        let calls = std::mem::take(&mut rec.calls);
        let expect = rec.expect_state.take();
        let held = self.recording.take();
        let body: Vec<Value> = calls
            .iter()
            .map(|(_, m, p)| json!({ "method": m, "params": p }))
            .collect();
        let mut params = json!({ "calls": body, "summary": summary });
        if let Some(e) = expect {
            params["expect_state"] = e;
        }
        let r = self.call("batch", params);
        self.recording = held;
        r.map(Some).map_err(|f| match f {
            Failure::Bridge(mut e) => {
                // "batch call 3 (`fx.add`): …" names the call; name the step.
                let i = e
                    .message
                    .strip_prefix("batch call ")
                    .and_then(|m| m.split(' ').next())
                    .and_then(|n| n.parse::<usize>().ok());
                if let Some((step, _, _)) = i.and_then(|i| calls.get(i)) {
                    let rest = e
                        .message
                        .split_once("): ")
                        .map_or(e.message.as_str(), |x| x.1);
                    e.message = format!("{step}: {rest}");
                }
                Failure::Bridge(e)
            }
            other => other,
        })
    }

    /// Handles a call while recording: `Some` answers it without asking
    /// the bridge. A job sends the held edits first; if it checks
    /// `expect_state`, it now expects the state they left, since it read
    /// the project before they were made.
    fn recorded(&mut self, method: &str, params: &mut Value) -> Result<Option<Value>, Failure> {
        if FLUSH_FIRST.contains(&method) {
            let label = self
                .recording
                .as_ref()
                .map(|r| r.label.clone())
                .unwrap_or_default();
            if let Some(r) = self.flush(&label)?
                && params.get("expect_state").is_some()
            {
                params["expect_state"] = r["state"].clone();
            }
            return Ok(None);
        }
        if method == "tracks.list" && self.pending() > 0 {
            let held = self.recording.take();
            let live = self.call(method, params.clone());
            self.recording = held;
            let mut list = live?;
            if let Some(r) = &self.recording {
                overlay(&mut list, &r.calls);
            }
            return Ok(Some(list));
        }
        if method == "fx.params" && params["fx"].as_str().is_some_and(|f| f.starts_with('$')) {
            return Err(Failure::Local(
                "this FX is added earlier in the same batch, so its parameters cannot be read yet: \
                 name the parameter rather than numbering it, or put `commit` before this step"
                    .into(),
            ));
        }
        let rec = self.recording.as_mut().expect("recording");
        let mut add = |m: &str, p: &Value| {
            if rec.expect_state.is_none() && !p["expect_state"].is_null() {
                rec.expect_state = Some(p["expect_state"].clone());
            }
            rec.calls.push((rec.step.clone(), m.to_owned(), p.clone()));
            rec.calls.len() - 1
        };
        if method == "batch" {
            let calls = params["calls"].as_array().cloned().unwrap_or_default();
            // The batch's own state stands for its calls'.
            let state = &params["expect_state"];
            let out: Vec<Value> = calls
                .iter()
                .map(|c| {
                    let mut p = c["params"].clone();
                    if p["expect_state"].is_null() && !state.is_null() {
                        p["expect_state"] = state.clone();
                    }
                    add(c["method"].as_str().unwrap_or(""), &p);
                    json!({ "pending": true })
                })
                .collect();
            return Ok(Some(Value::Array(out)));
        }
        if params.get("summary").is_none() {
            return Ok(None);
        }
        let k = add(method, params);
        Ok(Some(match method {
            "track.add" => json!({ "track": format!("${k}.track"), "pending": true }),
            "fx.add" => json!({ "fx": {
                "id": format!("${k}.fx.id"),
                "name": params["plugin"],
                "plugin": params["plugin"],
                "enabled": true,
            }, "pending": true }),
            _ => json!({ "pending": true }),
        }))
    }

    /// Calls a method, waiting up to `timeout` for the reply. Renders hold
    /// REAPER's main thread, so waiting on a job needs longer than usual.
    ///
    /// # Errors
    ///
    /// As [`Client::call`].
    pub fn call_within(
        &mut self,
        method: &str,
        mut params: Value,
        timeout: Duration,
    ) -> Result<Value, Failure> {
        if self.recording.is_some()
            && let Some(v) = self.recorded(method, &mut params)?
        {
            return Ok(v);
        }
        self.writer
            .set_read_timeout(Some(timeout))
            .map_err(|e| Failure::Local(e.to_string()))?;
        let id = RequestId::Number(self.next);
        self.next += 1;
        let line = Message::Request(Request::new(id.clone(), method, params)).to_line();
        self.writer
            .write_all(line.as_bytes())
            .and_then(|()| self.writer.write_all(b"\n"))
            .map_err(|e| self.lost(&e))?;
        loop {
            let mut reply = String::new();
            match self.reader.read_line(&mut reply) {
                Ok(0) => return Err(no_bridge("the bridge closed the connection")),
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(Failure::NoBridge(format!(
                        "the bridge at {} did not answer `{method}` within {} s. REAPER runs \
                         mort only when it is idle: close any open dialog in REAPER and try again",
                        self.path.display(),
                        timeout.as_secs()
                    )));
                }
                Err(e) => return Err(self.lost(&e)),
            }
            match Message::parse(&reply) {
                Ok(Message::Response(r)) if r.id.as_ref() == Some(&id) => {
                    return r.outcome.map_err(Failure::Bridge);
                }
                Ok(Message::Response(r)) if r.id.is_none() => {
                    return Err(Failure::Bridge(r.outcome.err().unwrap_or_else(|| {
                        ErrorObject::internal("a reply with no id and no error")
                    })));
                }
                // Notifications, and replies to other requests.
                Ok(_) => {}
                Err(e) => {
                    return Err(Failure::Local(format!(
                        "the bridge sent something unreadable: {e}"
                    )));
                }
            }
        }
    }

    fn lost(&self, e: &io::Error) -> Failure {
        no_bridge(&format!(
            "lost the connection to {}: {e}",
            self.path.display()
        ))
    }
}

/// Shows held `track.add` and `fx.add` calls in a `tracks.list` result.
fn overlay(list: &mut Value, calls: &[(String, String, Value)]) {
    for (k, (_, method, p)) in calls.iter().enumerate() {
        match method.as_str() {
            "track.add" => {
                if let Some(ts) = list["tracks"].as_array_mut() {
                    let index = ts.len();
                    ts.push(json!({
                        "id": format!("${k}.track"),
                        "index": index,
                        "name": p["name"],
                        "parent": p["parent"],
                        "main_send": true,
                        "volume": 1.0,
                        "pan": 0.0,
                        "mute": false,
                        "solo": false,
                        "sends": [],
                        "fx": [],
                    }));
                }
            }
            "fx.add" => {
                let master_matches = list["master"]["id"] == p["track"];
                let track = if master_matches {
                    Some(&mut list["master"])
                } else {
                    list["tracks"]
                        .as_array_mut()
                        .and_then(|ts| ts.iter_mut().find(|t| t["id"] == p["track"]))
                };
                if let Some(fx) = track.and_then(|t| t["fx"].as_array_mut()) {
                    let index = fx.len();
                    fx.push(json!({
                        "id": format!("${k}.fx.id"),
                        "index": index,
                        "name": p["plugin"],
                        "plugin": p["plugin"],
                        "enabled": true,
                    }));
                }
            }
            _ => {}
        }
    }
}
