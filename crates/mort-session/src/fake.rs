//! An in-memory [`Session`] for tests (ADR 0011 §1).
//!
//! The fake keeps a whole project in memory. Undo works the way REAPER's
//! does from the outside: each undo point restores the project as it was
//! before the block. The state count moves on every change, undo and
//! redo. Track chunks are the track serialised, so they change exactly
//! when the track does.
//!
//! Plugins come from a catalogue of [`PluginSpec`]s. Parameter display text
//! is the parameter's real value with its unit.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use mort_link::Record;

use mort_proto::model::{
    ControlChange, EnvelopeInfo, EnvelopePoint, EnvelopeTarget, FxInfo, ItemInfo, Note, ParamInfo,
    ProjectInfo, Region, SendInfo, TrackInfo,
};
use mort_proto::{EnvelopeId, FxId, Guid, ItemId, SteppedTempoMap, TakeId, TempoMap, TrackId};

use crate::audio::{self, Biquad, Source};
use crate::{NewTrack, Result, SendChange, Session, TrackChange, invalid, not_found};

/// How far past a range's end the fake renders, as REAPER does (ADR 0006
/// §5).
const RENDER_TAIL: f64 = 0.15;

/// The block size the fake's probes are fed in.
const PROBE_BLOCK: usize = 512;

/// The probe's plugin name in the catalogue.
const PROBE: &str = "CLAP: mort probe (Vulpus Labs)";

/// The EQ the fake applies to audio.
const REAEQ: &str = "VST: ReaEQ (Cockos)";

/// The largest linear gain a volume can take: +12 dB, as in REAPER.
const MAX_GAIN: f64 = 3.981_071_705_534_973;

/// One parameter of a fake plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginParam {
    /// Its name.
    pub name: String,
    /// Default normalised value.
    pub default: f64,
    /// Real value at 0.
    pub min: f64,
    /// Real value at 1.
    pub max: f64,
    /// Whether the mapping is logarithmic (`min` must be positive).
    pub log: bool,
    /// Unit shown after the value.
    pub unit: String,
    /// Number of steps, for stepped parameters.
    pub steps: Option<u32>,
}

impl PluginParam {
    /// A linear parameter.
    #[must_use]
    pub fn linear(name: &str, default: f64, min: f64, max: f64, unit: &str) -> Self {
        Self {
            name: name.into(),
            default,
            min,
            max,
            log: false,
            unit: unit.into(),
            steps: None,
        }
    }

    /// A logarithmic parameter.
    #[must_use]
    pub fn log(name: &str, default: f64, min: f64, max: f64, unit: &str) -> Self {
        Self {
            log: true,
            ..Self::linear(name, default, min, max, unit)
        }
    }

    /// The real value at normalised `v`.
    #[must_use]
    pub fn real(&self, v: f64) -> f64 {
        if self.log {
            self.min * (self.max / self.min).powf(v)
        } else {
            self.min + v * (self.max - self.min)
        }
    }

    fn display(&self, v: f64) -> String {
        let r = self.real(v);
        let text = if r.abs() >= 100.0 {
            format!("{r:.0}")
        } else {
            format!("{r:.2}")
        };
        if self.unit.is_empty() {
            text
        } else {
            format!("{text} {}", self.unit)
        }
    }
}

/// A plugin the fake can instantiate.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginSpec {
    /// The name `add_fx` matches, as REAPER would show it.
    pub name: String,
    /// Its parameters.
    pub params: Vec<PluginParam>,
    /// Named presets, as normalised values for every parameter.
    pub presets: Vec<(String, Vec<f64>)>,
}

impl PluginSpec {
    /// The plugins every fake session has: a one-band EQ, a compressor and
    /// the probe.
    #[must_use]
    pub fn defaults() -> Vec<PluginSpec> {
        vec![
            PluginSpec {
                name: "VST: ReaEQ (Cockos)".into(),
                params: vec![
                    PluginParam::log("Freq-Band 1", 0.5, 20.0, 20_000.0, "Hz"),
                    PluginParam::linear("Gain-Band 1", 0.5, -24.0, 24.0, "dB"),
                    PluginParam::log("Q-Band 1", 0.5, 0.1, 10.0, ""),
                ],
                presets: vec![("Mud cut".into(), vec![0.36, 0.4375, 0.6])],
            },
            PluginSpec {
                name: "VST: ReaComp (Cockos)".into(),
                params: vec![
                    PluginParam::linear("Thresh", 1.0, -60.0, 0.0, "dB"),
                    PluginParam::log("Ratio", 0.0, 1.0, 20.0, ":1"),
                ],
                presets: vec![],
            },
            PluginSpec {
                name: "CLAP: mort probe (Vulpus Labs)".into(),
                params: vec![
                    PluginParam {
                        steps: Some(65_536),
                        ..PluginParam::linear("slot", 0.0, 0.0, 65_535.0, "")
                    },
                    PluginParam {
                        steps: Some(2),
                        ..PluginParam::linear("attached", 0.0, 0.0, 1.0, "")
                    },
                ],
                presets: vec![],
            },
        ]
    }

    fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        let n = self.name.to_lowercase();
        n == q
            || n.split_once(": ")
                .is_some_and(|(_, rest)| rest == q || rest.starts_with(&format!("{q} (")))
    }
}

#[derive(Debug, Clone, Serialize)]
struct FakeFx {
    id: FxId,
    plugin: String,
    enabled: bool,
    values: Vec<f64>,
    state_tag: u64,
}

