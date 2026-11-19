//! The extension entry point, and [`Live`]: the [`Session`] over REAPER.
//!
//! REAPER loads `reaper_mort.dylib` from `UserPlugins` and calls
//! `ReaperPluginEntry`. We start the socket server, register a timer that
//! runs the dispatcher on the main thread, and keep everything in a
//! main-thread static until REAPER unloads us.
//!
//! [`Live`] implements only what the bridge skeleton needs (ticket 0003).
//! The rest of the session answers with an error naming the ticket that
//! adds it.

use std::cell::RefCell;
use std::error::Error;
use std::ffi::CStr;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use reaper_low::{PluginContext, PluginDestroyHook, register_plugin_destroy_hook};
use std::sync::atomic::{AtomicU32, Ordering};

use reaper_medium::{
    CommandId, HookCommand, OwnedGaccelRegister, PositionInQuarterNotes, PositionInSeconds,
    ProjectContext, Reaper, ReaperSession, RegistrationObject, ToggleAction, ToggleActionResult,
    UndoScope,
};

use mort_proto::model::{
    ControlChange, EnvelopeInfo, EnvelopePoint, EnvelopeTarget, FxInfo, ItemInfo, Note, ParamInfo,
    ProjectInfo, Region, TrackInfo,
};
use mort_proto::model::{SendInfo, Shape};
use mort_proto::{
    EnvelopeId, ErrorKind, ErrorObject, FxId, Guid, ItemId, TakeId, TempoMap, TrackId,
};
use mort_session::{NewTrack, Result, SendChange, Session, TrackChange, invalid, not_found};
use reaper_low::raw::{MediaItem, MediaItem_Take, MediaTrack, TrackEnvelope};
use std::os::raw::c_int;

use crate::{chunk, raw};

use crate::dispatch::{BUILD, Dispatcher};
use crate::server::{Server, socket_dir};

/// The dispatcher's time budget per timer tick (ADR 0002 §2).
const TICK_BUDGET: Duration = Duration::from_millis(8);

const PROJ: ProjectContext = ProjectContext::CurrentProject;

struct Bridge {
    live: Live,
    dispatcher: Dispatcher<Live>,
    _server: Server,
}

thread_local! {
    static BRIDGE: RefCell<Option<Bridge>> = const { RefCell::new(None) };
}

/// Appends a line to `$TMPDIR/mort/bridge.log`. Opening REAPER's console
/// on every start would be intrusive, so only failures go there.
fn log(line: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(socket_dir().join("bridge.log"))
    {
        let _ = writeln!(f, "[{}] {line}", std::process::id());
    }
}

#[reaper_macros::reaper_extension_plugin]
fn plugin_main(context: PluginContext) -> std::result::Result<(), Box<dyn Error>> {
    let session = ReaperSession::load(context);
    let reaper = session.reaper().clone();
    if let Err(e) = start(session) {
        let msg = format!("mort bridge {BUILD} failed to start: {e}\n");
        log(msg.trim_end());
        reaper.show_console_msg(msg.as_str());
        return Err(e);
    }
    register_plugin_destroy_hook(PluginDestroyHook {
        name: "mort bridge",
        callback: || {
            BRIDGE.with(|b| b.borrow_mut().take());
        },
    });
    Ok(())
}

fn start(mut session: ReaperSession) -> std::result::Result<(), Box<dyn Error>> {
    session.plugin_register_add_timer(tick)?;
    let hold = session.plugin_register_add_command_id("MORT_HOLD_EDITS")?;
    HOLD_COMMAND.store(hold.get(), Ordering::Relaxed);
    session.plugin_register_add_gaccel(OwnedGaccelRegister::without_key_binding(
        hold,
        "mort: hold edits",
    ))?;
    session.plugin_register_add_hook_command::<HoldEdits>()?;
    session.plugin_register_add_toggle_action::<HoldEdits>()?;
    // SAFETY: `attach` has the signature probes expect and lives as long as
    // the extension.
    unsafe {
        session.plugin_register_add(RegistrationObject::api(
            "mort_probe_attach",
            crate::probes::attach as *mut std::ffi::c_void,
        ))?;
    }
    crate::probes::start_draining();
    let (tx, rx) = mpsc::channel();
    let feed = std::sync::Arc::new(crate::feed::Feed::new());
    let server = Server::start_with_feed(
        &socket_dir(),
        &std::process::id().to_string(),
        tx,
        Some(std::sync::Arc::clone(&feed)),
    )?;
    log(&format!(
        "mort bridge {BUILD} listening at {}",
        server.path().display()
    ));
    let reaper = session.reaper().clone();
    BRIDGE.with(|b| {
        *b.borrow_mut() = Some(Bridge {
            live: Live {
                reaper,
                _session: Some(session),
            },
            dispatcher: {
                let mut d = Dispatcher::new(rx);
                crate::methods::register_reads(&mut d);
                crate::methods::register_edits(&mut d);
                crate::methods::register_automation(&mut d);
                crate::methods::register_midi(&mut d);
                crate::methods::register_transport(&mut d);
                crate::methods::register_probes(&mut d);
                crate::measure::register_measure_with_feed(&mut d, feed);
                d
            },
            _server: server,
        });
    });
    Ok(())
}

/// The command id REAPER gave *mort: hold edits*.
static HOLD_COMMAND: AtomicU32 = AtomicU32::new(0);

/// *mort: hold edits*: a toggle action that makes the bridge refuse edits
/// (ADR 0004 §4).
struct HoldEdits;

fn with_dispatcher<R>(f: impl FnOnce(&mut Dispatcher<Live>) -> R) -> Option<R> {
    BRIDGE.with(|b| {
        b.try_borrow_mut()
            .ok()
            .and_then(|mut b| b.as_mut().map(|br| f(&mut br.dispatcher)))
    })
}

impl HookCommand for HoldEdits {
    fn call(command_id: CommandId, _flag: i32) -> bool {
        if command_id.get() != HOLD_COMMAND.load(Ordering::Relaxed) {
            return false;
        }
        with_dispatcher(|d| {
            let held = !d.held();
            d.set_held(held);
            log(if held { "edits held" } else { "edits released" });
        });
        true
    }
}

impl ToggleAction for HoldEdits {
    fn call(command_id: CommandId) -> ToggleActionResult {
        if command_id.get() != HOLD_COMMAND.load(Ordering::Relaxed) {
            return ToggleActionResult::NotRelevant;
        }
        match with_dispatcher(|d| d.held()) {
            Some(true) => ToggleActionResult::On,
            _ => ToggleActionResult::Off,
        }
    }
}

