//! Voice-leading (ADR 0013 §6, §7): a part's voicings, chosen as the
//! cheapest path through each harmony's candidates, as labelled blocks.

use std::fmt;

use crate::chord::Role;
use crate::harmony::Harmony;
use crate::part::{Nct, Part, Spec, Weights};
use crate::pitch::Pitch;
use crate::sheet::{Section, Sheet, Span};
use crate::voicing::{Range, Vocabularies, VoicedNote, Voicing};

/// Which voice a note belongs to, from the bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Voice {
    /// The lowest.
    Bass,
    /// Between bass and top, numbered from 1 upwards.
    Inner(u8),
    /// The highest.
    Top,
}

impl Voice {
    /// The voice of the note at `index` from the bottom of `count`.
    #[must_use]
    pub fn of(index: usize, count: usize) -> Self {
        if index + 1 == count {
            Self::Top
        } else if index == 0 {
            Self::Bass
        } else {
            Self::Inner(u8::try_from(index).unwrap_or(u8::MAX))
        }
    }
}

impl fmt::Display for Voice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Voice::Bass => f.write_str("bass"),
            Voice::Inner(i) => write!(f, "inner{i}"),
            Voice::Top => f.write_str("top"),
        }
    }
}

/// Whether a note is a chord tone or a non-chord tone, and which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// A note of the voicing.
    Chord,
    /// A non-chord tone added between voicings.
    Nct(Nct),
}

impl fmt::Display for Tone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Tone::Chord => "chord",
            Tone::Nct(Nct::Passing) => "passing",
            Tone::Nct(Nct::Neighbour) => "neighbour",
            Tone::Nct(Nct::Suspension) => "suspension",
            Tone::Nct(Nct::Anticipation) => "anticipation",
            Tone::Nct(Nct::Pedal) => "pedal",
        };
        f.write_str(s)
    }
}

/// A note of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockNote {
    /// Its pitch.
    pub pitch: Pitch,
    /// Its voice.
    pub voice: Voice,
    /// Its role in the harmony it sounds over, if it has one there.
    pub role: Option<Role>,
    /// Chord tone or non-chord tone.
    pub tone: Tone,
    /// Ticks from the section's start.
    pub start: u32,
    /// Its length in ticks.
    pub length: u32,
}

/// A voiced harmony: one sustained sonority, with any non-chord tones.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// The section.
    pub section: String,
    /// Ticks from the section's start.
    pub start: u32,
    /// Its length in ticks.
    pub length: u32,
    /// The harmony as written.
    pub text: String,
    /// The harmony.
    pub harmony: Harmony,
    /// The notes, by voice from the bottom, then by start.
    pub notes: Vec<BlockNote>,
}

/// Why a part could not be voiced: where, and what to do.
#[derive(Debug, Clone, PartialEq)]
pub struct LeadError {
    /// The part.
    pub part: String,
    /// The section.
    pub section: String,
    /// The bar, from 1.
    pub bar: u32,
    /// The beat, from 1.
    pub beat: f64,
    /// What went wrong, and what to do.
    pub message: String,
}

impl fmt::Display for LeadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "part `{}`, section `{}`, bar {} beat {}: {}",
            self.part, self.section, self.bar, self.beat, self.message
        )
    }
}

impl std::error::Error for LeadError {}

/// One harmony to voice, with its candidates.
struct Step<'a> {
    section: &'a Section,
    span: &'a Span,
    spec: Spec,
    weights: Weights,
    range: Range,
    candidates: Vec<Voicing>,
}

/// The bar and beat, from 1, of `ticks` into `section`.
fn bar_beat(section: &Section, ticks: u32) -> (u32, f64) {
    let bar = section.meter.bar_ticks();
    (
        ticks / bar + 1,
        f64::from(ticks % bar) / f64::from(section.meter.beat_ticks()) + 1.0,
    )
}

