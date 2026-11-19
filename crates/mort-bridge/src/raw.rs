//! Thin wrappers over REAPER's C API for reading the project.
//!
//! Every function takes the low-level [`Reaper`] and pointers REAPER gave
//! us on this tick. The project is always the current one (a null project
//! pointer). Callers must be on the main thread.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr::null_mut;

use reaper_low::Reaper;
use reaper_low::raw::{GUID, MediaItem, MediaItem_Take, MediaTrack, TrackEnvelope};

use mort_proto::Guid;

/// Converts REAPER's GUID struct to ours. The text form
/// `{Data1-Data2-Data3-Data4[0..2]-Data4[2..8]}` reads as one big-endian
/// number.
#[must_use]
pub fn guid(g: &GUID) -> Guid {
    let mut n = u128::from(g.Data1) << 96 | u128::from(g.Data2) << 80 | u128::from(g.Data3) << 64;
    for (i, b) in g.Data4.iter().enumerate() {
        n |= u128::from(*b) << (8 * (7 - i));
    }
    Guid(n)
}

/// Reads a string REAPER writes into a buffer of `size` bytes.
pub fn read_buf(size: usize, fill: impl FnOnce(*mut c_char, c_int) -> bool) -> Option<String> {
    let mut buf = vec![0 as c_char; size];
    let len = c_int::try_from(size).unwrap_or(c_int::MAX);
    if !fill(buf.as_mut_ptr(), len) {
        return None;
    }
    // SAFETY: REAPER writes a NUL-terminated string within the buffer; the
    // last byte is still 0 even if it filled the rest.
    buf[size - 1] = 0;
    Some(
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned(),
    )
}

fn cstr(s: &str) -> CString {
    CString::new(s).expect("parameter names have no NUL")
}

/// The tracks in list order, not including the master.
pub fn tracks(r: &Reaper) -> Vec<*mut MediaTrack> {
    // SAFETY: main thread, current project.
    unsafe {
        let n = r.CountTracks(null_mut());
        (0..n).map(|i| r.GetTrack(null_mut(), i)).collect()
    }
}

/// The master track.
pub fn master(r: &Reaper) -> *mut MediaTrack {
    // SAFETY: main thread, current project.
    unsafe { r.GetMasterTrack(null_mut()) }
}

/// A track's GUID.
pub fn track_guid(r: &Reaper, t: *mut MediaTrack) -> Guid {
    // SAFETY: `t` is a live track; REAPER returns a pointer into it.
    unsafe { guid(&*r.GetTrackGUID(t)) }
}

/// A numeric track property, such as `D_VOL`.
pub fn track_value(r: &Reaper, t: *mut MediaTrack, name: &str) -> f64 {
    let n = cstr(name);
    // SAFETY: `t` is a live track.
    unsafe { r.GetMediaTrackInfo_Value(t, n.as_ptr()) }
}

/// A string track property, such as `P_NAME`.
pub fn track_string(r: &Reaper, t: *mut MediaTrack, name: &str) -> String {
    let n = cstr(name);
    // SAFETY: `t` is a live track; the buffer is the size REAPER asks for
    // with "NeedBig" strings.
    read_buf(4096, |b, _| unsafe {
        r.GetSetMediaTrackInfo_String(t, n.as_ptr(), b, false)
    })
    .unwrap_or_default()
}

/// The folder track a track is in, if any.
pub fn parent(r: &Reaper, t: *mut MediaTrack) -> Option<*mut MediaTrack> {
    // SAFETY: `t` is a live track.
    let p = unsafe { r.GetParentTrack(t) };
    (!p.is_null()).then_some(p)
}

/// The number of sends (category 0) on a track.
pub fn send_count(r: &Reaper, t: *mut MediaTrack) -> c_int {
    // SAFETY: `t` is a live track.
    unsafe { r.GetTrackNumSends(t, 0) }
}

/// A numeric property of a send.
pub fn send_value(r: &Reaper, t: *mut MediaTrack, send: c_int, name: &str) -> f64 {
    let n = cstr(name);
    // SAFETY: `t` is a live track and `send` is below `send_count`.
    unsafe { r.GetTrackSendInfo_Value(t, 0, send, n.as_ptr()) }
}