extern "C" fn tick() {
    let r = catch_unwind(AssertUnwindSafe(|| {
        BRIDGE.with(|b| {
            if let Ok(mut b) = b.try_borrow_mut()
                && let Some(br) = b.as_mut()
            {
                br.dispatcher
                    .tick(&mut br.live, Instant::now(), TICK_BUDGET);
            }
        });
    }));
    if r.is_err() {
        log("panic in the dispatcher tick; the bridge carries on");
    }
}

/// The [`Session`] over REAPER's current project.
struct Live {
    reaper: Reaper,
    /// Keeps the timer registered. Dropped last.
    _session: Option<ReaperSession>,
}

impl Live {
    fn low(&self) -> &reaper_low::Reaper {
        self.reaper.low()
    }

    fn all_tracks(&self) -> Vec<*mut MediaTrack> {
        let mut v = vec![raw::master(self.low())];
        v.extend(raw::tracks(self.low()));
        v
    }

    fn find_track(&self, id: TrackId) -> Result<*mut MediaTrack> {
        self.all_tracks()
            .into_iter()
            .find(|t| raw::track_guid(self.low(), *t) == id.0)
            .ok_or_else(|| not_found("track", id, "list tracks with `mort tracks`"))
    }

    fn find_fx(&self, id: FxId) -> Result<(*mut MediaTrack, c_int)> {
        let r = self.low();
        self.all_tracks()
            .into_iter()
            .find_map(|t| {
                (0..raw::fx_count(r, t))
                    .find(|i| raw::fx_guid(r, t, *i) == Some(id.0))
                    .map(|i| (t, i))
            })
            .ok_or_else(|| not_found("FX", id, "list a track's FX with `mort fx <track>`"))
    }

    fn param_index(&self, t: *mut MediaTrack, fx: c_int, id: FxId, index: u32) -> Result<c_int> {
        c_int::try_from(index)
            .ok()
            .filter(|i| *i < raw::param_count(self.low(), t, fx))
            .ok_or_else(|| {
                not_found(
                    "parameter",
                    format!("{id} #{index}"),
                    "list parameters with `mort params <track> <fx>`",
                )
            })
    }

    fn fx_info(&self, t: *mut MediaTrack, i: c_int) -> FxInfo {
        let r = self.low();
        let name = raw::fx_name(r, t, i);
        FxInfo {
            id: FxId(raw::fx_guid(r, t, i).unwrap_or(Guid(0))),
            index: usize::try_from(i).unwrap_or(0),
            plugin: raw::fx_config(r, t, i, "fx_name").unwrap_or_else(|| name.clone()),
            name,
            enabled: raw::fx_enabled(r, t, i),
        }
    }

    /// Adds a track as the last child of folder `p`, making `p` a folder if
    /// it is not one, and keeps every other track's nesting as it was.
    fn insert_in_folder(&self, p: *mut MediaTrack) -> *mut MediaTrack {
        let r = self.low();
        let all = raw::tracks(r);
        let pi = all.iter().position(|t| *t == p).expect("p is a track");
        let base = raw::track_depth(r, p);
        // The last track inside p, if any: the run of deeper tracks after it.
        let last = all[pi + 1..]
            .iter()
            .take_while(|t| raw::track_depth(r, **t) > base)
            .last()
            .copied();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "folder depths are small integers"
        )]
        let depth = |t| raw::track_value(r, t, "I_FOLDERDEPTH") as c_int;
        let (after, closing) = match last {
            None => {
                // p becomes a folder; the new track closes it and whatever p closed.
                let d = depth(p);
                raw::set_track_value(r, p, "I_FOLDERDEPTH", 1.0);
                (pi, d - 1)
            }
            Some(l) => {
                // l closes its own levels down to p's children; the new track
                // closes the rest.
                let li = all.iter().position(|t| *t == l).expect("l is a track");
                let level = raw::track_depth(r, l);
                let keep = -(level - base - 1);
                let d = depth(l);
                raw::set_track_value(r, l, "I_FOLDERDEPTH", f64::from(keep));
                (li, d - keep)
            }
        };
        let idx = c_int::try_from(after + 1).unwrap_or(c_int::MAX);
        let t = raw::insert_track(r, idx);
        raw::set_track_value(r, t, "I_FOLDERDEPTH", f64::from(closing));
        t
    }

    /// Moves `t`'s folder-depth change onto its neighbours before it is
    /// deleted, because REAPER's `DeleteTrack` leaves depths as they were.
    /// Every other track keeps its level, except the children of a deleted
    /// folder, which move up one level (as in the fake).
    fn unlink_from_folders(&self, t: *mut MediaTrack) {
        let r = self.low();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "folder depths are small integers"
        )]
        let depth = |t| raw::track_value(r, t, "I_FOLDERDEPTH") as c_int;
        let all = raw::tracks(r);
        let Some(i) = all.iter().position(|x| *x == t) else {
            return;
        };
        let d = depth(t);
        if d > 0 {
            // A folder: the track that closes it now closes one level fewer.
            let base = raw::track_depth(r, t);
            let closer = all[i + 1..]
                .iter()
                .take_while(|x| raw::track_depth(r, **x) > base)
                .last()
                .copied();
            if let Some(c) = closer {
                raw::set_track_value(r, c, "I_FOLDERDEPTH", f64::from(depth(c) + 1));
            }
        } else if d < 0 && i > 0 {
            // It closed folders: the track before it closes them instead.
            let prev = all[i - 1];
            raw::set_track_value(r, prev, "I_FOLDERDEPTH", f64::from(depth(prev) + d));
        }
    }

    fn find_midi(&self, id: ItemId) -> Result<(*mut MediaItem, *mut MediaItem_Take)> {
        let r = self.low();
        let item = self
            .all_tracks()
            .into_iter()
            .flat_map(|t| raw::items(r, t))
            .find(|i| raw::item_guid(r, *i) == Some(id.0))
            .ok_or_else(|| {
                not_found("item", id, "list a track's items with `mort midi <track>`")
            })?;
        let take = raw::active_take(r, item)
            .filter(|k| raw::take_is_midi(r, *k))
            .ok_or_else(|| {
                invalid(
                    "the item is not MIDI",
                    Some(id.to_string()),
                    "choose a MIDI item",
                )
            })?;
        Ok((item, take))
    }

    fn track_info(&self, t: *mut MediaTrack) -> TrackInfo {
        let r = self.low();
        let number = raw::track_value(r, t, "IP_TRACKNUMBER");
        let is_master = t == raw::master(r);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "REAPER track numbers are small positive integers held in a double"
        )]
        let index = (!is_master && number >= 1.0).then(|| number as usize - 1);
        let sends = (0..raw::send_count(r, t))
            .map(|s| SendInfo {
                dest: TrackId(raw::track_guid(r, raw::send_dest(r, t, s))),
                volume: raw::send_value(r, t, s, "D_VOL"),
                pan: raw::send_value(r, t, s, "D_PAN"),
                mute: raw::send_value(r, t, s, "B_MUTE") != 0.0,
            })
            .collect();
        TrackInfo {
            id: TrackId(raw::track_guid(r, t)),
            index,
            name: if is_master {
                "MASTER".into()
            } else {
                raw::track_string(r, t, "P_NAME")
            },
            parent: raw::parent(r, t).map(|p| TrackId(raw::track_guid(r, p))),
            main_send: is_master || raw::track_value(r, t, "B_MAINSEND") != 0.0,
            volume: raw::track_value(r, t, "D_VOL"),
            pan: raw::track_value(r, t, "D_PAN"),
            mute: raw::track_value(r, t, "B_MUTE") != 0.0,
            solo: raw::track_value(r, t, "I_SOLO") > 0.0,
            sends,
            fx: (0..raw::fx_count(r, t))
                .map(|i| self.fx_info(t, i))
                .collect(),
        }
    }
}