/// Voices `part` through the sheet: the cheapest path through every
/// harmony's candidates, with its non-chord tones, as blocks in form
/// order.
///
/// # Errors
///
/// [`LeadError`] naming the section, bar and constraint when a harmony
/// has no voicing that meets the part's hard constraints.
pub fn voice_part(
    sheet: &Sheet,
    part: &Part,
    vocabularies: &Vocabularies,
) -> Result<Vec<Block>, LeadError> {
    let steps = gather(sheet, part, vocabularies)?;
    let path = viterbi(&steps);
    let mut blocks: Vec<Block> = steps
        .iter()
        .zip(&path)
        .map(|(s, c)| block(s, &s.candidates[*c]))
        .collect();
    for i in 1..blocks.len() {
        if adjacent(sheet, &blocks[i - 1], &blocks[i]) {
            let (a, b) = blocks.split_at_mut(i);
            ornament(&steps[i - 1], &steps[i], &mut a[i - 1], &mut b[0]);
        }
    }
    for (s, b) in steps.iter().zip(&mut blocks) {
        if let Some(p) = s.spec.pedal {
            pedal(b, p);
        }
    }
    for b in &mut blocks {
        b.notes.sort_by_key(|n| (n.voice, n.start));
    }
    Ok(blocks)
}

fn gather<'a>(
    sheet: &'a Sheet,
    part: &Part,
    vocabularies: &Vocabularies,
) -> Result<Vec<Step<'a>>, LeadError> {
    let mut steps = Vec::new();
    for section in &sheet.sections {
        if !part.plays_in(&section.name) || section.spans.is_empty() {
            continue;
        }
        let err = |ticks: u32, message: String| {
            let (bar, beat) = bar_beat(section, ticks);
            LeadError {
                part: part.name.clone(),
                section: section.name.clone(),
                bar,
                beat,
                message,
            }
        };
        let spec = part.spec_in(&section.name);
        let (Some(name), Some(voices), Some(range)) = (&spec.vocabulary, spec.voices, spec.range)
        else {
            return Err(err(
                0,
                "the part has no voicing here; give it one, such as `voicing \"close\" voices=4 range=\"C3..C5\"`".into(),
            ));
        };
        let vocab = vocabularies.get(name).ok_or_else(|| {
            let known: Vec<&str> = vocabularies.iter().map(|v| v.name.as_str()).collect();
            err(
                0,
                format!(
                    "`{name}` is not a vocabulary; known ones are {}",
                    known.join(", ")
                ),
            )
        })?;
        let top = part.tops.iter().find(|t| t.section == section.name);
        if let Some(t) = top
            && t.notes.len() > section.spans.len()
        {
            return Err(err(
                0,
                format!(
                    "the top line has {} notes but the section has {} harmonies",
                    t.notes.len(),
                    section.spans.len()
                ),
            ));
        }
        let fixes: Vec<_> = part
            .fixes
            .iter()
            .filter(|f| f.section == section.name)
            .collect();
        for f in &fixes {
            let at = fix_tick(section, f);
            if !section.spans.iter().any(|s| s.start == at) {
                return Err(err(
                    at,
                    "a `fix` must fall where a harmony starts; move it to that harmony's bar and beat".into(),
                ));
            }
        }
        for (i, span) in section.spans.iter().enumerate() {
            let literal_here = fixes.iter().find(|f| fix_tick(section, f) == span.start);
            let mut candidates = match literal_here {
                Some(f) => vec![
                    literal(&span.harmony, &f.pitches, voices).map_err(|m| err(span.start, m))?,
                ],
                None => vocab
                    .candidates(&span.harmony, range, voices)
                    .map_err(|e| err(span.start, e.message))?,
            };
            if let Some(Some(p)) = top.and_then(|t| t.notes.get(i)) {
                candidates.retain(|c| c.notes.last().map(|n| n.pitch) == Some(*p));
                if candidates.is_empty() {
                    return Err(err(
                        span.start,
                        format!(
                            "no `{name}` voicing of {} within {range} has {p} on top; choose another top note, widen the range or change the vocabulary",
                            span.harmony
                        ),
                    ));
                }
            }
            steps.push(Step {
                section,
                span,
                weights: spec.weights(),
                spec: spec.clone(),
                range,
                candidates,
            });
        }
    }
    Ok(steps)
}

