//! The [`Session`] trait over a REAPER project, and [`FakeSession`]
//! (ADR 0011).
//!
//! The bridge reaches REAPER only through [`Session`]. Its REAPER
//! implementation lives in `mort-bridge`. [`FakeSession`] implements it
//! over an in-memory project, so the dispatcher, the capture path and the
//! CLI can be tested without REAPER.
//!
//! All methods run on REAPER's main thread. Errors are protocol errors,
//! ready to send to the client: they name the element and say what to do.

mod audio;
mod fake;

pub use audio::Source;
pub use fake::{FakeSession, PluginParam, PluginSpec};

use mort_proto::model::{
    ControlChange, EnvelopeInfo, EnvelopePoint, EnvelopeTarget, FxInfo, ItemInfo, Note, ParamInfo,
    ProjectInfo, Region, TrackInfo,
};
use std::collections::HashMap;

use mort_link::Record;
use mort_proto::{EnvelopeId, ErrorKind, ErrorObject, FxId, ItemId, TempoMap, TrackId};

/// What every session method returns.
pub type Result<T> = std::result::Result<T, ErrorObject>;

/// A new track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewTrack {
    /// Its name.
    pub name: String,
    /// Where to insert it in the track list; at the end if `None`.
    pub index: Option<usize>,
    /// The folder track to put it in, if any.
    pub parent: Option<TrackId>,
}

/// Changes to a track's own controls. Fields left `None` are unchanged.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackChange {
    /// New name.
    pub name: Option<String>,
    /// New linear gain.
    pub volume: Option<f64>,
    /// New pan.
    pub pan: Option<f64>,
    /// Mute on or off.
    pub mute: Option<bool>,
    /// Solo on or off.
    pub solo: Option<bool>,
}

/// Changes to a send. Fields left `None` are unchanged.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SendChange {
    /// New linear gain.
    pub volume: Option<f64>,
    /// New pan.
    pub pan: Option<f64>,
    /// Mute on or off.
    pub mute: Option<bool>,
}

/// A REAPER project, as the bridge needs it.
pub trait Session {
    // --- Project ---------------------------------------------------------

    /// The project as a whole.
    fn project(&self) -> ProjectInfo;

    /// REAPER's project state change count. It moves on every change,
    /// including undo and redo (ADR 0004 §2).
    fn state(&self) -> u64;

    /// The project's tempo map.
    fn tempo_map(&self) -> &dyn TempoMap;

    /// Regions and markers, in time order.
    fn regions(&self) -> Vec<Region>;

    /// Adds a region from `start` to `end`, or a marker at `start` when
    /// `end` is `None`, and returns its number.
    ///
    /// # Errors
    ///
    /// `invalid` if `end` is not after `start`.
    fn add_region(&mut self, name: &str, start: f64, end: Option<f64>) -> Result<u32>;

    // --- Tracks ----------------------------------------------------------

    /// The master track.
    fn master(&self) -> TrackId;

    /// All tracks except the master, in track-list order.
    fn tracks(&self) -> Vec<TrackInfo>;

    /// One track, including the master.
    ///
    /// # Errors
    ///
    /// `not_found` if there is no such track.
    fn track(&self, id: TrackId) -> Result<TrackInfo>;

    /// Adds a track.
    ///
    /// # Errors
    ///
    /// `not_found` for a missing parent; `invalid` for an index past the end.
    fn add_track(&mut self, spec: NewTrack) -> Result<TrackId>;

    /// Removes a track, its items, its sends and the sends to it. Tracks
    /// in it, if it is a folder, move up a level.
    ///
    /// # Errors
    ///
    /// `not_found`; `invalid` for the master.
    fn remove_track(&mut self, id: TrackId) -> Result<()>;

    /// Changes a track's own controls.
    ///
    /// # Errors
    ///
    /// `not_found`; `invalid` for out-of-range values.
    fn set_track(&mut self, id: TrackId, change: &TrackChange) -> Result<()>;

    /// Adds a send and returns its index on the sending track.
    ///
    /// # Errors
    ///
    /// `not_found` for either track; `invalid` for a send to itself.
    fn add_send(&mut self, from: TrackId, to: TrackId) -> Result<usize>;

    /// Changes a send.
    ///
    /// # Errors
    ///
    /// `not_found` for the track or send index.
    fn set_send(&mut self, from: TrackId, send: usize, change: &SendChange) -> Result<()>;

    /// The track's state chunk: text that changes whenever anything on the
    /// track changes (ADR 0006 §1).
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn track_chunk(&self, id: TrackId) -> Result<String>;