fn reaper_failed(call: &str, subject: &impl ToString) -> ErrorObject {
    ErrorObject::new(
        ErrorKind::Reaper,
        format!("REAPER failed at {call}"),
        Some(subject.to_string()),
        "check the element in REAPER and try again; report it if it keeps failing",
    )
}

/// REAPER's *File: Dry run render project, using the most recent render
/// settings* (found with `actions.find`, REAPER 7.67).
const DRY_RUN_RENDER: c_int = 43349;

/// The project render settings mort changes for a dry run, and restores.
const RENDER_SETTINGS: [&str; 6] = [
    "RENDER_BOUNDSFLAG",
    "RENDER_STARTPOS",
    "RENDER_ENDPOS",
    "RENDER_SETTINGS",
    "RENDER_TAILFLAG",
    "RENDER_ADDTOPROJ",
];

/// How far, in seconds, an event may lie outside its item and still be
/// written, allowing for rounding through REAPER's ticks.
const MIDI_SLACK: f64 = 1e-6;

/// The largest linear gain REAPER's faders allow: +12 dB.
const MAX_GAIN: f64 = 3.981_071_705_534_973;

fn check_gain(v: Option<f64>, subject: &impl ToString) -> Result<()> {
    match v {
        Some(v) if !(0.0..=MAX_GAIN).contains(&v) => Err(invalid(
            format!("volume {v} is outside 0 to +12 dB"),
            Some(subject.to_string()),
            "give a linear gain from 0 to 3.98",
        )),
        _ => Ok(()),
    }
}

fn check_pan(p: Option<f64>, subject: &impl ToString) -> Result<()> {
    match p {
        Some(p) if !(-1.0..=1.0).contains(&p) => Err(invalid(
            format!("pan {p} is outside −1 to 1"),
            Some(subject.to_string()),
            "give a pan from −1 (left) to 1 (right)",
        )),
        _ => Ok(()),
    }
}

/// Where an envelope lives, for finding or creating it.
#[derive(Clone, Copy)]
enum EnvLoc {
    /// A track envelope by chunk name, and the action that shows (creates) it.
    Chunk(&'static str, c_int),
    /// An FX parameter's envelope.
    Fx(c_int, c_int),
    /// A send's volume envelope.
    Send(c_int),
}

/// REAPER's actions *Track: Toggle track volume/pan envelope visible*.
const SHOW_VOLUME_ENVELOPE: c_int = 40406;
const SHOW_PAN_ENVELOPE: c_int = 40407;

/// REAPER's pan envelopes run the other way to the track's pan: +1 is left.
const PAN_ENVELOPE_SIGN: f64 = -1.0;

fn shape_to(s: Shape) -> c_int {
    match s {
        Shape::Linear => 0,
        Shape::Square => 1,
        Shape::Slow => 2,
        Shape::FastStart => 3,
        Shape::FastEnd => 4,
        Shape::Bezier => 5,
    }
}

fn shape_from(s: c_int) -> Shape {
    match s {
        1 => Shape::Square,
        2 => Shape::Slow,
        3 => Shape::FastStart,
        4 => Shape::FastEnd,
        5 => Shape::Bezier,
        _ => Shape::Linear,
    }
}

impl Live {
    fn locate(&self, target: EnvelopeTarget) -> Result<(*mut MediaTrack, EnvLoc)> {
        match target {
            EnvelopeTarget::Volume { track } => Ok((
                self.find_track(track)?,
                EnvLoc::Chunk("<VOLENV2", SHOW_VOLUME_ENVELOPE),
            )),
            EnvelopeTarget::Pan { track } => Ok((
                self.find_track(track)?,
                EnvLoc::Chunk("<PANENV2", SHOW_PAN_ENVELOPE),
            )),
            EnvelopeTarget::SendVolume { track, send } => {
                let t = self.find_track(track)?;
                let i = c_int::try_from(send)
                    .ok()
                    .filter(|i| *i < raw::send_count(self.low(), t))
                    .ok_or_else(|| {
                        not_found(
                            "send",
                            format!("{track} send {send}"),
                            "list sends with `mort tracks`",
                        )
                    })?;
                Ok((t, EnvLoc::Send(i)))
            }
            EnvelopeTarget::Param { fx, index } => {
                let (t, i) = self.find_fx(fx)?;
                let p = self.param_index(t, i, fx, index)?;
                Ok((t, EnvLoc::Fx(i, p)))
            }
        }
    }

    fn existing(&self, t: *mut MediaTrack, loc: EnvLoc) -> Option<*mut TrackEnvelope> {
        let r = self.low();
        match loc {
            // REAPER hands out a volume or pan envelope by chunk name even
            // when the track has none; it exists only if the track lists it.
            EnvLoc::Chunk(name, _) => {
                raw::envelope_by_chunk_name(r, t, name).filter(|e| raw::envelopes(r, t).contains(e))
            }
            EnvLoc::Fx(fx, p) => raw::fx_envelope(r, t, fx, p, false),
            EnvLoc::Send(i) => {
                raw::send_envelope(r, t, i).filter(|e| raw::envelopes(r, t).contains(e))
            }
        }
    }

