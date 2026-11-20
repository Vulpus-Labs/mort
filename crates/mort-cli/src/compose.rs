//! `mort compose` (ADR 0013 §9, §10; ticket 0028).
//!
//! ```text
//! mort compose show verse                 # voiced blocks, labelled
//! mort compose show verse --notes         # the notes each part would write
//! mort compose show verse --part Keys --sheet draft.kdl
//! mort compose write                      # write what changed, one undo point
//! mort compose write --part Pad --section verse --force
//! ```
//!
//! The sheet is `<project>.mort/sheet.kdl`. Each part writes one MIDI item
//! per section, covering the section's region. What was written is kept
//! in `<project>.mort/compose/state.json`, so only items whose notes would
//! change are written again, and items edited by hand are left alone.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use mort_compose::articulate::{Event, Performance, play_part};
use mort_compose::harmony::Harmony;
use mort_compose::lead::{Block, Tone, voice_part};
use mort_compose::pattern::Patterns;
use mort_compose::pitch::Pitch;
use mort_compose::record::{self, Action, Entry, Layers, Record};
use mort_compose::sheet::{self, Section, Sheet, TICKS_PER_QUARTER};
use mort_compose::voicing::Vocabularies;

use crate::client::{Client, Failure};
use crate::resolve;

/// `mort compose` subcommands.
#[derive(Subcommand)]
pub enum ComposeCmd {
    /// Show a section's voiced blocks (the default) or the notes each part
    /// would write. Writes nothing.
    Show {
        /// The section.
        section: String,
        /// Only this part.
        #[arg(long)]
        part: Option<String>,
        /// Show the notes after articulation and feel, not the blocks.
        #[arg(long)]
        notes: bool,
        #[command(flatten)]
        sheet: SheetArg,
    },
    /// Write the parts into the project: one MIDI item per part per
    /// section, only where the notes would change, as one undo point.
    Write {
        /// Only this part.
        #[arg(long)]
        part: Option<String>,
        /// Only this section.
        #[arg(long)]
        section: Option<String>,
        /// Overwrite items edited by hand, and rewrite unchanged ones.
        #[arg(long)]
        force: bool,
    },
}

/// Where the sheet is.
#[derive(Args)]
pub struct SheetArg {
    /// Read this sheet instead of the project's `<project>.mort/sheet.kdl`.
    #[arg(long, value_name = "PATH")]
    sheet: Option<PathBuf>,
}

fn local(m: impl Into<String>) -> Failure {
    Failure::Local(m.into())
}

/// The project's sidecar directory: `song.rpp` keeps it in `song.mort`.
fn sidecar(project: &Value) -> Option<PathBuf> {
    project["project"]["path"]
        .as_str()
        .map(|p| Path::new(p).with_extension("mort"))
}

/// The sheet, vocabularies and patterns.
struct Loaded {
    sheet: Sheet,
    vocabularies: Vocabularies,
    patterns: Patterns,
}

fn load(sheet_path: &Path, sidecar: Option<&Path>) -> Result<Loaded, Failure> {
    let text = std::fs::read_to_string(sheet_path).map_err(|e| {
        local(format!(
            "cannot read the sheet {}: {e}; write one there (see ADR 0013), or give --sheet",
            sheet_path.display()
        ))
    })?;
    let sheet = sheet::parse(&text).map_err(|e| local(format!("{}: {e}", sheet_path.display())))?;
    let vocabularies = Vocabularies::standard(sidecar).map_err(|e| local(e.to_string()))?;
    let patterns = Patterns::standard(sidecar).map_err(|e| local(e.to_string()))?;
    Ok(Loaded {
        sheet,
        vocabularies,
        patterns,
    })
}

/// Runs `mort compose …`.
pub fn run(client: &mut Client, cmd: &ComposeCmd) -> Result<(Vec<String>, Value), Failure> {
    let project = client.call("project.get", Value::Null)?;
    let side = sidecar(&project);
    match cmd {
        ComposeCmd::Show {
            section,
            part,
            notes,
            sheet,
        } => {
            let path = match (&sheet.sheet, &side) {
                (Some(p), _) => p.clone(),
                (None, Some(s)) => s.join("sheet.kdl"),
                (None, None) => {
                    return Err(local(
                        "the project is not saved, so it has no sheet beside it; save it, or give --sheet",
                    ));
                }
            };
            let l = load(&path, side.as_deref())?;
            let state = client.call("bridge.status", Value::Null)?["state"].clone();
            let (lines, value) = show(&l, section, part.as_deref(), *notes)?;
            Ok((lines, json!({ "parts": value, "state": state })))
        }
        ComposeCmd::Write {
            part,
            section,
            force,
        } => {
            let side = side.ok_or_else(|| {
                local("save the project first: composing keeps its record beside the project file")
            })?;
            let l = load(&side.join("sheet.kdl"), Some(&side))?;
            write(
                client,
                &project,
                &side,
                &l,
                part.as_deref(),
                section.as_deref(),
                *force,
            )
        }
    }
}

