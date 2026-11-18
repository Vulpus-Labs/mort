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
use reaper_medium::{
    PositionInQuarterNotes, PositionInSeconds, ProjectContext, Reaper, ReaperSession, UndoScope,
};

use mort_proto::model::SendInfo;
use mort_proto::model::{
    ControlChange, EnvelopeInfo, EnvelopePoint, EnvelopeTarget, FxInfo, ItemInfo, Note, ParamInfo,
    ProjectInfo, Region, TrackInfo,
};
use mort_proto::{
    EnvelopeId, ErrorKind, ErrorObject, FxId, Guid, ItemId, TakeId, TempoMap, TrackId,
};
use mort_session::{NewTrack, Result, SendChange, Session, TrackChange, not_found};
use reaper_low::raw::MediaTrack;
use std::os::raw::c_int;

use crate::raw;

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
    let (tx, rx) = mpsc::channel();
    let server = Server::start(&socket_dir(), &std::process::id().to_string(), tx)?;
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
                d
            },
            _server: server,
        });
    });
    Ok(())
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

fn later(ticket: &str) -> ErrorObject {
    ErrorObject::new(
        ErrorKind::Reaper,
        "the bridge does not do this yet",
        None,
        format!("ticket {ticket} adds it"),
    )
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

    fn add_track(&mut self, _: NewTrack) -> Result<TrackId> {
        Err(later("0006"))
    }

    fn remove_track(&mut self, _: TrackId) -> Result<()> {
        Err(later("0006"))
    }

    fn set_track(&mut self, _: TrackId, _: &TrackChange) -> Result<()> {
        Err(later("0006"))
    }

    fn add_send(&mut self, _: TrackId, _: TrackId) -> Result<usize> {
        Err(later("0006"))
    }

    fn set_send(&mut self, _: TrackId, _: usize, _: &SendChange) -> Result<()> {
        Err(later("0006"))
    }

    fn track_chunk(&self, _: TrackId) -> Result<String> {
        Err(later("0012"))
    }

    fn add_fx(&mut self, _: TrackId, _: &str, _: Option<usize>) -> Result<FxId> {
        Err(later("0006"))
    }

    fn remove_fx(&mut self, _: FxId) -> Result<()> {
        Err(later("0006"))
    }

    fn move_fx(&mut self, _: FxId, _: usize) -> Result<()> {
        Err(later("0006"))
    }

    fn set_fx_enabled(&mut self, _: FxId, _: bool) -> Result<()> {
        Err(later("0006"))
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

    fn set_param(&mut self, _: FxId, _: u32, _: f64) -> Result<()> {
        Err(later("0006"))
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

    fn fx_state(&self, _: FxId) -> Result<String> {
        Err(later("0006"))
    }

    fn set_fx_state(&mut self, _: FxId, _: &str) -> Result<()> {
        Err(later("0006"))
    }

    fn load_preset(&mut self, _: FxId, _: &str) -> Result<()> {
        Err(later("0006"))
    }

    fn envelope(&mut self, _: EnvelopeTarget, _: bool) -> Result<Option<EnvelopeInfo>> {
        Err(later("0007"))
    }

    fn envelopes(&self, _: TrackId) -> Result<Vec<EnvelopeInfo>> {
        Err(later("0007"))
    }

    fn envelope_points(&self, _: EnvelopeId, _: f64, _: f64) -> Result<Vec<EnvelopePoint>> {
        Err(later("0007"))
    }

    fn insert_envelope_points(&mut self, _: EnvelopeId, _: &[EnvelopePoint]) -> Result<()> {
        Err(later("0007"))
    }

    fn delete_envelope_points(&mut self, _: EnvelopeId, _: f64, _: f64) -> Result<()> {
        Err(later("0007"))
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

    fn add_midi_item(&mut self, _: TrackId, _: f64, _: f64) -> Result<ItemId> {
        Err(later("0008"))
    }

    fn notes(&self, _: ItemId) -> Result<Vec<Note>> {
        Err(later("0008"))
    }

    fn control_changes(&self, _: ItemId) -> Result<Vec<ControlChange>> {
        Err(later("0008"))
    }

    fn insert_midi(&mut self, _: ItemId, _: &[Note], _: &[ControlChange]) -> Result<()> {
        Err(later("0008"))
    }

    fn delete_midi(&mut self, _: ItemId, _: f64, _: f64) -> Result<()> {
        Err(later("0008"))
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

    fn play(&mut self, _: f64, _: f64) -> Result<()> {
        Err(later("0009"))
    }

    fn stop(&mut self) {
        self.reaper.on_stop_button_ex(PROJ);
    }

    fn is_playing(&self) -> bool {
        self.reaper.get_play_state_ex(PROJ).is_playing
    }

    fn position(&self) -> f64 {
        self.reaper.get_play_position_ex(PROJ).into_inner()
    }
}