/// The track a send goes to.
pub fn send_dest(r: &Reaper, t: *mut MediaTrack, send: c_int) -> *mut MediaTrack {
    // REAPER returns the pointer as a double; that is its documented API.
    let n = cstr("P_DESTTRACK");
    // SAFETY: `t` is a live track and `send` is below `send_count`.
    let p = unsafe { r.GetSetTrackSendInfo(t, 0, send, n.as_ptr(), null_mut()) };
    p.cast::<MediaTrack>()
}

/// The number of FX in a track's chain.
pub fn fx_count(r: &Reaper, t: *mut MediaTrack) -> c_int {
    // SAFETY: `t` is a live track.
    unsafe { r.TrackFX_GetCount(t) }
}

/// An FX instance's GUID.
pub fn fx_guid(r: &Reaper, t: *mut MediaTrack, fx: c_int) -> Option<Guid> {
    // SAFETY: `t` is a live track; a bad index gives null.
    unsafe {
        let g = r.TrackFX_GetFXGUID(t, fx);
        (!g.is_null()).then(|| guid(&*g))
    }
}

/// An FX instance's name as shown in the chain.
pub fn fx_name(r: &Reaper, t: *mut MediaTrack, fx: c_int) -> String {
    // SAFETY: `t` is a live track.
    read_buf(512, |b, n| unsafe { r.TrackFX_GetFXName(t, fx, b, n) }).unwrap_or_default()
}

/// A named config value of an FX instance, such as `fx_name`.
pub fn fx_config(r: &Reaper, t: *mut MediaTrack, fx: c_int, name: &str) -> Option<String> {
    let n = cstr(name);
    // SAFETY: `t` is a live track.
    read_buf(4096, |b, len| unsafe {
        r.TrackFX_GetNamedConfigParm(t, fx, n.as_ptr(), b, len)
    })
}

/// Whether an FX instance is enabled (not bypassed).
pub fn fx_enabled(r: &Reaper, t: *mut MediaTrack, fx: c_int) -> bool {
    // SAFETY: `t` is a live track.
    unsafe { r.TrackFX_GetEnabled(t, fx) }
}

/// The number of parameters an FX instance has.
pub fn param_count(r: &Reaper, t: *mut MediaTrack, fx: c_int) -> c_int {
    // SAFETY: `t` is a live track.
    unsafe { r.TrackFX_GetNumParams(t, fx) }
}

/// A parameter's name.
pub fn param_name(r: &Reaper, t: *mut MediaTrack, fx: c_int, p: c_int) -> String {
    // SAFETY: `t` is a live track.
    read_buf(256, |b, n| unsafe {
        r.TrackFX_GetParamName(t, fx, p, b, n)
    })
    .unwrap_or_default()
}

/// A parameter's normalised value.
pub fn param_value(r: &Reaper, t: *mut MediaTrack, fx: c_int, p: c_int) -> f64 {
    // SAFETY: `t` is a live track.
    unsafe { r.TrackFX_GetParamNormalized(t, fx, p) }
}

/// The plugin's display text for a parameter's current value.
pub fn param_display(r: &Reaper, t: *mut MediaTrack, fx: c_int, p: c_int) -> String {
    // SAFETY: `t` is a live track.
    read_buf(256, |b, n| unsafe {
        r.TrackFX_GetFormattedParamValue(t, fx, p, b, n)
    })
    .unwrap_or_default()
}

/// The plugin's display text for a parameter at `value`, if it can say.
pub fn param_format(
    r: &Reaper,
    t: *mut MediaTrack,
    fx: c_int,
    p: c_int,
    value: f64,
) -> Option<String> {
    // SAFETY: `t` is a live track.
    read_buf(256, |b, n| unsafe {
        r.TrackFX_FormatParamValueNormalized(t, fx, p, value, b, n)
    })
    .filter(|s| !s.is_empty())
}

