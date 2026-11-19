//! The main-thread dispatcher (ADR 0002 §2–4, ADR 0004 §1–4).
//!
//! The socket threads put [`Inbound`] messages on a channel. REAPER's timer
//! calls [`Dispatcher::tick`] on the main thread, which:
//!
//! 1. drains requests until the tick's budget is spent (at least one runs);
//! 2. polls the running job, and starts the next queued one when it ends;
//! 3. answers `job.wait` calls whose job has finished or whose time is up;
//! 4. forgets finished jobs after [`JOB_RESULT_TTL`];
//! 5. tells subscribers if the project's state count moved.
//!
//! Methods are registered with a flag saying whether they edit. Edits are
//! refused while edits are held, checked against `expect_state`, and
//! wrapped in one undo point labelled `mort: <summary>`. A failed edit is
//! rolled back.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use mort_proto::{
    BatchParams, ErrorKind, ErrorObject, Message, Notification, PROTOCOL_VERSION, Request,
    RequestId, Response,
};
use mort_session::Session;

/// How long a finished job's result is kept for `job.status` and
/// `job.wait`.
pub const JOB_RESULT_TTL: Duration = Duration::from_mins(10);

/// How long `job.wait` waits when the client gives no timeout.
pub const DEFAULT_WAIT: Duration = Duration::from_mins(1);

/// The bridge's build, reported by `bridge.hello`.
pub const BUILD: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("MORT_GIT_HASH"));

/// A client connection, numbered by the server.
pub type ConnId = u64;

/// What the socket threads hand to the main thread.
#[derive(Debug)]
pub enum Inbound {
    /// A client connected. Lines sent on `tx` go to it.
    Opened {
        /// The connection.
        conn: ConnId,
        /// Where to send its replies and notifications.
        tx: Sender<String>,
    },
    /// A request from a client.
    Request {
        /// The connection.
        conn: ConnId,
        /// The request.
        request: Request,
    },
    /// A client disconnected.
    Closed {
        /// The connection.
        conn: ConnId,
    },
}

/// What a method handler returns.
pub enum Outcome<S> {
    /// An immediate answer.
    Value(Result<Value, ErrorObject>),
    /// A job to queue. The client gets its id at once.
    Job(Box<dyn Job<S>>),
}

/// The state handed to a method handler.
pub struct Ctx<'a, S> {
    /// The project.
    pub session: &'a mut S,
}

/// A method handler.
pub type Handler<S> = Box<dyn Fn(&mut Ctx<'_, S>, Value) -> Outcome<S>>;

/// A registered method.
pub struct Method<S> {
    /// Whether it edits the project (ADR 0004).
    pub edits: bool,
    /// The handler.
    pub handler: Handler<S>,
}

/// What a job reports when polled.
#[derive(Debug, Clone, PartialEq)]
pub enum JobPoll {
    /// Still going, with optional progress to report.
    Running(Option<Value>),
    /// Finished with a result.
    Done(Value),
    /// Failed.
    Failed(ErrorObject),
}

/// Long-running work: playing, rendering, measuring (ADR 0002 §3).
pub trait Job<S> {
    /// Starts the work. Called on the tick the job leaves the queue.
    ///
    /// # Errors
    ///
    /// If the work cannot start; the job then fails with this error.
    fn start(&mut self, session: &mut S) -> Result<(), ErrorObject>;
    /// Checks on the work. Called once per tick while running.
    fn poll(&mut self, session: &mut S) -> JobPoll;
    /// Stops the work early.
    fn cancel(&mut self, session: &mut S);
}

#[derive(Debug, Clone, PartialEq)]
enum JobState {
    Queued,
    Running(Option<Value>),
    Done(Value),
    Failed(ErrorObject),
    Cancelled,
}

impl JobState {
    fn finished(&self) -> bool {
        matches!(
            self,
            JobState::Done(_) | JobState::Failed(_) | JobState::Cancelled
        )
    }

    fn name(&self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running(_) => "running",
            JobState::Done(_) => "done",
            JobState::Failed(_) => "failed",
            JobState::Cancelled => "cancelled",
        }
    }
}

struct JobEntry<S> {
    method: String,
    state: JobState,
    work: Option<Box<dyn Job<S>>>,
    finished_at: Option<Instant>,
}

struct Wait {
    conn: ConnId,
    id: RequestId,
    job: u64,
    deadline: Instant,
}

