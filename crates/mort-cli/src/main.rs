//! The `mort` command line (ADR 0009).
//!
//! Exit codes: 0 success, 1 an error from the bridge (or a local failure),
//! 2 a usage error, 3 no bridge running.

mod client;
mod install;

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

fn connect(cli: &Cli) -> Result<(Client, Value), Failure> {
    let path = match &cli.socket {
        Some(p) => p.clone(),
        None => client::current_socket()?,
    };
    Client::connect(&path)
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
        Command::Install {
            from,
            user_plugins,
            clap_dir,
        } => {
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
            Ok(install::run(&places, install::reaper_running())?)
        }
    }
}