/// Whether a parameter is a toggle.
pub fn param_is_toggle(r: &Reaper, t: *mut MediaTrack, fx: c_int, p: c_int) -> bool {
    let (mut step, mut small, mut large, mut toggle) = (0.0, 0.0, 0.0, false);
    // SAFETY: `t` is a live track; the out pointers are valid.
    unsafe {
        r.TrackFX_GetParameterStepSizes(
            t,
            fx,
            p,
            &raw mut step,
            &raw mut small,
            &raw mut large,
            &raw mut toggle,
        ) && toggle
    }
}

/// A track's items, in REAPER's order.
pub fn items(r: &Reaper, t: *mut MediaTrack) -> Vec<*mut MediaItem> {
    // SAFETY: `t` is a live track.
    unsafe {
        let n = r.CountTrackMediaItems(t);
        (0..n).map(|i| r.GetTrackMediaItem(t, i)).collect()
    }
}

/// A numeric item property, such as `D_POSITION`.
pub fn item_value(r: &Reaper, i: *mut MediaItem, name: &str) -> f64 {
    let n = cstr(name);
    // SAFETY: `i` is a live item.
    unsafe { r.GetMediaItemInfo_Value(i, n.as_ptr()) }
}

/// An item's GUID.
pub fn item_guid(r: &Reaper, i: *mut MediaItem) -> Option<Guid> {
    let n = cstr("GUID");
    // SAFETY: `i` is a live item.
    read_buf(4096, |b, _| unsafe {
        r.GetSetMediaItemInfo_String(i, n.as_ptr(), b, false)
    })
    .and_then(|s| s.parse().ok())
}

/// An item's active take, if any.
pub fn active_take(r: &Reaper, i: *mut MediaItem) -> Option<*mut MediaItem_Take> {
    // SAFETY: `i` is a live item.
    let t = unsafe { r.GetActiveTake(i) };
    (!t.is_null()).then_some(t)
}

/// A take's GUID.
pub fn take_guid(r: &Reaper, t: *mut MediaItem_Take) -> Option<Guid> {
    let n = cstr("GUID");
    // SAFETY: `t` is a live take.
    read_buf(4096, |b, _| unsafe {
        r.GetSetMediaItemTakeInfo_String(t, n.as_ptr(), b, false)
    })
    .and_then(|s| s.parse().ok())
}

/// Whether a take is MIDI.
pub fn take_is_midi(r: &Reaper, t: *mut MediaItem_Take) -> bool {
    // SAFETY: `t` is a live take.
    unsafe { r.TakeIsMIDI(t) }
}

/// A region or marker: (is region, number, name, start, end).
pub fn marker(r: &Reaper, idx: c_int) -> Option<(bool, c_int, String, f64, f64)> {
    let (mut is_rgn, mut pos, mut end, mut num) = (false, 0.0, 0.0, 0);
    let mut name: *const c_char = std::ptr::null();
    // SAFETY: main thread, current project; out pointers are valid. The name
    // pointer is owned by REAPER and read at once.
    unsafe {
        let next = r.EnumProjectMarkers3(
            null_mut(),
            idx,
            &raw mut is_rgn,
            &raw mut pos,
            &raw mut end,
            &raw mut name,
            &raw mut num,
            null_mut(),
        );
        if next == 0 {
            return None;
        }
        let name = if name.is_null() {
            String::new()
        } else {
            CStr::from_ptr(name).to_string_lossy().into_owned()
        };
        Some((is_rgn, num, name, pos, end))
    }
}

/// The current project's file, if it has been saved.
pub fn project_path(r: &Reaper) -> Option<String> {
    // SAFETY: main thread; -1 means the current project.
    read_buf(4096, |b, n| unsafe { !r.EnumProjects(-1, b, n).is_null() }).filter(|s| !s.is_empty())
}

// --- Edits -----------------------------------------------------------------

/// Inserts a track at `idx` with REAPER's default FX and envelopes, and
/// returns it.
pub fn insert_track(r: &Reaper, idx: c_int) -> *mut MediaTrack {
    // SAFETY: main thread, current project; `idx` is within 0..=count.
    unsafe {
        r.InsertTrackInProject(null_mut(), idx, 1);
        r.GetTrack(null_mut(), idx)
    }
}