#[derive(Debug, Clone, Serialize)]
struct FakeEnvelope {
    id: EnvelopeId,
    target: EnvelopeTarget,
    points: Vec<EnvelopePoint>,
}

#[derive(Debug, Clone, Serialize)]
struct FakeItem {
    id: ItemId,
    take: TakeId,
    position: f64,
    length: f64,
    notes: Vec<Note>,
    ccs: Vec<ControlChange>,
}

#[derive(Debug, Clone, Serialize)]
struct Sound {
    source: Source,
    from: f64,
    to: f64,
}

#[derive(Debug, Clone, Serialize)]
struct FakeTrack {
    id: TrackId,
    name: String,
    parent: Option<TrackId>,
    main_send: bool,
    volume: f64,
    pan: f64,
    mute: bool,
    solo: bool,
    sends: Vec<SendInfo>,
    fx: Vec<FakeFx>,
    envelopes: Vec<FakeEnvelope>,
    items: Vec<FakeItem>,
    sounds: Vec<Sound>,
}

impl FakeTrack {
    fn new(id: TrackId, name: String, parent: Option<TrackId>) -> Self {
        Self {
            id,
            name,
            parent,
            main_send: true,
            volume: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            sends: vec![],
            fx: vec![],
            envelopes: vec![],
            items: vec![],
            sounds: vec![],
        }
    }
}

#[derive(Debug, Clone)]
struct Project {
    master: FakeTrack,
    tracks: Vec<FakeTrack>,
    regions: Vec<Region>,
}

/// An in-memory REAPER project. See the module docs.
#[derive(Debug, Clone)]
pub struct FakeSession {
    name: String,
    sample_rate: f64,
    tempo: SteppedTempoMap,
    plugins: Vec<PluginSpec>,
    project: Project,
    state: u64,
    next_id: u128,
    open_block: Option<(Project, u64)>,
    undo_stack: Vec<(String, Project)>,
    redo_stack: Vec<(String, Project)>,
    playing: Option<(f64, f64)>,
    position: f64,
    saved_to: Vec<String>,
    saved_state: u64,
    renders: Vec<(f64, f64)>,
    path: Option<String>,
    next_slot: u32,
    run: u64,
    next_run: u64,
    frames: Vec<(u32, Record)>,
}

impl Default for FakeSession {
    fn default() -> Self {
        Self::new()
    }
}

fn track_fix() -> &'static str {
    "list tracks with `mort tracks`"
}

fn fx_fix() -> &'static str {
    "list a track's FX with `mort fx <track>`"
}

impl FakeSession {
    /// An empty project at 120 bpm in 4/4, 48 kHz, with the default
    /// plugins.
    #[must_use]
    pub fn new() -> Self {
        let master = FakeTrack::new(
            TrackId(Guid(0x6d6f_7274_0000_0000_0000_0000_0000_0001)),
            "MASTER".into(),
            None,
        );
        Self {
            name: "untitled".into(),
            sample_rate: 48_000.0,
            tempo: SteppedTempoMap::constant(120.0, 4, 4),
            plugins: PluginSpec::defaults(),
            project: Project {
                master,
                tracks: vec![],
                regions: vec![],
            },
            state: 1,
            next_id: 0x6d6f_7274_0000_0000_0000_0000_0000_0002,
            open_block: None,
            undo_stack: vec![],
            redo_stack: vec![],
            playing: None,
            position: 0.0,
            saved_to: vec![],
            saved_state: 1,
            renders: vec![],
            path: None,
            next_slot: 1,
            run: 0,
            next_run: 1,
            frames: vec![],
        }
    }

    /// Sets the project file, as if the person had saved the project there.
    pub fn set_path(&mut self, path: Option<&str>) {
        self.path = path.map(str::to_owned);
        if let Some(p) = path {
            self.name = std::path::Path::new(p)
                .file_stem()
                .map_or_else(|| "untitled".into(), |s| s.to_string_lossy().into_owned());
        }
        self.saved_state = self.state;
    }

    /// Adds a test signal to a track, over `span` or always. Counts as a
    /// change, like an edit to the track's items would.
    ///
    /// # Errors
    ///
    /// `not_found` for the track.
    pub fn add_sound(
        &mut self,
        track: TrackId,
        source: Source,
        span: Option<(f64, f64)>,
    ) -> Result<()> {
        let (from, to) = span.unwrap_or((f64::MIN, f64::MAX));
        self.get_mut(track)?.sounds.push(Sound { source, from, to });
        self.changed();
        Ok(())
    }

    /// Removes a track's test signals. Counts as a change.
    ///
    /// # Errors
    ///
    /// `not_found` for the track.
    pub fn clear_sounds(&mut self, track: TrackId) -> Result<()> {
        self.get_mut(track)?.sounds.clear();
        self.changed();
        Ok(())
    }

    /// Replaces the tempo map. Counts as a change.
    pub fn set_tempo_map(&mut self, map: SteppedTempoMap) {
        self.tempo = map;
        self.changed();
    }

    /// The ranges `render` has been asked for, oldest first.
    #[must_use]
    pub fn renders(&self) -> &[(f64, f64)] {
        &self.renders
    }

    /// The paths `save_project` has written to, oldest first.
    #[must_use]
    pub fn saved_to(&self) -> &[String] {
        &self.saved_to
    }

