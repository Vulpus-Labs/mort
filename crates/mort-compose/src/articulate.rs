//! Articulation and performance (ADR 0013 §8): blocks turned into note
//! events by a pattern, checked for playability, then given a feel.

use std::fmt;

use crate::lead::{Block, LeadError, Tone, Voice, voice_part};
use crate::part::Part;
use crate::pattern::{Approach, Arp, At, Hit, Order, Pattern, Patterns, Walk};
use crate::pitch::{Pitch, PitchClass};
use crate::sheet::{Section, Sheet};
use crate::voicing::{Range, Vocabularies};

/// A note to write.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Event {
    /// The section it starts in.
    pub section: String,
    /// Ticks from the section's start.
    pub start: u32,
    /// Its length in ticks.
    pub length: u32,
    /// MIDI pitch.
    pub pitch: u8,
    /// MIDI velocity.
    pub velocity: u8,
}

/// Something a player could not do, reported but not changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// The section.
    pub section: String,
    /// The bar, from 1.
    pub bar: u32,
    /// The beat, from 1.
    pub beat: f64,
    /// What, and what to do.
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "section `{}`, bar {} beat {}: {}",
            self.section, self.bar, self.beat, self.message
        )
    }
}

/// A part's notes, and what could not be played as written.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Performance {
    /// The notes, by section in form order, then start, then pitch.
    pub events: Vec<Event>,
    /// Playability problems.
    pub problems: Vec<Problem>,
}

/// Why a part could not be played.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayError {
    /// Voice-leading failed.
    Lead(LeadError),
    /// The articulation could not be applied.
    Pattern {
        /// The part.
        part: String,
        /// What went wrong, and what to do.
        message: String,
    },
}

impl fmt::Display for PlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlayError::Lead(e) => e.fmt(f),
            PlayError::Pattern { part, message } => write!(f, "part `{part}`: {message}"),
        }
    }
}

impl std::error::Error for PlayError {}

impl From<LeadError> for PlayError {
    fn from(e: LeadError) -> Self {
        PlayError::Lead(e)
    }
}

/// The bar and beat, from 1, of `ticks` into `section`.
fn bar_beat(section: &Section, ticks: u32) -> (u32, f64) {
    let bar = section.meter.bar_ticks();
    (
        ticks / bar + 1,
        f64::from(ticks % bar) / f64::from(section.meter.beat_ticks()) + 1.0,
    )
}

fn beats_to_ticks(section: &Section, beats: f64) -> u32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a non-negative tick count"
    )]
    let t = (beats * f64::from(section.meter.beat_ticks()))
        .round()
        .max(0.0) as u32;
    t
}