/// `1.1`, `2.3.5`: a bar and beat, from 1, within a section.
fn where_in(section: &Section, ticks: u32) -> String {
    let bar = section.meter.bar_ticks();
    let beat = f64::from(ticks % bar) / f64::from(section.meter.beat_ticks()) + 1.0;
    format!("{}.{}", ticks / bar + 1, beat)
}

fn parts_to_show<'a>(
    sheet: &'a Sheet,
    part: Option<&str>,
) -> Result<Vec<&'a mort_compose::part::Part>, Failure> {
    match part {
        Some(name) => sheet
            .part(name)
            .map(|p| vec![p])
            .ok_or_else(|| local(format!("the sheet has no part `{name}`"))),
        None => Ok(sheet.parts.iter().collect()),
    }
}

fn show(
    l: &Loaded,
    section: &str,
    part: Option<&str>,
    notes: bool,
) -> Result<(Vec<String>, Value), Failure> {
    let found = l.sheet.section(section).ok_or_else(|| {
        let names: Vec<&str> = l.sheet.sections.iter().map(|s| s.name.as_str()).collect();
        local(format!(
            "the sheet has no section `{section}`; it has {}",
            names.join(", ")
        ))
    })?;
    let mut lines = Vec::new();
    let mut out = Vec::new();
    for p in parts_to_show(&l.sheet, part)? {
        if !p.plays_in(section) {
            continue;
        }
        if notes {
            let perf = play_part(&l.sheet, p, &l.vocabularies, &l.patterns)
                .map_err(|e| local(e.to_string()))?;
            let blocks = voice_part(&l.sheet, p, &l.vocabularies).ok();
            lines.push(format!(
                "{}  ({})",
                p.name,
                p.spec_in(section)
                    .articulation
                    .unwrap_or_else(|| "sustain".into())
            ));
            lines.extend(note_lines(found, &perf, blocks.as_deref()));
            out.push(json!({
                "part": p.name,
                "events": perf.events.iter().filter(|e| e.section == section).map(|e| json!({
                    "start": e.start, "length": e.length, "pitch": e.pitch, "velocity": e.velocity
                })).collect::<Vec<_>>(),
                "problems": perf.problems.iter().filter(|x| x.section == section).map(ToString::to_string).collect::<Vec<_>>(),
            }));
        } else {
            let spec = p.spec_in(section);
            let blocks = match &p.from {
                Some((other, voices)) => {
                    let which = if voices.is_empty() {
                        "every voice".to_owned()
                    } else {
                        format!("the {} voice", voices.join(" and "))
                    };
                    lines.push(format!("{}  (plays {which} of {other})", p.name));
                    continue;
                }
                None if spec.vocabulary.is_none() => {
                    let pattern = spec.articulation.unwrap_or_else(|| "sustain".into());
                    let kind = if l.patterns.get(&pattern).is_some_and(|x| x.drums) {
                        "drums"
                    } else {
                        "no voicing"
                    };
                    lines.push(format!("{}  ({kind}: {pattern})", p.name));
                    continue;
                }
                None => {
                    voice_part(&l.sheet, p, &l.vocabularies).map_err(|e| local(e.to_string()))?
                }
            };
            lines.push(format!(
                "{}  ({}, {} voices, {})",
                p.name,
                spec.vocabulary.unwrap_or_default(),
                spec.voices.unwrap_or(0),
                spec.range.map(|r| r.to_string()).unwrap_or_default()
            ));
            let here: Vec<&Block> = blocks.iter().filter(|b| b.section == section).collect();
            lines.extend(block_lines(found, &here));
            out.push(json!({ "part": p.name, "blocks": here.iter().map(|b| block_json(b)).collect::<Vec<_>>() }));
        }
    }
    if lines.is_empty() {
        lines.push(format!("no part plays in `{section}`"));
    }
    Ok((lines, Value::Array(out)))
}