    /// Adds a plugin to the catalogue.
    pub fn add_plugin(&mut self, spec: PluginSpec) {
        self.plugins.push(spec);
    }

    /// Adds a region (with `end`) or a marker (without). Counts as a change.
    pub fn add_region(&mut self, name: &str, start: f64, end: Option<f64>) -> u32 {
        let number = u32::try_from(self.project.regions.len()).unwrap_or(u32::MAX) + 1;
        self.project.regions.push(Region {
            number,
            name: name.into(),
            start,
            end,
        });
        self.project
            .regions
            .sort_by(|a, b| a.start.total_cmp(&b.start));
        self.changed();
        number
    }

    /// Moves the fake's transport on by `seconds` if it is playing.
    pub fn advance_by(&mut self, seconds: f64) {
        if self.playing.is_some() {
            self.advance_to(self.position + seconds);
        }
    }

    /// Lets the fake's transport reach `time`, stopping at the end of the
    /// range being played.
    pub fn advance_to(&mut self, time: f64) {
        if let Some((_, end)) = self.playing {
            if time >= end {
                self.position = end;
                self.playing = None;
            } else {
                self.position = time;
            }
        }
    }

    fn changed(&mut self) {
        self.state += 1;
    }

    fn fresh(&mut self) -> Guid {
        let g = Guid(self.next_id);
        self.next_id += 1;
        g
    }

    fn all_tracks(&self) -> impl Iterator<Item = &FakeTrack> {
        std::iter::once(&self.project.master).chain(self.project.tracks.iter())
    }

    fn all_tracks_mut(&mut self) -> impl Iterator<Item = &mut FakeTrack> {
        std::iter::once(&mut self.project.master).chain(self.project.tracks.iter_mut())
    }

    fn get(&self, id: TrackId) -> Result<&FakeTrack> {
        self.all_tracks()
            .find(|t| t.id == id)
            .ok_or_else(|| not_found("track", id, track_fix()))
    }

    fn get_mut(&mut self, id: TrackId) -> Result<&mut FakeTrack> {
        self.all_tracks_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| not_found("track", id, track_fix()))
    }

    fn fx_location(&self, fx: FxId) -> Result<(TrackId, usize)> {
        self.all_tracks()
            .find_map(|t| t.fx.iter().position(|f| f.id == fx).map(|i| (t.id, i)))
            .ok_or_else(|| not_found("FX", fx, fx_fix()))
    }

    fn fx_ref(&self, fx: FxId) -> Result<(&FakeFx, &PluginSpec)> {
        let (t, i) = self.fx_location(fx)?;
        let f = &self.get(t)?.fx[i];
        let spec = self
            .plugins
            .iter()
            .find(|p| p.name == f.plugin)
            .expect("instances come from the catalogue");
        Ok((f, spec))
    }

    fn fx_mut(&mut self, fx: FxId) -> Result<&mut FakeFx> {
        let (t, i) = self.fx_location(fx)?;
        Ok(&mut self.get_mut(t)?.fx[i])
    }

    fn param_spec(&self, fx: FxId, index: u32) -> Result<PluginParam> {
        let (_, spec) = self.fx_ref(fx)?;
        spec.params.get(index as usize).cloned().ok_or_else(|| {
            not_found(
                "parameter",
                format!("{fx} #{index}"),
                "list parameters with `mort params <fx>`",
            )
        })
    }

    fn envelope_location(&self, env: EnvelopeId) -> Result<(TrackId, usize)> {
        self.all_tracks()
            .find_map(|t| {
                t.envelopes
                    .iter()
                    .position(|e| e.id == env)
                    .map(|i| (t.id, i))
            })
            .ok_or_else(|| {
                not_found(
                    "envelope",
                    env,
                    "list envelopes with `mort automate <track>`",
                )
            })
    }

    fn envelope_mut(&mut self, env: EnvelopeId) -> Result<&mut FakeEnvelope> {
        let (t, i) = self.envelope_location(env)?;
        Ok(&mut self.get_mut(t)?.envelopes[i])
    }

    fn item_location(&self, item: ItemId) -> Result<(TrackId, usize)> {
        self.all_tracks()
            .find_map(|t| t.items.iter().position(|e| e.id == item).map(|i| (t.id, i)))
            .ok_or_else(|| not_found("item", item, "list items with `mort midi <track>`"))
    }

    fn item(&self, item: ItemId) -> Result<&FakeItem> {
        let (t, i) = self.item_location(item)?;
        Ok(&self.get(t)?.items[i])
    }

    fn item_mut(&mut self, item: ItemId) -> Result<&mut FakeItem> {
        let (t, i) = self.item_location(item)?;
        Ok(&mut self.get_mut(t)?.items[i])
    }

    fn info(&self, t: &FakeTrack) -> TrackInfo {
        let index = self.project.tracks.iter().position(|x| x.id == t.id);
        TrackInfo {
            id: t.id,
            index,
            name: t.name.clone(),
            parent: t.parent,
            main_send: t.main_send,
            volume: t.volume,
            pan: t.pan,
            mute: t.mute,
            solo: t.solo,
            sends: t.sends.clone(),
            fx: t
                .fx
                .iter()
                .enumerate()
                .map(|(i, f)| fx_info(f, i))
                .collect(),
        }
    }

    /// The track that holds the envelope for `target`, checking that the
    /// target exists.
    fn target_track(&self, target: EnvelopeTarget) -> Result<TrackId> {
        match target {
            EnvelopeTarget::Volume { track } | EnvelopeTarget::Pan { track } => {
                self.get(track).map(|t| t.id)
            }
            EnvelopeTarget::SendVolume { track, send } => {
                let t = self.get(track)?;
                if send < t.sends.len() {
                    Ok(t.id)
                } else {
                    Err(not_found(
                        "send",
                        format!("{track} send {send}"),
                        "list sends with `mort tracks`",
                    ))
                }
            }
            EnvelopeTarget::Param { fx, index } => {
                self.param_spec(fx, index)?;
                Ok(self.fx_location(fx)?.0)
            }
        }
    }
}