/// Where a `fix` falls: ticks from its section's start.
fn fix_tick(section: &Section, f: &crate::part::Fix) -> u32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a beat within a bar"
    )]
    let beat = ((f.beat - 1.0) * f64::from(section.meter.beat_ticks())).round() as u32;
    (f.bar - 1) * section.meter.bar_ticks() + beat
}

/// A literal voicing, each note labelled with its role.
fn literal(h: &Harmony, pitches: &[Pitch], voices: usize) -> Result<Voicing, String> {
    if pitches.len() != voices {
        return Err(format!(
            "the `fix` has {} notes but the part has {voices} voices",
            pitches.len()
        ));
    }
    let notes = pitches
        .iter()
        .map(|p| {
            h.role_of(p.class())
                .map(|role| VoicedNote { pitch: *p, role })
                .ok_or_else(|| {
                    format!("{p} in the `fix` is not available in {h}; use its notes, or name a scale that holds it")
                })
        })
        .collect::<Result<_, _>>()?;
    Ok(Voicing { notes })
}

/// The lowest a pitch may be for each interval above it, in semitones
/// (1 to 11), before the interval muddies: the usual low-interval limits
/// of arranging texts.
const LOW_LIMITS: [u8; 12] = [
    0,  // unison: no limit
    52, // minor 2nd: E3
    51, // major 2nd: Eb3
    48, // minor 3rd: C3
    46, // major 3rd: Bb2
    45, // perfect 4th: A2
    46, // tritone: Bb2
    34, // perfect 5th: Bb1
    43, // minor 6th: G2
    41, // major 6th: F2
    41, // minor 7th: F2
    41, // major 7th: F2
];

fn unary(s: &Step<'_>, v: &Voicing) -> f64 {
    let p = v.pitches();
    let mut spacing = 0.0;
    for w in p.windows(2) {
        let interval = w[1] - w[0];
        if interval < 12 {
            let limit = LOW_LIMITS[usize::from(interval)];
            if w[0] < limit {
                spacing += f64::from(limit - w[0]);
            }
        }
    }
    let mean = p.iter().map(|x| f64::from(*x)).sum::<f64>()
        / f64::from(u8::try_from(p.len()).unwrap_or(1));
    let mid = f64::midpoint(f64::from(s.range.low.get()), f64::from(s.range.high.get()));
    s.weights.spacing * spacing + s.weights.centre * (mean - mid).abs()
}

fn transition(weights: &Weights, from: &Voicing, to: &Voicing) -> f64 {
    let (before, after) = (from.pitches(), to.pitches());
    if before.len() != after.len() {
        return 0.0;
    }
    let (mut moved, mut leaps, mut held, mut parallels, mut crossing) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let motion: Vec<i32> = before
        .iter()
        .zip(&after)
        .map(|(x, y)| i32::from(*y) - i32::from(*x))
        .collect();
    for (i, step) in motion.iter().enumerate() {
        let size = f64::from(step.unsigned_abs());
        moved += size;
        leaps += (size - 4.0).max(0.0);
        if *step == 0 {
            held += 1.0;
        }
        let overlaps_above = i + 1 < after.len() && after[i] > before[i + 1];
        let overlaps_below = i > 0 && after[i] < before[i - 1];
        if overlaps_above || overlaps_below {
            crossing += 1.0;
        }
        for j in i + 1..after.len() {
            let was = (i32::from(before[j]) - i32::from(before[i])).rem_euclid(12);
            let is = (i32::from(after[j]) - i32::from(after[i])).rem_euclid(12);
            if *step != 0
                && step.signum() == motion[j].signum()
                && was == is
                && (was == 0 || was == 7)
            {
                parallels += 1.0;
            }
        }
    }
    weights.movement * moved + weights.leaps * leaps - weights.common * held
        + weights.parallels * parallels
        + weights.crossing * crossing
}