/// Plays `part`: voices it (or takes another part's voices), applies its
/// articulation in each section, checks playability and applies its feel.
///
/// # Errors
///
/// [`PlayError`] if voicing fails, or a pattern, kit or `from` part is
/// unknown.
pub fn play_part(
    sheet: &Sheet,
    part: &Part,
    vocabularies: &Vocabularies,
    patterns: &Patterns,
) -> Result<Performance, PlayError> {
    let err = |message: String| PlayError::Pattern {
        part: part.name.clone(),
        message,
    };
    let pattern_in = |section: &str| -> Result<&Pattern, PlayError> {
        let name = part
            .spec_in(section)
            .articulation
            .unwrap_or_else(|| "sustain".into());
        patterns.get(&name).ok_or_else(|| {
            let known: Vec<&str> = patterns.iter().map(|p| p.name.as_str()).collect();
            err(format!(
                "`{name}` is not a pattern; known ones are {}",
                known.join(", ")
            ))
        })
    };
    let plays = |s: &Section| part.plays_in(&s.name);
    let mut pitched = false;
    for s in sheet.sections.iter().filter(|s| plays(s)) {
        pitched |= !pattern_in(&s.name)?.drums && !s.spans.is_empty();
    }
    let blocks = if pitched {
        blocks_for(sheet, part, vocabularies)?
    } else {
        Vec::new()
    };

    let mut out = Performance::default();
    let mut walked: Option<u8> = None;
    for section in sheet.sections.iter().filter(|s| plays(s)) {
        let pattern = pattern_in(&section.name)?;
        let spec = part.spec_in(&section.name);
        let mut events = Vec::new();
        if pattern.drums {
            drums(section, pattern, &spec, patterns, &mut events).map_err(err)?;
        } else {
            let here: Vec<(usize, &Block)> = blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| b.section == section.name)
                .collect();
            if here.is_empty() {
                continue;
            }
            let next = |i: usize| -> Option<&Block> {
                blocks
                    .get(i + 1)
                    .filter(|b| crate::lead::adjacent(sheet, &blocks[i], b))
            };
            if let Some(arp) = &pattern.arp {
                for (_, b) in &here {
                    arpeggiate(section, b, arp, pattern.sound_nct, &mut events);
                }
            } else if let Some(walk) = &pattern.walk {
                // A bass's usual walking range unless the part gives one.
                let range = spec
                    .instrument
                    .or_else(|| Range::parse("E1..G3"))
                    .ok_or_else(|| err("the default walking range does not read".into()))?;
                for (i, b) in &here {
                    walked = Some(walk_block(
                        section,
                        b,
                        next(*i),
                        *walk,
                        range,
                        walked,
                        &mut events,
                    ));
                }
            } else {
                for hit in &pattern.hits {
                    strike(
                        section,
                        pattern,
                        hit,
                        &here,
                        &next,
                        &mut events,
                        &mut out.problems,
                    );
                }
            }
            if let Some(range) = spec.instrument {
                out_of_range(section, &events, range, &mut out.problems);
            }
        }
        for e in &mut events {
            e.length = e
                .length
                .min(section.length().saturating_sub(e.start))
                .max(1);
        }
        events.sort();
        events.dedup_by(|a, b| a.start == b.start && a.pitch == b.pitch);
        crate::perform::apply(&mut events, section, &spec.feel, &part.name);
        out.events.extend(events);
    }
    Ok(out)
}

/// The blocks a pitched part plays: its own voicing, or the voices it
/// takes `from` another part.
fn blocks_for(
    sheet: &Sheet,
    part: &Part,
    vocabularies: &Vocabularies,
) -> Result<Vec<Block>, PlayError> {
    let Some((other, voices)) = &part.from else {
        return Ok(voice_part(sheet, part, vocabularies)?);
    };
    let source = sheet.part(other).ok_or_else(|| PlayError::Pattern {
        part: part.name.clone(),
        message: format!("`from` names part `{other}`, which the sheet does not have"),
    })?;
    let mut blocks = voice_part(sheet, source, vocabularies)?;
    blocks.retain(|b| part.plays_in(&b.section));
    if !voices.is_empty() {
        for b in &mut blocks {
            b.notes
                .retain(|n| voices.iter().any(|v| voice_matches(v, n.voice)));
        }
    }
    Ok(blocks)
}

/// A drum pattern's hits through a section, mapped by the part's kit.
fn drums(
    section: &Section,
    pattern: &Pattern,
    spec: &crate::part::Spec,
    patterns: &Patterns,
    events: &mut Vec<Event>,
) -> Result<(), String> {
    let kit_name = spec.kit.clone().unwrap_or_else(|| "gm".into());
    let kit = patterns.kit(&kit_name).ok_or_else(|| {
        format!("`{kit_name}` is not a kit; use `gm` or define one in a pattern file")
    })?;
    for hit in &pattern.hits {
        for piece in &hit.select {
            let Some(note) = kit.pieces.get(piece) else {
                return Err(format!(
                    "pattern `{}` strikes `{piece}`, which kit `{kit_name}` has no note for",
                    pattern.name
                ));
            };
            for (tick, len) in hit_times(section, pattern, hit, None) {
                events.push(Event {
                    section: section.name.clone(),
                    start: tick,
                    length: len.min(section.meter.beat_ticks() / 2).max(1),
                    pitch: *note,
                    velocity: hit.velocity,
                });
            }
        }
    }
    Ok(())
}

