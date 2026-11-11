//! Keeping the pill out of the picture.
//!
//! The overlay must never appear in a clip or recording — it is feedback *about*
//! capture, so capturing it would be absurd. Each platform has its own way of saying
//! "this window is not part of the screen":
//!
//! * Windows: `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`, which DXGI desktop
//!   duplication honours.
//! * macOS: `NSWindowSharingNone`, which ScreenCaptureKit honours.
//!
//! Both must be applied *after* the window exists and again if it is recreated, and
//! both fail quietly when the OS is too old, so the result is reported rather than
//! assumed. If exclusion fails the pill still works — it would simply be visible in
//! clips, which is a cosmetic problem, not a reason to stop recording.

/// Whether this platform can exclude a window from capture.
pub fn supported() -> bool {
    cfg!(any(target_os = "windows", target_os = "macos"))
}

/// The affinity value Windows uses to hide a window from capture.
///
/// Exposed so the platform layer and tests agree on the constant rather than repeating
/// a magic number (`WDA_EXCLUDEFROMCAPTURE`, Windows 10 2004+).
pub const WDA_EXCLUDEFROMCAPTURE: u32 = 0x11;

/// `NSWindowSharingNone`, which tells ScreenCaptureKit to leave the window out.
pub const NS_WINDOW_SHARING_NONE: u64 = 0;

/// Collection behaviour bits: show on all Spaces and over full-screen apps, so the
/// pill is visible while a game runs — the case it exists for.
pub const NS_COLLECTION_CAN_JOIN_ALL_SPACES: u64 = 1 << 0;
pub const NS_COLLECTION_FULL_SCREEN_AUXILIARY: u64 = 1 << 8;

pub fn ns_collection_behavior() -> u64 {
    NS_COLLECTION_CAN_JOIN_ALL_SPACES | NS_COLLECTION_FULL_SCREEN_AUXILIARY
}

/// Hide a window with the given native handle from screen capture.
///
/// Returns whether the platform accepted it. A `false` means the pill works but will
/// be recorded, which the caller should report once rather than retry forever.
pub fn exclude_from_capture(handle: u64) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::HWND;
        if handle == 0 || !windows_is_known(handle) {
            return false;
        }
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowDisplayAffinity, WINDOW_DISPLAY_AFFINITY,
        };
        if handle == 0 {
            return false;
        }
        // SAFETY: `handle` is an HWND owned by this process.
        unsafe {
            SetWindowDisplayAffinity(
                HWND(handle as *mut core::ffi::c_void),
                WINDOW_DISPLAY_AFFINITY(WDA_EXCLUDEFROMCAPTURE),
            )
            .is_ok()
        }
    }
    #[cfg(target_os = "macos")]
    {
        macos::set_sharing_none(handle)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = handle;
        false
    }
}

