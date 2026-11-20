//! `mort midi` (ticket 0008).
//!
//! ```text
//! mort midi bass                                  # list notes and CCs
//! mort midi bass 9.1..17.1                        # only those in a range
//! mort midi bass 9.1..17.1 C2:9.1:1b:100 E2:+1b:1b G2:+1b
//! mort midi bass 9.1..17.1 --replace C2:9.1:4b cc1:9.1=0 cc1:+4b=127
//! mort midi pad 9.1..17.1 --file chords.mid       # place a file at 9.1
//! mort midi pad 9.1..17.1 --file chords.txt       # the same tokens, in a file
//! ```
//!
//! A note is `PITCH:START[:LENGTH[:VELOCITY[:CHANNEL]]]`. Pitch is a name
//! (`C4` is middle C, 60; `F#2`, `Bb3`) or a number. Start is a position;
//! `+1b` is a quarter note after the previous event's start, so `+0b`
//! stacks a chord. Length is `1b` (quarter notes, the default) or `0.5s`.
//! Velocity defaults to 100, channel (1–16) to 1. A CC is
//! `ccN[@CHANNEL]:START=VALUE`.

use clap::Args;
use mort_compose::pitch::Pitch;
use serde_json::{Value, json};

use crate::client::{Client, Failure};
use crate::{resolve, show, smf};

/// `mort midi` arguments.
#[derive(Args)]
pub struct Midi {
    /// The track.
    track: String,
    /// The range to list, write into, or (with --replace) clear first.
    range: Option<String>,
    /// Notes and CCs to write; see `mort midi --help`.
    events: Vec<String>,
    /// Clear notes starting, and CCs falling, in the range before writing.
    #[arg(long)]
    replace: bool,
    /// Read events from a file: a `.mid` file placed at the range's start, or
    /// a text file of the same tokens.
    #[arg(long, value_name = "PATH")]
    file: Option<std::path::PathBuf>,
}

/// `60` for `C4`, `61` for `C#4` or `Db4`, or the number itself.
pub fn parse_pitch(text: &str) -> Result<u8, Failure> {
    let t = text.trim();
    let n = t
        .parse::<u8>()
        .ok()
        .filter(|n| *n <= 127)
        .or_else(|| Pitch::parse(t).map(Pitch::get));
    n.ok_or_else(|| {
        Failure::Local(format!(
            "`{text}` is not a pitch; write a name such as `C4` (middle C), `F#2` or `Bb3`, or 0–127"
        ))
    })
}

/// `C4` for 60.
#[must_use]
pub fn pitch_name(n: u8) -> String {
    Pitch::new(i32::from(n)).map_or_else(|| n.to_string(), |p| p.to_string())
}

enum Event {
    Note {
        pitch: u8,
        start: String,
        length: String,
        velocity: u8,
        channel: u8,
    },
    Cc {
        controller: u8,
        start: String,
        value: u8,
        channel: u8,
    },
}

fn channel(text: &str) -> Result<u8, Failure> {
    text.parse::<u8>()
        .ok()
        .filter(|c| (1..=16).contains(c))
        .map(|c| c - 1)
        .ok_or_else(|| Failure::Local(format!("`{text}` is not a MIDI channel; use 1–16")))
}

