//! The `mort` command line (ADR 0009).
//!
//! Exit codes: 0 success, 1 an error from the bridge (or a local failure),
//! 2 a usage error, 3 no bridge running.

mod automation;
mod client;
mod edits;
mod install;
mod measure;
mod midi;
mod probes;
mod report;
mod resolve;
mod script;
mod show;
mod smf;
mod transport;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::{Value, json};

use client::{Client, Failure};

/// Drive and measure a live REAPER session.
#[derive(Parser)]
#[command(name = "mort", version, about)]
struct Cli {
    /// Print the bridge's results as JSON instead of text.
    #[arg(long, global = true)]
    json: bool,

    /// The bridge's socket. Defaults to the one in `$TMPDIR/mort/current`.
    #[arg(long, global = true, value_name = "PATH")]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the bridge, the project, its state count and whether edits are
    /// held.
    Status,

    /// Send any RPC and print its result.
    Call {
        /// The method, for example `bridge.status`.
        method: String,
        /// Its parameters, as JSON. Defaults to none.
        params: Option<String>,
    },

    /// List tracks with their levels, FX and sends.
    Tracks,

    /// Add, remove or change a track.
    #[command(subcommand)]
    Track(edits::TrackCmd),

    /// Add or change a send.
    #[command(subcommand)]
    Send(edits::SendCmd),

    /// List the FX on a track (`mort fx <track>`), or change them.
    #[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
    Fx {
        /// The track: number, name, part of a name, id, or `master`.
        #[arg(required = true)]
        track: Option<String>,
        #[command(subcommand)]
        action: Option<edits::FxCmd>,
    },

    /// Set a parameter, by the text the plugin shows (`-3 dB`, `2.4 kHz`)
    /// or, with --normalised, by a value from 0 to 1.
    Set {
        /// The track.
        track: String,
        /// The FX.
        fx: String,
        /// The parameter: its number (from 0) or name.
        param: String,
        /// The value.
        #[arg(allow_hyphen_values = true)]
        value: String,
        /// Read the value as normalised, 0 to 1.
        #[arg(long)]
        normalised: bool,
    },

    /// List or write automation: volume, pan, sends and FX parameters.
    Automate(automation::Automate),

    /// List or write MIDI notes and CCs.
    Midi(midi::Midi),

    /// Play a range through the monitors, stopping at its end.
    Play(transport::RangeJob),

    /// Render a range faster than real time, without writing a file.
    Render(transport::RangeJob),

    /// Stop playback.
    Stop,

    /// List the probes, or put one last in each chain.
    Probes {
        #[command(subcommand)]
        action: Option<probes::ProbesCmd>,
    },

    /// Measure a range into the mix map: every track, or the tracks named
    /// with the buses they reach. Adds probes where needed.
    Measure(measure::Measure),

    /// Show what the mix map covers and what is stale.
    Map,

    /// Summarise the mix by section: the master, each track's level and
    /// share, and finding counts.
    Summary(report::Summary),

    /// List problems in the mix (masking, mud, harshness, headroom,
    /// loudness jumps, wide low end, register clashes), worst first.
    Findings(report::Findings),

    /// Show one track's features by section, and its share of its bus.
    Report(report::Report),

    /// Show a track's raw frames over a range, resampled.
    Frames(report::Frames),

    /// Compare the mix before and after an edit: two captures' features by
    /// section and track, and the findings that appeared or went.
    Compare(report::Compare),

    /// Run a script of mort commands (a `.kdl` file, or `-` for stdin).
    /// Edits in a row go to REAPER as one batch, with one undo point.
    Run(script::Run),

    /// Undo mort's last edit. Refuses if the last edit was not mort's.
    Undo,

    /// Redo mort's last undone edit.
    Redo,

    /// List an FX instance's parameters.
    Params {
        /// The track: number, name, part of a name, id, or `master`.
        track: String,
        /// The FX: position in the chain, name, part of a name, or id.
        fx: String,
        /// Show only parameters whose names contain this.
        filter: Option<String>,
    },

    /// Copy the bridge and probe into REAPER's folders.
    Install {
        /// The cargo target directory to install from. Defaults to the one
        /// this `mort` was built in.
        #[arg(long, value_name = "DIR")]
        from: Option<PathBuf>,
        /// Install the bridge here instead of REAPER's `UserPlugins`.
        #[arg(long, value_name = "DIR", hide = true)]
        user_plugins: Option<PathBuf>,
        /// Install the probe here instead of `~/Library/Audio/Plug-Ins/CLAP`.
        #[arg(long, value_name = "DIR", hide = true)]
        clap_dir: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    match run(&cli) {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
            ExitCode::SUCCESS
        }
        Err(Outcome::Usage(m)) => {
            eprintln!("mort: {m}");
            ExitCode::from(2)
        }
        Err(Outcome::Failed(f)) => {
            if json && let Failure::Bridge(e) = &f {
                println!("{}", json!({ "error": e }));
            }
            eprintln!("mort: {f}");
            ExitCode::from(u8::try_from(f.exit_code()).unwrap_or(1))
        }
    }
}

enum Outcome {
    Usage(String),
    Failed(Failure),
}

impl From<Failure> for Outcome {
    fn from(f: Failure) -> Self {
        Outcome::Failed(f)
    }
}

pub(crate) fn connect(cli: &Cli) -> Result<(Client, Value), Failure> {
    let path = match &cli.socket {
        Some(p) => p.clone(),
        None => client::current_socket()?,
    };
    Client::connect(&path)
}

