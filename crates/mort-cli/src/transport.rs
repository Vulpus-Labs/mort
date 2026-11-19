//! `mort play`, `mort render` and `mort stop` (ticket 0009).

use std::io::Write as _;
use std::time::Duration;

use clap::Args;
use serde_json::{Value, json};

use crate::client::{Client, Failure};

/// `mort play` and `mort render` arguments.
#[derive(Args)]
pub struct RangeJob {
    /// The range, such as `9.1..17.1`.
    range: String,
    /// Print the job id and return at once.
    #[arg(long)]
    no_wait: bool,
}

/// How long to wait for any one reply while a job runs.
const JOB_REPLY: Duration = Duration::from_mins(10);

/// Waits for a job, printing progress to stderr, and returns its status.
///
/// # Errors
///
/// A bridge error, or the job failing.
pub fn wait(client: &mut Client, job: &Value, what: &str) -> Result<Value, Failure> {
    let mut err = std::io::stderr();
    loop {
        let s = client.call_within(
            "job.wait",
            json!({ "id": job, "timeout_ms": 1000 }),
            JOB_REPLY,
        )?;
        match s["state"].as_str() {
            Some("done") => {
                let _ = writeln!(err);
                return Ok(s);
            }
            Some("failed") => {
                let _ = writeln!(err);
                let e: mort_proto::ErrorObject = serde_json::from_value(s["error"].clone())
                    .unwrap_or_else(|_| mort_proto::ErrorObject::internal("the job failed"));
                return Err(Failure::Bridge(e));
            }
            Some("cancelled") => {
                let _ = writeln!(err);
                return Err(Failure::Local(format!("{what} was cancelled")));
            }
            state => {
                let at = s["progress"]["position"]
                    .as_f64()
                    .map(|p| format!(" at {p:.1}s"))
                    .unwrap_or_default();
                let _ = write!(err, "\r{what}: {}{at}   ", state.unwrap_or("?"));
            }
        }
    }
}

/// Converts a range such as `9.1..17.1` to seconds through the bridge.
///
/// # Errors
///
/// A bridge error, or text that is not a range.
pub fn range(client: &mut Client, text: &str) -> Result<(f64, f64), Failure> {
    let r = client.call("time.convert", json!({ "to_seconds": [text] }))?;
    let pair = &r["seconds"][0];
    match (pair[0].as_f64(), pair[1].as_f64()) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => Err(Failure::Local(format!(
            "`{text}` is not a range; write `start..end`, such as `9.1..17.1`"
        ))),
    }
}

/// Runs `mort play` (`render` false) or `mort render` (`render` true).
pub fn run(
    client: &mut Client,
    a: &RangeJob,
    render: bool,
) -> Result<(Vec<String>, Value), Failure> {
    let (start, end) = range(client, &a.range)?;
    let method = if render { "render" } else { "play" };
    let r = client.call(method, json!({ "start": start, "end": end }))?;
    let job = r["job"].clone();
    if a.no_wait {
        return Ok((vec![format!("{method} {}: job {job}", a.range)], r));
    }
    let s = wait(client, &job, method)?;
    let res = &s["result"];
    let line = if render {
        format!(
            "rendered {} ({:.2} s of audio in {:.2} s, {:.0}× real time), run {}",
            a.range,
            end - start,
            res["seconds"].as_f64().unwrap_or(0.0),
            res["speed"].as_f64().unwrap_or(0.0),
            res["run"]
        )
    } else if res["stopped_early"] == true {
        format!(
            "played {} until stopped at {:.2}s, run {}",
            a.range,
            res["stopped_at"].as_f64().unwrap_or(0.0),
            res["run"]
        )
    } else {
        format!("played {}, run {}", a.range, res["run"])
    };
    let after = client.call("bridge.status", Value::Null)?;
    Ok((vec![line], json!({ "job": s, "state": after["state"] })))
}

/// Runs `mort stop`.
pub fn stop(client: &mut Client) -> Result<(Vec<String>, Value), Failure> {
    let r = client.call("stop", Value::Null)?;
    let after = client.call("bridge.status", Value::Null)?;
    Ok((
        vec![format!(
            "stopped at {:.2}s",
            r["position"].as_f64().unwrap_or(0.0)
        )],
        json!({ "position": r["position"], "state": after["state"] }),
    ))
}