/// Reports notes outside the instrument's range.
fn out_of_range(section: &Section, events: &[Event], range: Range, problems: &mut Vec<Problem>) {
    for e in events {
        if e.pitch < range.low.get() || e.pitch > range.high.get() {
            let (bar, beat) = bar_beat(section, e.start);
            let name = Pitch::new(i32::from(e.pitch))
                .map_or_else(|| e.pitch.to_string(), |p| p.to_string());
            problems.push(Problem {
                section: section.name.clone(),
                bar,
                beat,
                message: format!(
                    "{name} is outside the instrument's range {range}; move the part's voicing range or the pattern's octave"
                ),
            });
        }
    }
}

/// Whether a voice name (`bass`, `inner`, `inner2`, `top`, `all`) names
/// `voice`.
fn voice_matches(name: &str, voice: Voice) -> bool {
    match name {
        "all" => true,
        "bass" => voice == Voice::Bass,
        "top" => voice == Voice::Top,
        "inner" => matches!(voice, Voice::Inner(_)),
        other => other
            .strip_prefix("inner")
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| voice == Voice::Inner(n)),
    }
}

fn is_voice_name(name: &str) -> bool {
    matches!(name, "all" | "bass" | "top" | "inner") || name.starts_with("inner")
}

/// The ticks (from the section's start) and default lengths of a hit's
/// strikes in `section`. `At::Change` uses `blocks`' starts.
fn hit_times(
    section: &Section,
    pattern: &Pattern,
    hit: &Hit,
    blocks: Option<&[(usize, &Block)]>,
) -> Vec<(u32, u32)> {
    let bar_ticks = section.meter.bar_ticks();
    match &hit.at {
        At::Change => blocks
            .unwrap_or_default()
            .iter()
            .map(|(_, b)| (b.start, b.length))
            .collect(),
        At::Positions(ps) => {
            let cycle = pattern.bars * bar_ticks;
            // Positions past the end of this meter's bar (beat 4 in 3/4)
            // fall away.
            let mut offs: Vec<u32> = ps
                .iter()
                .map(|(bar, beat)| (bar, beats_to_ticks(section, *beat)))
                .filter(|(_, t)| *t < bar_ticks)
                .map(|(bar, t)| bar * bar_ticks + t)
                .collect();
            if offs.is_empty() {
                return Vec::new();
            }
            offs.sort_unstable();
            offs.dedup();
            let mut out = Vec::new();
            for bar in 0..section.bars {
                let base = (bar / pattern.bars) * cycle;
                for (k, off) in offs.iter().enumerate() {
                    if off / bar_ticks != bar % pattern.bars {
                        continue;
                    }
                    let next = offs.get(k + 1).copied().unwrap_or(offs[0] + cycle);
                    let len = hit
                        .length
                        .map_or(next - off, |l| beats_to_ticks(section, l));
                    let tick = base + off;
                    if tick < section.length() {
                        out.push((tick, len));
                    }
                }
            }
            out
        }
    }
}

/// The pitches a hit selects from `b` at `tick`.
fn select(b: &Block, hit: &Hit, sound_nct: bool, tick: u32) -> Vec<u8> {
    let (voices, roles): (Vec<&String>, Vec<&String>) =
        hit.select.iter().partition(|s| is_voice_name(s));
    let mut seen: Vec<Voice> = b.notes.iter().map(|n| n.voice).collect();
    seen.dedup();
    let mut out = Vec::new();
    for v in seen {
        let note = if sound_nct {
            b.notes
                .iter()
                .find(|n| n.voice == v && n.start <= tick && tick < n.start + n.length)
                .or_else(|| {
                    b.notes
                        .iter()
                        .find(|n| n.voice == v && n.tone == Tone::Chord)
                })
        } else {
            b.notes
                .iter()
                .find(|n| n.voice == v && n.tone == Tone::Chord)
        };
        let Some(n) = note.or_else(|| b.notes.iter().find(|n| n.voice == v)) else {
            continue;
        };
        let voice_ok = voices.is_empty() || voices.iter().any(|x| voice_matches(x, v));
        let role_ok = roles.is_empty()
            || n.role
                .is_some_and(|r| roles.iter().any(|x| role_matches(x, r)));
        if voice_ok && role_ok {
            out.push(n.pitch.get());
        }
    }
    out
}