/// The cheapest path: one candidate index per step. Ties go to the
/// earlier candidate, so the result does not depend on anything but the
/// input.
fn viterbi(steps: &[Step<'_>]) -> Vec<usize> {
    if steps.is_empty() {
        return Vec::new();
    }
    let mut cost: Vec<f64> = steps[0]
        .candidates
        .iter()
        .map(|v| unary(&steps[0], v))
        .collect();
    let mut back: Vec<Vec<usize>> = vec![vec![0; cost.len()]];
    for i in 1..steps.len() {
        let (prev, cur) = (&steps[i - 1], &steps[i]);
        let mut next = Vec::with_capacity(cur.candidates.len());
        let mut from = Vec::with_capacity(cur.candidates.len());
        for c in &cur.candidates {
            let mut best = (f64::INFINITY, 0);
            for (k, p) in prev.candidates.iter().enumerate() {
                let t = cost[k] + transition(&cur.weights, p, c);
                if t < best.0 {
                    best = (t, k);
                }
            }
            next.push(best.0 + unary(cur, c));
            from.push(best.1);
        }
        cost = next;
        back.push(from);
    }
    let mut end = 0;
    for (k, c) in cost.iter().enumerate() {
        if *c < cost[end] {
            end = k;
        }
    }
    let mut path = vec![end; steps.len()];
    for i in (1..steps.len()).rev() {
        path[i - 1] = back[i][path[i]];
    }
    path
}

fn block(s: &Step<'_>, v: &Voicing) -> Block {
    let n = v.notes.len();
    Block {
        section: s.section.name.clone(),
        start: s.span.start,
        length: s.span.length,
        text: s.span.text.clone(),
        harmony: s.span.harmony.clone(),
        notes: v
            .notes
            .iter()
            .enumerate()
            .map(|(i, x)| BlockNote {
                pitch: x.pitch,
                voice: Voice::of(i, n),
                role: Some(x.role),
                tone: Tone::Chord,
                start: s.span.start,
                length: s.span.length,
            })
            .collect(),
    }
}

/// Whether `b` follows `a` directly: the next harmony in the same section,
/// or the first of the next section.
fn adjacent(sheet: &Sheet, a: &Block, b: &Block) -> bool {
    if a.section == b.section {
        return a.start + a.length == b.start;
    }
    let pos = |name: &str| sheet.sections.iter().position(|s| s.name == name);
    match (pos(&a.section), pos(&b.section), sheet.section(&a.section)) {
        (Some(i), Some(j), Some(sa)) => {
            j == i + 1 && b.start == 0 && a.start + a.length == sa.length()
        }
        _ => false,
    }
}

/// The chord-tone note of `voice` in a block.
fn chord_note(b: &mut Block, voice: Voice) -> Option<&mut BlockNote> {
    b.notes
        .iter_mut()
        .find(|n| n.voice == voice && n.tone == Tone::Chord)
}

/// How long a non-chord tone lasts in `b`: the part's `nct beats` (one
/// beat by default), at most half the block.
fn nct_length(s: &Step<'_>, b: &Block) -> u32 {
    let beats = s.spec.nct_beats.unwrap_or(1.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a positive tick count"
    )]
    let t = (beats * f64::from(s.section.meter.beat_ticks())).round() as u32;
    t.min(b.length / 2)
}

