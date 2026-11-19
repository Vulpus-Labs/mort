//! A dispatcher over the fake, driven in-process (shared by the tests).

#![allow(dead_code, reason = "each test file uses part of it")]

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use mort_bridge::dispatch::{Dispatcher, Inbound};
use mort_proto::{ErrorObject, Message, Request, RequestId};
use mort_session::FakeSession;

pub struct Bench {
    pub s: FakeSession,
    pub d: Dispatcher<FakeSession>,
    tx: Sender<Inbound>,
    replies: Receiver<String>,
    n: i64,
}

impl Bench {
    pub fn new(s: FakeSession) -> Self {
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

    pub fn tick(&mut self) {
        self.d
            .tick(&mut self.s, Instant::now(), Duration::from_millis(50));
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ErrorObject> {
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

    pub fn ok(&mut self, method: &str, params: Value) -> Value {
        self.call(method, params)
            .unwrap_or_else(|e| panic!("{method}: {e:?}"))
    }

    /// Runs a measure to the end and returns its result.
    pub fn measure(&mut self, params: Value) -> Value {
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

    pub fn stale(&mut self) -> Vec<String> {
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