    /// What an envelope on track `t` controls, if it is one mort handles.
    fn identify(&self, t: *mut MediaTrack, e: *mut TrackEnvelope) -> Option<EnvelopeTarget> {
        let r = self.low();
        let track = TrackId(raw::track_guid(r, t));
        if let Some((fx, p)) = raw::envelope_fx_param(r, e) {
            return Some(EnvelopeTarget::Param {
                fx: FxId(raw::fx_guid(r, t, fx)?),
                index: u32::try_from(p).ok()?,
            });
        }
        if raw::envelope_by_chunk_name(r, t, "<VOLENV2") == Some(e) {
            return Some(EnvelopeTarget::Volume { track });
        }
        if raw::envelope_by_chunk_name(r, t, "<PANENV2") == Some(e) {
            return Some(EnvelopeTarget::Pan { track });
        }
        (0..raw::send_count(r, t))
            .find(|i| raw::send_envelope(r, t, *i) == Some(e))
            .and_then(|i| usize::try_from(i).ok())
            .map(|send| EnvelopeTarget::SendVolume { track, send })
    }

    fn find_envelope(
        &self,
        id: EnvelopeId,
    ) -> Result<(*mut MediaTrack, *mut TrackEnvelope, EnvelopeTarget)> {
        let r = self.low();
        self.all_tracks()
            .into_iter()
            .find_map(|t| {
                raw::envelopes(r, t)
                    .into_iter()
                    .find(|e| raw::envelope_guid(r, *e) == Some(id.0))
                    .and_then(|e| Some((t, e, self.identify(t, e)?)))
            })
            .ok_or_else(|| {
                not_found(
                    "envelope",
                    id,
                    "list envelopes with `mort automate <track>`",
                )
            })
    }

    fn param_range_of(&self, t: *mut MediaTrack, target: EnvelopeTarget) -> (f64, f64) {
        match target {
            EnvelopeTarget::Param { fx, index } => self
                .find_fx(fx)
                .ok()
                .and_then(|(_, i)| {
                    Some(raw::param_range(
                        self.low(),
                        t,
                        i,
                        c_int::try_from(index).ok()?,
                    ))
                })
                .unwrap_or((0.0, 1.0)),
            _ => (0.0, 1.0),
        }
    }

    fn value_to_raw(
        &self,
        t: *mut MediaTrack,
        e: *mut TrackEnvelope,
        target: EnvelopeTarget,
        v: f64,
    ) -> f64 {
        match target {
            EnvelopeTarget::Volume { .. } | EnvelopeTarget::SendVolume { .. } => self
                .low()
                .ScaleToEnvelopeMode(raw::envelope_scaling(self.low(), e), v),
            EnvelopeTarget::Pan { .. } => PAN_ENVELOPE_SIGN * v,
            EnvelopeTarget::Param { .. } => {
                let (lo, hi) = self.param_range_of(t, target);
                lo + v * (hi - lo)
            }
        }
    }

    fn raw_to_value(
        &self,
        t: *mut MediaTrack,
        e: *mut TrackEnvelope,
        target: EnvelopeTarget,
        v: f64,
    ) -> f64 {
        match target {
            EnvelopeTarget::Volume { .. } | EnvelopeTarget::SendVolume { .. } => self
                .low()
                .ScaleFromEnvelopeMode(raw::envelope_scaling(self.low(), e), v),
            EnvelopeTarget::Pan { .. } => PAN_ENVELOPE_SIGN * v,
            EnvelopeTarget::Param { .. } => {
                let (lo, hi) = self.param_range_of(t, target);
                if hi > lo { (v - lo) / (hi - lo) } else { v }
            }
        }
    }
}

impl TempoMap for Live {
    fn bar_start_qn(&self, bar: u32) -> f64 {
        let i = i32::try_from(bar.saturating_sub(1)).unwrap_or(i32::MAX);
        self.reaper
            .time_map_get_measure_info(PROJ, i)
            .start_qn
            .into_inner()
    }

    fn time_sig(&self, bar: u32) -> (u32, u32) {
        let i = i32::try_from(bar.saturating_sub(1)).unwrap_or(i32::MAX);
        let ts = self
            .reaper
            .time_map_get_measure_info(PROJ, i)
            .time_signature;
        (ts.numerator.get(), ts.denominator.get())
    }

    fn bar_at_qn(&self, qn: f64) -> u32 {
        let r = self
            .reaper
            .time_map_qn_to_measures(PROJ, PositionInQuarterNotes::new_panic(qn));
        // Despite reaper-medium's field name, REAPER returns the bar number
        // counted from 1 (checked against REAPER 7.67 by scripts/smoke.sh).
        u32::try_from(r.measure_index).map_or(1, |m| m.max(1))
    }

    fn qn_to_seconds(&self, qn: f64) -> f64 {
        self.reaper
            .time_map_2_qn_to_time(PROJ, PositionInQuarterNotes::new_panic(qn))
            .into_inner()
    }

    fn seconds_to_qn(&self, seconds: f64) -> f64 {
        self.reaper
            .time_map_2_time_to_qn(PROJ, PositionInSeconds::new_panic(seconds))
            .into_inner()
    }
}

impl Session for Live {
    fn project(&self) -> ProjectInfo {
        let mut buf = [0 as std::os::raw::c_char; 1024];
        // SAFETY: a null project means the current one; the buffer and its
        // size match.
        let (name, length, rate) = unsafe {
            self.low()
                .GetProjectName(std::ptr::null_mut(), buf.as_mut_ptr(), 1024);
            (
                CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned(),
                self.low().GetProjectLength(std::ptr::null_mut()),
                self.low().GetSetProjectInfo(
                    std::ptr::null_mut(),
                    c"PROJECT_SRATE".as_ptr(),
                    0.0,
                    false,
                ),
            )
        };
        let name = name
            .strip_suffix(".rpp")
            .or_else(|| name.strip_suffix(".RPP"))
            .unwrap_or(&name);
        ProjectInfo {
            name: if name.is_empty() {
                "untitled".into()
            } else {
                name.into()
            },
            path: raw::project_path(self.low()),
            sample_rate: rate,
            length,
            // SAFETY: a null project means the current one.
            dirty: unsafe { self.low().IsProjectDirty(std::ptr::null_mut()) } != 0,
        }
    }