/// Deletes a track.
pub fn delete_track(r: &Reaper, t: *mut MediaTrack) {
    // SAFETY: `t` is a live track; it is not used afterwards.
    unsafe { r.DeleteTrack(t) }
}

/// Sets a numeric track property.
pub fn set_track_value(r: &Reaper, t: *mut MediaTrack, name: &str, v: f64) -> bool {
    let n = cstr(name);
    // SAFETY: `t` is a live track.
    unsafe { r.SetMediaTrackInfo_Value(t, n.as_ptr(), v) }
}

/// Sets a string track property, such as `P_NAME`.
pub fn set_track_string(r: &Reaper, t: *mut MediaTrack, name: &str, v: &str) -> bool {
    let n = cstr(name);
    let mut v = cstr(v).into_bytes_with_nul();
    // SAFETY: `t` is a live track; REAPER reads the NUL-terminated value.
    unsafe { r.GetSetMediaTrackInfo_String(t, n.as_ptr(), v.as_mut_ptr().cast(), true) }
}

/// How deep a track is nested in folders; 0 at the top level.
pub fn track_depth(r: &Reaper, t: *mut MediaTrack) -> c_int {
    // SAFETY: `t` is a live track.
    unsafe { r.GetTrackDepth(t) }
}

/// Adds a send and returns its index.
pub fn create_send(r: &Reaper, from: *mut MediaTrack, to: *mut MediaTrack) -> c_int {
    // SAFETY: both are live tracks.
    unsafe { r.CreateTrackSend(from, to) }
}

/// Sets a numeric property of a send.
pub fn set_send_value(r: &Reaper, t: *mut MediaTrack, send: c_int, name: &str, v: f64) -> bool {
    let n = cstr(name);
    // SAFETY: `t` is a live track and `send` is below `send_count`.
    unsafe { r.SetTrackSendInfo_Value(t, 0, send, n.as_ptr(), v) }
}

/// Adds a plugin by name, at `index` or the end. Returns its position, or
/// `None` if REAPER cannot find the plugin.
pub fn fx_add(r: &Reaper, t: *mut MediaTrack, plugin: &str, index: Option<c_int>) -> Option<c_int> {
    let n = cstr(plugin);
    // -1 always adds a new instance; -1000 - n inserts at position n.
    let instantiate = index.map_or(-1, |i| -1000 - i);
    // SAFETY: `t` is a live track.
    let i = unsafe { r.TrackFX_AddByName(t, n.as_ptr(), false, instantiate) };
    (i >= 0).then_some(i)
}

/// Removes an FX instance.
pub fn fx_delete(r: &Reaper, t: *mut MediaTrack, fx: c_int) -> bool {
    // SAFETY: `t` is a live track.
    unsafe { r.TrackFX_Delete(t, fx) }
}

/// Moves an FX instance within its chain.
pub fn fx_move(r: &Reaper, t: *mut MediaTrack, fx: c_int, to: c_int) {
    // SAFETY: `t` is a live track; both indices are within the chain.
    unsafe { r.TrackFX_CopyToTrack(t, fx, t, to, true) }
}

/// Enables or bypasses an FX instance.
pub fn fx_set_enabled(r: &Reaper, t: *mut MediaTrack, fx: c_int, enabled: bool) {
    // SAFETY: `t` is a live track.
    unsafe { r.TrackFX_SetEnabled(t, fx, enabled) }
}

/// Sets a parameter, normalised.
pub fn param_set(r: &Reaper, t: *mut MediaTrack, fx: c_int, p: c_int, v: f64) -> bool {
    // SAFETY: `t` is a live track.
    unsafe { r.TrackFX_SetParamNormalized(t, fx, p, v) }
}

/// Loads a named preset.
pub fn fx_set_preset(r: &Reaper, t: *mut MediaTrack, fx: c_int, name: &str) -> bool {
    let n = cstr(name);
    // SAFETY: `t` is a live track.
    unsafe { r.TrackFX_SetPreset(t, fx, n.as_ptr()) }
}