struct Conn {
    tx: Sender<String>,
    subscriptions: HashSet<String>,
}

/// Events a client can subscribe to.
const EVENTS: [&str; 2] = ["project.changed", "job.finished"];

/// The dispatcher. See the module docs.
pub struct Dispatcher<S> {
    inbound: Receiver<Inbound>,
    conns: HashMap<ConnId, Conn>,
    methods: HashMap<String, Method<S>>,
    jobs: HashMap<u64, JobEntry<S>>,
    queue: VecDeque<u64>,
    running: Option<u64>,
    next_job: u64,
    waits: Vec<Wait>,
    held: bool,
    seen_state: Option<u64>,
    mort_state: Option<u64>,
    /// Whether the last edit left an undo point with its label.
    last_undo_point: bool,
}

fn invalid(message: impl Into<String>, fix: &str) -> ErrorObject {
    ErrorObject::new(ErrorKind::Invalid, message, None, fix)
}

fn parse<T: for<'de> Deserialize<'de>>(method: &str, params: Value) -> Result<T, ErrorObject> {
    let params = if params.is_null() { json!({}) } else { params };
    serde_json::from_value(params).map_err(|e| ErrorObject::invalid_params(method, &e.to_string()))
}

#[derive(Deserialize)]
struct EditParams {
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    expect_state: Option<u64>,
}

#[derive(Deserialize)]
struct HelloParams {
    #[serde(default)]
    protocol: Option<(u32, u32)>,
}

#[derive(Deserialize)]
struct JobParams {
    id: u64,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
struct SubscribeParams {
    events: Vec<String>,
}

impl<S: Session> Dispatcher<S> {
    /// A dispatcher reading from `inbound`, with the built-in methods.
    #[must_use]
    pub fn new(inbound: Receiver<Inbound>) -> Self {
        Self {
            inbound,
            conns: HashMap::new(),
            methods: HashMap::new(),
            jobs: HashMap::new(),
            queue: VecDeque::new(),
            running: None,
            next_job: 1,
            waits: Vec::new(),
            held: false,
            seen_state: None,
            mort_state: None,
            last_undo_point: false,
        }
    }