fn parse_event(token: &str) -> Result<Event, Failure> {
    let bad = |why: &str| Failure::Local(format!("`{token}`: {why}"));
    if let Some(rest) = token
        .strip_prefix("cc")
        .or_else(|| token.strip_prefix("CC"))
    {
        let (head, value) = rest
            .rsplit_once('=')
            .ok_or_else(|| bad("a CC is written `ccN:START=VALUE`, such as `cc1:9.1=64`"))?;
        let (ctl, start) = head
            .split_once(':')
            .ok_or_else(|| bad("a CC is written `ccN:START=VALUE`, such as `cc1:9.1=64`"))?;
        let (num, chan) = match ctl.split_once('@') {
            Some((n, c)) => (n, channel(c)?),
            None => (ctl, 0),
        };
        let controller = num
            .parse::<u8>()
            .ok()
            .filter(|n| *n <= 127)
            .ok_or_else(|| bad("the controller is 0–127"))?;
        let value = value
            .parse::<u8>()
            .ok()
            .filter(|n| *n <= 127)
            .ok_or_else(|| bad("the value is 0–127"))?;
        return Ok(Event::Cc {
            controller,
            start: start.to_owned(),
            value,
            channel: chan,
        });
    }
    let parts: Vec<&str> = token.split(':').collect();
    if parts.len() < 2 || parts.len() > 5 {
        return Err(bad(
            "a note is written `PITCH:START[:LENGTH[:VELOCITY[:CHANNEL]]]`, such as `C4:9.1:1b:100`",
        ));
    }
    let length = parts.get(2).copied().unwrap_or("1b");
    if !(length.ends_with('b') || length.ends_with('s')) {
        return Err(bad(
            "a length is in quarter notes (`1b`) or seconds (`0.5s`)",
        ));
    }
    let velocity = match parts.get(3) {
        None => 100,
        Some(v) => v
            .parse::<u8>()
            .ok()
            .filter(|v| (1..=127).contains(v))
            .ok_or_else(|| bad("velocity is 1–127"))?,
    };
    let chan = match parts.get(4) {
        None => 0,
        Some(c) => channel(c)?,
    };
    Ok(Event::Note {
        pitch: parse_pitch(parts[0])?,
        start: parts[1].to_owned(),
        length: length.to_owned(),
        velocity,
        channel: chan,
    })
}

fn seconds(client: &mut Client, texts: &[String]) -> Result<Vec<Value>, Failure> {
    if texts.is_empty() {
        return Ok(vec![]);
    }
    let r = client.call("time.convert", json!({ "to_seconds": texts }))?;
    Ok(r["seconds"].as_array().cloned().unwrap_or_default())
}

/// Turns tokens into wire notes and CCs.
fn from_tokens(
    client: &mut Client,
    tokens: &[String],
) -> Result<(Vec<Value>, Vec<Value>), Failure> {
    let events: Vec<Event> = tokens
        .iter()
        .map(|t| parse_event(t))
        .collect::<Result<_, _>>()?;
    let start_texts: Vec<String> = events
        .iter()
        .map(|e| match e {
            Event::Note { start, .. } | Event::Cc { start, .. } => start.clone(),
        })
        .collect();
    let starts = seconds(client, &start_texts)?;
    // Each note's end is its start plus its length: pairs of "Xs", "+L".
    let mut pairs = Vec::new();
    for (e, s) in events.iter().zip(&starts) {
        if let Event::Note { length, .. } = e {
            pairs.push(format!("{}s", s.as_f64().unwrap_or(0.0)));
            pairs.push(format!("+{length}"));
        }
    }
    let ends = seconds(client, &pairs)?;
    let mut ends = ends.iter().skip(1).step_by(2);
    let (mut notes, mut ccs) = (Vec::new(), Vec::new());
    for (e, s) in events.iter().zip(&starts) {
        match e {
            Event::Note { pitch, velocity, channel, .. } => notes.push(json!({
                "pitch": pitch, "start": s, "end": ends.next(), "velocity": velocity, "channel": channel
            })),
            Event::Cc { controller, value, channel, .. } => ccs.push(json!({
                "time": s, "controller": controller, "value": value, "channel": channel
            })),
        }
    }
    Ok((notes, ccs))
}

/// Places a MIDI file's events at `start` seconds.
fn from_smf(
    client: &mut Client,
    bytes: &[u8],
    start: f64,
    path: &str,
) -> Result<(Vec<Value>, Vec<Value>), Failure> {
    let (notes, ccs) = smf::read(bytes).map_err(|e| Failure::Local(format!("{path}: {e}")))?;
    let mut texts = Vec::new();
    for n in &notes {
        for qn in [n.start, n.end] {
            texts.push(format!("{start}s"));
            texts.push(format!("+{qn}b"));
        }
    }
    for c in &ccs {
        texts.push(format!("{start}s"));
        texts.push(format!("+{}b", c.time));
    }
    let secs = seconds(client, &texts)?;
    let mut at = secs.iter().skip(1).step_by(2);
    let mut out_notes = Vec::new();
    for n in &notes {
        let (s, e) = (at.next(), at.next());
        out_notes.push(json!({ "pitch": n.pitch, "start": s, "end": e, "velocity": n.velocity, "channel": n.channel }));
    }
    let out_ccs = ccs
        .iter()
        .map(|c| json!({ "time": at.next(), "controller": c.controller, "value": c.value, "channel": c.channel }))
        .collect();
    Ok((out_notes, out_ccs))
}