fn block_lines(sec: &Section, blocks: &[&Block]) -> Vec<String> {
    let mut lines = Vec::new();
    for b in blocks {
        let chord: Vec<String> = b
            .notes
            .iter()
            .filter(|n| n.tone == Tone::Chord)
            .map(|n| {
                let role = n.role.map(|r| format!(" {r}")).unwrap_or_default();
                format!("{}{role}", b.harmony.spell(n.pitch))
            })
            .collect();
        lines.push(format!(
            "  {:<7} {:<26} {}",
            where_in(sec, b.start),
            b.harmony.to_string(),
            chord.join(" · ")
        ));
        for n in b.notes.iter().filter(|n| n.tone != Tone::Chord) {
            lines.push(format!(
                "  {:<7} {:<26} + {} {} ({}) to {}",
                where_in(sec, n.start),
                "",
                n.tone,
                b.harmony.spell(n.pitch),
                n.voice,
                where_in(sec, n.start + n.length)
            ));
        }
    }
    lines
}

fn block_json(b: &Block) -> Value {
    json!({
        "start": b.start,
        "length": b.length,
        "harmony": b.text,
        "scale": b.harmony.scale.to_string(),
        "notes": b.notes.iter().map(|n| json!({
            "pitch": n.pitch.get(),
            "name": b.harmony.spell(n.pitch),
            "voice": n.voice.to_string(),
            "role": n.role.map(|r| r.to_string()),
            "tone": n.tone.to_string(),
            "start": n.start,
            "length": n.length,
        })).collect::<Vec<_>>(),
    })
}

fn note_lines(sec: &Section, perf: &Performance, blocks: Option<&[Block]>) -> Vec<String> {
    let harmony_at = |t: u32| -> Option<&Harmony> {
        blocks?
            .iter()
            .find(|b| b.section == sec.name && b.start <= t && t < b.start + b.length)
            .map(|b| &b.harmony)
    };
    let mut by_start: BTreeMap<u32, Vec<&Event>> = BTreeMap::new();
    for e in perf.events.iter().filter(|e| e.section == sec.name) {
        by_start.entry(e.start).or_default().push(e);
    }
    let mut lines = Vec::new();
    for (t, mut es) in by_start {
        es.sort_by_key(|e| e.pitch);
        let mut line = format!("  {:<9}", where_in(sec, t));
        for e in es {
            let name = Pitch::new(i32::from(e.pitch)).map_or_else(
                || e.pitch.to_string(),
                |p| harmony_at(t).map_or_else(|| p.to_string(), |h| h.spell(p)),
            );
            let _ = write!(line, " {name}");
        }
        lines.push(line);
    }
    for p in perf.problems.iter().filter(|p| p.section == sec.name) {
        lines.push(format!("  ! {p}"));
    }
    lines
}

/// What one part in one section would write.
struct Plan {
    part: String,
    section: String,
    track: Value,
    region: (f64, f64),
    events: Vec<Event>,
    drums: bool,
    generated: String,
    layers: Layers,
}