    fn state(&self) -> u64 {
        u64::from(self.reaper.get_project_state_change_count(PROJ))
    }

    fn tempo_map(&self) -> &dyn TempoMap {
        self
    }

    fn regions(&self) -> Vec<Region> {
        (0..)
            .map_while(|i| raw::marker(self.low(), i))
            .map(|(is_region, number, name, start, end)| Region {
                number: u32::try_from(number).unwrap_or(0),
                name,
                start,
                end: is_region.then_some(end),
            })
            .collect()
    }

    fn add_region(&mut self, name: &str, start: f64, end: Option<f64>) -> Result<u32> {
        if end.is_some_and(|e| e <= start) {
            return Err(mort_session::invalid(
                format!("region {start}..{} is empty", end.unwrap_or(start)),
                Some(name.to_owned()),
                "give an end after the start",
            ));
        }
        let n = raw::add_marker(self.low(), name, start, end);
        u32::try_from(n).map_err(|_| reaper_failed("AddProjectMarker2", &name))
    }

    fn master(&self) -> TrackId {
        TrackId(raw::track_guid(self.low(), raw::master(self.low())))
    }

    fn tracks(&self) -> Vec<TrackInfo> {
        raw::tracks(self.low())
            .into_iter()
            .map(|t| self.track_info(t))
            .collect()
    }

    fn track(&self, id: TrackId) -> Result<TrackInfo> {
        self.find_track(id).map(|t| self.track_info(t))
    }

    fn add_track(&mut self, spec: NewTrack) -> Result<TrackId> {
        let r = self.low();
        let count = c_int::try_from(raw::tracks(r).len()).unwrap_or(c_int::MAX);
        let t =
            match spec.parent {
                None => {
                    let idx =
                        match spec.index {
                            None => count,
                            Some(i) => c_int::try_from(i).ok().filter(|i| *i <= count).ok_or_else(
                                || {
                                    invalid(
                                        format!("track index {i} is past the end ({count})"),
                                        None,
                                        "leave the index out to add at the end",
                                    )
                                },
                            )?,
                        };
                    let t = raw::insert_track(r, idx);
                    // A track inserted inside a folder's range joins the folder;
                    // at the top level it must not close anything.
                    if raw::track_value(r, t, "I_FOLDERDEPTH") != 0.0 {
                        raw::set_track_value(r, t, "I_FOLDERDEPTH", 0.0);
                    }
                    t
                }
                Some(parent) => {
                    if spec.index.is_some() {
                        return Err(invalid(
                            "a track added to a folder goes at the folder's end",
                            None,
                            "give a parent or an index, not both",
                        ));
                    }
                    let p = self.find_track(parent)?;
                    if p == raw::master(r) {
                        return Err(invalid(
                            "the master is not a folder",
                            Some(parent.to_string()),
                            "leave the parent out to add a top-level track",
                        ));
                    }
                    self.insert_in_folder(p)
                }
            };
        raw::set_track_string(r, t, "P_NAME", &spec.name);
        Ok(TrackId(raw::track_guid(r, t)))
    }

    fn remove_track(&mut self, id: TrackId) -> Result<()> {
        let t = self.find_track(id)?;
        if t == raw::master(self.low()) {
            return Err(invalid(
                "the master cannot be removed",
                Some(id.to_string()),
                "remove other tracks instead",
            ));
        }
        self.unlink_from_folders(t);
        raw::delete_track(self.low(), t);
        Ok(())
    }

    fn set_track(&mut self, id: TrackId, change: &TrackChange) -> Result<()> {
        let t = self.find_track(id)?;
        check_gain(change.volume, &id)?;
        check_pan(change.pan, &id)?;
        let r = self.low();
        if let Some(n) = &change.name {
            raw::set_track_string(r, t, "P_NAME", n);
        }
        if let Some(v) = change.volume {
            raw::set_track_value(r, t, "D_VOL", v);
        }
        if let Some(p) = change.pan {
            raw::set_track_value(r, t, "D_PAN", p);
        }
        if let Some(m) = change.mute {
            raw::set_track_value(r, t, "B_MUTE", f64::from(u8::from(m)));
        }
        if let Some(on) = change.solo {
            raw::set_track_value(r, t, "I_SOLO", if on { 2.0 } else { 0.0 });
        }
        Ok(())
    }

    fn add_send(&mut self, from: TrackId, to: TrackId) -> Result<usize> {
        let (f, t) = (self.find_track(from)?, self.find_track(to)?);
        if f == t {
            return Err(invalid(
                "a track cannot send to itself",
                Some(from.to_string()),
                "choose another destination",
            ));
        }
        let i = raw::create_send(self.low(), f, t);
        usize::try_from(i).map_err(|_| reaper_failed("CreateTrackSend", &from))
    }

    fn set_send(&mut self, from: TrackId, send: usize, change: &SendChange) -> Result<()> {
        let t = self.find_track(from)?;
        let r = self.low();
        let i = c_int::try_from(send)
            .ok()
            .filter(|i| *i < raw::send_count(r, t))
            .ok_or_else(|| {
                not_found(
                    "send",
                    format!("{from} send {send}"),
                    "list sends with `mort tracks`",
                )
            })?;
        check_gain(change.volume, &from)?;
        check_pan(change.pan, &from)?;
        if let Some(v) = change.volume {
            raw::set_send_value(r, t, i, "D_VOL", v);
        }
        if let Some(p) = change.pan {
            raw::set_send_value(r, t, i, "D_PAN", p);
        }
        if let Some(m) = change.mute {
            raw::set_send_value(r, t, i, "B_MUTE", f64::from(u8::from(m)));
        }
        Ok(())
    }

    fn track_chunk(&self, id: TrackId) -> Result<String> {
        let t = self.find_track(id)?;
        raw::track_chunk(self.low(), t).ok_or_else(|| reaper_failed("GetTrackStateChunk", &id))
    }

    fn audio_chunk(&self, id: TrackId) -> Result<String> {
        Ok(crate::chunk::audio_chunk(&self.track_chunk(id)?))
    }