    /// Registers a method. Replaces any method of the same name.
    pub fn register(
        &mut self,
        name: &str,
        edits: bool,
        handler: impl Fn(&mut Ctx<'_, S>, Value) -> Outcome<S> + 'static,
    ) {
        self.methods.insert(
            name.to_owned(),
            Method {
                edits,
                handler: Box::new(handler),
            },
        );
    }

    /// Whether edits are held (ADR 0004 §4).
    #[must_use]
    pub fn held(&self) -> bool {
        self.held
    }

    /// Holds or releases edits.
    pub fn set_held(&mut self, held: bool) {
        self.held = held;
    }

    /// Runs one tick. See the module docs.
    pub fn tick(&mut self, session: &mut S, now: Instant, budget: Duration) {
        let started = Instant::now();
        let mut first = true;
        while first || started.elapsed() < budget {
            let Ok(msg) = self.inbound.try_recv() else {
                break;
            };
            first = false;
            match msg {
                Inbound::Opened { conn, tx } => {
                    self.conns.insert(
                        conn,
                        Conn {
                            tx,
                            subscriptions: HashSet::new(),
                        },
                    );
                }
                Inbound::Closed { conn } => {
                    self.conns.remove(&conn);
                    self.waits.retain(|w| w.conn != conn);
                }
                Inbound::Request { conn, request } => {
                    if let Some(reply) = self.request(session, now, conn, request) {
                        self.send(conn, &Message::Response(reply));
                    }
                }
            }
        }
        self.run_jobs(session, now);
        self.answer_waits(now);
        self.jobs.retain(|_, j| {
            j.finished_at
                .is_none_or(|t| now.duration_since(t) < JOB_RESULT_TTL)
        });
        let state = session.state();
        if self.seen_state != Some(state) {
            if self.seen_state.is_some() {
                let by_mort = self.mort_state == Some(state);
                self.notify(
                    "project.changed",
                    json!({ "state": state, "by_mort": by_mort }),
                );
            }
            self.seen_state = Some(state);
        }
    }

    fn send(&self, conn: ConnId, msg: &Message) {
        if let Some(c) = self.conns.get(&conn) {
            let _ = c.tx.send(msg.to_line());
        }
    }

    fn notify(&self, event: &str, params: Value) {
        let line = Message::Notification(Notification::new(event, params)).to_line();
        for c in self
            .conns
            .values()
            .filter(|c| c.subscriptions.contains(event))
        {
            let _ = c.tx.send(line.clone());
        }
    }

    /// Handles one request. `None` means the reply is deferred (`job.wait`).
    fn request(
        &mut self,
        session: &mut S,
        now: Instant,
        conn: ConnId,
        req: Request,
    ) -> Option<Response> {
        let id = req.id.clone();
        let result = match req.method.as_str() {
            "bridge.hello" => Self::hello(req.params),
            "bridge.status" => Ok(self.status(session)),
            "bridge.subscribe" => self.subscribe(conn, req.params),
            "job.status" => {
                parse::<JobParams>(&req.method, req.params).and_then(|p| self.job_status(p.id))
            }
            "job.cancel" => parse::<JobParams>(&req.method, req.params)
                .and_then(|p| self.cancel_job(session, p.id)),
            "job.wait" => match parse::<JobParams>(&req.method, req.params) {
                Err(e) => Err(e),
                Ok(p) => match self.job_status(p.id) {
                    Err(e) => Err(e),
                    Ok(status) => {
                        if self.jobs[&p.id].state.finished() {
                            Ok(status)
                        } else {
                            let wait = p.timeout_ms.map_or(DEFAULT_WAIT, Duration::from_millis);
                            self.waits.push(Wait {
                                conn,
                                id,
                                job: p.id,
                                deadline: now + wait,
                            });
                            return None;
                        }
                    }
                },
            },
            "batch" => self.batch(session, req.params),
            "undo" => self.undo_redo(session, true),
            "redo" => self.undo_redo(session, false),
            method => self.call(session, method, req.params),
        };
        Some(match result {
            Ok(v) => Response::ok(id, v),
            Err(e) => Response::err(Some(id), e),
        })
    }

    fn hello(params: Value) -> Result<Value, ErrorObject> {
        let p: HelloParams = parse("bridge.hello", params)?;
        if let Some((major, minor)) = p.protocol
            && major != PROTOCOL_VERSION.0
        {
            return Err(invalid(
                format!(
                    "protocol mismatch: the client speaks {major}.{minor}, the bridge speaks {}.{}",
                    PROTOCOL_VERSION.0, PROTOCOL_VERSION.1
                ),
                "install a matching client and bridge with `mort install`, then restart REAPER",
            ));
        }
        Ok(json!({
            "protocol": [PROTOCOL_VERSION.0, PROTOCOL_VERSION.1],
            "bridge": BUILD,
            "probe": Value::Null,
        }))
    }

    fn status(&self, session: &S) -> Value {
        json!({
            "project": session.project().name,
            "state": session.state(),
            "held": self.held,
            "playing": session.is_playing(),
            "run": crate::runs::current(),
            "jobs": {
                "running": self.running,
                "queued": self.queue.len(),
            },
        })
    }

    fn subscribe(&mut self, conn: ConnId, params: Value) -> Result<Value, ErrorObject> {
        let p: SubscribeParams = parse("bridge.subscribe", params)?;
        if let Some(bad) = p.events.iter().find(|e| !EVENTS.contains(&e.as_str())) {
            return Err(invalid(
                format!("no event `{bad}`"),
                &format!("subscribe to any of: {}", EVENTS.join(", ")),
            ));
        }
        if let Some(c) = self.conns.get_mut(&conn) {
            c.subscriptions.extend(p.events);
        }
        Ok(json!({ "events": EVENTS }))
    }

    fn job_status(&self, id: u64) -> Result<Value, ErrorObject> {
        let j = self.jobs.get(&id).ok_or_else(|| {
            ErrorObject::new(
                ErrorKind::NotFound,
                "no such job",
                Some(id.to_string()),
                "job results are kept for 10 minutes after they finish",
            )
        })?;
        let mut v = json!({ "id": id, "method": j.method, "state": j.state.name() });
        match &j.state {
            JobState::Running(Some(p)) => v["progress"] = p.clone(),
            JobState::Done(r) => v["result"] = r.clone(),
            JobState::Failed(e) => {
                v["error"] = serde_json::to_value(e).expect("errors serialise");
            }
            _ => {}
        }
        Ok(v)
    }

    fn cancel_job(&mut self, session: &mut S, id: u64) -> Result<Value, ErrorObject> {
        self.job_status(id)?;
        let j = self.jobs.get_mut(&id).expect("checked above");
        if !j.state.finished() {
            if matches!(j.state, JobState::Running(_))
                && let Some(w) = j.work.as_mut()
            {
                w.cancel(session);
            }
            j.state = JobState::Cancelled;
            j.work = None;
            j.finished_at = Some(Instant::now());
            self.queue.retain(|q| *q != id);
            if self.running == Some(id) {
                self.running = None;
            }
            self.notify("job.finished", json!({ "id": id, "state": "cancelled" }));
        }
        self.job_status(id)
    }

    fn run_jobs(&mut self, session: &mut S, now: Instant) {
        loop {
            if let Some(id) = self.running {
                let j = self.jobs.get_mut(&id).expect("running job exists");
                let w = j.work.as_mut().expect("running job has work");
                match w.poll(session) {
                    JobPoll::Running(p) => {
                        j.state = JobState::Running(p);
                        return;
                    }
                    JobPoll::Done(v) => j.state = JobState::Done(v),
                    JobPoll::Failed(e) => j.state = JobState::Failed(e),
                }
                self.finish(id, now);
            }
            let Some(next) = self.queue.pop_front() else {
                return;
            };
            let j = self.jobs.get_mut(&next).expect("queued job exists");
            let w = j.work.as_mut().expect("queued job has work");
            match w.start(session) {
                Ok(()) => {
                    j.state = JobState::Running(None);
                    self.running = Some(next);
                    // Poll on the next tick: work started this tick has not run.
                    return;
                }
                Err(e) => {
                    j.state = JobState::Failed(e);
                    self.finish(next, now);
                }
            }
        }
    }

    fn finish(&mut self, id: u64, now: Instant) {
        let j = self.jobs.get_mut(&id).expect("job exists");
        j.work = None;
        j.finished_at = Some(now);
        let state = j.state.name();
        if self.running == Some(id) {
            self.running = None;
        }
        self.notify("job.finished", json!({ "id": id, "state": state }));
    }

    fn answer_waits(&mut self, now: Instant) {
        let waits = std::mem::take(&mut self.waits);
        for w in waits {
            let finished = self.jobs.get(&w.job).is_none_or(|j| j.state.finished());
            if finished || now >= w.deadline {
                let reply = match self.job_status(w.job) {
                    Ok(v) => Response::ok(w.id, v),
                    Err(e) => Response::err(Some(w.id), e),
                };
                self.send(w.conn, &Message::Response(reply));
            } else {
                self.waits.push(w);
            }
        }
    }

    /// Runs a registered method, wrapping edits (ADR 0004).
    fn call(&mut self, session: &mut S, method: &str, params: Value) -> Result<Value, ErrorObject> {
        let edits = self
            .methods
            .get(method)
            .ok_or_else(|| ErrorObject::method_not_found(method))?
            .edits;
        if !edits {
            return self.run(session, method, params);
        }
        let ep: EditParams = parse(method, params.clone())?;
        let label = ep.summary.unwrap_or_else(|| method.to_owned());
        let mut v = self.edit(session, ep.expect_state, &label, true, |d, s| {
            d.run(s, method, params)
        })?;
        if let Value::Object(o) = &mut v {
            o.insert("state".into(), json!(session.state()));
            o.insert("undo_point".into(), json!(self.last_undo_point));
        }
        Ok(v)
    }

    fn held_error() -> ErrorObject {
        ErrorObject::new(
            ErrorKind::Busy,
            "edits are held in REAPER",
            None,
            "wait, or ask the person at REAPER to release *mort: hold edits*",
        )
    }

    /// Undoes or redoes the top undo point, if it is mort's (ADR 0004 §5).
    fn undo_redo(&mut self, session: &mut S, undo: bool) -> Result<Value, ErrorObject> {
        if self.held {
            return Err(Self::held_error());
        }
        let (word, label) = if undo {
            ("undo", session.undo_label())
        } else {
            ("redo", session.redo_label())
        };
        let Some(label) = label else {
            return Err(invalid(
                format!("there is nothing to {word}"),
                "nothing to do",
            ));
        };
        if !label.starts_with("mort:") {
            return Err(ErrorObject::new(
                ErrorKind::Invalid,
                format!("the next {word} is not mort's: `{label}`"),
                Some(label),
                format!("mort only {word}s its own edits; {word} this one in REAPER"),
            ));
        }
        let done = if undo { session.undo() } else { session.redo() };
        self.mort_state = Some(session.state());
        Ok(json!({ "label": done, "state": session.state() }))
    }

    fn run(&mut self, session: &mut S, method: &str, params: Value) -> Result<Value, ErrorObject> {
        let m = self
            .methods
            .get(method)
            .ok_or_else(|| ErrorObject::method_not_found(method))?;
        match (m.handler)(&mut Ctx { session }, params) {
            Outcome::Value(v) => v,
            Outcome::Job(work) => {
                let id = self.next_job;
                self.next_job += 1;
                self.jobs.insert(
                    id,
                    JobEntry {
                        method: method.to_owned(),
                        state: JobState::Queued,
                        work: Some(work),
                        finished_at: None,
                    },
                );
                self.queue.push_back(id);
                Ok(json!({ "job": id }))
            }
        }
    }

    fn edit(
        &mut self,
        session: &mut S,
        expect_state: Option<u64>,
        label: &str,
        may_queue: bool,
        body: impl FnOnce(&mut Self, &mut S) -> Result<Value, ErrorObject>,
    ) -> Result<Value, ErrorObject> {
        if self.held {
            return Err(Self::held_error());
        }
        let before = session.state();
        if let Some(expected) = expect_state
            && expected != before
        {
            return Err(ErrorObject::new(
                ErrorKind::Stale,
                format!("the project changed since it was read (state {expected}, now {before})"),
                None,
                "read the project again and decide again",
            ));
        }
        let jobs_before = self.next_job;
        let label = format!("mort: {label}");
        session.begin_undo();
        // REAPER moves the state count when a block opens or closes, even
        // with nothing in it, so change is judged inside the block only.
        let opened = session.state();
        let result = body(self, session);
        let changed = session.state() != opened;
        session.end_undo(&label);
        let result = if may_queue || self.next_job == jobs_before {
            result
        } else {
            // A batch may not start a job; undo the queueing.
            for id in jobs_before..self.next_job {
                self.jobs.remove(&id);
                self.queue.retain(|q| *q != id);
            }
            Err(invalid(
                "a batch cannot start a job",
                "send play, render and measure on their own",
            ))
        };
        // Roll back a failed edit, but only through its own undo point:
        // never undo whatever came before it.
        if result.is_err() && changed && session.undo_label().as_deref() == Some(label.as_str()) {
            session.undo();
        }
        self.last_undo_point =
            result.is_ok() && session.undo_label().as_deref() == Some(label.as_str());
        self.mort_state = Some(session.state());
        result
    }

    fn batch(&mut self, session: &mut S, params: Value) -> Result<Value, ErrorObject> {
        let p: BatchParams = parse("batch", params)?;
        let mut edits = false;
        for c in &p.calls {
            if ["batch", "undo", "redo"].contains(&c.method.as_str())
                || c.method.starts_with("job.")
                || c.method.starts_with("bridge.")
            {
                return Err(invalid(
                    format!("`{}` cannot be part of a batch", c.method),
                    "send it on its own",
                ));
            }
            edits |= self
                .methods
                .get(&c.method)
                .ok_or_else(|| ErrorObject::method_not_found(&c.method))?
                .edits;
        }
        let run_all = |d: &mut Self, s: &mut S| -> Result<Value, ErrorObject> {
            let mut out = Vec::with_capacity(p.calls.len());
            for (i, c) in p.calls.iter().enumerate() {
                let v = d.run(s, &c.method, c.params.clone()).map_err(|mut e| {
                    e.message = format!("batch call {i} (`{}`): {}", c.method, e.message);
                    e
                })?;
                out.push(v);
            }
            Ok(Value::Array(out))
        };
        if edits {
            let label = p.summary.clone().unwrap_or_else(|| "batch".to_owned());
            self.edit(session, p.expect_state, &label, false, run_all)
        } else {
            let jobs_before = self.next_job;
            let r = run_all(self, session);
            if self.next_job != jobs_before {
                for id in jobs_before..self.next_job {
                    self.jobs.remove(&id);
                    self.queue.retain(|q| *q != id);
                }
                return Err(invalid(
                    "a batch cannot start a job",
                    "send play, render and measure on their own",
                ));
            }
            r
        }
    }
}