/// Text lines with the state last (ADR 0009 §2), or the raw result.
pub(crate) fn with_state(cli: &Cli, result: &Value, mut lines: Vec<String>) -> Vec<String> {
    if cli.json {
        return vec![result.to_string()];
    }
    lines.push(format!("state {}", result["state"]));
    lines
}

fn yes_no(v: &Value) -> &'static str {
    if v.as_bool() == Some(true) {
        "yes"
    } else {
        "no"
    }
}

fn run(cli: &Cli) -> Result<Vec<String>, Outcome> {
    match &cli.command {
        Command::Status => {
            let (mut c, hello) = connect(cli)?;
            let mut s = c.call("bridge.status", Value::Null)?;
            if cli.json {
                s["bridge"] = hello["bridge"].clone();
                return Ok(vec![s.to_string()]);
            }
            let jobs = &s["jobs"];
            let running = jobs["running"]
                .as_u64()
                .map_or_else(|| "none running".to_owned(), |j| format!("job {j} running"));
            Ok(vec![
                format!("bridge   {}", hello["bridge"].as_str().unwrap_or("?")),
                format!("project  {}", s["project"].as_str().unwrap_or("?")),
                format!("held     {}", yes_no(&s["held"])),
                format!("playing  {}", yes_no(&s["playing"])),
                format!("jobs     {running}, {} queued", jobs["queued"]),
                format!("state    {}", s["state"]),
            ])
        }
        Command::Call { method, params } => {
            let params: Value = match params {
                None => Value::Null,
                Some(p) => serde_json::from_str(p)
                    .map_err(|e| Outcome::Usage(format!("parameters are not JSON: {e}")))?,
            };
            let (mut c, _) = connect(cli)?;
            let r = c.call(method, params)?;
            Ok(vec![if cli.json {
                r.to_string()
            } else {
                serde_json::to_string_pretty(&r).expect("values serialise")
            }])
        }
        Command::Run(r) => script::run(cli, r),
        Command::Install {
            from,
            user_plugins,
            clap_dir,
        } => Ok(install_cmd(
            from.as_ref(),
            user_plugins.as_ref(),
            clap_dir.as_ref(),
        )?),
        other => {
            let (mut c, _) = connect(cli)?;
            let (lines, value) = step(other, &mut c)?;
            Ok(with_state(cli, &value, lines))
        }
    }
}

/// Runs a command that needs only a connection: its text lines and its
/// result, whose `state` the last line shows. Scripts run their steps
/// through this too.
fn step(cmd: &Command, c: &mut Client) -> Result<(Vec<String>, Value), Outcome> {
    Ok(match cmd {
        Command::Tracks => {
            let list = c.call("tracks.list", Value::Null)?;
            (show::tracks(&list), list)
        }
        Command::Track(cmd) => edits::track(c, cmd)?,
        Command::Send(cmd) => edits::send(c, cmd)?,
        Command::Fx {
            action: Some(cmd), ..
        } => edits::fx(c, cmd)?,
        Command::Set {
            track,
            fx,
            param,
            value,
            normalised,
        } => edits::set(c, track, fx, param, value, *normalised)?,
        Command::Fx {
            track: None,
            action: None,
        } => return Err(Outcome::Usage("name a track: `mort fx <track>`".into())),
        Command::Fx {
            track: Some(track),
            action: None,
        } => {
            let list = c.call("tracks.list", Value::Null)?;
            let t = resolve::track(&list, track)?;
            (show::fx(t), json!({ "track": t, "state": list["state"] }))
        }
        Command::Params { track, fx, filter } => {
            let list = c.call("tracks.list", Value::Null)?;
            let t = resolve::track(&list, track)?;
            let f = resolve::fx(t, fx)?;
            let r = c.call("fx.params", json!({ "fx": f["id"] }))?;
            let name = t["name"].as_str().unwrap_or("?");
            (show::params(&r, name, filter.as_deref()), r)
        }
        Command::Automate(a) => automation::run(c, a)?,
        Command::Midi(m) => midi::run(c, m)?,
        Command::Play(a) => transport::run(c, a, false)?,
        Command::Render(a) => transport::run(c, a, true)?,
        Command::Stop => transport::stop(c)?,
        Command::Probes { action } => probes::run(c, action.as_ref())?,
        Command::Measure(m) => measure::run(c, m)?,
        Command::Map => measure::map(c)?,
        Command::Summary(a) => report::summary(c, a)?,
        Command::Findings(a) => report::findings(c, a)?,
        Command::Report(a) => report::report(c, a)?,
        Command::Frames(a) => report::frames(c, a)?,
        Command::Compare(a) => report::compare(c, a)?,
        Command::Undo => edits::undo(c, false)?,
        Command::Redo => edits::undo(c, true)?,
        Command::Status | Command::Call { .. } | Command::Install { .. } | Command::Run(_) => {
            return Err(Outcome::Usage(
                "status, call, install and run are not script steps".into(),
            ));
        }
    })
}

fn install_cmd(
    from: Option<&PathBuf>,
    user_plugins: Option<&PathBuf>,
    clap_dir: Option<&PathBuf>,
) -> Result<Vec<String>, Failure> {
    let mut places = install::Places::default_for_user();
    if let Some(f) = from {
        places.from.clone_from(f);
    }
    if let Some(u) = user_plugins {
        places.user_plugins.clone_from(u);
    }
    if let Some(c) = clap_dir {
        places.clap.clone_from(c);
    }
    install::run(&places, install::reaper_running())
}
