//! The change feed (ADR 0012 §8, ticket 0018).
//!
//! The main thread notes every change to the map, its staleness and the
//! thresholds here, and the transport's position on every tick. Socket
//! threads answer `map.watch` from it directly, waiting on a condition
//! variable, so a watch never queues on the dispatcher or touches REAPER.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use mort_map::Span;
use mort_proto::TrackId;

/// How many changes the feed remembers. A watcher further behind is told
/// to fetch everything again.
const LOG: usize = 256;

/// The longest a watch may wait.
pub const MAX_WAIT: Duration = Duration::from_mins(1);

/// What changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A capture was added: a measure or an audition.
    Capture,
    /// Data went stale, or fresh again, after the project changed.
    Stale,
    /// The thresholds changed.
    Thresholds,
    /// A different project, or the same one saved somewhere new.
    Project,
}

/// One change.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Change {
    /// The version it made.
    pub version: u64,
    /// What changed.
    pub kind: Kind,
    /// The tracks it touched; empty means any.
    pub tracks: Vec<TrackId>,
    /// Where; empty means anywhere.
    pub spans: Vec<Span>,
}

#[derive(Debug, Default)]
struct State {
    version: u64,
    log: VecDeque<Change>,
    position: f64,
    playing: bool,
}

/// The feed. See the module docs.
#[derive(Debug, Default)]
pub struct Feed {
    state: Mutex<State>,
    changed: Condvar,
}

impl Feed {
    /// A feed at version 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current version.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.state.lock().map_or(0, |s| s.version)
    }

    /// Notes a change, waking watchers. Returns the new version.
    pub fn note(&self, kind: Kind, tracks: Vec<TrackId>, spans: Vec<Span>) -> u64 {
        let Ok(mut s) = self.state.lock() else {
            return 0;
        };
        s.version += 1;
        let version = s.version;
        s.log.push_back(Change {
            version,
            kind,
            tracks,
            spans,
        });
        while s.log.len() > LOG {
            s.log.pop_front();
        }
        drop(s);
        self.changed.notify_all();
        version
    }

    /// Records where the transport is. Does not wake watchers.
    pub fn transport(&self, position: f64, playing: bool) {
        if let Ok(mut s) = self.state.lock() {
            s.position = position;
            s.playing = playing;
        }
    }

    /// Waits until the version passes `since`, or `timeout` (at most
    /// [`MAX_WAIT`]) ends, and says what changed since then. A watcher
    /// further behind than the log reaches gets `reset: true` and should
    /// fetch everything again.
    #[must_use]
    pub fn watch(&self, since: u64, timeout: Duration) -> Value {
        let deadline = Instant::now() + timeout.min(MAX_WAIT);
        let Ok(mut s) = self.state.lock() else {
            return json!({ "version": 0, "changes": [], "reset": true });
        };
        while s.version <= since {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match self.changed.wait_timeout(s, deadline - now) {
                Ok((g, _)) => s = g,
                Err(_) => return json!({ "version": 0, "changes": [], "reset": true }),
            }
        }
        let oldest = s.log.front().map_or(s.version + 1, |c| c.version);
        let reset = since + 1 < oldest && s.version > since;
        let changes: Vec<&Change> = s.log.iter().filter(|c| c.version > since).collect();
        json!({
            "version": s.version,
            "changes": changes,
            "reset": reset,
            "position": s.position,
            "playing": s.playing,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    #[expect(clippy::many_single_char_names, reason = "short test bindings")]
    fn watch_wakes_on_a_change_and_times_out_otherwise() {
        let f = Arc::new(Feed::new());
        let t = Instant::now();
        let v = f.watch(0, Duration::from_millis(30));
        assert!(t.elapsed() >= Duration::from_millis(30));
        assert_eq!(v["version"], 0);
        assert_eq!(v["changes"], json!([]));

        let g = Arc::clone(&f);
        let h = std::thread::spawn(move || g.watch(0, Duration::from_secs(5)));
        std::thread::sleep(Duration::from_millis(20));
        f.transport(1.5, true);
        f.note(Kind::Thresholds, vec![], vec![]);
        let v = h.join().unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["changes"][0]["kind"], "thresholds");
        assert_eq!(v["position"], 1.5);
        assert_eq!(v["reset"], false);

        for _ in 0..(LOG + 5) {
            f.note(Kind::Capture, vec![], vec![]);
        }
        let v = f.watch(1, Duration::ZERO);
        assert_eq!(v["reset"], true, "too far behind");
        assert_eq!(f.watch(f.version(), Duration::ZERO)["changes"], json!([]));
    }
}