/// A track's state chunk.
pub fn track_chunk(r: &Reaper, t: *mut MediaTrack) -> Option<String> {
    let mut size = 1 << 16;
    while size <= 1 << 28 {
        // SAFETY: `t` is a live track; REAPER fails rather than overflow.
        if let Some(s) = read_buf(size, |b, n| unsafe { r.GetTrackStateChunk(t, b, n, false) }) {
            // A full buffer may mean the chunk was cut short.
            if s.len() + 1 < size {
                return Some(s);
            }
        }
        size *= 4;
    }
    None
}

/// Replaces a track's state chunk.
pub fn set_track_chunk(r: &Reaper, t: *mut MediaTrack, chunk: &str) -> bool {
    let Ok(c) = CString::new(chunk) else {
        return false;
    };
    // SAFETY: `t` is a live track.
    unsafe { r.SetTrackStateChunk(t, c.as_ptr(), false) }
}

/// A numeric project setting, such as `RENDER_STARTPOS`.
pub fn project_info(r: &Reaper, name: &str) -> f64 {
    let n = cstr(name);
    // SAFETY: main thread, current project.
    unsafe { r.GetSetProjectInfo(null_mut(), n.as_ptr(), 0.0, false) }
}

/// Sets a numeric project setting.
pub fn set_project_info(r: &Reaper, name: &str, v: f64) {
    let n = cstr(name);
    // SAFETY: main thread, current project.
    unsafe {
        r.GetSetProjectInfo(null_mut(), n.as_ptr(), v, true);
    }
}

/// Runs a main-section action on the current project.
pub fn run_action(r: &Reaper, command: c_int) {
    // SAFETY: main thread, current project.
    unsafe { r.Main_OnCommandEx(command, 0, null_mut()) }
}

/// Moves the edit cursor without moving the view or seeking playback.
pub fn set_cursor(r: &Reaper, t: f64) {
    // SAFETY: main thread, current project.
    unsafe { r.SetEditCurPos2(null_mut(), t, false, false) }
}

/// Starts playback from the edit cursor.
pub fn play(r: &Reaper) {
    // SAFETY: main thread, current project.
    unsafe { r.OnPlayButtonEx(null_mut()) }
}

/// The position audio is being played at.
pub fn play_position(r: &Reaper) -> f64 {
    // SAFETY: main thread, current project.
    unsafe { r.GetPlayPosition2Ex(null_mut()) }
}

/// Actions in the main section whose names contain `text`, ignoring case.
pub fn find_actions(r: &Reaper, text: &str, limit: usize) -> Vec<(c_int, String)> {
    let want = text.to_lowercase();
    let mut out = Vec::new();
    // SAFETY: main thread; a null section is the main section; REAPER owns
    // the name strings, which are read at once.
    unsafe {
        for idx in 0.. {
            let mut name: *const c_char = std::ptr::null();
            let id = r.kbd_enumerateActions(null_mut(), idx, &raw mut name);
            if id == 0 {
                break;
            }
            if name.is_null() {
                continue;
            }
            let n = CStr::from_ptr(name).to_string_lossy();
            if n.to_lowercase().contains(&want) {
                out.push((id, n.into_owned()));
                if out.len() >= limit {
                    break;
                }
            }
        }
    }
    out
}

/// Saves the current project to its file, or as a copy at `path`.
pub fn save_project(r: &Reaper, path: Option<&str>) {
    // SAFETY: main thread, current project.
    unsafe {
        match path {
            None => r.Main_SaveProject(null_mut(), false),
            Some(p) => {
                let c = cstr(p);
                r.Main_SaveProjectEx(null_mut(), c.as_ptr(), 0);
            }
        }
    }
}