#[cfg(target_os = "windows")]
fn windows_is_known(handle: u64) -> bool {
    windows_known()
        .lock()
        .map(|known| known.contains(&handle))
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn windows_known() -> &'static std::sync::Mutex<Vec<u64>> {
    static KNOWN: std::sync::OnceLock<std::sync::Mutex<Vec<u64>>> = std::sync::OnceLock::new();
    KNOWN.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Record a window handle as real before styling it. Call when the window is created.
pub fn register_window(handle: u64) {
    if handle == 0 {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(mut known) = windows_known().lock() {
            if !known.contains(&handle) {
                known.push(handle);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        macos::register_window(handle);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = handle;
    }
}

/// Forget a window handle once the window is gone.
pub fn forget_window(handle: u64) {
    #[cfg(target_os = "windows")]
    {
        if let Ok(mut known) = windows_known().lock() {
            known.retain(|h| *h != handle);
        }
    }
    #[cfg(target_os = "macos")]
    {
        macos::forget_window(handle);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = handle;
    }
}

/// Keep a window above others without stealing focus.
pub fn keep_on_top(handle: u64) -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::HWND;
        if handle == 0 || !windows_is_known(handle) {
            return false;
        }
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        };
        if handle == 0 {
            return false;
        }
        // SAFETY: `handle` is an HWND owned by this process.
        unsafe {
            SetWindowPos(
                HWND(handle as *mut core::ffi::c_void),
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .is_ok()
        }
    }
    #[cfg(target_os = "macos")]
    {
        macos::keep_on_top(handle)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = handle;
        false
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;
    use std::sync::Mutex;

    #[link(name = "objc")]
    extern "C" {
        fn objc_msgSend();
    }

    /// Call `[obj selector:]` for a void-returning method, optionally with an argument.
    fn send(obj: *mut c_void, selector: &str, arg: Option<u64>) {
        if obj.is_null() {
            return;
        }
        let sel = selector_bytes(selector);
        // SAFETY: objc_msgSend is called with the signature its selector expects:
        // a NSUInteger argument where one is passed, and none otherwise.
        unsafe {
            let name = objc_sel_registerName(sel.as_ptr());
            match arg {
                Some(value) => {
                    let f: extern "C" fn(*mut c_void, *mut c_void, u64) =
                        std::mem::transmute(objc_msgSend as *const ());
                    f(obj, name, value);
                }
                None => {
                    let f: extern "C" fn(*mut c_void, *mut c_void) =
                        std::mem::transmute(objc_msgSend as *const ());
                    f(obj, name);
                }
            }
        }
    }

    #[link(name = "objc")]
    extern "C" {
        fn sel_registerName(name: *const std::os::raw::c_char) -> *mut c_void;
    }

    #[allow(non_snake_case)]
    fn objc_sel_registerName(name: *const std::os::raw::c_char) -> *mut c_void {
        // SAFETY: forwarding a NUL-terminated selector name to the runtime.
        unsafe { sel_registerName(name) }
    }

    fn selector_bytes(selector: &str) -> Vec<std::os::raw::c_char> {
        let mut bytes: Vec<std::os::raw::c_char> = selector.bytes().map(|b| b as _).collect();
        bytes.push(0);
        bytes
    }

    /// `[NSView window]`: the NSWindow that owns a view handle.
    fn window_for(view: u64) -> *mut c_void {
        if view == 0 {
            return std::ptr::null_mut();
        }
        // SAFETY: `view` is an NSView* owned by this process.
        unsafe {
            let f: extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                std::mem::transmute(objc_msgSend as *const ());
            let sel = selector_bytes("window");
            f(view as *mut c_void, objc_sel_registerName(sel.as_ptr()))
        }
    }

    /// Windows this process has created and handed over for styling.
    ///
    /// Handles arrive from the toolkit, and a stale or foreign one must never reach
    /// `objc_msgSend`: sending a message to garbage is a segfault, not an error. Only a
    /// handle registered here is treated as real.
    fn known_windows() -> &'static Mutex<Vec<u64>> {
        static KNOWN: std::sync::LazyLock<Mutex<Vec<u64>>> =
            std::sync::LazyLock::new(|| Mutex::new(Vec::new()));
        &KNOWN
    }

    /// Record a window handle as real before styling it.
    pub fn register_window(handle: u64) {
        if handle == 0 {
            return;
        }
        if let Ok(mut known) = known_windows().lock() {
            if !known.contains(&handle) {
                known.push(handle);
            }
        }
    }

    /// Forget a window that has been closed.
    pub fn forget_window(handle: u64) {
        if let Ok(mut known) = known_windows().lock() {
            known.retain(|h| *h != handle);
        }
    }

    fn is_known(handle: u64) -> bool {
        known_windows()
            .lock()
            .map(|known| known.contains(&handle))
            .unwrap_or(false)
    }

    pub fn set_sharing_none(handle: u64) -> bool {
        if !is_known(handle) {
            return false;
        }
        let win = window_for(handle);
        if win.is_null() {
            return false;
        }
        send(win, "setSharingType:", Some(super::NS_WINDOW_SHARING_NONE));
        send(
            win,
            "setCollectionBehavior:",
            Some(super::ns_collection_behavior()),
        );
        true
    }

    pub fn keep_on_top(handle: u64) -> bool {
        if !is_known(handle) {
            return false;
        }
        let win = window_for(handle);
        if win.is_null() {
            return false;
        }
        // A high level keeps it above normal windows without activating the app.
        const NSFLOATINGWINDOWLEVEL: i64 = 3;
        // SAFETY: setLevel: takes an NSInteger.
        unsafe {
            let sel = selector_bytes("setLevel:");
            let f: extern "C" fn(*mut c_void, *mut c_void, i64) =
                std::mem::transmute(objc_msgSend as *const ());
            f(
                win,
                objc_sel_registerName(sel.as_ptr()),
                NSFLOATINGWINDOWLEVEL,
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusion_is_reported_as_supported_on_desktop_platforms() {
        assert_eq!(
            supported(),
            cfg!(any(target_os = "windows", target_os = "macos"))
        );
    }

    #[test]
    fn the_windows_affinity_value_is_the_documented_one() {
        // WDA_EXCLUDEFROMCAPTURE; getting this wrong would mean the pill lands in clips.
        assert_eq!(WDA_EXCLUDEFROMCAPTURE, 0x11);
    }

    #[test]
    fn macos_sharing_none_is_zero() {
        assert_eq!(NS_WINDOW_SHARING_NONE, 0);
    }

    #[test]
    fn macos_collection_behaviour_shows_over_fullscreen_and_on_all_spaces() {
        let behavior = ns_collection_behavior();
        assert_ne!(behavior & NS_COLLECTION_CAN_JOIN_ALL_SPACES, 0);
        assert_ne!(behavior & NS_COLLECTION_FULL_SCREEN_AUXILIARY, 0);
        assert_eq!(behavior, 0x101);
    }

    #[test]
    fn a_null_handle_is_refused_rather_than_passed_to_the_os() {
        assert!(!exclude_from_capture(0));
        assert!(!keep_on_top(0));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_unregistered_handle_is_refused_without_touching_the_os() {
        // Handles come from the toolkit. Sending an Objective-C message to a stale one
        // is a segfault, not an error, so anything not registered here must be refused
        // before it reaches objc_msgSend. This test would crash if that guard regressed.
        assert!(!exclude_from_capture(0xdead_beef));
        assert!(!keep_on_top(1));
        assert!(!exclude_from_capture(0));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn registering_and_forgetting_a_window_round_trips() {
        // A handle is only accepted while registered; forgetting it refuses it again.
        register_window(0x1234_5678);
        forget_window(0x1234_5678);
        assert!(!exclude_from_capture(0x1234_5678));
    }
}
