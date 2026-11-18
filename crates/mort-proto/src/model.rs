//! The project as mort sees it: the records the bridge reads and returns.
//!
//! Times are project seconds. Gains are linear (1.0 is 0 dB). Pan runs from
//! −1 (left) to 1 (right). Parameter values are normalised 0–1.

use serde::{Deserialize, Serialize};

use crate::ids::{EnvelopeId, FxId, ItemId, TakeId, TrackId};

/// The project as a whole.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// The project's name, without extension; `untitled` if never saved.
    pub name: String,
    /// The project file, if it has been saved.
    pub path: Option<String>,
    /// The project sample rate in Hz.
    pub sample_rate: f64,
    /// The project length in seconds.
    pub length: f64,
}

/// A region (a section of the piece) or a marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    /// REAPER's region or marker number.
    pub number: u32,
    /// Its name; may be empty.
    pub name: String,
    /// Where it starts.
    pub start: f64,
    /// Where it ends; `None` for a marker.
    pub end: Option<f64>,
}

/// A send from one track to another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendInfo {
    /// The receiving track.
    pub dest: TrackId,
    /// Linear gain.
    pub volume: f64,
    /// Pan, −1 to 1.
    pub pan: f64,
    /// Muted.
    pub mute: bool,
}

/// An FX instance in a chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxInfo {
    /// The instance.
    pub id: FxId,
    /// Position in the chain, from 0.
    pub index: usize,
    /// The instance's name as shown in REAPER.
    pub name: String,
    /// The plugin, as REAPER names it, for example `CLAP: mort probe`.
    pub plugin: String,
    /// Not bypassed.
    pub enabled: bool,
}

/// A track. The master track is a track whose `parent` is `None` and whose
/// `index` is `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackInfo {
    /// The track.
    pub id: TrackId,
    /// Position in the track list, from 0; `None` for the master.
    pub index: Option<usize>,
    /// Its name.
    pub name: String,
    /// The folder track it sits in, if any.
    pub parent: Option<TrackId>,
    /// Whether it feeds its parent (or the master, at top level).
    pub main_send: bool,
    /// Linear gain.
    pub volume: f64,
    /// Pan, −1 to 1.
    pub pan: f64,
    /// Muted.
    pub mute: bool,
    /// Soloed.
    pub solo: bool,
    /// Sends to other tracks.
    pub sends: Vec<SendInfo>,
    /// The FX chain, in order.
    pub fx: Vec<FxInfo>,
}

/// One parameter of an FX instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamInfo {
    /// Its index in the plugin.
    pub index: u32,
    /// Its name.
    pub name: String,
    /// Normalised value, 0–1.
    pub value: f64,
    /// The plugin's display text for the value.
    pub display: String,
    /// Number of steps, for stepped parameters.
    pub steps: Option<u32>,
}

/// A media item on a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemInfo {
    /// The item.
    pub id: ItemId,
    /// Its active take, if any.
    pub take: Option<TakeId>,
    /// Where it starts.
    pub position: f64,
    /// How long it is.
    pub length: f64,
    /// Whether the active take is MIDI.
    pub midi: bool,
}

/// What an envelope controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnvelopeTarget {
    /// Track volume.
    Volume {
        /// The track.
        track: TrackId,
    },
    /// Track pan.
    Pan {
        /// The track.
        track: TrackId,
    },
    /// The volume of one of a track's sends.
    SendVolume {
        /// The sending track.
        track: TrackId,
        /// The send's index on that track.
        send: usize,
    },
    /// An FX parameter.
    Param {
        /// The FX instance.
        fx: FxId,
        /// The parameter index.
        index: u32,
    },
}

/// The shape of an envelope segment, from a point to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// Straight line.
    #[default]
    Linear,
    /// Hold, then jump.
    Square,
    /// S-curve.
    Slow,
    /// Fast start, slow end.
    FastStart,
    /// Slow start, fast end.
    FastEnd,
    /// Bézier, bent by the point's tension.
    Bezier,
}

/// A point on an envelope.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EnvelopePoint {
    /// When.
    pub time: f64,
    /// The value: linear gain for volume, −1 to 1 for pan, normalised for
    /// parameters.
    pub value: f64,
    /// The shape of the segment that starts here.
    #[serde(default)]
    pub shape: Shape,
    /// Bézier tension, −1 to 1.
    #[serde(default)]
    pub tension: f64,
}

/// An envelope and where it lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeInfo {
    /// The envelope.
    pub id: EnvelopeId,
    /// What it controls.
    pub target: EnvelopeTarget,
}

/// A MIDI note.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// MIDI note number.
    pub pitch: u8,
    /// Start, in project seconds.
    pub start: f64,
    /// End, in project seconds.
    pub end: f64,
    /// Velocity, 1–127.
    pub velocity: u8,
    /// Channel, 0–15.
    #[serde(default)]
    pub channel: u8,
}

/// A MIDI control change.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ControlChange {
    /// When, in project seconds.
    pub time: f64,
    /// Controller number.
    pub controller: u8,
    /// Value, 0–127.
    pub value: u8,
    /// Channel, 0–15.
    #[serde(default)]
    pub channel: u8,
}