fn role_matches(name: &str, r: crate::chord::Role) -> bool {
    use crate::chord::Role;
    match name {
        "root" => r == Role::Root,
        "2nd" => r == Role::Second,
        "3rd" => r == Role::Third,
        "4th" => r == Role::Fourth,
        "5th" => r == Role::Fifth,
        "6th" => r == Role::Sixth,
        "7th" => r == Role::Seventh,
        "9th" => r == Role::Tension(9),
        "11th" => r == Role::Tension(11),
        "13th" => r == Role::Tension(13),
        "tension" => matches!(r, Role::Tension(_)),
        _ => false,
    }
}

/// Strikes a pitched hit through a section's blocks.
fn strike<'a>(
    section: &Section,
    pattern: &Pattern,
    hit: &Hit,
    here: &[(usize, &'a Block)],
    next: &dyn Fn(usize) -> Option<&'a Block>,
    events: &mut Vec<Event>,
    problems: &mut Vec<Problem>,
) {
    for (tick, len) in hit_times(section, pattern, hit, Some(here)) {
        let Some((i, b)) = here
            .iter()
            .find(|(_, b)| b.start <= tick && tick < b.start + b.length)
        else {
            continue;
        };
        let (src, lookup, room) = match next(*i).filter(|_| hit.anticipate) {
            Some(n) => (n, n.start, b.start + b.length - tick + n.length),
            None => (*b, tick, b.start + b.length - tick),
        };
        let mut pitches: Vec<i32> = select(src, hit, pattern.sound_nct, lookup)
            .into_iter()
            .map(|p| i32::from(p) + 12 * i32::from(hit.octave))
            .collect();
        let base = pitches.clone();
        for d in &hit.double {
            pitches.extend(base.iter().map(|p| p + i32::from(*d)));
        }
        let mut struck: Vec<u8> = pitches
            .into_iter()
            .filter_map(|p| u8::try_from(p).ok().filter(|p| *p <= 127))
            .collect();
        struck.sort_unstable();
        struck.dedup();
        if let (Some(hand), Some(lo), Some(hi)) = (&hit.hand, struck.first(), struck.last()) {
            let span = pattern.span(hand);
            if hi - lo > span {
                let (bar, beat) = bar_beat(section, tick);
                let name = |p: u8| {
                    Pitch::new(i32::from(p)).map_or_else(|| p.to_string(), |x| x.to_string())
                };
                problems.push(Problem {
                    section: section.name.clone(),
                    bar,
                    beat,
                    message: format!(
                        "the {hand} hand would span {} semitones ({} to {}), more than its {span}; narrow the voicing or widen the hand's span in pattern `{}`",
                        hi - lo,
                        name(*lo),
                        name(*hi),
                        pattern.name
                    ),
                });
            }
        }
        for p in struck {
            events.push(Event {
                section: section.name.clone(),
                start: tick,
                length: len.min(room),
                pitch: p,
                velocity: hit.velocity,
            });
        }
    }
}

fn arpeggiate(section: &Section, b: &Block, arp: &Arp, sound_nct: bool, events: &mut Vec<Event>) {
    let step = beats_to_ticks(section, arp.every).max(1);
    let mut chord: Vec<u8> = b
        .notes
        .iter()
        .filter(|n| sound_nct || n.tone == Tone::Chord)
        .map(|n| n.pitch.get())
        .collect();
    chord.sort_unstable();
    chord.dedup();
    if chord.is_empty() {
        return;
    }
    let order: Vec<u8> = match arp.order {
        Order::Up => chord.clone(),
        Order::Down => chord.iter().rev().copied().collect(),
        Order::UpDown => {
            let mut v = chord.clone();
            v.extend(
                chord
                    .iter()
                    .rev()
                    .skip(1)
                    .take(chord.len().saturating_sub(2)),
            );
            v
        }
    };
    let mut t = b.start;
    let mut k = 0;
    while t < b.start + b.length {
        events.push(Event {
            section: section.name.clone(),
            start: t,
            length: step.min(b.start + b.length - t),
            pitch: order[k % order.len()],
            velocity: arp.velocity,
        });
        t += step;
        k += 1;
    }
}

/// The pitch with class `pc` nearest `near` within `range`.
fn nearest(pc: PitchClass, near: i32, range: Range) -> i32 {
    let (lo, hi) = (i32::from(range.low.get()), i32::from(range.high.get()));
    (lo..=hi)
        .filter(|p| PitchClass::new(*p) == pc)
        .min_by_key(|p| ((p - near).abs(), *p))
        .unwrap_or_else(|| near.clamp(lo, hi))
}

/// A walking line through one block: its bass on the first beat, scale
/// steps towards the next block's bass, and an approach note on the last
/// beat. Returns the last pitch played.
fn walk_block(
    section: &Section,
    b: &Block,
    next: Option<&Block>,
    walk: Walk,
    range: Range,
    prev: Option<u8>,
    events: &mut Vec<Event>,
) -> u8 {
    let bass_pc = |x: &Block| {
        x.notes
            .iter()
            .find(|n| n.voice == Voice::Bass && n.tone == Tone::Chord)
            .or_else(|| x.notes.first())
            .map_or(x.harmony.bass, |n| n.pitch.class())
    };
    let beat = section.meter.beat_ticks();
    let count = (b.length / beat).max(1);
    let mid = i32::midpoint(i32::from(range.low.get()), i32::from(range.high.get()));
    let start = nearest(bass_pc(b), prev.map_or(mid, i32::from), range);
    let target = next.map(|n| nearest(bass_pc(n), start, range));
    let approach = target.map(|t| match walk.approach {
        Approach::Chromatic => {
            if t >= start {
                t - 1
            } else {
                t + 1
            }
        }
        Approach::Scale => {
            let dir = if t >= start { -1 } else { 1 };
            (1..=2)
                .map(|k| t + dir * k)
                .find(|p| b.harmony.scale.contains(PitchClass::new(*p)))
                .unwrap_or(t + dir)
        }
    });
    let chord: Vec<PitchClass> = b.harmony.tones.iter().map(|t| t.pc).collect();
    let mut line = vec![start];
    for k in 1..count {
        let last = *line.last().unwrap_or(&start);
        if k + 1 == count
            && let Some(a) = approach
        {
            line.push(a);
            break;
        }
        // Head for the approach note, or rise and fall around the start.
        let goal = approach.unwrap_or(start + 7);
        let want = start
            + (goal - start) * i32::try_from(k).unwrap_or(1)
                / i32::try_from(count.max(2) - 1).unwrap_or(1);
        let pick = (i32::from(range.low.get())..=i32::from(range.high.get()))
            .filter(|p| *p != last && b.harmony.scale.contains(PitchClass::new(*p)))
            .min_by_key(|p| {
                let off_chord = i32::from(!chord.contains(&PitchClass::new(*p)));
                ((p - want).abs() * 2 + off_chord, *p)
            })
            .unwrap_or(last);
        line.push(pick);
    }
    for (k, p) in line.iter().enumerate() {
        let t = b.start + u32::try_from(k).unwrap_or(0) * beat;
        if t >= b.start + b.length {
            break;
        }
        events.push(Event {
            section: section.name.clone(),
            start: t,
            length: beat.min(b.start + b.length - t),
            pitch: u8::try_from(*p).unwrap_or(0),
            velocity: walk.velocity,
        });
    }
    u8::try_from(*line.last().unwrap_or(&start)).unwrap_or(0)
}