    fn add_fx(&mut self, track: TrackId, plugin: &str, index: Option<usize>) -> Result<FxId> {
        let t = self.find_track(track)?;
        let r = self.low();
        let len = raw::fx_count(r, t);
        let index =
            match index {
                None => None,
                Some(i) => Some(c_int::try_from(i).ok().filter(|i| *i <= len).ok_or_else(
                    || {
                        invalid(
                            format!("FX index {i} is past the end of the chain ({len})"),
                            Some(track.to_string()),
                            "leave the index out to add at the end",
                        )
                    },
                )?),
            };
        let i = raw::fx_add(r, t, plugin, index).ok_or_else(|| {
            not_found(
                "plugin",
                plugin,
                "use the name REAPER's FX browser shows, such as `ReaEQ` or `CLAP: mort probe`",
            )
        })?;
        raw::fx_guid(r, t, i)
            .map(FxId)
            .ok_or_else(|| reaper_failed("TrackFX_GetFXGUID", &track))
    }

    fn remove_fx(&mut self, id: FxId) -> Result<()> {
        let (t, i) = self.find_fx(id)?;
        if raw::fx_delete(self.low(), t, i) {
            Ok(())
        } else {
            Err(reaper_failed("TrackFX_Delete", &id))
        }
    }

    fn move_fx(&mut self, id: FxId, index: usize) -> Result<()> {
        let (t, i) = self.find_fx(id)?;
        let len = raw::fx_count(self.low(), t);
        let to = c_int::try_from(index)
            .ok()
            .filter(|to| *to < len)
            .ok_or_else(|| {
                invalid(
                    format!("FX index {index} is past the end of the chain ({len})"),
                    Some(id.to_string()),
                    "give an index within the chain",
                )
            })?;
        if to != i {
            raw::fx_move(self.low(), t, i, to);
        }
        Ok(())
    }

    fn set_fx_enabled(&mut self, id: FxId, enabled: bool) -> Result<()> {
        let (t, i) = self.find_fx(id)?;
        raw::fx_set_enabled(self.low(), t, i, enabled);
        Ok(())
    }

    fn fx(&self, id: FxId) -> Result<(TrackId, FxInfo)> {
        let (t, i) = self.find_fx(id)?;
        Ok((TrackId(raw::track_guid(self.low(), t)), self.fx_info(t, i)))
    }

    fn params(&self, id: FxId) -> Result<Vec<ParamInfo>> {
        let (t, fx) = self.find_fx(id)?;
        let r = self.low();
        Ok((0..raw::param_count(r, t, fx))
            .map(|p| ParamInfo {
                index: u32::try_from(p).unwrap_or(0),
                name: raw::param_name(r, t, fx, p),
                value: raw::param_value(r, t, fx, p),
                display: raw::param_display(r, t, fx, p),
                steps: raw::param_is_toggle(r, t, fx, p).then_some(2),
            })
            .collect())
    }

    fn set_param(&mut self, id: FxId, index: u32, value: f64) -> Result<()> {
        let (t, fx) = self.find_fx(id)?;
        let p = self.param_index(t, fx, id, index)?;
        if !(0.0..=1.0).contains(&value) {
            return Err(invalid(
                format!("parameter value {value} is outside 0–1"),
                Some(format!("{id} #{index}")),
                "give a normalised value from 0 to 1",
            ));
        }
        if raw::param_set(self.low(), t, fx, p, value) {
            Ok(())
        } else {
            Err(reaper_failed("TrackFX_SetParamNormalized", &id))
        }
    }

    fn format_param(&self, id: FxId, index: u32, value: f64) -> Result<Option<String>> {
        let (t, fx) = self.find_fx(id)?;
        let p = self.param_index(t, fx, id, index)?;
        Ok(raw::param_format(
            self.low(),
            t,
            fx,
            p,
            value.clamp(0.0, 1.0),
        ))
    }

    fn fx_state(&self, id: FxId) -> Result<String> {
        let (t, _) = self.find_fx(id)?;
        let tid = TrackId(raw::track_guid(self.low(), t));
        let c = raw::track_chunk(self.low(), t)
            .ok_or_else(|| reaper_failed("GetTrackStateChunk", &tid))?;
        chunk::fx_state(&c, &id.to_string())
            .ok_or_else(|| reaper_failed("finding the FX in its track chunk", &id))
    }

    fn set_fx_state(&mut self, id: FxId, state: &str) -> Result<()> {
        let (t, _) = self.find_fx(id)?;
        let tid = TrackId(raw::track_guid(self.low(), t));
        let c = raw::track_chunk(self.low(), t)
            .ok_or_else(|| reaper_failed("GetTrackStateChunk", &tid))?;
        let new = chunk::replace_fx_state(&c, &id.to_string(), state).map_err(|e| match e {
            chunk::ReplaceError::NotFound => {
                reaper_failed("finding the FX in its track chunk", &id)
            }
            chunk::ReplaceError::WrongKind { have, given } => invalid(
                format!("the state is a `{given}` block but the FX is `{have}`"),
                Some(id.to_string()),
                "pass state read from this FX with `fx.state`",
            ),
        })?;
        if raw::set_track_chunk(self.low(), t, &new) {
            Ok(())
        } else {
            Err(invalid(
                "REAPER rejected the state",
                Some(id.to_string()),
                "pass state read from this FX with `fx.state`",
            ))
        }
    }

    fn load_preset(&mut self, id: FxId, preset: &str) -> Result<()> {
        let (t, i) = self.find_fx(id)?;
        if raw::fx_set_preset(self.low(), t, i, preset) {
            Ok(())
        } else {
            Err(not_found(
                "preset",
                preset,
                "use a name from the preset menu in the FX's window",
            ))
        }
    }

    fn envelope(&mut self, target: EnvelopeTarget, create: bool) -> Result<Option<EnvelopeInfo>> {
        let (t, loc) = self.locate(target)?;
        let r = self.low();
        let mut env = self.existing(t, loc);
        if env.is_none() && create {
            env = match loc {
                EnvLoc::Chunk(_, command) => {
                    raw::action_on_track(r, t, command);
                    self.existing(t, loc)
                }
                EnvLoc::Fx(fx, p) => raw::fx_envelope(r, t, fx, p, true),
                EnvLoc::Send(_) => {
                    return Err(invalid(
                        "mort cannot create send envelopes yet",
                        Some(format!("{target:?}")),
                        "show the send's volume envelope once in REAPER (the send's button, then Volume envelope); mort can then write it",
                    ));
                }
            };
            if env.is_none() {
                return Err(reaper_failed(
                    "creating the envelope",
                    &format!("{target:?}"),
                ));
            }
        }
        Ok(env
            .and_then(|e| raw::envelope_guid(r, e))
            .map(|g| EnvelopeInfo {
                id: EnvelopeId(g),
                target,
            }))
    }