/// Adds non-chord tones between two adjacent blocks, in the inner and top
/// voices, the first kind in the part's order that fits each voice.
fn ornament(sa: &Step<'_>, sb: &Step<'_>, a: &mut Block, b: &mut Block) {
    let Some(kinds) = sa.spec.nct.clone() else {
        return;
    };
    let count = a.notes.len();
    if count != b.notes.len() {
        return;
    }
    let (la, lb) = (nct_length(sa, a), nct_length(sb, b));
    for i in 1..count {
        let voice = Voice::of(i, count);
        let (Some(pa), Some(pb)) = (
            chord_note(a, voice).map(|n| n.pitch),
            chord_note(b, voice).map(|n| n.pitch),
        ) else {
            continue;
        };
        let d = i32::from(pb.get()) - i32::from(pa.get());
        for kind in &kinds {
            let added = match kind {
                Nct::Passing if (3..=4).contains(&d.abs()) => {
                    let mid = f64::from(pa.get()) + f64::from(d) / 2.0;
                    let between = (1..d.abs())
                        .filter_map(|k| Pitch::new(i32::from(pa.get()) + k * d.signum()))
                        .filter(|p| a.harmony.scale.contains(p.class()))
                        .min_by(|x, y| {
                            (f64::from(x.get()) - mid)
                                .abs()
                                .total_cmp(&(f64::from(y.get()) - mid).abs())
                                .then(x.cmp(y))
                        });
                    between.is_some_and(|p| {
                        let role = a.harmony.role_of(p.class());
                        end_of(a, voice, p, role, *kind, la)
                    })
                }
                Nct::Neighbour if d == 0 => (1..=2)
                    .filter_map(|k| Pitch::new(i32::from(pa.get()) + k))
                    .find(|p| a.harmony.scale.contains(p.class()))
                    .is_some_and(|p| {
                        let role = a.harmony.role_of(p.class());
                        end_of(a, voice, p, role, *kind, la)
                    }),
                // A held note that is not a chord tone of the next harmony,
                // but is in its scale: a chromatic one (E over E flat
                // lydian) only clashes.
                Nct::Suspension
                    if (1..=2).contains(&-d)
                        && !b.harmony.tones.iter().any(|t| t.pc == pa.class())
                        && b.harmony.scale.contains(pa.class()) =>
                {
                    let role = b.harmony.role_of(pa.class());
                    start_of(b, voice, pa, role, lb)
                }
                Nct::Anticipation if d != 0 => {
                    // The next harmony's note, so its role there.
                    let role = b.harmony.role_of(pb.class());
                    end_of(a, voice, pb, role, *kind, la)
                }
                _ => false,
            };
            if added {
                break;
            }
        }
    }
}

/// Ends `voice`'s chord tone in `a` early and plays `p` for the last
/// `len`. Returns whether it did.
fn end_of(a: &mut Block, voice: Voice, p: Pitch, role: Option<Role>, kind: Nct, len: u32) -> bool {
    let Some(n) = chord_note(a, voice) else {
        return false;
    };
    if len == 0 || n.length <= len {
        return false;
    }
    n.length -= len;
    let start = n.start + n.length;
    a.notes.push(BlockNote {
        pitch: p,
        voice,
        role,
        tone: Tone::Nct(kind),
        start,
        length: len,
    });
    true
}

/// Holds `p` into `b` for the first `len` of `voice`, resolving after.
/// Returns whether it did.
fn start_of(b: &mut Block, voice: Voice, p: Pitch, role: Option<Role>, len: u32) -> bool {
    let Some(n) = chord_note(b, voice) else {
        return false;
    };
    if len == 0 || n.length <= len {
        return false;
    }
    let start = n.start;
    n.start += len;
    n.length -= len;
    b.notes.push(BlockNote {
        pitch: p,
        voice,
        role,
        tone: Tone::Nct(Nct::Suspension),
        start,
        length: len,
    });
    true
}

/// Replaces the bass voice with a held pedal note.
fn pedal(b: &mut Block, p: Pitch) {
    let role = b.harmony.role_of(p.class());
    for n in b.notes.iter_mut().filter(|n| n.voice == Voice::Bass) {
        n.pitch = p;
        n.role = role;
        n.tone = Tone::Nct(Nct::Pedal);
    }
}
