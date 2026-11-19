//! `mort probes` (ticket 0011).

use clap::Subcommand;
use serde_json::{Value, json};

use crate::client::{Client, Failure};
use crate::{resolve, show};

/// `mort probes` actions; with none, lists the probes.
#[derive(Subcommand)]
pub enum ProbesCmd {
    /// Put one probe last in each chain: every track and the master, or
    /// the tracks named.
    Ensure {
        /// Tracks to probe; all of them if none are named.
        tracks: Vec<String>,
        /// Leave the master out.
        #[arg(long)]
        no_master: bool,
    },
}

/// Runs `mort probes` with an optional action.
pub fn run(client: &mut Client, cmd: Option<&ProbesCmd>) -> Result<(Vec<String>, Value), Failure> {
    match cmd {
        None => list(client),
        Some(ProbesCmd::Ensure { tracks, no_master }) => {
            let list = client.call("tracks.list", Value::Null)?;
            let mut p = json!({ "master": !no_master, "expect_state": list["state"] });
            if !tracks.is_empty() {
                let ids: Vec<Value> = tracks
                    .iter()
                    .map(|t| resolve::track(&list, t).map(|t| t["id"].clone()))
                    .collect::<Result<_, _>>()?;
                p["tracks"] = json!(ids);
            }
            p["summary"] = json!("ensure probes");
            let r = client.call("probes.ensure", p)?;
            let mut lines = vec![format!(
                "probes: {} added, {} moved to the end, {} duplicates removed",
                r["added"], r["moved"], r["removed"]
            )];
            let (shown, _) = list_probes(client)?;
            lines.extend(shown);
            Ok((lines, r))
        }
    }
}

fn list(client: &mut Client) -> Result<(Vec<String>, Value), Failure> {
    list_probes(client)
}

fn list_probes(client: &mut Client) -> Result<(Vec<String>, Value), Failure> {
    let r = client.call("probes.list", Value::Null)?;
    let probes = r["probes"].as_array().cloned().unwrap_or_default();
    let mut rows = vec![vec![
        "track".to_owned(),
        "slot".into(),
        "state".into(),
        "rate".into(),
        "frames".into(),
        "dropped".into(),
    ]];
    for p in &probes {
        let state = if !p["last"].as_bool().unwrap_or(false) {
            "not last"
        } else if p["enabled"] == false {
            "bypassed"
        } else if p["attached"] != true {
            "detached"
        } else if p["active"] != true {
            "inactive"
        } else {
            "ok"
        };
        rows.push(vec![
            p["name"].as_str().unwrap_or("?").to_owned(),
            p["slot"]
                .as_u64()
                .map_or_else(|| "-".to_owned(), |s| s.to_string()),
            state.to_owned(),
            p["sample_rate"]
                .as_f64()
                .map_or_else(|| "-".to_owned(), |r| format!("{r:.0}")),
            p["pushed"]
                .as_u64()
                .map_or_else(|| "-".to_owned(), |n| n.to_string()),
            p["dropped"]
                .as_u64()
                .map_or_else(|| "-".to_owned(), |n| n.to_string()),
        ]);
    }
    let mut out = vec![format!("{} probes", probes.len())];
    out.extend(show::table(&rows).into_iter().map(|l| format!("  {l}")));
    let missing: Vec<&str> = r["unmeasured"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str())
        .collect();
    if !missing.is_empty() {
        out.push(format!(
            "no probe: {} (add with `mort probes ensure`)",
            missing.join(", ")
        ));
    }
    Ok((out, r))
}