fn fx_info(f: &FakeFx, index: usize) -> FxInfo {
    FxInfo {
        id: f.id,
        index,
        // REAPER shows the plugin's full name, prefix included, until renamed.
        name: f.plugin.clone(),
        plugin: f.plugin.clone(),
        enabled: f.enabled,
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "callers pass ids by value for brevity"
)]
fn check_unit(v: f64, what: &str, subject: impl ToString) -> Result<()> {
    if (0.0..=1.0).contains(&v) {
        Ok(())
    } else {
        Err(invalid(
            format!("{what} {v} is outside 0–1"),
            Some(subject.to_string()),
            "give a normalised value from 0 to 1",
        ))
    }
}

fn check_range(start: f64, end: f64) -> Result<()> {
    if end > start {
        Ok(())
    } else {
        Err(invalid(
            format!("range {start}..{end} is empty"),
            None,
            "give an end after the start",
        ))
    }
}

fn envelope_range(target: EnvelopeTarget) -> (f64, f64) {
    match target {
        EnvelopeTarget::Volume { .. } | EnvelopeTarget::SendVolume { .. } => (0.0, MAX_GAIN),
        EnvelopeTarget::Pan { .. } => (-1.0, 1.0),
        EnvelopeTarget::Param { .. } => (0.0, 1.0),
    }
}

impl Session for FakeSession {
    fn project(&self) -> ProjectInfo {
        let length = self
            .all_tracks()
            .flat_map(|t| t.items.iter().map(|i| i.position + i.length))
            .chain(
                self.project
                    .regions
                    .iter()
                    .map(|r| r.end.unwrap_or(r.start)),
            )
            .fold(0.0, f64::max);
        ProjectInfo {
            name: self.name.clone(),
            path: self.path.clone(),
            sample_rate: self.sample_rate,
            length,
            dirty: self.state != self.saved_state,
        }
    }

    fn state(&self) -> u64 {
        self.state
    }

    fn tempo_map(&self) -> &dyn TempoMap {
        &self.tempo
    }

    fn regions(&self) -> Vec<Region> {
        self.project.regions.clone()
    }

    fn master(&self) -> TrackId {
        self.project.master.id
    }

    fn tracks(&self) -> Vec<TrackInfo> {
        self.project.tracks.iter().map(|t| self.info(t)).collect()
    }

    fn track(&self, id: TrackId) -> Result<TrackInfo> {
        self.get(id).map(|t| self.info(t))
    }

    fn add_track(&mut self, spec: NewTrack) -> Result<TrackId> {
        if let Some(p) = spec.parent {
            if p == self.project.master.id {
                return Err(invalid(
                    "the master is not a folder",
                    Some(p.to_string()),
                    "leave the parent out to add a top-level track",
                ));
            }
            self.get(p)?;
        }
        let index = spec.index.unwrap_or(self.project.tracks.len());
        if index > self.project.tracks.len() {
            return Err(invalid(
                format!(
                    "track index {index} is past the end ({})",
                    self.project.tracks.len()
                ),
                None,
                "leave the index out to add at the end",
            ));
        }
        let id = TrackId(self.fresh());
        self.project
            .tracks
            .insert(index, FakeTrack::new(id, spec.name, spec.parent));
        self.changed();
        Ok(id)
    }

    fn remove_track(&mut self, id: TrackId) -> Result<()> {
        if id == self.project.master.id {
            return Err(invalid(
                "the master cannot be removed",
                Some(id.to_string()),
                "remove other tracks instead",
            ));
        }
        let parent = self.get(id)?.parent;
        self.project.tracks.retain(|t| t.id != id);
        for t in self.all_tracks_mut() {
            if t.parent == Some(id) {
                t.parent = parent;
            }
            let removed: Vec<usize> = t
                .sends
                .iter()
                .enumerate()
                .filter(|(_, s)| s.dest == id)
                .map(|(i, _)| i)
                .collect();
            t.sends.retain(|s| s.dest != id);
            let me = t.id;
            t.envelopes.retain(|e| match e.target {
                EnvelopeTarget::SendVolume { send, .. } => !removed.contains(&send),
                _ => true,
            });
            for e in &mut t.envelopes {
                if let EnvelopeTarget::SendVolume { track, send } = &mut e.target {
                    debug_assert_eq!(*track, me);
                    *send -= removed.iter().filter(|r| **r < *send).count();
                }
            }
        }
        self.changed();
        Ok(())
    }