fn name(v: &Value) -> &str {
    v["name"].as_str().unwrap_or("?")
}

/// Runs `mort midi`.
pub fn run(client: &mut Client, m: &Midi) -> Result<(Vec<String>, Value), Failure> {
    // The range is optional, so a first argument without `..` is an event.
    let (range_text, events): (Option<&String>, Vec<String>) = match &m.range {
        Some(r) if !r.contains("..") => (
            None,
            std::iter::once(r.clone())
                .chain(m.events.iter().cloned())
                .collect(),
        ),
        r => (r.as_ref(), m.events.clone()),
    };
    let list = client.call("tracks.list", Value::Null)?;
    let state = list["state"].clone();
    let track = resolve::track(&list, &m.track)?.clone();
    let range = match range_text {
        None => None,
        Some(r) => {
            let s = seconds(client, std::slice::from_ref(r))?;
            Some((
                s[0][0].as_f64().unwrap_or(0.0),
                s[0][1].as_f64().unwrap_or(0.0),
            ))
        }
    };
    if events.is_empty() && m.file.is_none() {
        return list_midi(client, &track, range);
    }
    let (notes, ccs) = match &m.file {
        None => from_tokens(client, &events)?,
        Some(path) => {
            if !events.is_empty() {
                return Err(Failure::Local("give events or --file, not both".into()));
            }
            let bytes = std::fs::read(path)
                .map_err(|e| Failure::Local(format!("{}: {e}", path.display())))?;
            if bytes.starts_with(b"MThd") {
                let (start, _) = range.ok_or_else(|| {
                    Failure::Local(
                        "a MIDI file needs a range to say where it starts, such as `9.1..17.1`"
                            .into(),
                    )
                })?;
                from_smf(client, &bytes, start, &path.display().to_string())?
            } else {
                let text = String::from_utf8_lossy(&bytes);
                let tokens: Vec<String> = text
                    .lines()
                    .map(|l| l.split('#').next().unwrap_or(""))
                    .flat_map(str::split_whitespace)
                    .map(str::to_owned)
                    .collect();
                from_tokens(client, &tokens)?
            }
        }
    };
    if m.replace && range.is_none() {
        return Err(Failure::Local(
            "--replace needs a range to clear, such as `9.1..17.1`".into(),
        ));
    }
    let mut params = json!({
        "track": track["id"],
        "notes": notes,
        "ccs": ccs,
        "mode": if m.replace { "replace" } else { "add" },
        "expect_state": state,
    });
    if let Some((s, e)) = range {
        params["start"] = json!(s);
        params["end"] = json!(e);
    }
    let what = format!(
        "{} note{}, {} CC{}",
        notes.len(),
        if notes.len() == 1 { "" } else { "s" },
        ccs.len(),
        if ccs.len() == 1 { "" } else { "s" }
    );
    params["summary"] = json!(format!(
        "MIDI on {}: {}{what}",
        name(&track),
        if m.replace { "replace with " } else { "add " }
    ));
    let r = client.call("midi.write", params)?;
    let mut lines = vec![format!(
        "{}: wrote {what}{}",
        name(&track),
        if r["created"] == true {
            " into a new item"
        } else {
            ""
        }
    )];
    let (shown, _) = list_midi(client, &track, range)?;
    lines.extend(shown);
    Ok((lines, r))
}

