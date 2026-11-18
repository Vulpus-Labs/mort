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

use mort_proto::model::{
    ControlChange, EnvelopeInfo, EnvelopePoint, EnvelopeTarget, FxInfo, ItemInfo, Note, ParamInfo,
    ProjectInfo, Region, TrackInfo,
};
use mort_proto::{EnvelopeId, ErrorKind, ErrorObject, FxId, Guid, ItemId, TempoMap, TrackId};
use mort_session::{NewTrack, Result, SendChange, Session, TrackChange};

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
            dispatcher: Dispatcher::new(rx),
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
        u32::try_from(r.measure_index).map_or(1, |m| m + 1)
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
            path: None,
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
        Vec::new()
    }

    fn master(&self) -> TrackId {
        TrackId(Guid(0))
    }

    fn tracks(&self) -> Vec<TrackInfo> {
        Vec::new()
    }

    fn track(&self, _: TrackId) -> Result<TrackInfo> {
        Err(later("0005"))
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

    fn fx(&self, _: FxId) -> Result<(TrackId, FxInfo)> {
        Err(later("0005"))
    }

    fn params(&self, _: FxId) -> Result<Vec<ParamInfo>> {
        Err(later("0005"))
    }

    fn set_param(&mut self, _: FxId, _: u32, _: f64) -> Result<()> {
        Err(later("0006"))
    }

    fn format_param(&self, _: FxId, _: u32, _: f64) -> Result<Option<String>> {
        Err(later("0005"))
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

    fn items(&self, _: TrackId) -> Result<Vec<ItemInfo>> {
        Err(later("0005"))
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
