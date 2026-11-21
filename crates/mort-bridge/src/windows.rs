//! Closing the window a dry-run render leaves open.
//!
//! When a dry run finishes, REAPER leaves a results window up, titled
//! like "Finished in 0:00 (17x realtime)". REAPER's "close when finished"
//! preference does not apply to it (tried, every bit of
//! `renderclosewhendone`), so the bridge closes it through SWELL: it notes
//! which such windows exist before a render and closes those that appeared
//! during it, so a results window from the person's own render stays.
//! Main thread only.

use std::cell::RefCell;
use std::os::raw::c_char;

use reaper_low::raw::{BOOL, HWND, IDCANCEL, LPARAM, WM_CLOSE, WM_COMMAND};
use reaper_low::{PluginContext, Swell};

thread_local! {
    static SWELL: RefCell<Option<Swell>> = const { RefCell::new(None) };
}

/// Loads SWELL's functions. Call once, from the plugin's entry point.
pub fn init(context: PluginContext) {
    SWELL.with(|s| *s.borrow_mut() = Some(Swell::load(context)));
}

/// Whether a window title is a finished render's results window.
#[must_use]
pub fn is_results_title(title: &str) -> bool {
    title.starts_with("Finished in ") && title.contains("realtime")
}

unsafe extern "C" fn collect(hwnd: HWND, lp: LPARAM) -> BOOL {
    // SAFETY: `lp` is the `Vec` passed by `results_windows` for the length
    // of the `EnumWindows` call.
    let found = unsafe { &mut *(lp as *mut Vec<HWND>) };
    found.push(hwnd);
    1
}

/// The results windows open now.
#[must_use]
pub fn results_windows() -> Vec<HWND> {
    SWELL.with(|s| {
        let Some(swell) = *s.borrow() else {
            return Vec::new();
        };
        let mut all: Vec<HWND> = Vec::new();
        // SAFETY: main thread; `collect` only pushes into `all`, which
        // outlives the call.
        unsafe { swell.EnumWindows(Some(collect), (&raw mut all) as LPARAM) };
        all.into_iter()
            .filter(|h| {
                let mut buf = [0 as c_char; 256];
                // SAFETY: `h` was just enumerated; `buf` holds 256 bytes.
                unsafe { swell.GetWindowText(*h, buf.as_mut_ptr(), 256) };
                // SAFETY: SWELL writes a NUL-terminated string into `buf`.
                let title = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
                is_results_title(&title.to_string_lossy())
            })
            .collect()
    })
}

/// Closes the results windows that are open now but were not in `before`.
/// Returns how many it closed.
pub fn close_new_results(before: &[HWND]) -> usize {
    let new: Vec<HWND> = results_windows()
        .into_iter()
        .filter(|h| !before.contains(h))
        .collect();
    SWELL.with(|s| {
        if let Some(swell) = s.borrow().as_ref() {
            for h in &new {
                // The dialog may ignore WM_CLOSE; its Close button sends
                // IDCANCEL. Send both.
                // SAFETY: `h` is a live window, just enumerated.
                unsafe {
                    swell.PostMessage(*h, WM_COMMAND, IDCANCEL as usize, 0);
                    swell.PostMessage(*h, WM_CLOSE, 0, 0);
                }
            }
        }
    });
    new.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_results_titles() {
        assert!(is_results_title("Finished in 0:00 (17x realtime)"));
        assert!(is_results_title("Finished in 1:23 (4.5x realtime)"));
        assert!(!is_results_title("Render to File"));
        assert!(!is_results_title("Finished"));
        assert!(!is_results_title("Rendering to file... 50%"));
    }
}