    fn set_track(&mut self, id: TrackId, change: &TrackChange) -> Result<()> {
        if let Some(v) = change.volume
            && !(0.0..=MAX_GAIN).contains(&v)
        {
            return Err(invalid(
                format!("volume {v} is outside 0 to +12 dB"),
                Some(id.to_string()),
                "give a linear gain from 0 to 3.98",
            ));
        }
        if let Some(p) = change.pan
            && !(-1.0..=1.0).contains(&p)
        {
            return Err(invalid(
                format!("pan {p} is outside −1 to 1"),
                Some(id.to_string()),
                "give a pan from −1 (left) to 1 (right)",
            ));
        }
        let t = self.get_mut(id)?;
        if let Some(n) = &change.name {
            t.name.clone_from(n);
        }
        if let Some(v) = change.volume {
            t.volume = v;
        }
        if let Some(p) = change.pan {
            t.pan = p;
        }
        if let Some(m) = change.mute {
            t.mute = m;
        }
        if let Some(s) = change.solo {
            t.solo = s;
        }
        self.changed();
        Ok(())
    }

    fn add_send(&mut self, from: TrackId, to: TrackId) -> Result<usize> {
        self.get(to)?;
        if from == to {
            return Err(invalid(
                "a track cannot send to itself",
                Some(from.to_string()),
                "choose another destination",
            ));
        }
        let t = self.get_mut(from)?;
        t.sends.push(SendInfo {
            dest: to,
            volume: 1.0,
            pan: 0.0,
            mute: false,
        });
        let i = t.sends.len() - 1;
        self.changed();
        Ok(i)
    }

    fn set_send(&mut self, from: TrackId, send: usize, change: &SendChange) -> Result<()> {
        let t = self.get_mut(from)?;
        let s = t.sends.get_mut(send).ok_or_else(|| {
            not_found(
                "send",
                format!("{from} send {send}"),
                "list sends with `mort tracks`",
            )
        })?;
        if let Some(v) = change.volume {
            s.volume = v;
        }
        if let Some(p) = change.pan {
            s.pan = p;
        }
        if let Some(m) = change.mute {
            s.mute = m;
        }
        self.changed();
        Ok(())
    }

    fn track_chunk(&self, id: TrackId) -> Result<String> {
        let t = self.get(id)?;
        Ok(serde_json::to_string(t).expect("fake tracks serialise"))
    }

    fn audio_chunk(&self, id: TrackId) -> Result<String> {
        let mut t = self.get(id)?.clone();
        t.name.clear();
        t.fx.retain(|f| f.plugin != PROBE);
        Ok(serde_json::to_string(&t).expect("fake tracks serialise"))
    }

    fn add_fx(&mut self, track: TrackId, plugin: &str, index: Option<usize>) -> Result<FxId> {
        let spec = self
            .plugins
            .iter()
            .find(|p| p.matches(plugin))
            .cloned()
            .ok_or_else(|| {
                let names: Vec<&str> = self.plugins.iter().map(|p| p.name.as_str()).collect();
                not_found(
                    "plugin",
                    plugin,
                    &format!("use one of: {}", names.join(", ")),
                )
            })?;
        let id = FxId(self.fresh());
        let mut values: Vec<f64> = spec.params.iter().map(|p| p.default).collect();
        if spec.name == PROBE {
            // An attached probe shows its slot (ADR 0005 §5).
            values = vec![f64::from(self.next_slot) / 65_535.0, 1.0];
            self.next_slot += 1;
        }
        let t = self.get_mut(track)?;
        let index = index.unwrap_or(t.fx.len());
        if index > t.fx.len() {
            return Err(invalid(
                format!(
                    "FX index {index} is past the end of the chain ({})",
                    t.fx.len()
                ),
                Some(track.to_string()),
                "leave the index out to add at the end",
            ));
        }
        t.fx.insert(
            index,
            FakeFx {
                id,
                plugin: spec.name.clone(),
                enabled: true,
                values,
                state_tag: 0,
            },
        );
        self.changed();
        Ok(id)
    }

    fn remove_fx(&mut self, fx: FxId) -> Result<()> {
        let (t, i) = self.fx_location(fx)?;
        let t = self.get_mut(t)?;
        t.fx.remove(i);
        t.envelopes
            .retain(|e| !matches!(e.target, EnvelopeTarget::Param { fx: f, .. } if f == fx));
        self.changed();
        Ok(())
    }

    fn move_fx(&mut self, fx: FxId, index: usize) -> Result<()> {
        let (t, i) = self.fx_location(fx)?;
        let t = self.get_mut(t)?;
        if index >= t.fx.len() {
            return Err(invalid(
                format!(
                    "FX index {index} is past the end of the chain ({})",
                    t.fx.len()
                ),
                Some(fx.to_string()),
                "give an index within the chain",
            ));
        }
        let f = t.fx.remove(i);
        t.fx.insert(index, f);
        self.changed();
        Ok(())
    }

    fn set_fx_enabled(&mut self, fx: FxId, enabled: bool) -> Result<()> {
        self.fx_mut(fx)?.enabled = enabled;
        self.changed();
        Ok(())
    }

    fn fx(&self, fx: FxId) -> Result<(TrackId, FxInfo)> {
        let (t, i) = self.fx_location(fx)?;
        Ok((t, fx_info(&self.get(t)?.fx[i], i)))
    }

    fn params(&self, fx: FxId) -> Result<Vec<ParamInfo>> {
        let (f, spec) = self.fx_ref(fx)?;
        Ok(spec
            .params
            .iter()
            .zip(&f.values)
            .zip(0u32..)
            .map(|((p, &v), index)| ParamInfo {
                index,
                name: p.name.clone(),
                value: v,
                display: p.display(v),
                steps: p.steps,
            })
            .collect())
    }