fn list_midi(
    client: &mut Client,
    track: &Value,
    range: Option<(f64, f64)>,
) -> Result<(Vec<String>, Value), Failure> {
    let mut p = json!({ "track": track["id"] });
    if let Some((s, e)) = range {
        p["start"] = json!(s);
        p["end"] = json!(e);
    }
    let r = client.call("midi.read", p)?;
    let items = r["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        return Ok((vec![format!("{}: no MIDI", name(track))], r));
    }
    let mut out = Vec::new();
    for it in &items {
        let notes = it["notes"].as_array().cloned().unwrap_or_default();
        let ccs = it["ccs"].as_array().cloned().unwrap_or_default();
        let pos = it["item"]["position"].as_f64().unwrap_or(0.0);
        let end = pos + it["item"]["length"].as_f64().unwrap_or(0.0);
        let mut times = vec![pos, end];
        times.extend(notes.iter().flat_map(|n| {
            [
                n["start"].as_f64().unwrap_or(0.0),
                n["end"].as_f64().unwrap_or(0.0),
            ]
        }));
        times.extend(ccs.iter().map(|c| c["time"].as_f64().unwrap_or(0.0)));
        let m = client.call("time.convert", json!({ "to_musical": times }))?;
        let mus: Vec<String> = m["musical"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| v.as_str().unwrap_or("?").to_owned())
            .collect();
        out.push(format!(
            "{}: item {}–{}, {} notes, {} CCs",
            name(track),
            mus[0],
            mus[1],
            notes.len(),
            ccs.len()
        ));
        let mut rows = Vec::new();
        for (i, n) in notes.iter().enumerate() {
            let pitch = u8::try_from(n["pitch"].as_u64().unwrap_or(0)).unwrap_or(0);
            rows.push(vec![
                format!("{}–{}", mus[2 + 2 * i], mus[3 + 2 * i]),
                format!("{} ({pitch})", pitch_name(pitch)),
                format!("v{}", n["velocity"]),
                format!("ch{}", n["channel"].as_u64().unwrap_or(0) + 1),
            ]);
        }
        let base = 2 + 2 * notes.len();
        for (i, c) in ccs.iter().enumerate() {
            rows.push(vec![
                mus[base + i].clone(),
                format!("cc{} = {}", c["controller"], c["value"]),
                String::new(),
                format!("ch{}", c["channel"].as_u64().unwrap_or(0) + 1),
            ]);
        }
        out.extend(show::table(&rows).into_iter().map(|l| format!("  {l}")));
    }
    Ok((out, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitches() {
        assert_eq!(parse_pitch("C4").unwrap(), 60);
        assert_eq!(parse_pitch("c#4").unwrap(), 61);
        assert_eq!(parse_pitch("Db4").unwrap(), 61);
        assert_eq!(parse_pitch("C-1").unwrap(), 0);
        assert_eq!(parse_pitch("G9").unwrap(), 127);
        assert_eq!(parse_pitch("64").unwrap(), 64);
        assert!(parse_pitch("H2").is_err());
        assert!(parse_pitch("G#9").is_err());
        assert_eq!(pitch_name(60), "C4");
        assert_eq!(pitch_name(70), "A#4");
        assert_eq!(pitch_name(0), "C-1");
    }

    #[test]
    fn tokens() {
        let Event::Note {
            pitch,
            start,
            length,
            velocity,
            channel,
        } = parse_event("E2:+1b").unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (pitch, start.as_str(), length.as_str(), velocity, channel),
            (40, "+1b", "1b", 100, 0)
        );
        let Event::Note {
            velocity,
            channel,
            length,
            ..
        } = parse_event("C4:9.1:0.5s:64:10").unwrap()
        else {
            panic!()
        };
        assert_eq!((velocity, channel, length.as_str()), (64, 9, "0.5s"));
        let Event::Cc {
            controller,
            value,
            channel,
            start,
        } = parse_event("cc74@2:9.2=127").unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (controller, value, channel, start.as_str()),
            (74, 127, 1, "9.2")
        );
        assert!(parse_event("C4").is_err());
        assert!(parse_event("C4:9.1:2").is_err());
        assert!(parse_event("C4:9.1:1b:0").is_err());
        assert!(parse_event("cc1:9.1").is_err());
    }
}
