//! Thin wrappers over REAPER's C API for reading the project.
//!
//! Every function takes the low-level [`Reaper`] and pointers REAPER gave
//! us on this tick. The project is always the current one (a null project
//! pointer). Callers must be on the main thread.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr::null_mut;

use reaper_low::Reaper;
use reaper_low::raw::{GUID, MediaItem, MediaItem_Take, MediaTrack};

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
