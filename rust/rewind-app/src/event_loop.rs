//! The main thread's message pump.
//!
//! A tray icon is a window, and a window only hears about clicks while the thread that
//! created it is pumping messages. `tray-icon` states that requirement plainly: on
//! Windows "an event loop must be running on the thread". Without one the icon still
//! appears — `Shell_NotifyIcon` is called directly when it is added — but every click,
//! every menu selection, and the `TaskbarCreated` broadcast that re-adds the icon after
//! Explorer restarts is queued and never read. The menu simply never opens.
//!
//! [`pump`] drains whatever is already queued and returns; it never blocks, because the
//! app's own loop also has to poll the engine and the hotkey queue.

/// Dispatch every message already in this thread's queue.
///
/// Call it from the thread that created the tray icon, once per tick of the app loop.
/// It is cheap when the queue is empty: one `PeekMessageW` that returns immediately.
pub fn pump() {
    #[cfg(target_os = "windows")]
    win32::pump();
}

#[cfg(target_os = "windows")]
mod win32 {
    use ::windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    pub fn pump() {
        let mut msg = MSG::default();
        loop {
            // SAFETY: `msg` is a live MSG and this thread owns the queue being read.
            let pending = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() };
            if !pending {
                return;
            }
            // SAFETY: PeekMessageW just filled `msg`, which stays alive for the call.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    use ::windows::Win32::System::Threading::GetCurrentThreadId;
    use ::windows::Win32::UI::WindowsAndMessaging::{
        PeekMessageW, PostThreadMessageW, MSG, PM_REMOVE,
    };

    #[test]
    fn pump_drains_what_is_queued() {
        // A posted thread message is the same shape as the tray icon's window messages:
        // it sits in the queue until someone reads it. If `pump` did not drain it the
        // tray would look alive and be deaf, which is the bug this module exists for.
        const WM_APP: u32 = 0x8000;
        // SAFETY: posting to this thread's own queue; no pointers cross the call.
        let posted = unsafe {
            PostThreadMessageW(
                GetCurrentThreadId(),
                WM_APP,
                Default::default(),
                Default::default(),
            )
        };
        assert!(posted.is_ok(), "could not queue a message: {posted:?}");

        pump();

        let mut msg = MSG::default();
        // SAFETY: reading this thread's queue with a live MSG.
        let still_there = unsafe { PeekMessageW(&mut msg, None, WM_APP, WM_APP, PM_REMOVE) };
        assert!(
            !still_there.as_bool(),
            "pump left a queued message undispatched"
        );
    }
}
