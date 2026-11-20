//! What `mort compose write` remembers about the items it wrote (ADR 0013
//! §10), and how it decides what to write next time.
//!
//! The record lives in `<project>.mort/compose/state.json`. For each part
//! and section it keeps the item, a hash of the notes mort generated, a
//! hash of the notes as REAPER held them just after writing, and a hash of
//! each layer's inputs.
//!
//! Whether to rewrite an item is decided by its *generated notes*, not its
//! inputs: voice-leading runs across section boundaries, so a chord
//! changed in one section can rightly move the voicing in the next, and
//! only comparing the output catches exactly that. The layer hashes say
//! why an item changed.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::articulate::Event;
use crate::part::Part;
use crate::pattern::Patterns;
use crate::sheet::Sheet;
use crate::voicing::Vocabularies;

/// The record's format version.
pub const VERSION: u32 = 1;

/// A short, stable hash of some text (FNV-1a, 64 bits, in hex).
#[must_use]
pub fn hash(text: &str) -> String {
    let h = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{h:016x}")
}

/// The hash of a part's events in one section.
#[must_use]
pub fn events_hash(events: &[&Event]) -> String {
    let mut text = String::new();
    for e in events {
        let _ = writeln!(text, "{} {} {} {}", e.start, e.length, e.pitch, e.velocity);
    }
    hash(&text)
}

/// The hash of notes as read back from REAPER: (pitch, start and end in
/// seconds from the item's section start, velocity), rounded to 0.1 ms.
#[must_use]
pub fn written_hash(notes: &[(u8, f64, f64, u8)]) -> String {
    let mut sorted: Vec<(u8, i64, i64, u8)> = notes
        .iter()
        .map(|(p, s, e, v)| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "tenths of a millisecond in a song"
            )]
            let r = |x: f64| (x * 10_000.0).round() as i64;
            (*p, r(*s), r(*e), *v)
        })
        .collect();
    sorted.sort_unstable_by_key(|n| (n.1, n.0, n.2, n.3));
    let mut text = String::new();
    for (p, s, e, v) in sorted {
        let _ = writeln!(text, "{p} {s} {e} {v}");
    }
    hash(&text)
}

/// Hashes of each layer's inputs for a part in a section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Layers {
    /// The section's form and harmony.
    pub harmony: String,
    /// The voicing spec, vocabulary, top line and fixes.
    pub voicing: String,
    /// The pattern, instrument range and kit.
    pub articulation: String,
    /// The feel.
    pub feel: String,
}

impl Layers {
    /// The layers' input hashes for `part` in `section`.
    #[must_use]
    pub fn of(
        sheet: &Sheet,
        part: &Part,
        section: &str,
        vocabularies: &Vocabularies,
        patterns: &Patterns,
    ) -> Self {
        let harmony = sheet.section(section).map_or_else(String::new, |s| {
            let spans: Vec<String> = s
                .spans
                .iter()
                .map(|x| format!("{}@{}+{} {}", x.text, x.start, x.length, x.harmony.scale))
                .collect();
            format!("{} {} {}", s.bars, s.meter, spans.join(";"))
        });
        // A part that plays another's voices is voiced by that part.
        let source = part
            .from
            .as_ref()
            .and_then(|(o, v)| sheet.part(o).map(|p| (p, format!("{v:?}"))));
        let (vp, taken) = source.unwrap_or((part, String::new()));
        let vspec = vp.spec_in(section);
        let vocab = vspec
            .vocabulary
            .as_deref()
            .and_then(|n| vocabularies.get(n))
            .map(|v| format!("{v:?}"))
            .unwrap_or_default();
        let tops: Vec<String> = vp
            .tops
            .iter()
            .filter(|t| t.section == section)
            .map(|t| format!("{t:?}"))
            .collect();
        let fixes: Vec<String> = vp
            .fixes
            .iter()
            .filter(|f| f.section == section)
            .map(|f| format!("{f:?}"))
            .collect();
        let voicing = format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {:?} {vocab} {tops:?} {fixes:?} {taken}",
            vspec.vocabulary,
            vspec.voices,
            vspec.range,
            vspec.weights,
            vspec.nct,
            vspec.nct_beats,
            vspec.pedal
        );
        let spec = part.spec_in(section);
        let pattern_name = spec
            .articulation
            .clone()
            .unwrap_or_else(|| "sustain".into());
        let pattern = patterns
            .get(&pattern_name)
            .map(|p| format!("{p:?}"))
            .unwrap_or_default();
        let kit = spec
            .kit
            .as_deref()
            .and_then(|k| patterns.kit(k))
            .map(|k| format!("{k:?}"))
            .unwrap_or_default();
        Self {
            harmony: hash(&harmony),
            voicing: hash(&voicing),
            articulation: hash(&format!(
                "{pattern_name} {pattern} {:?} {kit}",
                spec.instrument
            )),
            feel: hash(&format!("{:?}", spec.feel)),
        }
    }

    /// The layers that differ from `before`, by name.
    #[must_use]
    pub fn changed_from(&self, before: &Layers) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.harmony != before.harmony {
            out.push("harmony");
        }
        if self.voicing != before.voicing {
            out.push("voicing");
        }
        if self.articulation != before.articulation {
            out.push("articulation");
        }
        if self.feel != before.feel {
            out.push("feel");
        }
        out
    }
}