/// The label of the undo point that redo would redo.
pub fn redo_label(r: &Reaper) -> Option<String> {
    // SAFETY: main thread, current project; the string is REAPER's and
    // read at once.
    unsafe {
        let p = r.Undo_CanRedo2(null_mut());
        (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
    }
}

// --- Envelopes -------------------------------------------------------------

/// A track envelope by its chunk name, such as `<VOLENV2`.
pub fn envelope_by_chunk_name(
    r: &Reaper,
    t: *mut MediaTrack,
    name: &str,
) -> Option<*mut TrackEnvelope> {
    let n = cstr(name);
    // SAFETY: `t` is a live track.
    let e = unsafe { r.GetTrackEnvelopeByChunkName(t, n.as_ptr()) };
    (!e.is_null()).then_some(e)
}

/// An FX parameter's envelope, created if `create` is set.
pub fn fx_envelope(
    r: &Reaper,
    t: *mut MediaTrack,
    fx: c_int,
    p: c_int,
    create: bool,
) -> Option<*mut TrackEnvelope> {
    // SAFETY: `t` is a live track; bad indices give null.
    let e = unsafe { r.GetFXEnvelope(t, fx, p, create) };
    (!e.is_null()).then_some(e)
}

/// A send's volume envelope, if it has one.
pub fn send_envelope(r: &Reaper, t: *mut MediaTrack, send: c_int) -> Option<*mut TrackEnvelope> {
    let n = cstr("P_ENV:<VOLENV");
    // SAFETY: `t` is a live track and `send` is below `send_count`.
    let e = unsafe { r.GetSetTrackSendInfo(t, 0, send, n.as_ptr(), null_mut()) };
    (!e.is_null()).then(|| e.cast::<TrackEnvelope>())
}

/// A track's envelopes, including its FX parameters'.
pub fn envelopes(r: &Reaper, t: *mut MediaTrack) -> Vec<*mut TrackEnvelope> {
    // SAFETY: `t` is a live track.
    unsafe {
        let n = r.CountTrackEnvelopes(t);
        (0..n)
            .map(|i| r.GetTrackEnvelope(t, i))
            .filter(|e| !e.is_null())
            .collect()
    }
}

/// An envelope's GUID.
pub fn envelope_guid(r: &Reaper, e: *mut TrackEnvelope) -> Option<Guid> {
    let n = cstr("GUID");
    // SAFETY: `e` is a live envelope.
    read_buf(4096, |b, _| unsafe {
        r.GetSetEnvelopeInfo_String(e, n.as_ptr(), b, false)
    })
    .and_then(|s| s.parse().ok())
}

/// The FX and parameter an envelope belongs to, if it is a parameter's.
pub fn envelope_fx_param(r: &Reaper, e: *mut TrackEnvelope) -> Option<(c_int, c_int)> {
    let (mut fx, mut p) = (-1, -1);
    // SAFETY: `e` is a live envelope; the out pointers are valid.
    unsafe { r.Envelope_GetParentTrack(e, &raw mut fx, &raw mut p) };
    (fx >= 0 && p >= 0).then_some((fx, p))
}

/// The envelope's scaling mode: 0 none, 1 fader scaling.
pub fn envelope_scaling(r: &Reaper, e: *mut TrackEnvelope) -> c_int {
    // SAFETY: `e` is a live envelope.
    unsafe { r.GetEnvelopeScalingMode(e) }
}

/// A raw envelope value as REAPER displays it.
pub fn envelope_format(r: &Reaper, e: *mut TrackEnvelope, raw: f64) -> String {
    // SAFETY: `e` is a live envelope.
    read_buf(128, |b, n| unsafe {
        r.Envelope_FormatValue(e, raw, b, n);
        true
    })
    .unwrap_or_default()
}

/// One point: (time, raw value, shape, tension).
pub fn envelope_points(r: &Reaper, env: *mut TrackEnvelope) -> Vec<(f64, f64, c_int, f64)> {
    // SAFETY: `env` is a live envelope; the out pointers are valid.
    unsafe {
        let count = r.CountEnvelopePoints(env);
        (0..count)
            .filter_map(|idx| {
                let (mut time, mut value, mut shape, mut tension, mut sel) =
                    (0.0, 0.0, 0, 0.0, false);
                r.GetEnvelopePoint(
                    env,
                    idx,
                    &raw mut time,
                    &raw mut value,
                    &raw mut shape,
                    &raw mut tension,
                    &raw mut sel,
                )
                .then_some((time, value, shape, tension))
            })
            .collect()
    }
}

/// Adds points (time, raw value, shape, tension), then sorts once.
pub fn envelope_insert(r: &Reaper, e: *mut TrackEnvelope, points: &[(f64, f64, c_int, f64)]) {
    let mut no_sort = true;
    // SAFETY: `e` is a live envelope.
    unsafe {
        for &(t, v, s, k) in points {
            r.InsertEnvelopePoint(e, t, v, s, k, false, &raw mut no_sort);
        }
        r.Envelope_SortPoints(e);
    }
}

/// Removes the points from `start` to `end`.
pub fn envelope_delete(r: &Reaper, e: *mut TrackEnvelope, start: f64, end: f64) {
    // SAFETY: `e` is a live envelope.
    unsafe {
        r.DeleteEnvelopePointRange(e, start, end);
    }
}

/// A parameter's native range, for parameter envelopes.
pub fn param_range(r: &Reaper, t: *mut MediaTrack, fx: c_int, p: c_int) -> (f64, f64) {
    let (mut lo, mut hi) = (0.0, 1.0);
    // SAFETY: `t` is a live track; the out pointers are valid.
    unsafe { r.TrackFX_GetParam(t, fx, p, &raw mut lo, &raw mut hi) };
    (lo, hi)
}

/// Runs a REAPER action on `t` alone, keeping the track selection as it
/// was.
pub fn action_on_track(r: &Reaper, t: *mut MediaTrack, command: c_int) {
    // SAFETY: main thread, current project; `t` and the selected tracks are
    // live for the length of this call.
    unsafe {
        let n = r.CountSelectedTracks(null_mut());
        let selected: Vec<*mut MediaTrack> =
            (0..n).map(|i| r.GetSelectedTrack(null_mut(), i)).collect();
        r.SetOnlyTrackSelected(t);
        r.Main_OnCommandEx(command, 0, null_mut());
        r.SetTrackSelected(t, false);
        for s in selected {
            r.SetTrackSelected(s, true);
        }
    }
}

// --- MIDI ------------------------------------------------------------------

/// Creates an empty MIDI item from `start` to `end` seconds.
pub fn create_midi_item(
    r: &Reaper,
    t: *mut MediaTrack,
    start: f64,
    end: f64,
) -> Option<*mut MediaItem> {
    let qn = false;
    // SAFETY: `t` is a live track.
    let i = unsafe { r.CreateNewMIDIItemInProj(t, start, end, &raw const qn) };
    (!i.is_null()).then_some(i)
}

/// A note as REAPER stores it: (start ppq, end ppq, channel, pitch, velocity).
pub type RawNote = (f64, f64, c_int, c_int, c_int);

/// A control change: (ppq, channel, controller, value).
pub type RawCc = (f64, c_int, c_int, c_int);

/// The notes in a take.
pub fn midi_notes(r: &Reaper, k: *mut MediaItem_Take) -> Vec<RawNote> {
    // SAFETY: `k` is a live MIDI take; the out pointers are valid.
    unsafe {
        let (mut notes, mut ccs, mut texts) = (0, 0, 0);
        r.MIDI_CountEvts(k, &raw mut notes, &raw mut ccs, &raw mut texts);
        (0..notes)
            .filter_map(|idx| {
                let (mut sel, mut muted, mut start, mut end) = (false, false, 0.0, 0.0);
                let (mut chan, mut pitch, mut vel) = (0, 0, 0);
                r.MIDI_GetNote(
                    k,
                    idx,
                    &raw mut sel,
                    &raw mut muted,
                    &raw mut start,
                    &raw mut end,
                    &raw mut chan,
                    &raw mut pitch,
                    &raw mut vel,
                )
                .then_some((start, end, chan, pitch, vel))
            })
            .collect()
    }
}

/// The control changes (status 0xB0) in a take, with their indices.
pub fn midi_ccs(r: &Reaper, k: *mut MediaItem_Take) -> Vec<(c_int, RawCc)> {
    // SAFETY: `k` is a live MIDI take; the out pointers are valid.
    unsafe {
        let (mut notes, mut ccs, mut texts) = (0, 0, 0);
        r.MIDI_CountEvts(k, &raw mut notes, &raw mut ccs, &raw mut texts);
        (0..ccs)
            .filter_map(|idx| {
                let (mut sel, mut muted, mut ppq) = (false, false, 0.0);
                let (mut msg, mut chan, mut num, mut val) = (0, 0, 0, 0);
                r.MIDI_GetCC(
                    k,
                    idx,
                    &raw mut sel,
                    &raw mut muted,
                    &raw mut ppq,
                    &raw mut msg,
                    &raw mut chan,
                    &raw mut num,
                    &raw mut val,
                )
                .then_some((msg, idx, (ppq, chan, num, val)))
            })
            .filter(|(msg, ..)| *msg == 0xB0)
            .map(|(_, idx, cc)| (idx, cc))
            .collect()
    }
}

/// Adds notes and control changes, then sorts the take once.
pub fn midi_insert(r: &Reaper, k: *mut MediaItem_Take, notes: &[RawNote], ccs: &[RawCc]) {
    let no_sort = true;
    // SAFETY: `k` is a live MIDI take.
    unsafe {
        for &(start, end, chan, pitch, vel) in notes {
            r.MIDI_InsertNote(
                k,
                false,
                false,
                start,
                end,
                chan,
                pitch,
                vel,
                &raw const no_sort,
            );
        }
        for &(ppq, chan, num, val) in ccs {
            r.MIDI_InsertCC(k, false, false, ppq, 0xB0, chan, num, val);
        }
        r.MIDI_Sort(k);
    }
}

/// Deletes notes by index, highest first so indices stay valid.
pub fn midi_delete_notes(r: &Reaper, k: *mut MediaItem_Take, mut idx: Vec<c_int>) {
    idx.sort_unstable_by(|a, b| b.cmp(a));
    // SAFETY: `k` is a live MIDI take; the indices came from it.
    unsafe {
        for i in idx {
            r.MIDI_DeleteNote(k, i);
        }
        r.MIDI_Sort(k);
    }
}

/// Deletes events from the CC list by index, highest first.
pub fn midi_delete_ccs(r: &Reaper, k: *mut MediaItem_Take, mut idx: Vec<c_int>) {
    idx.sort_unstable_by(|a, b| b.cmp(a));
    // SAFETY: `k` is a live MIDI take; the indices came from it.
    unsafe {
        for i in idx {
            r.MIDI_DeleteCC(k, i);
        }
        r.MIDI_Sort(k);
    }
}

/// Project seconds to the take's ticks.
pub fn ppq_at(r: &Reaper, k: *mut MediaItem_Take, seconds: f64) -> f64 {
    // SAFETY: `k` is a live take.
    unsafe { r.MIDI_GetPPQPosFromProjTime(k, seconds) }
}

/// The take's ticks to project seconds.
pub fn seconds_at(r: &Reaper, k: *mut MediaItem_Take, ppq: f64) -> f64 {
    // SAFETY: `k` is a live take.
    unsafe { r.MIDI_GetProjTimeFromPPQPos(k, ppq) }
}

/// Marks an item changed, so the open undo block records it, and redraws
/// it. REAPER's MIDI calls do neither on their own.
pub fn update_item(r: &Reaper, i: *mut MediaItem) {
    // SAFETY: `i` is a live item on a live track.
    unsafe {
        r.MarkTrackItemsDirty(r.GetMediaItemTrack(i), i);
        r.UpdateItemInProject(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_matches_text_form() {
        let g = GUID {
            Data1: 0x0A1B_2C3D,
            Data2: 0x4E5F,
            Data3: 0x6071,
            Data4: [0x82, 0x93, 0xA4, 0xB5, 0xC6, 0xD7, 0xE8, 0xF9],
        };
        assert_eq!(
            guid(&g).to_string(),
            "{0A1B2C3D-4E5F-6071-8293-A4B5C6D7E8F9}"
        );
    }
}