    fn set_param(&mut self, fx: FxId, index: u32, value: f64) -> Result<()> {
        self.param_spec(fx, index)?;
        check_unit(value, "parameter value", format!("{fx} #{index}"))?;
        self.fx_mut(fx)?.values[index as usize] = value;
        self.changed();
        Ok(())
    }

    fn format_param(&self, fx: FxId, index: u32, value: f64) -> Result<Option<String>> {
        let p = self.param_spec(fx, index)?;
        Ok(Some(p.display(value.clamp(0.0, 1.0))))
    }

    fn fx_state(&self, fx: FxId) -> Result<String> {
        let (f, _) = self.fx_ref(fx)?;
        Ok(serde_json::to_string(&(&f.values, f.state_tag)).expect("values serialise"))
    }

    fn set_fx_state(&mut self, fx: FxId, state: &str) -> Result<()> {
        let n = self.fx_ref(fx)?.0.values.len();
        let bad = || {
            invalid(
                "the plugin rejected the state",
                Some(fx.to_string()),
                "pass state read from this plugin with `fx.state`",
            )
        };
        let (values, tag): (Vec<f64>, u64) = serde_json::from_str(state).map_err(|_| bad())?;
        if values.len() != n {
            return Err(bad());
        }
        let f = self.fx_mut(fx)?;
        f.values = values;
        f.state_tag = tag;
        self.changed();
        Ok(())
    }

    fn load_preset(&mut self, fx: FxId, preset: &str) -> Result<()> {
        let (_, spec) = self.fx_ref(fx)?;
        let values = spec
            .presets
            .iter()
            .find(|(n, _)| n == preset)
            .map(|(_, v)| v.clone())
            .ok_or_else(|| {
                let names: Vec<&str> = spec.presets.iter().map(|(n, _)| n.as_str()).collect();
                not_found(
                    "preset",
                    preset,
                    &if names.is_empty() {
                        "this plugin has no presets".to_owned()
                    } else {
                        format!("use one of: {}", names.join(", "))
                    },
                )
            })?;
        self.fx_mut(fx)?.values = values;
        self.changed();
        Ok(())
    }

    fn envelope(&mut self, target: EnvelopeTarget, create: bool) -> Result<Option<EnvelopeInfo>> {
        let t = self.target_track(target)?;
        if let Some(e) = self.get(t)?.envelopes.iter().find(|e| e.target == target) {
            return Ok(Some(EnvelopeInfo { id: e.id, target }));
        }
        if !create {
            return Ok(None);
        }
        let id = EnvelopeId(self.fresh());
        self.get_mut(t)?.envelopes.push(FakeEnvelope {
            id,
            target,
            points: vec![],
        });
        self.changed();
        Ok(Some(EnvelopeInfo { id, target }))
    }

    fn envelopes(&self, track: TrackId) -> Result<Vec<EnvelopeInfo>> {
        Ok(self
            .get(track)?
            .envelopes
            .iter()
            .map(|e| EnvelopeInfo {
                id: e.id,
                target: e.target,
            })
            .collect())
    }

    fn envelope_points(&self, env: EnvelopeId, start: f64, end: f64) -> Result<Vec<EnvelopePoint>> {
        let (t, i) = self.envelope_location(env)?;
        Ok(self.get(t)?.envelopes[i]
            .points
            .iter()
            .filter(|p| p.time >= start && p.time < end)
            .copied()
            .collect())
    }

    fn format_envelope_value(&self, env: EnvelopeId, value: f64) -> Result<Option<String>> {
        let (t, i) = self.envelope_location(env)?;
        let target = self.get(t)?.envelopes[i].target;
        Ok(Some(match target {
            EnvelopeTarget::Volume { .. } | EnvelopeTarget::SendVolume { .. } => {
                if value <= 0.0 {
                    "-inf".into()
                } else {
                    format!("{:.2}dB", 20.0 * value.log10())
                }
            }
            EnvelopeTarget::Pan { .. } => {
                let pct = (value.abs() * 100.0).round();
                if pct == 0.0 {
                    "center".into()
                } else if value < 0.0 {
                    format!("{pct}%L")
                } else {
                    format!("{pct}%R")
                }
            }
            EnvelopeTarget::Param { .. } => format!("{value:.3}"),
        }))
    }

    fn insert_envelope_points(&mut self, env: EnvelopeId, points: &[EnvelopePoint]) -> Result<()> {
        let e = self.envelope_mut(env)?;
        let (lo, hi) = envelope_range(e.target);
        if let Some(p) = points.iter().find(|p| !(lo..=hi).contains(&p.value)) {
            return Err(invalid(
                format!("value {} at {}s is outside {lo} to {hi}", p.value, p.time),
                Some(env.to_string()),
                "give values in the envelope's range",
            ));
        }
        e.points.extend_from_slice(points);
        e.points.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.changed();
        Ok(())
    }

    fn delete_envelope_points(&mut self, env: EnvelopeId, start: f64, end: f64) -> Result<()> {
        let e = self.envelope_mut(env)?;
        e.points.retain(|p| p.time < start || p.time >= end);
        self.changed();
        Ok(())
    }

    fn items(&self, track: TrackId) -> Result<Vec<ItemInfo>> {
        let mut items: Vec<ItemInfo> = self
            .get(track)?
            .items
            .iter()
            .map(|i| ItemInfo {
                id: i.id,
                take: Some(i.take),
                position: i.position,
                length: i.length,
                midi: true,
            })
            .collect();
        items.sort_by(|a, b| a.position.total_cmp(&b.position));
        Ok(items)
    }