#[expect(
    clippy::too_many_lines,
    reason = "one pass over parts and sections, in order"
)]
fn write(
    client: &mut Client,
    project: &Value,
    side: &Path,
    l: &Loaded,
    only_part: Option<&str>,
    only_section: Option<&str>,
    force: bool,
) -> Result<(Vec<String>, Value), Failure> {
    let list = client.call("tracks.list", Value::Null)?;
    let state = list["state"].clone();
    let mut rec = Record::load(side).map_err(local)?;
    if let Some(s) = only_section
        && l.sheet.section(s).is_none()
    {
        return Err(local(format!("the sheet has no section `{s}`")));
    }

    // Regions by name, each checked against its section's length.
    let regions: Vec<Value> = project["regions"].as_array().cloned().unwrap_or_default();
    let mut region_of: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for sec in &l.sheet.sections {
        let r = regions
            .iter()
            .find(|r| r["name"].as_str() == Some(sec.name.as_str()) && r["end"].is_number())
            .ok_or_else(|| {
                local(format!(
                    "section `{}` has no region; add one with `mort region add \"{}\" <start>..<end>`",
                    sec.name, sec.name
                ))
            })?;
        let (start, end) = (
            r["start"].as_f64().unwrap_or(0.0),
            r["end"].as_f64().unwrap_or(0.0),
        );
        let qn = f64::from(sec.length()) / f64::from(TICKS_PER_QUARTER);
        let s = client.call(
            "time.convert",
            json!({ "to_seconds": [format!("{start}s"), format!("+{qn}b")] }),
        )?;
        let want = s["seconds"][1].as_f64().unwrap_or(start);
        if (want - end).abs() > 0.002 {
            return Err(local(format!(
                "region `{}` runs {start:.3}s to {end:.3}s, but the sheet's {} bars of {} end at {want:.3}s; make them agree",
                sec.name, sec.bars, sec.meter
            )));
        }
        region_of.insert(sec.name.clone(), (start, end));
    }

    // Plan every part and section.
    let mut plans = Vec::new();
    let mut problems = Vec::new();
    for p in parts_to_show(&l.sheet, only_part)? {
        let track = resolve::track(&list, &p.track)
            .map_err(|_| {
                local(format!(
                    "part `{}` writes to track `{}`, which the project does not have; add it with `mort track add \"{}\"`",
                    p.name, p.track, p.track
                ))
            })?
            .clone();
        let perf = play_part(&l.sheet, p, &l.vocabularies, &l.patterns)
            .map_err(|e| local(e.to_string()))?;
        problems.extend(perf.problems.iter().map(|x| format!("{}: {x}", p.name)));
        for sec in &l.sheet.sections {
            if !p.plays_in(&sec.name) || only_section.is_some_and(|s| s != sec.name) {
                continue;
            }
            let events: Vec<Event> = perf
                .events
                .iter()
                .filter(|e| e.section == sec.name)
                .cloned()
                .collect();
            let drums = l
                .patterns
                .get(
                    &p.spec_in(&sec.name)
                        .articulation
                        .unwrap_or_else(|| "sustain".into()),
                )
                .is_some_and(|x| x.drums);
            if events.is_empty() && !drums && sec.spans.is_empty() {
                continue;
            }
            plans.push(Plan {
                part: p.name.clone(),
                section: sec.name.clone(),
                track: track.clone(),
                region: region_of[&sec.name],
                generated: record::events_hash(&events.iter().collect::<Vec<_>>()),
                layers: Layers::of(&l.sheet, p, &sec.name, &l.vocabularies, &l.patterns),
                events,
                drums,
            });
        }
    }
    for (i, a) in plans.iter().enumerate() {
        if let Some(b) = plans[i + 1..]
            .iter()
            .find(|b| b.track["id"] == a.track["id"] && b.section == a.section)
        {
            return Err(local(format!(
                "parts `{}` and `{}` both write track `{}` in section `{}`; give each its own track",
                a.part,
                b.part,
                a.track["name"].as_str().unwrap_or("?"),
                a.section
            )));
        }
    }

    // What is there now: mort's items and their notes, and anyone else's.
    let ours: Vec<String> = rec.entries.values().map(|e| e.item.clone()).collect();
    let mut decisions = Vec::new();
    for plan in &plans {
        let (start, end) = plan.region;
        let items = client.call("items.list", json!({ "track": plan.track["id"] }))?;
        let foreign = items["items"].as_array().into_iter().flatten().find(|it| {
            let (pos, len) = (
                it["position"].as_f64().unwrap_or(0.0),
                it["length"].as_f64().unwrap_or(0.0),
            );
            it["midi"].as_bool() == Some(true)
                && pos < end - 1e-4
                && pos + len > start + 1e-4
                && !ours.iter().any(|o| it["id"].as_str() == Some(o))
        });
        if foreign.is_some() {
            return Err(local(format!(
                "track `{}` has MIDI in section `{}` that mort did not write; move it, or write part `{}` to another track",
                plan.track["name"].as_str().unwrap_or("?"),
                plan.section,
                plan.part
            )));
        }
        let before = rec.entries.get(&Record::key(&plan.part, &plan.section));
        let present = match before {
            Some(b) => current_hash(client, &plan.track["id"], plan.region, &b.item)?,
            None => None,
        };
        decisions.push(record::decide(
            before,
            present.as_deref(),
            &plan.generated,
            &plan.layers,
            force,
        ));
    }

    // Convert the notes to write into seconds through the tempo map.
    let mut calls = Vec::new();
    let mut written = Vec::new();
    for (plan, action) in plans.iter().zip(&decisions) {
        if !matches!(action, Action::Write(_)) {
            continue;
        }
        let base = format!("{}s", plan.region.0);
        let mut texts = Vec::with_capacity(plan.events.len() * 4);
        for e in &plan.events {
            let q = |t: u32| format!("+{:.6}b", f64::from(t) / f64::from(TICKS_PER_QUARTER));
            texts.extend([
                base.clone(),
                q(e.start),
                base.clone(),
                q(e.start + e.length),
            ]);
        }
        let secs = if texts.is_empty() {
            Vec::new()
        } else {
            client.call("time.convert", json!({ "to_seconds": texts }))?["seconds"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        };
        let channel = if plan.drums { 9 } else { 0 };
        let notes: Vec<Value> = plan
            .events
            .iter()
            .enumerate()
            .map(|(i, e)| {
                json!({
                    "pitch": e.pitch,
                    "start": secs.get(i * 4 + 1).and_then(Value::as_f64).unwrap_or(plan.region.0),
                    "end": secs.get(i * 4 + 3).and_then(Value::as_f64).unwrap_or(plan.region.1).min(plan.region.1),
                    "velocity": e.velocity,
                    "channel": channel,
                })
            })
            .collect();
        calls.push(json!({
            "method": "midi.write",
            "params": {
                "track": plan.track["id"],
                "start": plan.region.0,
                "end": plan.region.1,
                "notes": notes,
                "mode": "replace",
            },
        }));
        written.push(plan);
    }

    let mut result = json!({ "state": state });
    if !calls.is_empty() {
        let label = match only_part {
            Some(p) => format!("compose {p}"),
            None => "compose".to_owned(),
        };
        let r = client.call(
            "batch",
            json!({ "calls": calls, "summary": label, "expect_state": state }),
        )?;
        let results = r["results"].as_array().cloned().unwrap_or_default();
        for (plan, res) in written.iter().zip(&results) {
            let item = res["item"].as_str().unwrap_or_default().to_owned();
            let now =
                current_hash(client, &plan.track["id"], plan.region, &item)?.unwrap_or_default();
            rec.entries.insert(
                Record::key(&plan.part, &plan.section),
                Entry {
                    part: plan.part.clone(),
                    section: plan.section.clone(),
                    track: plan.track["id"].as_str().unwrap_or_default().to_owned(),
                    item,
                    generated: plan.generated.clone(),
                    written: now,
                    layers: plan.layers.clone(),
                },
            );
        }
        rec.save(side).map_err(local)?;
        result = r;
    }

    let mut lines = Vec::new();
    let mut summary = Vec::new();
    for (plan, action) in plans.iter().zip(&decisions) {
        let what = match action {
            Action::Write(why) => format!("written: {why}"),
            Action::Unchanged => "unchanged".to_owned(),
            Action::EditedByHand => "edited by hand; left alone (--force overwrites it)".to_owned(),
        };
        lines.push(format!("{:<10} {:<12} {what}", plan.part, plan.section));
        summary.push(json!({ "part": plan.part, "section": plan.section, "action": what }));
    }
    for p in &problems {
        lines.push(format!("! {p}"));
    }
    if lines.is_empty() {
        lines.push("nothing to write".to_owned());
    }
    result["compose"] = json!({ "items": summary, "problems": problems });
    Ok((lines, result))
}

/// The hash of the notes in item `item` on `track` within `region`, or
/// `None` if the item is gone.
fn current_hash(
    client: &mut Client,
    track: &Value,
    region: (f64, f64),
    item: &str,
) -> Result<Option<String>, Failure> {
    let r = client.call(
        "midi.read",
        json!({ "track": track, "start": region.0, "end": region.1 }),
    )?;
    let Some(it) = r["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|i| i["item"]["id"].as_str() == Some(item))
    else {
        return Ok(None);
    };
    let notes: Vec<(u8, f64, f64, u8)> = it["notes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|n| {
            (
                u8::try_from(n["pitch"].as_u64().unwrap_or(0)).unwrap_or(0),
                n["start"].as_f64().unwrap_or(0.0) - region.0,
                n["end"].as_f64().unwrap_or(0.0) - region.0,
                u8::try_from(n["velocity"].as_u64().unwrap_or(0)).unwrap_or(0),
            )
        })
        .collect();
    Ok(Some(record::written_hash(&notes)))
}
