//! One window per notes folder: launching the app again brings the window
//! that is already open to the front instead of starting a second copy.

use std::ffi::c_void;

#[link(name = "user32")]
unsafe extern "system" {
    fn FindWindowW(class: *const u16, title: *const u16) -> *mut c_void;
    fn IsIconic(hwnd: *mut c_void) -> i32;
    fn ShowWindow(hwnd: *mut c_void, command: i32) -> i32;
    fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
}

const SW_RESTORE: i32 = 9;
/// The window class GPUI registers for its windows.
const WINDOW_CLASS: &str = "Zed::Window";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Bring the window titled `title` to the front. Returns false when there is
/// no such window, so the caller should start normally.
pub fn focus_existing(title: &str) -> bool {
    let (class, title) = (wide(WINDOW_CLASS), wide(title));
    // SAFETY: both strings are NUL-terminated and outlive the calls; the
    // handle is only passed back to user32, which checks it.
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), title.as_ptr());
        if hwnd.is_null() {
            return false;
        }
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
        true
    }
}