    fn add_midi_item(&mut self, track: TrackId, start: f64, end: f64) -> Result<ItemId> {
        check_range(start, end)?;
        self.get(track)?;
        let id = ItemId(self.fresh());
        let take = TakeId(self.fresh());
        self.get_mut(track)?.items.push(FakeItem {
            id,
            take,
            position: start,
            length: end - start,
            notes: vec![],
            ccs: vec![],
        });
        self.changed();
        Ok(id)
    }

    fn notes(&self, item: ItemId) -> Result<Vec<Note>> {
        Ok(self.item(item)?.notes.clone())
    }

    fn control_changes(&self, item: ItemId) -> Result<Vec<ControlChange>> {
        Ok(self.item(item)?.ccs.clone())
    }

    fn insert_midi(&mut self, item: ItemId, notes: &[Note], ccs: &[ControlChange]) -> Result<()> {
        let i = self.item_mut(item)?;
        let (lo, hi) = (i.position, i.position + i.length);
        let outside = notes
            .iter()
            .map(|n| (n.start, n.end))
            .chain(ccs.iter().map(|c| (c.time, c.time)))
            .find(|&(s, e)| s < lo || e > hi || e < s);
        if let Some((s, e)) = outside {
            return Err(invalid(
                format!(
                    "an event at {s}–{e}s lies outside the item ({lo}–{hi}s) or ends before it starts"
                ),
                Some(item.to_string()),
                "lengthen the item or move the event",
            ));
        }
        i.notes.extend_from_slice(notes);
        i.notes.sort_by(|a, b| a.start.total_cmp(&b.start));
        i.ccs.extend_from_slice(ccs);
        i.ccs.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.changed();
        Ok(())
    }

    fn delete_midi(&mut self, item: ItemId, start: f64, end: f64) -> Result<()> {
        let i = self.item_mut(item)?;
        i.notes.retain(|n| n.start < start || n.start >= end);
        i.ccs.retain(|c| c.time < start || c.time >= end);
        self.changed();
        Ok(())
    }

    fn find_actions(&self, text: &str, limit: usize) -> Vec<(i32, String)> {
        let t = text.to_lowercase();
        [
            (40044, "Transport: Play/stop"),
            (
                41824,
                "File: Render project, using the most recent render settings",
            ),
        ]
        .into_iter()
        .filter(|(_, n)| n.to_lowercase().contains(&t))
        .take(limit)
        .map(|(i, n)| (i, n.to_owned()))
        .collect()
    }

    fn save_project(&mut self, path: Option<&str>) -> Result<()> {
        match path {
            Some(p) => {
                self.saved_to.push(p.to_owned());
                Ok(())
            }
            None => Err(invalid(
                "the project has never been saved",
                None,
                "give a path to save it to",
            )),
        }
    }

    fn begin_undo(&mut self) {
        // REAPER moves the state count when a block opens, even if nothing
        // then changes; the fake does the same so callers cannot rely on it.
        self.changed();
        self.open_block = Some((self.project.clone(), self.state));
    }

    fn end_undo(&mut self, label: &str) {
        if let Some((before, state)) = self.open_block.take()
            && state != self.state
        {
            self.undo_stack.push((label.to_owned(), before));
            self.redo_stack.clear();
        }
    }

    fn undo_label(&self) -> Option<String> {
        self.undo_stack.last().map(|(l, _)| l.clone())
    }

    fn redo_label(&self) -> Option<String> {
        self.redo_stack.last().map(|(l, _)| l.clone())
    }

    fn undo(&mut self) -> Option<String> {
        let (label, before) = self.undo_stack.pop()?;
        let now = std::mem::replace(&mut self.project, before);
        self.redo_stack.push((label.clone(), now));
        self.changed();
        Some(label)
    }

    fn redo(&mut self) -> Option<String> {
        let (label, after) = self.redo_stack.pop()?;
        let now = std::mem::replace(&mut self.project, after);
        self.undo_stack.push((label.clone(), now));
        self.changed();
        Some(label)
    }

    fn play(&mut self, start: f64, end: f64) -> Result<()> {
        check_range(start, end)?;
        self.playing = Some((start, end));
        self.position = start;
        // Probes hear the whole range now; frames past where playback
        // stops are dropped when it stops.
        self.synthesize(start, end);
        Ok(())
    }

    fn render(&mut self, start: f64, end: f64) -> Result<()> {
        check_range(start, end)?;
        self.renders.push((start, end));
        self.synthesize(start, end);
        Ok(())
    }

    fn stop(&mut self) {
        if self.playing.take().is_some() {
            let (run, at) = (self.run, self.position);
            self.frames
                .retain(|(_, r)| r.run != run || r.start + 0.05 <= at);
        }
    }

    fn is_playing(&self) -> bool {
        self.playing.is_some()
    }

    fn position(&self) -> f64 {
        self.position
    }

    fn begin_run(&mut self) -> u64 {
        self.run = self.next_run;
        self.next_run += 1;
        self.run
    }

    fn end_run(&mut self, run: u64) {
        if self.run == run {
            self.run = 0;
        }
    }

    fn take_frames(&mut self, run: u64) -> HashMap<u32, Vec<Record>> {
        let mut out: HashMap<u32, Vec<Record>> = HashMap::new();
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.frames)
            .into_iter()
            .partition(|(_, r)| r.run == run);
        self.frames = rest;
        for (slot, r) in mine {
            out.entry(slot).or_default().push(r);
        }
        out
    }
}

