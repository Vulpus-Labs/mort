//! `mort run`: scripts of mort commands (ADR 0009 §4, ticket 0014).
//!
//! A script is a KDL document. Each node is one `mort` command line: the
//! node's name and arguments are its words, and each property `key=value`
//! becomes `--key value` (`key=#true` becomes `--key`; `#false` leaves it
//! out):
//!
//! ```kdl
//! fx add "Pad 2" ReaEQ
//! set "Pad 2" ReaEQ "Gain-Band 1" "-12 dB"
//! track set Bass volume=-3
//! measure "1.1..9.1" stale=#true
//! findings
//! ```
//!
//! Edits in a row are held and sent as one batch, so they share one undo
//! point and are checked against the state the first of them read. A step
//! that needs the edits made (measure, play, render, stop, undo) sends the
//! batch first, and so does a `commit` node. Reads see the tracks and FX
//! that held edits add. If a step fails, the script stops there and names
//! it; held edits are not sent.

use std::fmt::Write as _;
use std::path::PathBuf;

use clap::{Args, Parser};
use kdl::{KdlDocument, KdlValue};
use serde_json::{Value, json};

use crate::client::Failure;
use crate::{Cli, Command, Outcome, connect, step};

/// `mort run` arguments.
#[derive(Args)]
pub struct Run {
    /// The script: a `.kdl` file, or `-` for stdin.
    file: PathBuf,
    /// The undo label for the script's batches; the file name by default.
    #[arg(long)]
    summary: Option<String>,
}

/// One step: where it is, its words, and the command they make.
struct Step {
    line: usize,
    text: String,
    command: Option<Command>,
}

fn word(v: &KdlValue) -> String {
    match v {
        KdlValue::String(s) => s.clone(),
        KdlValue::Integer(i) => i.to_string(),
        KdlValue::Float(f) => f.to_string(),
        KdlValue::Bool(b) => b.to_string(),
        KdlValue::Null => "null".into(),
    }
}

fn quote(w: &str) -> String {
    if w.is_empty() || w.contains(char::is_whitespace) {
        format!("{w:?}")
    } else {
        w.to_owned()
    }
}

/// Reads a script into steps, checking every one before any runs.
fn parse(src: &str) -> Result<Vec<Step>, Outcome> {
    let doc: KdlDocument = src
        .parse()
        .map_err(|e: kdl::KdlError| Outcome::Usage(format!("the script is not KDL: {e}")))?;
    let mut steps = Vec::new();
    for node in doc.nodes() {
        let line = src[..node.span().offset()].matches('\n').count() + 1;
        let name = node.name().value().to_owned();
        if node.children().is_some() {
            return Err(Outcome::Usage(format!(
                "line {line}: `{name}` has a block; script steps are single lines"
            )));
        }
        let mut words = vec![name.clone()];
        for e in node.entries() {
            match e.name() {
                None => words.push(word(e.value())),
                Some(key) => match e.value() {
                    KdlValue::Bool(false) => {}
                    KdlValue::Bool(true) => words.push(format!("--{}", key.value())),
                    v => {
                        words.push(format!("--{}", key.value()));
                        words.push(word(v));
                    }
                },
            }
        }
        let text = words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ");
        if name == "commit" && words.len() == 1 {
            steps.push(Step {
                line,
                text,
                command: None,
            });
            continue;
        }
        let cli =
            Cli::try_parse_from(std::iter::once("mort".to_owned()).chain(words)).map_err(|e| {
                let first = e.to_string();
                let first = first
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim_start_matches("error: ");
                Outcome::Usage(format!("line {line}: `{text}`: {first}"))
            })?;
        if matches!(
            cli.command,
            Command::Status | Command::Call { .. } | Command::Install { .. } | Command::Run(_)
        ) {
            return Err(Outcome::Usage(format!(
                "line {line}: `{text}`: status, call, install and run cannot be script steps"
            )));
        }
        steps.push(Step {
            line,
            text,
            command: Some(cli.command),
        });
    }
    Ok(steps)
}

fn batch_line(r: &Value, n: usize) -> String {
    format!(
        "sent {n} edit{} as one batch: state {}{}",
        if n == 1 { "" } else { "s" },
        r["state"],
        if r["undo_point"] == true {
            ""
        } else {
            " (no undo point: nothing changed)"
        }
    )
}

/// Runs `mort run`.
pub fn run(cli: &Cli, args: &Run) -> Result<Vec<String>, Outcome> {
    let (src, name) = if args.file.as_os_str() == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)
            .map_err(|e| Failure::Local(format!("cannot read the script from stdin: {e}")))?;
        (s, "stdin".to_owned())
    } else {
        let s = std::fs::read_to_string(&args.file)
            .map_err(|e| Failure::Local(format!("cannot read {}: {e}", args.file.display())))?;
        let n = args
            .file
            .file_name()
            .map_or_else(|| "script".into(), |f| f.to_string_lossy().into_owned());
        (s, n)
    };
    let steps = parse(&src)?;
    let label = args
        .summary
        .clone()
        .unwrap_or_else(|| format!("script {name}"));
    let (mut c, _) = connect(cli)?;
    c.record(&label);
    let mut lines = Vec::new();
    let mut results = Vec::new();
    for (n, st) in steps.iter().enumerate() {
        let tag = format!("step {} (line {}: `{}`)", n + 1, st.line, st.text);
        c.set_step(&tag);
        lines.push(format!("> {}", st.text));
        let held = c.pending();
        let out = match &st.command {
            None => c
                .flush(&label)
                .map(|r| {
                    let line = r
                        .as_ref()
                        .map_or_else(|| "nothing to send".to_owned(), |r| batch_line(r, held));
                    (vec![line], r.unwrap_or(Value::Null))
                })
                .map_err(Outcome::from),
            Some(cmd) => step(cmd, &mut c),
        };
        let (out, value) = match out {
            Ok(x) => x,
            Err(e) => {
                let mut msg = String::new();
                for l in &lines {
                    let _ = writeln!(msg, "{l}");
                }
                let unsent = c.pending();
                let _ = write!(
                    msg,
                    "{tag} failed{}",
                    if unsent > 0 {
                        format!("; {unsent} held edits were not sent")
                    } else {
                        String::new()
                    }
                );
                eprintln!("{msg}");
                return Err(e);
            }
        };
        if st.command.is_some() && c.pending() > held {
            lines.push("  held for the batch".into());
        } else {
            lines.extend(out.into_iter().map(|l| format!("  {l}")));
        }
        results.push(json!({ "step": n + 1, "line": st.line, "text": st.text, "result": value }));
    }
    let held = c.pending();
    if let Some(r) = c.flush(&label)? {
        lines.push(batch_line(&r, held));
    }
    let status = c.call("bridge.status", Value::Null)?;
    if cli.json {
        return Ok(vec![
            json!({ "steps": results, "state": status["state"] }).to_string(),
        ]);
    }
    lines.push(format!("state {}", status["state"]));
    Ok(lines)
}