    /// The parts of the track's state chunk that affect what it sounds like
    /// or where its audio goes: the chunk without its name, selection and
    /// window state, and without mort's probes. Captures hash it (ADR 0006
    /// §1), so selecting a track or adding a probe leaves its measurements
    /// fresh.
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn audio_chunk(&self, id: TrackId) -> Result<String>;

    // --- FX --------------------------------------------------------------

    /// Adds a plugin to a track's chain, at `index` or the end.
    ///
    /// # Errors
    ///
    /// `not_found` for the track or plugin; `invalid` for a bad index.
    fn add_fx(&mut self, track: TrackId, plugin: &str, index: Option<usize>) -> Result<FxId>;

    /// Removes an FX instance.
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn remove_fx(&mut self, fx: FxId) -> Result<()>;

    /// Moves an FX instance within its chain.
    ///
    /// # Errors
    ///
    /// `not_found`; `invalid` for a bad index.
    fn move_fx(&mut self, fx: FxId, index: usize) -> Result<()>;

    /// Bypasses (`false`) or enables (`true`) an FX instance.
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn set_fx_enabled(&mut self, fx: FxId, enabled: bool) -> Result<()>;

    /// Which track an FX instance is on, and the instance's details.
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn fx(&self, fx: FxId) -> Result<(TrackId, FxInfo)>;

    /// An FX instance's parameters.
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn params(&self, fx: FxId) -> Result<Vec<ParamInfo>>;

    /// Sets a parameter, normalised 0–1.
    ///
    /// # Errors
    ///
    /// `not_found` for the FX or index; `invalid` for a value outside 0–1.
    fn set_param(&mut self, fx: FxId, index: u32, value: f64) -> Result<()>;

    /// The plugin's display text for a parameter at a given normalised
    /// value, without changing it. `None` if the plugin cannot say.
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn format_param(&self, fx: FxId, index: u32, value: f64) -> Result<Option<String>>;

    /// The instance's state, as REAPER stores it in the project.
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn fx_state(&self, fx: FxId) -> Result<String>;

    /// Replaces the instance's state with one from [`Session::fx_state`].
    ///
    /// # Errors
    ///
    /// `not_found`; `invalid` if the plugin rejects it.
    fn set_fx_state(&mut self, fx: FxId, state: &str) -> Result<()>;

    /// Loads a named preset.
    ///
    /// # Errors
    ///
    /// `not_found` for the FX or preset.
    fn load_preset(&mut self, fx: FxId, preset: &str) -> Result<()>;

    // --- Envelopes -------------------------------------------------------

    /// The envelope for a target. If there is none, creates it when
    /// `create` is set and returns `None` otherwise.
    ///
    /// # Errors
    ///
    /// `not_found` for the track, send, FX or parameter.
    fn envelope(&mut self, target: EnvelopeTarget, create: bool) -> Result<Option<EnvelopeInfo>>;

    /// The envelopes that exist on a track, including its FX parameters'.
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn envelopes(&self, track: TrackId) -> Result<Vec<EnvelopeInfo>>;

    /// Points from `start` (inclusive) to `end` (exclusive), in time
    /// order.
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn envelope_points(&self, env: EnvelopeId, start: f64, end: f64) -> Result<Vec<EnvelopePoint>>;

    /// How REAPER would display `value` on this envelope, such as `-6.02dB`
    /// or `50%L`. `None` if it cannot say.
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn format_envelope_value(&self, env: EnvelopeId, value: f64) -> Result<Option<String>>;

    /// Adds points, keeping the envelope in time order.
    ///
    /// # Errors
    ///
    /// `not_found`; `invalid` for a value outside the target's range.
    fn insert_envelope_points(&mut self, env: EnvelopeId, points: &[EnvelopePoint]) -> Result<()>;

    /// Removes the points from `start` (inclusive) to `end` (exclusive).
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn delete_envelope_points(&mut self, env: EnvelopeId, start: f64, end: f64) -> Result<()>;

    // --- Items and MIDI --------------------------------------------------

    /// A track's items, in time order.
    ///
    /// # Errors
    ///
    /// `not_found`.
    fn items(&self, track: TrackId) -> Result<Vec<ItemInfo>>;

    /// Adds an empty MIDI item.
    ///
    /// # Errors
    ///
    /// `not_found`; `invalid` if `end` is not after `start`.
    fn add_midi_item(&mut self, track: TrackId, start: f64, end: f64) -> Result<ItemId>;

    /// The notes in a MIDI item, in start order.
    ///
    /// # Errors
    ///
    /// `not_found`; `invalid` if the item is not MIDI.
    fn notes(&self, item: ItemId) -> Result<Vec<Note>>;