/// Left and right gains for a pan, as a balance control.
fn balance(pan: f64) -> (f64, f64) {
    ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
}

impl FakeSession {
    /// Runs the project's audio from `start` to a little past `end` and
    /// feeds every enabled probe's input to the real feature code, keeping
    /// the frames under the current run (ADR 0011 §2).
    fn synthesize(&mut self, start: f64, end: f64) {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "test ranges are short"
        )]
        let n = ((end + RENDER_TAIL - start) * self.sample_rate).round() as usize;
        let mut graph = Graph {
            start,
            n,
            done: HashMap::new(),
            visiting: HashSet::new(),
            taps: vec![],
        };
        for t in self.all_tracks() {
            graph.out(self, t.id);
        }
        let rate = self.sample_rate;
        for (slot, [l, r]) in graph.taps {
            let mut a = mort_features::Analyzer::new(rate, PROBE_BLOCK)
                .expect("the fake's rate is supported");
            #[expect(clippy::cast_possible_truncation, reason = "audio to f32")]
            let (l, r): (Vec<f32>, Vec<f32>) = (
                l.iter().map(|v| *v as f32).collect(),
                r.iter().map(|v| *v as f32).collect(),
            );
            for (b, (lb, rb)) in l.chunks(PROBE_BLOCK).zip(r.chunks(PROBE_BLOCK)).enumerate() {
                #[expect(clippy::cast_precision_loss, reason = "sample counts are small")]
                let pos = start + (b * PROBE_BLOCK) as f64 / rate;
                let run = self.run;
                let frames = &mut self.frames;
                a.process(lb, rb, |f| {
                    frames.push((
                        slot,
                        Record {
                            run,
                            start: pos + (f64::from(f.end_offset) - f64::from(f.len)) / rate,
                            frame: *f,
                        },
                    ));
                });
            }
        }
    }
}

/// One pass of the fake's mixer.
struct Graph {
    start: f64,
    n: usize,
    done: HashMap<TrackId, [Vec<f64>; 2]>,
    visiting: HashSet<TrackId>,
    taps: Vec<(u32, [Vec<f64>; 2])>,
}

impl Graph {
    /// A track's audio after its fader, working out its inputs first.
    fn out(&mut self, sess: &FakeSession, id: TrackId) -> [Vec<f64>; 2] {
        if let Some(v) = self.done.get(&id) {
            return v.clone();
        }
        let silent = [vec![0.0; self.n], vec![0.0; self.n]];
        // REAPER refuses routing loops; the fake just breaks them.
        if !self.visiting.insert(id) {
            return silent;
        }
        let Ok(track) = sess.get(id) else {
            return silent;
        };
        // REAPER does not process muted tracks by default: no FX run, so
        // no probe hears them.
        if track.mute {
            self.visiting.remove(&id);
            self.done.insert(id, silent.clone());
            return silent;
        }
        let rate = sess.sample_rate;
        let mut mono = vec![0.0; self.n];
        let notes: Vec<Note> = track.items.iter().flat_map(|i| i.notes.clone()).collect();
        for (k, snd) in track.sounds.iter().enumerate() {
            let seed = u64::try_from(id.0.0 & u128::from(u64::MAX)).unwrap_or(1)
                ^ (k as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            audio::add(
                &mut mono,
                self.start,
                rate,
                &snd.source,
                (snd.from, snd.to),
                seed,
                &notes,
            );
        }
        let mut left = mono.clone();
        let mut right = mono;
        let master = sess.project.master.id;
        let mut inputs: Vec<(TrackId, f64, f64)> = vec![];
        for u in &sess.project.tracks {
            let feeds = if id == master {
                u.parent.is_none()
            } else {
                u.parent == Some(id)
            };
            if u.main_send && feeds {
                inputs.push((u.id, 1.0, 0.0));
            }
            for send in u.sends.iter().filter(|x| x.dest == id && !x.mute) {
                inputs.push((u.id, send.volume, send.pan));
            }
        }
        for (u, g, p) in inputs {
            let [ul, ur] = self.out(sess, u);
            let (gl, gr) = balance(p);
            for i in 0..self.n {
                left[i] += ul[i] * g * gl;
                right[i] += ur[i] * g * gr;
            }
        }
        for fx in track.fx.iter().filter(|f| f.enabled) {
            if fx.plugin == PROBE {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "slots are small"
                )]
                let slot = (fx.values[0] * 65_535.0).round() as u32;
                self.taps.push((slot, [left.clone(), right.clone()]));
            } else if fx.plugin == REAEQ
                && let Some(spec) = sess.plugins.iter().find(|p| p.name == REAEQ)
            {
                let real = |i: usize| spec.params[i].real(fx.values[i]);
                let band = Biquad::peak(real(0), rate, real(2), real(1));
                for ch in [&mut left, &mut right] {
                    let mut filter = band;
                    for v in ch.iter_mut() {
                        *v = filter.run(*v);
                    }
                }
            }
        }
        let (gl, gr) = balance(track.pan);
        let gain = track.volume;
        for x in &mut left {
            *x *= gain * gl;
        }
        for x in &mut right {
            *x *= gain * gr;
        }
        self.visiting.remove(&id);
        self.done.insert(id, [left.clone(), right.clone()]);
        [left, right]
    }
}