    fn format_envelope_value(&self, id: EnvelopeId, value: f64) -> Result<Option<String>> {
        let (t, e, target) = self.find_envelope(id)?;
        let s = raw::envelope_format(self.low(), e, self.value_to_raw(t, e, target, value));
        Ok((!s.is_empty()).then_some(s))
    }

    fn envelopes(&self, track: TrackId) -> Result<Vec<EnvelopeInfo>> {
        let t = self.find_track(track)?;
        let r = self.low();
        Ok(raw::envelopes(r, t)
            .into_iter()
            .filter_map(|e| {
                Some(EnvelopeInfo {
                    id: EnvelopeId(raw::envelope_guid(r, e)?),
                    target: self.identify(t, e)?,
                })
            })
            .collect())
    }

    fn envelope_points(&self, id: EnvelopeId, start: f64, end: f64) -> Result<Vec<EnvelopePoint>> {
        let (t, e, target) = self.find_envelope(id)?;
        Ok(raw::envelope_points(self.low(), e)
            .into_iter()
            .filter(|(time, ..)| *time >= start && *time < end)
            .map(|(time, v, shape, tension)| EnvelopePoint {
                time,
                value: self.raw_to_value(t, e, target, v),
                shape: shape_from(shape),
                tension,
            })
            .collect())
    }

    fn insert_envelope_points(&mut self, id: EnvelopeId, points: &[EnvelopePoint]) -> Result<()> {
        let (t, e, target) = self.find_envelope(id)?;
        let (lo, hi) = match target {
            EnvelopeTarget::Volume { .. } | EnvelopeTarget::SendVolume { .. } => (0.0, MAX_GAIN),
            EnvelopeTarget::Pan { .. } => (-1.0, 1.0),
            EnvelopeTarget::Param { .. } => (0.0, 1.0),
        };
        if let Some(p) = points.iter().find(|p| !(lo..=hi).contains(&p.value)) {
            return Err(invalid(
                format!("value {} at {}s is outside {lo} to {hi}", p.value, p.time),
                Some(id.to_string()),
                "give values in the envelope's range",
            ));
        }
        let raw_points: Vec<_> = points
            .iter()
            .map(|p| {
                (
                    p.time,
                    self.value_to_raw(t, e, target, p.value),
                    shape_to(p.shape),
                    p.tension,
                )
            })
            .collect();
        raw::envelope_insert(self.low(), e, &raw_points);
        Ok(())
    }

    fn delete_envelope_points(&mut self, id: EnvelopeId, start: f64, end: f64) -> Result<()> {
        let (_, e, _) = self.find_envelope(id)?;
        raw::envelope_delete(self.low(), e, start, end);
        Ok(())
    }

    fn items(&self, id: TrackId) -> Result<Vec<ItemInfo>> {
        let t = self.find_track(id)?;
        let r = self.low();
        let mut items: Vec<ItemInfo> = raw::items(r, t)
            .into_iter()
            .filter_map(|i| {
                let take = raw::active_take(r, i);
                Some(ItemInfo {
                    id: ItemId(raw::item_guid(r, i)?),
                    take: take.and_then(|k| raw::take_guid(r, k)).map(TakeId),
                    position: raw::item_value(r, i, "D_POSITION"),
                    length: raw::item_value(r, i, "D_LENGTH"),
                    midi: take.is_some_and(|k| raw::take_is_midi(r, k)),
                })
            })
            .collect();
        items.sort_by(|a, b| a.position.total_cmp(&b.position));
        Ok(items)
    }

    fn add_midi_item(&mut self, track: TrackId, start: f64, end: f64) -> Result<ItemId> {
        if end <= start {
            return Err(invalid(
                format!("range {start}..{end} is empty"),
                None,
                "give an end after the start",
            ));
        }
        let t = self.find_track(track)?;
        let r = self.low();
        let i = raw::create_midi_item(r, t, start, end)
            .ok_or_else(|| reaper_failed("CreateNewMIDIItemInProj", &track))?;
        raw::update_item(r, i);
        raw::item_guid(r, i)
            .map(ItemId)
            .ok_or_else(|| reaper_failed("reading the new item's GUID", &track))
    }

    fn notes(&self, item: ItemId) -> Result<Vec<Note>> {
        let (_, k) = self.find_midi(item)?;
        let r = self.low();
        Ok(raw::midi_notes(r, k)
            .into_iter()
            .map(|(start, end, chan, pitch, vel)| Note {
                pitch: u8::try_from(pitch).unwrap_or(0),
                start: raw::seconds_at(r, k, start),
                end: raw::seconds_at(r, k, end),
                velocity: u8::try_from(vel).unwrap_or(0),
                channel: u8::try_from(chan).unwrap_or(0),
            })
            .collect())
    }

    fn control_changes(&self, item: ItemId) -> Result<Vec<ControlChange>> {
        let (_, k) = self.find_midi(item)?;
        let r = self.low();
        Ok(raw::midi_ccs(r, k)
            .into_iter()
            .map(|(_, (ppq, chan, num, val))| ControlChange {
                time: raw::seconds_at(r, k, ppq),
                controller: u8::try_from(num).unwrap_or(0),
                value: u8::try_from(val).unwrap_or(0),
                channel: u8::try_from(chan).unwrap_or(0),
            })
            .collect())
    }