    /// The control changes in a MIDI item, in time order.
    ///
    /// # Errors
    ///
    /// `not_found`; `invalid` if the item is not MIDI.
    fn control_changes(&self, item: ItemId) -> Result<Vec<ControlChange>>;

    /// Adds notes and control changes to a MIDI item.
    ///
    /// # Errors
    ///
    /// `not_found`; `invalid` if the item is not MIDI or an event lies
    /// outside it.
    fn insert_midi(&mut self, item: ItemId, notes: &[Note], ccs: &[ControlChange]) -> Result<()>;

    /// Removes notes that start, and control changes that fall, from
    /// `start` (inclusive) to `end` (exclusive).
    ///
    /// # Errors
    ///
    /// `not_found`; `invalid` if the item is not MIDI.
    fn delete_midi(&mut self, item: ItemId, start: f64, end: f64) -> Result<()>;

    /// Saves the project to its own file, or, given `path`, writes a copy
    /// there. A copy leaves the project's file and unsaved changes as they
    /// were (that is what REAPER's `Main_SaveProjectEx` does). This is the
    /// only way mort writes a project file (ADR 0004 §6).
    ///
    /// # Errors
    ///
    /// `invalid` if the project has never been saved and no path is given.
    fn save_project(&mut self, path: Option<&str>) -> Result<()>;

    /// REAPER actions in the main section whose names contain `text`
    /// (ignoring case), as (command id, name), at most `limit` of them.
    fn find_actions(&self, text: &str, limit: usize) -> Vec<(i32, String)>;

    // --- Undo ------------------------------------------------------------

    /// Starts an undo block. Every change until [`Session::end_undo`] is
    /// one undo point.
    fn begin_undo(&mut self);

    /// Ends the undo block with a label. A block with no changes leaves no
    /// undo point.
    fn end_undo(&mut self, label: &str);

    /// The label of the undo point that `undo` would undo.
    fn undo_label(&self) -> Option<String>;

    /// The label of the undo point that `redo` would redo.
    fn redo_label(&self) -> Option<String>;

    /// Undoes the last undo point and returns its label.
    fn undo(&mut self) -> Option<String>;

    /// Redoes the last undone point and returns its label.
    fn redo(&mut self) -> Option<String>;

    // --- Transport -------------------------------------------------------

    /// Starts playing from `start`, to stop at `end`.
    ///
    /// # Errors
    ///
    /// `invalid` if `end` is not after `start`.
    fn play(&mut self, start: f64, end: f64) -> Result<()>;

    /// Renders `start` to `end` faster than real time, through every
    /// track's FX, without writing a file (REAPER's dry-run render). It
    /// returns when the render is done. The project's own render settings
    /// are left as they were.
    ///
    /// # Errors
    ///
    /// `invalid` if `end` is not after `start`; `reaper` if REAPER cannot
    /// render.
    fn render(&mut self, start: f64, end: f64) -> Result<()>;

    /// Stops playback.
    fn stop(&mut self);

    /// Whether the transport is playing.
    fn is_playing(&self) -> bool;

    /// The play position, or the edit cursor when stopped.
    fn position(&self) -> f64;

    /// The play rate: 1.0 at normal speed.
    fn play_rate(&self) -> f64;

    /// Tracks armed for recording with input monitoring on during
    /// playback: what their probes hear includes live input.
    fn monitored(&self) -> Vec<TrackId>;

    // --- Runs ------------------------------------------------------------

    /// Starts a run (ADR 0005 §3): frames probes produce from now on carry
    /// its id. Returns the id.
    fn begin_run(&mut self) -> u64;

    /// Ends the run `run`, if it is still current.
    fn end_run(&mut self, run: u64);

    /// Takes the frames of run `run` that probes have produced so far, by
    /// probe slot.
    fn take_frames(&mut self, run: u64) -> HashMap<u32, Vec<Record>>;
}

/// A `not_found` error for an element.
#[must_use]
#[expect(
    clippy::needless_pass_by_value,
    reason = "callers pass ids by value for brevity"
)]
pub fn not_found(what: &str, subject: impl ToString, fix: &str) -> ErrorObject {
    ErrorObject::new(
        ErrorKind::NotFound,
        format!("no such {what}"),
        Some(subject.to_string()),
        fix,
    )
}

/// An `invalid` error.
#[must_use]
pub fn invalid(message: impl Into<String>, subject: Option<String>, fix: &str) -> ErrorObject {
    ErrorObject::new(ErrorKind::Invalid, message, subject, fix)
}