/// What mort wrote for one part in one section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The part.
    pub part: String,
    /// The section.
    pub section: String,
    /// The track's id.
    pub track: String,
    /// The item's id.
    pub item: String,
    /// The hash of the notes mort generated.
    pub generated: String,
    /// The hash of the notes as REAPER held them after writing.
    pub written: String,
    /// The layers' input hashes.
    pub layers: Layers,
}

/// The whole record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The format version.
    pub version: u32,
    /// Entries by `part/section`.
    pub entries: BTreeMap<String, Entry>,
}

impl Default for Record {
    fn default() -> Self {
        Self {
            version: VERSION,
            entries: BTreeMap::new(),
        }
    }
}

/// The record's file in a sidecar directory.
#[must_use]
pub fn path(sidecar: &Path) -> PathBuf {
    sidecar.join("compose").join("state.json")
}

impl Record {
    /// The key for a part in a section.
    #[must_use]
    pub fn key(part: &str, section: &str) -> String {
        format!("{part}/{section}")
    }

    /// Reads the record in `sidecar`, or an empty one if there is none.
    ///
    /// # Errors
    ///
    /// A message naming the file if it exists but does not read.
    pub fn load(sidecar: &Path) -> Result<Self, String> {
        let p = path(sidecar);
        match std::fs::read_to_string(&p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", p.display())),
            Ok(text) => {
                let r: Record = serde_json::from_str(&text).map_err(|e| {
                    format!(
                        "{} does not read: {e}; move it aside to start a new record",
                        p.display()
                    )
                })?;
                if r.version == VERSION {
                    Ok(r)
                } else {
                    Err(format!(
                        "{} is format {}, and this mort reads {VERSION}; move it aside to start a new record",
                        p.display(),
                        r.version
                    ))
                }
            }
        }
    }

    /// Writes the record into `sidecar`.
    ///
    /// # Errors
    ///
    /// A message naming the file if it cannot be written.
    pub fn save(&self, sidecar: &Path) -> Result<(), String> {
        let p = path(sidecar);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&p, text + "\n").map_err(|e| format!("{}: {e}", p.display()))
    }
}

/// What to do with one part in one section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Write it: new, or changed for these reasons.
    Write(String),
    /// Its notes would be the same.
    Unchanged,
    /// Someone edited its item since mort wrote it; leave it alone.
    EditedByHand,
}

/// Decides what to do with one part in one section.
///
/// - `before`: what mort last wrote there, if anything;
/// - `present`: whether that item is still in the project, and the hash of
///   its notes now;
/// - `generated` and `layers`: what mort would write now.
#[must_use]
pub fn decide(
    before: Option<&Entry>,
    present: Option<&str>,
    generated: &str,
    layers: &Layers,
    force: bool,
) -> Action {
    let (Some(b), Some(now)) = (before, present) else {
        return Action::Write("new".into());
    };
    if now != b.written {
        return if force {
            Action::Write("edited by hand; overwritten (--force)".into())
        } else {
            Action::EditedByHand
        };
    }
    if b.generated == generated {
        return if force {
            Action::Write("unchanged; rewritten (--force)".into())
        } else {
            Action::Unchanged
        };
    }
    let changed = layers.changed_from(&b.layers);
    Action::Write(if changed.is_empty() {
        "voice-leading moved with a neighbouring section or another part".into()
    } else {
        format!("{} changed", changed.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
        Entry {
            part: "Pad".into(),
            section: "verse".into(),
            track: "t".into(),
            item: "i".into(),
            generated: "g".into(),
            written: "w".into(),
            layers: Layers {
                harmony: "h".into(),
                ..Layers::default()
            },
        }
    }

    #[test]
    fn decisions() {
        let e = entry();
        let same = e.layers.clone();
        assert_eq!(
            decide(None, None, "g", &same, false),
            Action::Write("new".into())
        );
        assert_eq!(
            decide(Some(&e), None, "g", &same, false),
            Action::Write("new".into()),
            "the item is gone"
        );
        assert_eq!(
            decide(Some(&e), Some("w"), "g", &same, false),
            Action::Unchanged
        );
        assert_eq!(
            decide(Some(&e), Some("x"), "g2", &same, false),
            Action::EditedByHand
        );
        assert!(
            matches!(decide(Some(&e), Some("x"), "g2", &same, true), Action::Write(r) if r.contains("--force"))
        );
        let moved = Layers {
            harmony: "h2".into(),
            ..same.clone()
        };
        assert_eq!(
            decide(Some(&e), Some("w"), "g2", &moved, false),
            Action::Write("harmony changed".into())
        );
        assert_eq!(
            decide(Some(&e), Some("w"), "g2", &same, false),
            Action::Write("voice-leading moved with a neighbouring section or another part".into())
        );
    }

    #[test]
    fn hashes_are_stable() {
        assert_eq!(hash(""), "cbf29ce484222325");
        assert_eq!(
            written_hash(&[(60, 0.0, 0.5, 90), (64, 0.0, 0.5, 90)]),
            written_hash(&[(64, 0.0, 0.500_01, 90), (60, 0.0, 0.5, 90)])
        );
    }

    #[test]
    fn round_trips() {
        let dir = std::env::temp_dir().join(format!("mort-record-{}", std::process::id()));
        let mut r = Record::default();
        r.entries.insert(Record::key("Pad", "verse"), entry());
        r.save(&dir).unwrap();
        assert_eq!(Record::load(&dir).unwrap(), r);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(Record::load(&dir).unwrap(), Record::default());
    }
}