    fn insert_midi(&mut self, item: ItemId, notes: &[Note], ccs: &[ControlChange]) -> Result<()> {
        let (i, k) = self.find_midi(item)?;
        let r = self.low();
        let lo = raw::item_value(r, i, "D_POSITION");
        let hi = lo + raw::item_value(r, i, "D_LENGTH");
        let outside = notes
            .iter()
            .map(|n| (n.start, n.end))
            .chain(ccs.iter().map(|c| (c.time, c.time)))
            .find(|&(s, e)| s < lo - MIDI_SLACK || e > hi + MIDI_SLACK || e < s);
        if let Some((s, e)) = outside {
            return Err(invalid(
                format!(
                    "an event at {s}–{e}s lies outside the item ({lo}–{hi}s) or ends before it starts"
                ),
                Some(item.to_string()),
                "lengthen the item or move the event",
            ));
        }
        if let Some(n) = notes
            .iter()
            .find(|n| n.pitch > 127 || n.velocity == 0 || n.velocity > 127 || n.channel > 15)
        {
            return Err(invalid(
                format!(
                    "note pitch {}, velocity {}, channel {} is out of range",
                    n.pitch, n.velocity, n.channel
                ),
                Some(item.to_string()),
                "pitch 0–127, velocity 1–127, channel 0–15",
            ));
        }
        let raw_notes: Vec<raw::RawNote> = notes
            .iter()
            .map(|n| {
                (
                    raw::ppq_at(r, k, n.start),
                    raw::ppq_at(r, k, n.end),
                    c_int::from(n.channel),
                    c_int::from(n.pitch),
                    c_int::from(n.velocity),
                )
            })
            .collect();
        let raw_ccs: Vec<raw::RawCc> = ccs
            .iter()
            .map(|c| {
                (
                    raw::ppq_at(r, k, c.time),
                    c_int::from(c.channel),
                    c_int::from(c.controller),
                    c_int::from(c.value),
                )
            })
            .collect();
        raw::midi_insert(r, k, &raw_notes, &raw_ccs);
        raw::update_item(r, i);
        Ok(())
    }

    fn delete_midi(&mut self, item: ItemId, start: f64, end: f64) -> Result<()> {
        let (i, k) = self.find_midi(item)?;
        let r = self.low();
        let notes: Vec<c_int> = raw::midi_notes(r, k)
            .iter()
            .zip(0..)
            .filter(|((s, ..), _)| {
                let t = raw::seconds_at(r, k, *s);
                t >= start && t < end
            })
            .map(|(_, idx)| idx)
            .collect();
        raw::midi_delete_notes(r, k, notes);
        let ccs: Vec<c_int> = raw::midi_ccs(r, k)
            .into_iter()
            .filter(|(_, (ppq, ..))| {
                let t = raw::seconds_at(r, k, *ppq);
                t >= start && t < end
            })
            .map(|(idx, _)| idx)
            .collect();
        raw::midi_delete_ccs(r, k, ccs);
        raw::update_item(r, i);
        Ok(())
    }

    fn find_actions(&self, text: &str, limit: usize) -> Vec<(i32, String)> {
        raw::find_actions(self.low(), text, limit)
    }

    fn save_project(&mut self, path: Option<&str>) -> Result<()> {
        if path.is_none() && raw::project_path(self.low()).is_none() {
            return Err(invalid(
                "the project has never been saved",
                None,
                "give a path to save it to",
            ));
        }
        if path.is_some_and(|p| p.contains('\0')) {
            return Err(invalid(
                "the path contains a NUL byte",
                None,
                "give a plain path",
            ));
        }
        raw::save_project(self.low(), path);
        Ok(())
    }

    fn begin_undo(&mut self) {
        self.reaper.undo_begin_block_2(PROJ);
    }

    fn end_undo(&mut self, label: &str) {
        self.reaper.undo_end_block_2(PROJ, label, UndoScope::All);
    }

    fn undo_label(&self) -> Option<String> {
        self.reaper.undo_can_undo_2(PROJ, |s| s.to_str().to_owned())
    }

    fn redo_label(&self) -> Option<String> {
        raw::redo_label(self.low())
    }

    fn undo(&mut self) -> Option<String> {
        let label = self.undo_label()?;
        self.reaper.undo_do_undo_2(PROJ).then_some(label)
    }

    fn redo(&mut self) -> Option<String> {
        let label = self
            .reaper
            .undo_can_redo_2(PROJ, |s| s.to_str().to_owned())?;
        self.reaper.undo_do_redo_2(PROJ).then_some(label)
    }

    fn play(&mut self, start: f64, end: f64) -> Result<()> {
        if end <= start {
            return Err(invalid(
                format!("range {start}..{end} is empty"),
                None,
                "give an end after the start",
            ));
        }
        let r = self.low();
        self.reaper.on_stop_button_ex(PROJ);
        raw::set_cursor(r, start);
        raw::play(r);
        Ok(())
    }

    fn render(&mut self, start: f64, end: f64) -> Result<()> {
        if end <= start {
            return Err(invalid(
                format!("range {start}..{end} is empty"),
                None,
                "give an end after the start",
            ));
        }
        let r = self.low();
        // The project's render settings are borrowed for the dry run and
        // put back afterwards.
        let saved: Vec<(&str, f64)> = RENDER_SETTINGS
            .iter()
            .map(|k| (*k, raw::project_info(r, k)))
            .collect();
        raw::set_project_info(r, "RENDER_BOUNDSFLAG", 0.0); // custom time bounds
        raw::set_project_info(r, "RENDER_STARTPOS", start);
        raw::set_project_info(r, "RENDER_ENDPOS", end);
        raw::set_project_info(r, "RENDER_SETTINGS", 0.0); // master mix
        raw::set_project_info(r, "RENDER_TAILFLAG", 0.0); // no tail
        raw::set_project_info(r, "RENDER_ADDTOPROJ", 0.0);
        raw::run_action(r, DRY_RUN_RENDER);
        for (k, v) in saved {
            raw::set_project_info(r, k, v);
        }
        Ok(())
    }

    fn stop(&mut self) {
        self.reaper.on_stop_button_ex(PROJ);
    }

    fn is_playing(&self) -> bool {
        self.reaper.get_play_state_ex(PROJ).is_playing
    }

    fn position(&self) -> f64 {
        raw::play_position(self.low())
    }

    fn play_rate(&self) -> f64 {
        raw::play_rate(self.low())
    }

    fn monitored(&self) -> Vec<TrackId> {
        // I_RECMON: 0 off, 1 on, 2 only when not playing (tape style).
        let r = self.low();
        raw::tracks(r)
            .into_iter()
            .filter(|t| {
                raw::track_value(r, *t, "I_RECARM") > 0.5
                    && (raw::track_value(r, *t, "I_RECMON") - 1.0).abs() < 0.5
            })
            .map(|t| TrackId(raw::track_guid(r, t)))
            .collect()
    }

    fn begin_run(&mut self) -> u64 {
        crate::runs::begin()
    }

    fn end_run(&mut self, run: u64) {
        crate::runs::end(run);
    }

    fn take_frames(&mut self, run: u64) -> std::collections::HashMap<u32, Vec<mort_link::Record>> {
        crate::probes::take(run)
    }
}
